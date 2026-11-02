//! Phones, seen from the computer. While sharing runs (as a server or as a client), the computer
//! also listens and announces itself on `port + 1`, and the wifishare Android app dials in: to
//! pair, to keep a control link, and for files connections. The phone always dials, so when the
//! window wants to browse the phone it asks over the control link for a dial-back, the same way a
//! server browses a client. Phones get no keyboard, mouse, clipboard or sound.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::{TcpListener, TcpStream};

use crate::config::{Config, Identity, NetworkCfg, Paths, Peers, Role};
use crate::discovery::{self, Beacon};
use crate::files::Files;
use crate::netwatch;
use crate::pair;
use crate::proto::{Hello, Msg, Os, PROTOCOL_VERSION};
use crate::status::{Hub, PhoneInfo};
use crate::wire::{self, Link, Purpose, Secured};

/// The TCP and UDP port phones use.
pub fn port(cfg: &Config) -> u16 {
    cfg.port.saturating_add(1)
}

struct Service {
    name: String,
    paths: Paths,
    id: Identity,
    hub: Arc<Hub>,
    files: Arc<Files>,
    network: NetworkCfg,
    /// One pairing question at a time.
    pair_lock: tokio::sync::Mutex<()>,
}

/// Accepts phones until the runtime shuts down. Failing to listen (the port is taken) only
/// costs the phone connections, so it is logged rather than returned.
pub async fn run(cfg: Config, paths: Paths, id: Identity, hub: Arc<Hub>, files: Arc<Files>) {
    if !cfg.phone.enabled {
        return;
    }
    let port = port(&cfg);
    let listener = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(e) => {
            log::warn!("phones: cannot listen on TCP port {port} ({e}); phones cannot connect");
            return;
        }
    };
    log::info!("phones: listening on port {port}");
    let beacon = Beacon { key: id.public.clone(), port, name: cfg.name.clone() };
    tokio::spawn(discovery::announce(beacon, port, netwatch::spawn()));
    let svc = Arc::new(Service { name: cfg.name, paths, id, hub, files, network: cfg.network, pair_lock: tokio::sync::Mutex::new(()) });
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                tokio::spawn(handle(svc.clone(), stream, addr));
            }
            Err(e) => {
                log::warn!("phones: accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

async fn handle(svc: Arc<Service>, stream: TcpStream, addr: SocketAddr) {
    let (purpose, mut s) = match wire::accept(stream, &svc.id, Duration::from_secs(5)).await {
        Ok(x) => x,
        Err(e) => {
            log::debug!("phones: {addr}: handshake failed: {e:#}");
            return;
        }
    };
    if purpose == Purpose::Pair {
        let desk = pair::Desk { name: svc.name.clone(), paths: &svc.paths, hub: &svc.hub, lock: &svc.pair_lock };
        if let Err(e) = pair::serve(desk, s, Role::Phone).await {
            log::warn!("pairing with the phone at {addr} failed: {e:#}");
        }
        return;
    }
    let peers = Peers::load(&svc.paths).unwrap_or_default();
    let Some(phone) = peers.by_key(&s.remote_key).filter(|p| p.role == Role::Phone).cloned() else {
        log::warn!("phones: {addr}: an unpaired phone tried to connect; pair it first");
        let _ = s.writer.send(&Msg::Bye(format!("this phone is not paired with {}; pair it first", svc.name))).await;
        return;
    };
    match purpose {
        Purpose::Control => control(svc, s, phone.name).await,
        Purpose::Files => {
            if let Err(e) = svc.files.accept(s, true).await {
                log::warn!("shared folders with {}: {e:#}", phone.name);
            }
        }
        Purpose::Bulk => log::debug!("phones: {} opened a clipboard connection; ignored", phone.name),
        Purpose::Pair => unreachable!(),
    }
}

async fn control(svc: Arc<Service>, s: Secured, name: String) {
    let addr = s.peer_addr;
    let mut link = Link::spawn(
        s.reader,
        s.writer,
        Duration::from_millis(svc.network.heartbeat_ms),
        Duration::from_millis(svc.network.timeout_ms),
        format!("phone {name} ({})", addr.ip()),
    );
    let hello = match tokio::time::timeout(Duration::from_secs(5), link.rx.recv()).await {
        Ok(Some(Msg::Hello(h))) => h,
        _ => {
            log::warn!("phone {name} ({addr}) did not introduce itself");
            return;
        }
    };
    if hello.version != PROTOCOL_VERSION {
        log::warn!("phone {name} runs protocol {} but this computer runs {PROTOCOL_VERSION}; update both", hello.version);
        let _ = link.tx.send(Msg::Bye(format!("this computer speaks protocol {PROTOCOL_VERSION}; update wifishare")));
        tokio::time::sleep(Duration::from_millis(200)).await;
        return;
    }
    let _ = link.tx.send(Msg::Hello(Hello { version: PROTOCOL_VERSION, name: svc.name.clone(), os: Os::current(), screen: None }));
    let id = svc.files.attach(&name, link.tx.clone());
    log::info!("phone {name} connected from {}", addr.ip());
    let info = PhoneInfo { link: id, name: name.clone(), addr: addr.ip(), since: Instant::now(), rtt: link.rtt.clone() };
    svc.hub.update(|s| {
        s.phones.retain(|p| p.name != name);
        s.phones.push(info);
    });
    while let Some(msg) = link.rx.recv().await {
        if let Msg::Bye(reason) = msg {
            log::info!("phone {name}: {reason}");
            break;
        }
    }
    svc.files.detach(&name, id);
    svc.hub.update(|s| s.phones.retain(|p| p.link != id));
    log::info!("phone {name} disconnected");
}
