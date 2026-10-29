//! The client: receives input from the server and injects it, keeping the link alive through
//! Wi-Fi drops, IP changes and sleep.
//!
//! Reconnecting never gives up. Every attempt dials all known addresses for the server at once
//! (configured host name re-resolved each time, last working address, latest beacons), and a
//! beacon from the server or a network change cuts any backoff short. A link is declared dead
//! after `network.timeout_ms` of silence, and whatever keys or buttons were held are released.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;

use crate::audio::LinkAudio;
use crate::clipboard;
use crate::clipnet::{self, ClientCtx, ClipNet};
use crate::config::{Config, Identity, Paths, Peer, Peers};
use crate::discovery::{self, Beacon};
use crate::files::Files;
use crate::geom::Screen;
use crate::netwatch::{self, NetState};
use crate::platform::{self, Injector};
use crate::proto::{Button, Hello, Msg, Os, PROTOCOL_VERSION};
use crate::server::describe_screen;
use crate::status::{Approval, Discovered, Hub, LinkState};
use crate::wire::{self, Link, Purpose, Secured};

const MAX_BACKOFF: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Wraps the platform injector and remembers what is held, so it can all be released when the
/// cursor leaves or the link drops (no stuck Ctrl after a Wi-Fi blip).
pub struct InjectState {
    inj: Box<dyn Injector>,
    keys: HashSet<u16>,
    buttons: HashSet<Button>,
}

impl InjectState {
    pub fn new(inj: Box<dyn Injector>) -> Self {
        InjectState { inj, keys: HashSet::new(), buttons: HashSet::new() }
    }

    pub fn set_screen(&mut self, s: &Screen) {
        self.inj.set_screen(s);
    }

    pub fn handle(&mut self, msg: &Msg) {
        match *msg {
            Msg::Enter { x, y } | Msg::Motion { x, y } => self.inj.motion(x, y),
            Msg::Leave => self.release_all(),
            Msg::Button { button, down } => {
                if down {
                    self.buttons.insert(button);
                } else {
                    self.buttons.remove(&button);
                }
                self.inj.button(button, down);
            }
            Msg::Wheel { dx, dy } => self.inj.wheel(dx, dy),
            Msg::Key { code, down, repeat } => {
                if repeat && self.inj.os_repeats_keys() {
                    return;
                }
                if down {
                    self.keys.insert(code);
                } else {
                    self.keys.remove(&code);
                }
                self.inj.key(code, down);
            }
            _ => {}
        }
    }

    pub fn release_all(&mut self) {
        for code in self.keys.drain().collect::<Vec<_>>() {
            self.inj.key(code, false);
        }
        for b in self.buttons.drain().collect::<Vec<_>>() {
            self.inj.button(b, false);
        }
    }
}

/// Logs instead of injecting (`WIFISHARE_FAKE_INPUT=1`), for tests.
struct LogInjector;

impl Injector for LogInjector {
    fn set_screen(&mut self, _screen: &Screen) {}
    fn motion(&mut self, x: f64, y: f64) {
        log::info!("inject: motion {x:.0},{y:.0}");
    }
    fn button(&mut self, button: Button, down: bool) {
        log::info!("inject: button {button:?} {}", if down { "down" } else { "up" });
    }
    fn wheel(&mut self, dx: i32, dy: i32) {
        log::info!("inject: wheel {dx},{dy}");
    }
    fn key(&mut self, code: u16, down: bool) {
        log::info!("inject: key {code} {}", if down { "down" } else { "up" });
    }
    fn os_repeats_keys(&self) -> bool {
        true
    }
}

pub fn detect_screen(cfg: &Config) -> Screen {
    if let Some(monitors) = cfg.client.screen_override() {
        return Screen { monitors };
    }
    platform::screen().unwrap_or_else(|e| {
        log::warn!("monitor detection failed ({e:#}); assuming 1920x1080 (set client.screen in the config)");
        platform::fallback_screen()
    })
}

/// Picks the paired server to follow.
fn choose_server(peers: &Peers, hint: Option<&str>) -> Result<Peer> {
    let servers: Vec<&Peer> = peers.servers().collect();
    if servers.is_empty() {
        bail!("this machine is not paired with a server yet. Start the server, then run `wifishare pair` here");
    }
    if let Some(h) = hint
        && let Some(p) = servers.iter().find(|p| p.name.eq_ignore_ascii_case(h))
    {
        return Ok((*p).clone());
    }
    Ok(servers[servers.len() - 1].clone())
}

/// The paired server and an address it answers on, for one-off commands that do not keep a link.
pub async fn locate(cfg: &Config, paths: &Paths, id: &Identity) -> Result<(Peer, SocketAddr)> {
    let server = choose_server(&Peers::load(paths)?, cfg.client.server.as_deref())?;
    let last = server.last_addr.as_deref().and_then(|a| a.parse().ok());
    let candidates = candidates(cfg.client.server.as_deref(), cfg.port, last, &HashMap::new()).await;
    if candidates.is_empty() {
        bail!("no address known for {}; set client.server in the config", server.name);
    }
    let key = hex::decode(&server.key)?;
    let mut s = connect_any(&candidates, id, &key, Purpose::Files).await.with_context(|| format!("cannot reach {}", server.name))?;
    let addr = s.peer_addr;
    s.writer.send(&crate::proto::FsOpen { token: None }).await?;
    Ok((server, addr))
}

/// Follows the paired server until `stop` fires (or its sender is dropped).
pub async fn run(
    cfg: Config,
    paths: Paths,
    id: Identity,
    server_arg: Option<String>,
    hub: Arc<Hub>,
    stop: oneshot::Receiver<()>,
) -> Result<()> {
    let result = tokio::select! {
        r = follow(cfg, paths, id, server_arg, hub.clone()) => r,
        _ = stop => Ok(()),
    };
    hub.update(|s| s.link = LinkState::Idle);
    result
}

async fn follow(cfg: Config, paths: Paths, id: Identity, server_arg: Option<String>, hub: Arc<Hub>) -> Result<()> {
    let problems = platform::permission_problems(false);
    for p in &problems {
        log::warn!("{p}");
    }
    hub.update(|s| {
        s.running = true;
        s.warnings = problems;
    });
    let net = netwatch::spawn();
    let (found_tx, mut found_rx) = mpsc::unbounded_channel();
    let (probing_tx, probing_rx) = watch::channel(true);
    tokio::spawn(discovery::listen(cfg.port, found_tx, probing_rx, net.clone()));

    // Without a paired server the window waits (listing servers it hears) until one is paired.
    let host = server_arg.or_else(|| cfg.client.server.clone());
    let server = loop {
        match choose_server(&Peers::load(&paths)?, host.as_deref()) {
            Ok(s) => break s,
            Err(e) if hub.approval == Approval::Terminal => return Err(e),
            Err(_) => hub.update(|s| s.link = LinkState::Unpaired),
        }
        let deadline = tokio::time::sleep(Duration::from_secs(1));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                f = found_rx.recv() => if let Some((b, addr)) = f { heard(&hub, &b, addr) },
            }
        }
    };
    let server_key = hex::decode(&server.key)?;
    log::info!("following server '{}' (key {})", server.name, crate::config::fingerprint(&server_key));

    let injector: Box<dyn Injector> = if std::env::var_os("WIFISHARE_FAKE_INPUT").is_some() {
        Box::new(LogInjector)
    } else {
        platform::injector(cfg.client.scroll_speed.unwrap_or(1.0)).context("setting up input injection")?
    };
    let inject = Arc::new(Mutex::new(InjectState::new(injector)));
    let mut screen = detect_screen(&cfg);
    inject.lock().unwrap().set_screen(&screen);
    log::info!("this desktop: {}", describe_screen(&screen));
    hub.update(|s| s.local_screen = Some(screen.clone()));

    let clip = cfg.clipboard.enabled.then(|| {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = clipboard::start(cfg.clipboard.clone(), tx);
        let ctx = ClientCtx { identity: id.clone(), server_key: server_key.clone(), link: Mutex::new(None) };
        let net = ClipNet::new(cfg.clipboard.clone(), paths.incoming_dir(), handle, clipnet::Role::Client(ctx), hub.clone());
        tokio::spawn(net.clone().run_local(rx));
        net
    });

    let files = Files::client(paths.clone(), id.clone(), hub.clone(), server_key.clone());
    hub.set_files(Some(files.clone()));

    let mut beacons: HashMap<SocketAddr, Instant> = HashMap::new();
    let mut last_addr: Option<SocketAddr> = server.last_addr.as_deref().and_then(|a| a.parse().ok());
    let mut backoff = Duration::from_millis(250);
    let mut failing_since = Instant::now();
    let mut last_error: Option<String> = None;
    let mut last_report = Instant::now() - Duration::from_secs(3600);
    let mut net_rx = net.clone();

    loop {
        // Drain beacons heard meanwhile; only ones carrying our server's key count.
        while let Ok((b, addr)) = found_rx.try_recv() {
            heard(&hub, &b, addr);
            note_beacon(&mut beacons, &b, addr, &server_key);
        }
        beacons.retain(|_, seen| seen.elapsed() < Duration::from_secs(30));
        let error = last_error.clone();
        hub.update(|s| s.link = LinkState::Searching { server: server.name.clone(), since: failing_since, error });

        let candidates = candidates(host.as_deref(), cfg.port, last_addr, &beacons).await;
        let attempt = if candidates.is_empty() {
            Err(anyhow::anyhow!("no address known yet"))
        } else {
            connect_any(&candidates, &id, &server_key, Purpose::Control).await
        };
        match attempt {
            Ok(s) => {
                let _ = probing_tx.send(false);
                let addr = s.peer_addr;
                if last_error.take().is_some() {
                    log::info!("reconnected after {:.1}s", failing_since.elapsed().as_secs_f64());
                }
                if last_addr != Some(addr) {
                    last_addr = Some(addr);
                    remember_addr(&paths, &server.key, addr);
                }
                let started = Instant::now();
                session(&cfg, &paths, s, &inject, &mut screen, (clip.as_ref(), &files), &mut net_rx, &mut found_rx, &hub).await;
                inject.lock().unwrap().release_all();
                if let Some(c) = clip.as_ref().and_then(|c| c.client_ctx()) {
                    *c.link.lock().unwrap() = None;
                }
                let _ = probing_tx.send(true);
                failing_since = Instant::now();
                last_error = Some("the link dropped".into());
                // A link that died right after connecting should not be hammered.
                backoff = if started.elapsed() < Duration::from_secs(2) { MAX_BACKOFF } else { Duration::from_millis(100) };
            }
            Err(e) => {
                if last_report.elapsed() > Duration::from_secs(30) {
                    log::info!("server not reachable yet ({e:#}); retrying (down {:.0}s)", failing_since.elapsed().as_secs_f64());
                    last_report = Instant::now();
                }
                last_error = Some(format!("{e:#}"));
            }
        }

        // Wait out the backoff, unless the server announces itself or the network changes.
        let deadline = tokio::time::sleep(backoff);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                f = found_rx.recv() => {
                    let Some((b, addr)) = f else { break };
                    heard(&hub, &b, addr);
                    if note_beacon(&mut beacons, &b, addr, &server_key) {
                        break;
                    }
                }
                r = net_rx.changed() => {
                    if r.is_ok() { break; }
                }
            }
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Lists a server heard on the LAN (for the window); entries not heard for 10 s drop out.
fn heard(hub: &Hub, b: &Beacon, addr: SocketAddr) {
    hub.update(|s| {
        s.discovered.retain(|d| d.key != b.key && d.seen.elapsed() < Duration::from_secs(10));
        s.discovered.push(Discovered { name: b.name.clone(), addr, key: b.key.clone(), seen: Instant::now() });
        s.discovered.sort_by(|a, b| a.name.cmp(&b.name));
    });
}

/// Records a beacon from our server; true if it is news (an address not seen recently).
fn note_beacon(beacons: &mut HashMap<SocketAddr, Instant>, b: &Beacon, addr: SocketAddr, key: &[u8]) -> bool {
    if b.key != key {
        return false;
    }
    let fresh = beacons.get(&addr).is_none_or(|t| t.elapsed() > Duration::from_secs(5));
    beacons.insert(addr, Instant::now());
    fresh
}

async fn candidates(host: Option<&str>, port: u16, last: Option<SocketAddr>, beacons: &HashMap<SocketAddr, Instant>) -> Vec<SocketAddr> {
    let mut out: Vec<SocketAddr> = Vec::new();
    let mut by_recency: Vec<_> = beacons.iter().collect();
    by_recency.sort_by_key(|(_, t)| std::cmp::Reverse(**t));
    out.extend(by_recency.into_iter().map(|(a, _)| *a));
    if let Some(h) = host {
        // Re-resolved every time: the server's address may have changed since the last attempt.
        let target = if h.contains(':') && !h.starts_with('[') && h.parse::<std::net::Ipv6Addr>().is_err() {
            h.to_string()
        } else {
            format!("{h}:{port}")
        };
        if let Ok(Ok(addrs)) = tokio::time::timeout(Duration::from_secs(2), tokio::net::lookup_host(target)).await {
            out.extend(addrs.filter(|a| a.is_ipv4()));
        }
    }
    out.extend(last);
    let mut seen = HashSet::new();
    out.retain(|a| seen.insert(*a));
    out
}

/// Dials every candidate at once and keeps the first that turns out to be our server.
async fn connect_any(candidates: &[SocketAddr], id: &Identity, key: &[u8], purpose: Purpose) -> Result<Secured> {
    let mut set = JoinSet::new();
    for &addr in candidates {
        let id = id.clone();
        set.spawn(async move { wire::connect(addr, &id, purpose, CONNECT_TIMEOUT).await });
    }
    let mut last_err = anyhow::anyhow!("no candidates");
    while let Some(r) = set.join_next().await {
        match r {
            Ok(Ok(s)) if s.remote_key == key => return Ok(s),
            Ok(Ok(s)) => last_err = anyhow::anyhow!("{} is a different wifishare machine", s.peer_addr),
            Ok(Err(e)) => last_err = e,
            Err(e) => last_err = e.into(),
        }
    }
    Err(last_err)
}

fn remember_addr(paths: &Paths, key: &str, addr: SocketAddr) {
    let Ok(mut peers) = Peers::load(paths) else { return };
    if let Some(p) = peers.peers.iter_mut().find(|p| p.key == key) {
        p.last_addr = Some(addr.to_string());
        let _ = peers.save(paths);
    }
}

#[allow(clippy::too_many_arguments)]
async fn session(
    cfg: &Config,
    paths: &Paths,
    s: Secured,
    inject: &Arc<Mutex<InjectState>>,
    screen: &mut Screen,
    (clip, files): (Option<&Arc<ClipNet>>, &Arc<Files>),
    net: &mut watch::Receiver<NetState>,
    found: &mut mpsc::UnboundedReceiver<(Beacon, SocketAddr)>,
    hub: &Arc<Hub>,
) {
    let (addr, local) = (s.peer_addr, s.local_addr);
    let mut link = Link::spawn(
        s.reader,
        s.writer,
        Duration::from_millis(cfg.network.heartbeat_ms),
        Duration::from_millis(cfg.network.timeout_ms),
        format!("server {}", addr.ip()),
    );
    let _ = link.tx.send(Msg::Hello(Hello {
        version: PROTOCOL_VERSION,
        name: cfg.name.clone(),
        os: Os::current(),
        screen: Some(screen.clone()),
    }));
    let hello = match tokio::time::timeout(Duration::from_secs(5), link.rx.recv()).await {
        Ok(Some(Msg::Hello(h))) => h,
        Ok(Some(Msg::Bye(reason))) => {
            log::warn!("server refused the link: {reason}");
            return;
        }
        _ => {
            log::warn!("server at {addr} did not answer");
            return;
        }
    };
    if hello.version != PROTOCOL_VERSION {
        log::warn!("server runs protocol {} but this client runs {PROTOCOL_VERSION}; update both", hello.version);
        return;
    }
    log::info!("connected to {} at {addr}", hello.name);
    hub.update(|s| s.link = LinkState::Connected { server: hello.name.clone(), addr, since: Instant::now(), rtt: link.rtt.clone() });
    if let Some(c) = clip {
        if let Some(ctx) = c.client_ctx() {
            *ctx.link.lock().unwrap() = Some((link.tx.clone(), addr));
        }
        c.on_link_up(&link.tx);
    }
    files.set_link(Some(addr));
    let mut audio_cfg = cfg.audio.clone();
    let mut audio = LinkAudio::new(&hello.name, addr.ip(), hub.clone(), paths.cache_dir.clone());
    audio.apply(&audio_cfg, &link.tx, false);
    let mut reload = hub.reload.subscribe();

    let mut screen_tick = tokio::time::interval(Duration::from_secs(3));
    screen_tick.tick().await;
    loop {
        tokio::select! {
            r = reload.changed() => {
                if r.is_ok() && let Ok(fresh) = Config::load(paths) {
                    audio_cfg = fresh.audio;
                    audio.apply(&audio_cfg, &link.tx, true);
                }
            }
            msg = link.rx.recv() => {
                let Some(msg) = msg else { break };
                if audio.handle(&msg, &audio_cfg, &link.tx) {
                    continue;
                }
                match msg {
                    Msg::Clip(c) => {
                        if let Some(clip) = clip {
                            clip.on_message(c, None);
                        }
                    }
                    Msg::FilesDial { token } => files.on_dial(token),
                    Msg::Bye(reason) => {
                        log::info!("server closed the link: {reason}");
                        break;
                    }
                    m => inject.lock().unwrap().handle(&m),
                }
            }
            f = found.recv() => {
                if let Some((b, addr)) = f {
                    heard(hub, &b, addr);
                }
            }
            _ = screen_tick.tick() => {
                let now = detect_screen(cfg);
                if now != *screen {
                    log::info!("this desktop changed: {}", describe_screen(&now));
                    inject.lock().unwrap().set_screen(&now);
                    let _ = link.tx.send(Msg::Screen(now.clone()));
                    hub.update(|s| s.local_screen = Some(now.clone()));
                    *screen = now;
                }
            }
            r = net.changed() => {
                if r.is_err() { continue; }
                // If the address this link runs over is gone, the link is dead even if TCP
                // has not noticed yet.
                let gone = match local.ip() {
                    std::net::IpAddr::V4(ip) => !ip.is_loopback() && !net.borrow().addrs.contains(&ip),
                    _ => false,
                };
                if gone {
                    log::info!("local address {} went away; reconnecting", local.ip());
                    break;
                }
            }
        }
    }
    files.set_link(None);
    log::info!("disconnected from {}", hello.name);
}
