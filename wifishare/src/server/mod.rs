//! The server: owns the keyboard and mouse, accepts client links, announces itself on the LAN.

pub mod core;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use self::core::{ClientSettings, ServerCore};
use crate::audio::LinkAudio;
use crate::clipboard;
use crate::clipnet::{self, ClipNet};
use crate::config::{Config, Identity, Paths, Peers, Role};
use crate::discovery::{self, Beacon};
use crate::files::Files;
use crate::geom::Side;
use crate::netwatch;
use crate::pair;
use crate::platform;
use crate::proto::{Hello, Msg, Os, PROTOCOL_VERSION};
use crate::status::{ClientInfo, Hub};
use crate::wire::{self, Link, Purpose, Secured};

pub struct Shared {
    /// Re-read when the hub signals a reload, so layout edits apply to connected clients.
    pub cfg: Mutex<Config>,
    pub paths: Paths,
    pub id: Identity,
    pub core: Arc<Mutex<ServerCore>>,
    pub clip: Option<Arc<ClipNet>>,
    pub files: Arc<Files>,
    pub hub: Arc<Hub>,
    /// One pairing question at a time.
    pub pair_lock: tokio::sync::Mutex<()>,
    side_override: Option<Side>,
}

impl Shared {
    pub fn name(&self) -> String {
        self.cfg.lock().unwrap().name.clone()
    }
}

pub struct Options {
    /// Overrides `server.default_side`.
    pub side: Option<Side>,
    /// Do not capture input (clipboard sharing only).
    pub no_input: bool,
}

/// Runs until `stop` fires (or its sender is dropped), then hands the cursor back and returns.
pub async fn run(cfg: Config, paths: Paths, id: Identity, opts: Options, hub: Arc<Hub>, stop: oneshot::Receiver<()>) -> Result<()> {
    let problems = platform::permission_problems(true);
    for p in &problems {
        log::warn!("{p}");
    }
    let screen = platform::screen().unwrap_or_else(|e| {
        log::warn!("monitor detection failed ({e:#}); assuming 1920x1080");
        platform::fallback_screen()
    });
    log::info!("this desktop: {}", describe_screen(&screen));
    let core = Arc::new(Mutex::new(ServerCore::new(screen.clone())));
    let mut capturing = false;
    let mut warnings = problems;
    if !opts.no_input {
        match platform::start_capture(core.clone()) {
            Ok(ctl) => {
                core.lock().unwrap().set_ctl(ctl);
                capturing = true;
            }
            Err(e) => {
                log::warn!("{e:#}\nrunning without keyboard/mouse sharing (clipboard only)");
                let e = format!("{e:#}");
                if !warnings.contains(&e) {
                    warnings.push(e);
                }
            }
        }
    }
    {
        let hub = hub.clone();
        core.lock().unwrap().set_active_observer(Box::new(move |name| {
            let name = name.map(str::to_string);
            hub.update(|s| s.active = name);
        }));
    }
    hub.update(|s| {
        s.running = true;
        s.capturing = capturing;
        s.warnings = warnings;
        s.local_screen = Some(screen);
    });
    tokio::spawn(watch_local_screen(core.clone(), hub.clone()));

    let net = netwatch::spawn();
    let beacon = Beacon { key: id.public.clone(), port: cfg.port, name: cfg.name.clone() };
    tokio::spawn(discovery::announce(beacon, cfg.port, net.clone()));

    let clip = cfg.clipboard.enabled.then(|| {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = clipboard::start(cfg.clipboard.clone(), tx);
        let net = ClipNet::new(cfg.clipboard.clone(), paths.incoming_dir(), handle, clipnet::Role::Server(core.clone()), hub.clone());
        tokio::spawn(net.clone().run_local(rx));
        net
    });

    let listener = TcpListener::bind(("0.0.0.0", cfg.port)).await.with_context(|| format!("listening on TCP port {}", cfg.port))?;
    log::info!("server '{}' listening on port {} (key {})", cfg.name, cfg.port, id.fingerprint());
    if Peers::load(&paths)?.peers.iter().all(|p| p.role != Role::Client) {
        log::info!("no clients paired yet: run `wifishare pair` on the other machine");
    }

    let files = Files::server(paths.clone(), id.clone(), hub.clone(), core.clone());
    hub.set_files(Some(files.clone()));
    let shared = Arc::new(Shared {
        cfg: Mutex::new(cfg),
        paths,
        id,
        core: core.clone(),
        clip,
        files,
        hub: hub.clone(),
        pair_lock: tokio::sync::Mutex::new(()),
        side_override: opts.side,
    });
    tokio::spawn(apply_reloads(shared.clone()));
    tokio::pin!(stop);
    loop {
        tokio::select! {
            r = listener.accept() => match r {
                Ok((stream, addr)) => { tokio::spawn(handle(shared.clone(), stream, addr)); }
                Err(e) => {
                    log::warn!("accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            },
            _ = &mut stop => {
                core.lock().unwrap().shutdown();
                log::info!("server stopped");
                return Ok(());
            }
        }
    }
}

pub fn describe_screen(s: &crate::geom::Screen) -> String {
    let b = s.bbox();
    format!("{} monitor(s), {}x{} at {},{}", s.monitors.len(), b.w, b.h, b.x, b.y)
}

/// Follows monitor plug/unplug and resolution changes.
async fn watch_local_screen(core: Arc<Mutex<ServerCore>>, hub: Arc<Hub>) {
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let Ok(s) = tokio::task::spawn_blocking(platform::screen).await else { continue };
        let Ok(s) = s else { continue };
        let mut core = core.lock().unwrap();
        if *core.local_screen() != s {
            log::info!("this desktop changed: {}", describe_screen(&s));
            core.set_local_screen(s.clone());
            drop(core);
            hub.update(|st| st.local_screen = Some(s));
        }
    }
}

/// Re-reads the config when asked and applies layout changes to connected clients.
async fn apply_reloads(sh: Arc<Shared>) {
    let mut reload = sh.hub.reload.subscribe();
    loop {
        if reload.changed().await.is_err() {
            return;
        }
        let fresh = match Config::load(&sh.paths) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("not applying the edited config: {e:#}");
                continue;
            }
        };
        let clients: Vec<(String, Os)> = sh.hub.snapshot().clients.iter().map(|c| (c.name.clone(), c.os)).collect();
        let settings: Vec<(String, ClientSettings)> =
            clients.iter().map(|(n, os)| (n.clone(), settings_for(&fresh, sh.side_override, n, *os))).collect();
        {
            let mut core = sh.core.lock().unwrap();
            for (name, s) in &settings {
                core.set_settings(name, s.clone());
            }
        }
        sh.hub.update(|st| {
            for c in &mut st.clients {
                if let Some((_, s)) = settings.iter().find(|(n, _)| *n == c.name) {
                    c.side = s.side;
                }
            }
        });
        *sh.cfg.lock().unwrap() = fresh;
        log::info!("layout updated");
    }
}

async fn handle(sh: Arc<Shared>, stream: TcpStream, addr: SocketAddr) {
    let (purpose, mut s) = match wire::accept(stream, &sh.id, Duration::from_secs(5)).await {
        Ok(x) => x,
        Err(e) => {
            log::debug!("{addr}: handshake failed: {e:#}");
            return;
        }
    };
    if purpose == Purpose::Pair {
        if let Err(e) = pair::serve(&sh, s).await {
            log::warn!("pairing with {addr} failed: {e:#}");
        }
        return;
    }
    let peers = Peers::load(&sh.paths).unwrap_or_default();
    let Some(peer) = peers.by_key(&s.remote_key).filter(|p| p.role == Role::Client).cloned() else {
        log::warn!("{addr}: an unpaired machine tried to connect; pair it first");
        let _ = s.writer.send(&Msg::Bye("this machine is not paired with the server; pair it first".into())).await;
        return;
    };
    match purpose {
        Purpose::Control => control(sh, s, peer.name).await,
        Purpose::Bulk => {
            if let Some(clip) = sh.clip.clone()
                && let Err(e) = clip.serve_bulk(s).await
            {
                log::warn!("clipboard transfer with {}: {e:#}", peer.name);
            }
        }
        Purpose::Files => {
            if let Err(e) = sh.files.accept(s).await {
                log::warn!("shared folders with {}: {e:#}", peer.name);
            }
        }
        Purpose::Pair => unreachable!(),
    }
}

fn settings_for(cfg: &Config, side_override: Option<Side>, name: &str, os: Os) -> ClientSettings {
    let layout = cfg.layout_for(name);
    let mixed = (Os::current() == Os::MacOs) != (os == Os::MacOs);
    ClientSettings {
        side: layout.map(|l| l.side).or(side_override).unwrap_or(cfg.server.default_side),
        speed: layout.map(|l| l.speed).unwrap_or(1.0),
        swap_cmd_ctrl: layout.and_then(|l| l.swap_cmd_ctrl).unwrap_or(cfg.server.swap_cmd_ctrl && mixed),
    }
}

async fn control(sh: Arc<Shared>, s: Secured, paired_name: String) {
    let addr = s.peer_addr;
    let (net, my_name) = {
        let cfg = sh.cfg.lock().unwrap();
        (cfg.network.clone(), cfg.name.clone())
    };
    let mut link = Link::spawn(
        s.reader,
        s.writer,
        Duration::from_millis(net.heartbeat_ms),
        Duration::from_millis(net.timeout_ms),
        format!("{paired_name} ({})", addr.ip()),
    );
    let hello = match tokio::time::timeout(Duration::from_secs(5), link.rx.recv()).await {
        Ok(Some(Msg::Hello(h))) => h,
        _ => {
            log::warn!("{paired_name} ({addr}) did not introduce itself");
            return;
        }
    };
    if hello.version != PROTOCOL_VERSION {
        log::warn!("{} runs protocol {} but this server runs {PROTOCOL_VERSION}; update both", hello.name, hello.version);
        let _ = link.tx.send(Msg::Bye(format!("server speaks protocol {PROTOCOL_VERSION}; update wifishare")));
        tokio::time::sleep(Duration::from_millis(200)).await;
        return;
    }
    let _ = link.tx.send(Msg::Hello(Hello { version: PROTOCOL_VERSION, name: my_name, os: Os::current(), screen: None }));
    let name = hello.name;
    let settings = settings_for(&sh.cfg.lock().unwrap(), sh.side_override, &name, hello.os);
    let screen = hello.screen.unwrap_or_else(platform::fallback_screen);
    log::info!(
        "{name} connected from {} ({:?}, {}), {} edge{}",
        addr.ip(),
        hello.os,
        describe_screen(&screen),
        settings.side,
        if settings.swap_cmd_ctrl { ", Cmd/Ctrl swapped" } else { "" }
    );
    let side = settings.side;
    let conn = sh.core.lock().unwrap().attach(&name, screen.clone(), settings, link.tx.clone());
    let info =
        ClientInfo { conn, name: name.clone(), os: hello.os, addr: addr.ip(), screen, side, since: Instant::now(), rtt: link.rtt.clone() };
    sh.hub.update(|s| {
        s.clients.retain(|c| c.name != name);
        s.clients.push(info);
    });
    if let Some(clip) = &sh.clip {
        clip.on_link_up(&link.tx);
    }
    let mut audio_cfg = sh.cfg.lock().unwrap().audio.clone();
    let mut audio = LinkAudio::new(&name, addr.ip(), sh.hub.clone(), sh.paths.cache_dir.clone());
    audio.apply(&audio_cfg, &link.tx, false);
    let mut reload = sh.hub.reload.subscribe();
    loop {
        let msg = tokio::select! {
            m = link.rx.recv() => match m {
                Some(m) => m,
                None => break,
            },
            r = reload.changed() => {
                if r.is_ok() && let Ok(fresh) = Config::load(&sh.paths) {
                    audio_cfg = fresh.audio;
                    audio.apply(&audio_cfg, &link.tx, true);
                }
                continue;
            }
        };
        if audio.handle(&msg, &audio_cfg, &link.tx) {
            continue;
        }
        match msg {
            Msg::Screen(s) => {
                log::info!("{name}: desktop changed: {}", describe_screen(&s));
                sh.core.lock().unwrap().update_screen(conn, s.clone());
                sh.hub.update(|st| {
                    if let Some(c) = st.clients.iter_mut().find(|c| c.conn == conn) {
                        c.screen = s;
                    }
                });
            }
            Msg::Clip(c) => {
                if let Some(clip) = &sh.clip {
                    clip.on_message(c, Some(conn));
                }
            }
            Msg::Bye(reason) => {
                log::info!("{name}: {reason}");
                break;
            }
            _ => {}
        }
    }
    sh.core.lock().unwrap().detach(conn);
    sh.hub.update(|s| s.clients.retain(|c| c.conn != conn));
    log::info!("{name} disconnected");
}
