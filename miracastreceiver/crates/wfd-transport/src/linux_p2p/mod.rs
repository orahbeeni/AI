//! Native Wi-Fi Direct Miracast sink on Linux through wpa_supplicant's D-Bus
//! API. NetworkManager keeps running and keeps the normal Wi-Fi connection:
//! we only use the dedicated P2P device (`p2p-dev-<wifi>`), set our WFD IEs
//! and device info on it, and run the P2P group link ourselves.
//!
//! When NetworkManager manages the P2P device (the usual desktop case) steps
//! 2-3 go through it instead, see [`nm`]. The native flow below is for systems
//! without it.
//!
//! Flow:
//! 1. WFD IEs (primary sink, RTSP 7236, UIBC) + device type "Display/TV" +
//!    optional MS-MICE vendor extension; `Find` keeps us discoverable.
//! 2. Source asks (GO negotiation request or provision discovery) -> the app
//!    shows Allow/Deny (PBC) or a PIN -> `Connect` (we ask to be GO).
//! 3. `GroupStarted`: as GO we put 192.168.173.1/24 on the group interface
//!    and serve DHCP; the lease gives the source's IP. As client we ask the
//!    GO for an address instead.
//! 4. `PeerConnected { peer_ip, rtsp_port }` -> the app runs RTSP.
//! 5. Session end -> remove the group -> discoverable again.
//!
//! Needs: D-Bus access to wpa_supplicant (netdev group or a policy file) and,
//! without NetworkManager, CAP_NET_ADMIN + CAP_NET_BIND_SERVICE + CAP_NET_RAW
//! (see scripts/setup-linux.sh).

pub mod netlink;
pub mod nm;
pub mod wpa;

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use self::nm::{ActiveState, Nm};
use self::wpa::{IFACE_P2P, PeerInfo, Wpa};
use crate::dhcp::{self, DhcpServer};
use crate::{
    BoxFuture, DiscoveryTransport, PairingMethod, PairingReply, PairingRequest, PeerId, ReceiverIdentity, TransportCommand, TransportError,
    TransportEvent, TransportKind, TransportState, mice, net,
};

const KIND: TransportKind = TransportKind::WifiDirect;
/// WPS primary device type: category 7 (Displays), WFA OUI, sub-category 1 (TV).
const DEVICE_TYPE_DISPLAY: [u8; 8] = [0x00, 0x07, 0x00, 0x50, 0xF2, 0x04, 0x00, 0x01];
const APPROVAL_VALID: Duration = Duration::from_secs(120);
/// Give up on a group that hasn't started this long after `Connect`.
const FORMATION_TIMEOUT: Duration = Duration::from_secs(90);
/// A source repeats its request while we are already connecting to it.
const REPEAT_QUIET: Duration = Duration::from_secs(30);
/// wpa_supplicant's own keywords; it silently ignores unknown ones such as
/// "pbc". Push button only, like a TV: offering a PIN method as well makes
/// Samsung phones ask for a PIN instead of connecting with one tap.
const CONFIG_METHODS: &str = "virtual_push_button";

#[derive(Debug, Clone)]
pub struct P2pOptions {
    /// Our GO intent (15 = always ask to be group owner, which Android
    /// sources expect: they connect with intent 0).
    pub go_intent: i32,
    /// Subnet for the group when we are GO (first free of these is used).
    pub go_subnets: Vec<Ipv4Addr>,
    /// Tests only: don't check capabilities or configure addresses.
    #[doc(hidden)]
    pub skip_netconfig: bool,
}

impl Default for P2pOptions {
    fn default() -> Self {
        Self {
            go_intent: 15,
            go_subnets: vec![Ipv4Addr::new(192, 168, 173, 1), Ipv4Addr::new(192, 168, 174, 1), Ipv4Addr::new(10, 74, 73, 1)],
            skip_netconfig: false,
        }
    }
}

pub struct LinuxP2p {
    identity: ReceiverIdentity,
    opts: P2pOptions,
}

impl LinuxP2p {
    pub fn new(identity: ReceiverIdentity, opts: P2pOptions) -> Self {
        Self { identity, opts }
    }
}

impl DiscoveryTransport for LinuxP2p {
    fn kind(&self) -> TransportKind {
        KIND
    }

    fn run(
        self: Box<Self>,
        events: mpsc::UnboundedSender<TransportEvent>,
        commands: mpsc::UnboundedReceiver<TransportCommand>,
    ) -> BoxFuture<'static, Result<(), TransportError>> {
        Box::pin(async move {
            let events2 = events.clone();
            let r = run(*self, events, commands).await;
            if let Err(e) = &r {
                let _ = events2.send(TransportEvent::Status { kind: KIND, state: TransportState::Unavailable { reason: e.to_string() } });
            }
            r
        })
    }
}

fn status(events: &mpsc::UnboundedSender<TransportEvent>, state: TransportState) {
    let _ = events.send(TransportEvent::Status { kind: KIND, state });
}

fn mac_string(m: &[u8; 6]) -> String {
    dhcp::format_mac(m)
}

/// 8-digit WPS PIN with a valid checksum digit.
pub fn generate_wps_pin() -> String {
    let b = uuid::Uuid::new_v4().into_bytes();
    let n = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) % 10_000_000;
    format!("{n:07}{}", wps_checksum(n))
}

pub fn wps_checksum(mut pin: u32) -> u32 {
    let mut acc = 0;
    while pin != 0 {
        acc += 3 * (pin % 10);
        pin /= 10;
        acc += pin % 10;
        pin /= 10;
    }
    (10 - acc % 10) % 10
}

/// How the peer wants to authenticate, from the WPS Device Password ID it
/// sent: 4 = push button, 5 = it displays a PIN (we type it), 1 = it types a
/// PIN (we display it).
fn method_for_password_id(id: u16) -> (PairingMethod, &'static str) {
    match id {
        4 => (PairingMethod::PushButton, "pbc"),
        5 => (PairingMethod::EnterPin, "keypad"),
        _ => (PairingMethod::DisplayPin(generate_wps_pin()), "display"),
    }
}

#[derive(Debug, Clone)]
struct Approval {
    method: &'static str,
    pin: Option<String>,
    at: Instant,
}

enum Internal {
    Decision {
        peer_path: String,
        reply: PairingReply,
        method: &'static str,
        pin: Option<String>,
        peer_intent: u8,
        authorize_only: bool,
    },
    GroupReady {
        peer_ip: IpAddr,
    },
    GroupFailed {
        reason: String,
    },
    /// A NetworkManager activation is over.
    NmEnded {
        active: OwnedObjectPath,
    },
}

struct Group {
    iface_path: String,
    ifname: String,
    peer: Option<(String, PeerInfo)>,
    task: Option<JoinHandle<()>>,
    connected: bool,
}

struct Ctx {
    wpa: Wpa,
    dev: String,
    our_paths: HashSet<String>,
    identity: ReceiverIdentity,
    opts: P2pOptions,
    events: mpsc::UnboundedSender<TransportEvent>,
    internal: mpsc::UnboundedSender<Internal>,
    approvals: HashMap<String, Approval>,
    prompting: HashSet<String>,
    /// GO negotiation requests (password id, intent) that arrived while the
    /// user was still being asked.
    waiting_negotiations: HashMap<String, (u16, u8)>,
    /// A group is being formed: when `Connect` was called, and whether it is
    /// waiting for the source to start the negotiation.
    forming: Option<(Instant, bool)>,
    peers: HashMap<String, PeerInfo>,
    current_peer: Option<String>,
    group: Option<Group>,
    finding: bool,
    /// Set when NetworkManager manages the P2P device.
    nm: Option<Nm>,
    /// Our current NetworkManager activation and the task watching it.
    nm_active: Option<(OwnedObjectPath, JoinHandle<()>)>,
}

async fn run(
    t: LinuxP2p,
    events: mpsc::UnboundedSender<TransportEvent>,
    mut commands: mpsc::UnboundedReceiver<TransportCommand>,
) -> Result<(), TransportError> {
    status(&events, TransportState::Starting);
    let wpa = Wpa::connect().await.map_err(|e| TransportError::Unavailable(format!("cannot reach the system D-Bus: {e}")))?;
    let caps = wpa.capabilities().await.map_err(|e| {
        if e.access_denied {
            TransportError::Unavailable(
                "Not allowed to talk to wpa_supplicant over D-Bus. Run scripts/setup-linux.sh once (it adds a D-Bus policy for your user)."
                    .into(),
            )
        } else {
            TransportError::Unavailable(format!("wpa_supplicant is not running or has no D-Bus interface ({e})"))
        }
    })?;
    if !caps.iter().any(|c| c == "p2p") {
        return Err(TransportError::Unavailable("this wpa_supplicant was built without Wi-Fi Direct (P2P) support".into()));
    }
    let ifaces = wpa.interfaces().await.map_err(|e| TransportError::Unavailable(e.to_string()))?;
    let p2p_dev = ifaces.iter().find(|(_, n)| n.starts_with("p2p-dev-")).cloned();
    let parent = match &p2p_dev {
        Some((_, n)) => ifaces.iter().find(|(_, m)| Some(m.as_str()) == n.strip_prefix("p2p-dev-")).cloned(),
        None => ifaces.iter().find(|(_, n)| !n.starts_with("p2p-")).cloned(),
    };
    let Some((dev_path, dev_name)) = p2p_dev.clone().or_else(|| parent.clone()) else {
        return Err(TransportError::Unavailable("no Wi-Fi interface is managed by wpa_supplicant".into()));
    };
    let mut our_paths: HashSet<String> = HashSet::new();
    our_paths.insert(dev_path.to_string());
    if let Some((p, _)) = &parent {
        our_paths.insert(p.to_string());
    }
    let wifi_name = parent.as_ref().map_or(dev_name.as_str(), |(_, n)| n.as_str());
    let nm = Nm::detect(&wpa.conn, wifi_name).await;
    let missing = if t.opts.skip_netconfig || nm.is_some() { Vec::new() } else { netlink::missing_caps() };
    if !missing.is_empty() {
        return Err(TransportError::Unavailable(format!(
            "Wi-Fi Direct needs network capabilities this program lacks ({}). Run scripts/setup-linux.sh, or: sudo setcap cap_net_admin,cap_net_bind_service,cap_net_raw+ep <binary>",
            missing.join(", ")
        )));
    }
    let dev = dev_path.to_string();
    info!(device = %dev_name, path = %dev, network_manager = nm.is_some(), "using wpa_supplicant P2P device");

    // remember what we change so we can put it back
    let saved_ies = wpa.wfd_ies().await.unwrap_or_default();
    let saved_cfg = wpa.p2p_config(&dev).await.unwrap_or_default();
    let saved_methods = wpa.config_methods(&dev).await.ok();

    let apply = async {
        wpa.set_wfd_ies(&wfd_core::ie::sink_subelements(wfd_core::ie::DEFAULT_RTSP_PORT, t.identity.uibc)).await?;
        let mut cfg: HashMap<&str, Value<'_>> = HashMap::new();
        cfg.insert("DeviceName", Value::from(t.identity.friendly_name.clone()));
        cfg.insert("PrimaryDeviceType", Value::from(DEVICE_TYPE_DISPLAY.to_vec()));
        cfg.insert("GOIntent", Value::from(t.opts.go_intent.clamp(0, 15) as u32));
        cfg.insert("PersistentReconnect", Value::from(false));
        let mut vendor: Vec<Vec<u8>> = Vec::new();
        if t.identity.advertise_mice {
            let ips: Vec<IpAddr> = net::primary_ipv4().map(IpAddr::V4).into_iter().collect();
            vendor.push(mice::vendor_extension(&t.identity.host_name, &ips));
        }
        cfg.insert("VendorExtension", Value::from(vendor));
        wpa.set_p2p_config(&dev, cfg).await?;
        wpa.set_config_methods(&dev, CONFIG_METHODS).await?;
        Ok::<(), wpa::WpaError>(())
    };
    if let Err(e) = apply.await {
        return Err(TransportError::Unavailable(format!("could not configure wpa_supplicant: {e}")));
    }

    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(wpa::SERVICE)
        .and_then(|b| b.interface(IFACE_P2P))
        .map(|b| b.build())
        .map_err(|e| TransportError::Other(e.to_string()))?;
    let mut signals = zbus::MessageStream::for_match_rule(rule, &wpa.conn, Some(64))
        .await
        .map_err(|e| TransportError::Other(format!("cannot subscribe to wpa_supplicant signals: {e}")))?;

    let (itx, mut irx) = mpsc::unbounded_channel();
    let mut ctx = Ctx {
        wpa: wpa.clone(),
        dev: dev.clone(),
        our_paths,
        identity: t.identity.clone(),
        opts: t.opts.clone(),
        events: events.clone(),
        internal: itx,
        approvals: HashMap::new(),
        prompting: HashSet::new(),
        waiting_negotiations: HashMap::new(),
        forming: None,
        peers: HashMap::new(),
        current_peer: None,
        group: None,
        finding: false,
        nm,
        nm_active: None,
    };
    ctx.start_find().await;
    status(&events, TransportState::Ready { detail: format!("{dev_name} · discoverable as \"{}\"", t.identity.friendly_name) });

    let mut tick = tokio::time::interval(Duration::from_secs(20));
    loop {
        tokio::select! {
            m = signals.next() => match m {
                Some(Ok(msg)) => ctx.on_signal(msg).await,
                Some(Err(e)) => warn!("D-Bus signal error: {e}"),
                None => {
                    status(&events, TransportState::Error { message: "lost connection to wpa_supplicant".into() });
                    break;
                }
            },
            Some(i) = irx.recv() => ctx.on_internal(i).await,
            cmd = commands.recv() => match cmd {
                Some(TransportCommand::SessionEnded { peer }) => ctx.end_session(&peer).await,
                Some(TransportCommand::Shutdown) | None => break,
            },
            _ = tick.tick() => {
                ctx.on_tick().await;
            }
        }
    }

    // restore
    if ctx.forming.take().is_some() && ctx.nm_active.is_none() {
        let _ = wpa.cancel(&dev).await;
    }
    ctx.stop_nm().await;
    let _ = wpa.stop_find(&dev).await;
    if let Some(g) = ctx.group.take() {
        if let Some(t) = g.task {
            t.abort();
        }
        let _ = wpa.disconnect_group(&g.iface_path).await;
    }
    let _ = wpa.set_wfd_ies(&saved_ies).await;
    let mut restore: HashMap<&str, Value<'_>> = HashMap::new();
    for key in ["DeviceName", "PrimaryDeviceType", "GOIntent", "PersistentReconnect", "VendorExtension"] {
        if let Some(v) = saved_cfg.get(key).map(|v| Value::from(v.clone())) {
            restore.insert(key, v);
        }
    }
    if !restore.is_empty() {
        let _ = wpa.set_p2p_config(&dev, restore).await;
    }
    if let Some(m) = saved_methods {
        let _ = wpa.set_config_methods(&dev, &m).await;
    }
    info!("Wi-Fi Direct sink stopped; wpa_supplicant settings restored");
    Ok(())
}

fn dict_path(d: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    d.get(key).and_then(|v| OwnedObjectPath::try_from(v.clone()).ok()).map(|p| p.to_string())
}

fn dict_str(d: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    d.get(key).and_then(|v| String::try_from(v.clone()).ok())
}

fn dict_ip(d: &HashMap<String, OwnedValue>, key: &str) -> Option<Ipv4Addr> {
    let v: Vec<u8> = d.get(key).and_then(|v| Vec::<u8>::try_from(v.clone()).ok())?;
    (v.len() == 4).then(|| Ipv4Addr::new(v[0], v[1], v[2], v[3]))
}

impl Ctx {
    async fn start_find(&mut self) {
        match self.wpa.find(&self.dev).await {
            Ok(()) => self.finding = true,
            Err(e) => warn!("P2P Find failed: {e}"),
        }
    }

    async fn on_tick(&mut self) {
        if self.group.is_some() {
            return;
        }
        if let Some((since, waiting_for_source)) = self.forming {
            if since.elapsed() >= FORMATION_TIMEOUT {
                warn!("the source did not complete the Wi-Fi Direct connection; cancelling");
                self.forming = None;
                self.current_peer = None;
                if self.nm_active.is_some() {
                    self.stop_nm().await;
                    self.advertise().await;
                } else if let Err(e) = self.wpa.cancel(&self.dev).await {
                    debug!("P2P Cancel failed: {e}");
                }
                self.start_find().await;
                self.ready_status();
                return;
            }
            // Find would interrupt a negotiation we started ourselves
            if !waiting_for_source {
                return;
            }
        }
        // wpa_supplicant stops Find on its own now and then
        if !self.finding {
            self.start_find().await;
        }
    }

    async fn peer(&mut self, path: &str) -> PeerInfo {
        if let Some(p) = self.peers.get(path) {
            return p.clone();
        }
        let info = self.wpa.peer_info(path).await.unwrap_or_default();
        self.peers.insert(path.to_string(), info.clone());
        info
    }

    /// Ends our NetworkManager activation, if any.
    async fn stop_nm(&mut self) {
        if let Some((active, watcher)) = self.nm_active.take() {
            watcher.abort();
            if let Some(nm) = &self.nm {
                nm.deactivate(&active).await;
            }
        }
    }

    /// NetworkManager may replace our WFD IEs while it connects.
    async fn advertise(&self) {
        let ies = wfd_core::ie::sink_subelements(wfd_core::ie::DEFAULT_RTSP_PORT, self.identity.uibc);
        if let Err(e) = self.wpa.set_wfd_ies(&ies).await {
            warn!("could not restore the WFD information elements: {e}");
        }
    }

    /// Whether we are already connecting (or connected) to this source.
    fn busy_with(&self, peer_path: &str) -> bool {
        self.current_peer.as_deref() == Some(peer_path)
            && (self.group.is_some() || self.forming.is_some_and(|(since, _)| since.elapsed() < REPEAT_QUIET))
    }

    fn rtsp_port(info: &PeerInfo) -> u16 {
        wfd_core::ie::parse_peer_info(&info.wfd_ies)
            .filter(|p| p.device.device_type.is_source())
            .map(|p| p.device.control_port)
            .filter(|p| *p != 0)
            .unwrap_or(wfd_core::ie::DEFAULT_RTSP_PORT)
    }

    /// Ask the user (once per attempt) and report the decision internally.
    async fn prompt(&mut self, peer_path: String, method: PairingMethod, wps: &'static str, peer_intent: u8, authorize_only: bool) {
        if self.prompting.contains(&peer_path) || self.busy_with(&peer_path) {
            return;
        }
        let info = self.peer(&peer_path).await;
        let name = if info.name.is_empty() { mac_string(&info.device_address) } else { info.name.clone() };
        if !wfd_core::ie::parse_peer_info(&info.wfd_ies).is_some_and(|p| p.device.device_type.is_source()) {
            debug!(peer = %name, "peer has no WFD source IE; offering it anyway");
        }
        self.prompting.insert(peer_path.clone());
        let pin = match &method {
            PairingMethod::DisplayPin(p) => Some(p.clone()),
            _ => None,
        };
        let (tx, rx) = oneshot::channel();
        let _ = self.events.send(TransportEvent::PairingRequest(PairingRequest {
            kind: KIND,
            peer: PeerId(mac_string(&info.device_address)),
            device_name: name,
            method,
            reply: tx,
        }));
        status(&self.events, TransportState::Busy { detail: "waiting for you to allow the connection".into() });
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let reply = tokio::time::timeout(Duration::from_secs(90), rx).await.ok().and_then(Result::ok).unwrap_or(PairingReply::Reject);
            let pin = match &reply {
                PairingReply::AcceptWithPin(p) => Some(p.clone()),
                _ => pin,
            };
            let _ = internal.send(Internal::Decision { peer_path, reply, method: wps, pin, peer_intent, authorize_only });
        });
    }

    async fn connect(&mut self, peer_path: &str, method: &'static str, pin: Option<&str>, peer_intent: u8, authorize_only: bool) {
        if let Some(nm) = self.nm.clone() {
            self.connect_with_nm(nm, peer_path, method).await;
            return;
        }
        let intent = if peer_intent >= 15 { 0 } else { self.opts.go_intent };
        info!(peer = %peer_path, method, intent, authorize_only, "P2P Connect");
        self.current_peer = Some(peer_path.to_string());
        self.finding = false;
        match self.wpa.connect_peer(&self.dev, peer_path, method, pin, intent, authorize_only).await {
            Ok(_) => {
                self.forming = Some((Instant::now(), authorize_only));
                status(&self.events, TransportState::Busy { detail: "forming Wi-Fi Direct group".into() });
            }
            Err(e) => {
                warn!("P2P Connect failed: {e}");
                status(&self.events, TransportState::Error { message: format!("Wi-Fi Direct connect failed: {e}") });
                self.start_find().await;
            }
        }
    }

    async fn connect_with_nm(&mut self, nm: Nm, peer_path: &str, method: &'static str) {
        if method != "pbc" {
            let message = "PIN pairing isn't available here; use push-button pairing on the device".to_string();
            warn!("{message}");
            let _ = self.wpa.reject_peer(&self.dev, peer_path).await;
            status(&self.events, TransportState::Error { message });
            return;
        }
        let mac = mac_string(&self.peer(peer_path).await.device_address);
        self.stop_nm().await;
        info!(peer = %mac, "asking NetworkManager to connect");
        let ies = wfd_core::ie::sink_subelements(wfd_core::ie::DEFAULT_RTSP_PORT, self.identity.uibc);
        match nm.activate(&mac, &ies).await {
            Ok(active) => {
                self.current_peer = Some(peer_path.to_string());
                self.finding = false;
                self.forming = Some((Instant::now(), false));
                let internal = self.internal.clone();
                let watched = active.clone();
                let watcher = tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        if nm.state(&watched).await == ActiveState::Ended {
                            let _ = internal.send(Internal::NmEnded { active: watched });
                            return;
                        }
                    }
                });
                self.nm_active = Some((active, watcher));
                status(&self.events, TransportState::Busy { detail: "forming Wi-Fi Direct group".into() });
            }
            Err(e) => {
                warn!("NetworkManager could not connect: {e}");
                status(&self.events, TransportState::Error { message: format!("Wi-Fi Direct connect failed: {e}") });
                self.start_find().await;
            }
        }
    }

    async fn on_internal(&mut self, i: Internal) {
        match i {
            Internal::Decision { peer_path, reply, method, pin, peer_intent, authorize_only } => {
                self.prompting.remove(&peer_path);
                let negotiation = self.waiting_negotiations.remove(&peer_path);
                if reply == PairingReply::Reject {
                    info!(peer = %peer_path, "connection refused");
                    let _ = self.wpa.reject_peer(&self.dev, &peer_path).await;
                    self.ready_status();
                    return;
                }
                let (peer_intent, authorize_only) = match negotiation {
                    // The source already asked, and wpa_supplicant answered
                    // "not ready": it now waits for us to start.
                    Some((pw_id, intent)) => {
                        let (new_method, wps) = method_for_password_id(pw_id);
                        if wps != method {
                            self.prompt(peer_path, new_method, wps, intent, false).await;
                            return;
                        }
                        (intent, false)
                    }
                    None => (peer_intent, authorize_only),
                };
                self.approvals.insert(peer_path.clone(), Approval { method, pin: pin.clone(), at: Instant::now() });
                self.connect(&peer_path, method, pin.as_deref(), peer_intent, authorize_only).await;
            }
            Internal::GroupReady { peer_ip } => {
                let Some(g) = self.group.as_mut() else { return };
                let Some((_, info)) = g.peer.clone() else { return };
                g.connected = true;
                if !self.opts.skip_netconfig {
                    reduce_latency(&g.ifname);
                }
                let name = if info.name.is_empty() { mac_string(&info.device_address) } else { info.name.clone() };
                let rtsp_port = Self::rtsp_port(&info);
                info!(%peer_ip, rtsp_port, peer = %name, "source reachable over Wi-Fi Direct");
                status(&self.events, TransportState::Busy { detail: format!("connected to {name} on {}", g.ifname) });
                let _ = self.events.send(TransportEvent::PeerConnected {
                    kind: KIND,
                    peer: PeerId(mac_string(&info.device_address)),
                    peer_ip,
                    rtsp_port,
                    device_name: name,
                });
            }
            Internal::GroupFailed { reason } => {
                warn!(%reason, "group setup failed");
                status(&self.events, TransportState::Error { message: reason });
                if let (Some(nm), Some((active, _))) = (&self.nm, &self.nm_active) {
                    nm.deactivate(active).await;
                } else if let Some(g) = &self.group {
                    let _ = self.wpa.disconnect_group(&g.iface_path).await;
                }
            }
            Internal::NmEnded { active } => {
                if self.nm_active.as_ref().is_none_or(|(a, _)| *a != active) {
                    return;
                }
                self.nm_active = None;
                let failed = self.forming.take().is_some() || self.group.as_ref().is_some_and(|g| !g.connected);
                if let Some(g) = self.group.take() {
                    self.group_gone(g);
                }
                self.current_peer = None;
                self.advertise().await;
                self.start_find().await;
                if failed {
                    let message = "NetworkManager could not set up the Wi-Fi Direct connection".to_string();
                    warn!("{message} (see: journalctl -u NetworkManager)");
                    status(&self.events, TransportState::Error { message });
                } else {
                    self.ready_status();
                }
            }
        }
    }

    fn ready_status(&self) {
        status(&self.events, TransportState::Ready { detail: format!("discoverable as \"{}\"", self.identity.friendly_name) });
    }

    /// Stops the group's task and tells the app if the source was connected.
    fn group_gone(&self, g: Group) {
        info!(ifname = %g.ifname, "P2P group finished");
        if let Some(t) = g.task {
            t.abort();
        }
        if g.connected
            && let Some((_, info)) = g.peer
        {
            let _ = self.events.send(TransportEvent::PeerDisconnected {
                kind: KIND,
                peer: PeerId(mac_string(&info.device_address)),
                reason: "Wi-Fi Direct link closed".into(),
            });
        }
    }

    async fn end_session(&mut self, peer: &PeerId) {
        if let Some(g) = &self.group
            && g.peer.as_ref().is_some_and(|(_, i)| mac_string(&i.device_address) == peer.0)
        {
            info!(ifname = %g.ifname, "removing P2P group");
            if let (Some(nm), Some((active, _))) = (&self.nm, &self.nm_active) {
                nm.deactivate(active).await;
            } else if let Err(e) = self.wpa.disconnect_group(&g.iface_path).await {
                warn!("group Disconnect failed: {e}");
            }
        }
    }

    async fn on_signal(&mut self, msg: zbus::Message) {
        let header = msg.header();
        let (Some(member), Some(path)) = (header.member().map(|m| m.to_string()), header.path().map(|p| p.to_string())) else {
            return;
        };
        if !self.our_paths.contains(&path) {
            return;
        }
        let body = msg.body();
        debug!(%member, "wpa_supplicant signal");
        match member.as_str() {
            "GONegotiationRequest" => {
                let Ok((peer, pw_id, intent)) = body.deserialize::<(OwnedObjectPath, u16, u8)>() else { return };
                let peer = peer.to_string();
                if self.nm.is_some() && self.busy_with(&peer) {
                    return;
                }
                if self.group.is_some() {
                    if self.current_peer.as_deref() == Some(peer.as_str()) {
                        return;
                    }
                    info!("already in a session; refusing a second source");
                    let _ = self.wpa.reject_peer(&self.dev, &peer).await;
                    return;
                }
                if self.prompting.contains(&peer) {
                    self.waiting_negotiations.insert(peer, (pw_id, intent));
                    return;
                }
                let (method, wps) = method_for_password_id(pw_id);
                // An approval from a provision-discovery prompt (or an earlier
                // try of this negotiation) covers the retry, if the method matches.
                if let Some(a) = self.approvals.get(&peer).filter(|a| a.at.elapsed() < APPROVAL_VALID && a.method == wps).cloned() {
                    self.connect(&peer, a.method, a.pin.as_deref(), intent, false).await;
                } else {
                    self.prompt(peer, method, wps, intent, false).await;
                }
            }
            "ProvisionDiscoveryPBCRequest" => {
                let Ok((peer,)) = body.deserialize::<(OwnedObjectPath,)>() else { return };
                self.prompt(peer.to_string(), PairingMethod::PushButton, "pbc", 0, false).await;
            }
            "ProvisionDiscoveryRequestDisplayPin" => {
                let Ok((peer, pin)) = body.deserialize::<(OwnedObjectPath, String)>() else { return };
                self.prompt(peer.to_string(), PairingMethod::DisplayPin(pin), "display", 0, true).await;
            }
            "ProvisionDiscoveryRequestEnterPin" => {
                let Ok((peer,)) = body.deserialize::<(OwnedObjectPath,)>() else { return };
                self.prompt(peer.to_string(), PairingMethod::EnterPin, "keypad", 0, false).await;
            }
            "GONegotiationFailure" | "GroupFormationFailure" | "WpsFailed" => {
                warn!(%member, "Wi-Fi Direct group formation failed");
                status(&self.events, TransportState::Error { message: format!("Wi-Fi Direct: {member}") });
                self.current_peer = None;
                self.forming = None;
                self.start_find().await;
            }
            "GroupStarted" => {
                let Ok((d,)) = body.deserialize::<(HashMap<String, OwnedValue>,)>() else { return };
                self.on_group_started(d).await;
            }
            "GroupFinished" => {
                let Ok((d,)) = body.deserialize::<(HashMap<String, OwnedValue>,)>() else { return };
                let iface = dict_path(&d, "interface_object").unwrap_or_default();
                if let Some(g) = self.group.take() {
                    if !iface.is_empty() && iface != g.iface_path {
                        self.group = Some(g);
                        return;
                    }
                    self.group_gone(g);
                }
                self.current_peer = None;
                if self.nm.is_some() {
                    self.stop_nm().await;
                    self.advertise().await;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                self.start_find().await;
                self.ready_status();
            }
            "FindStopped" => self.finding = false,
            _ => {}
        }
    }

    async fn on_group_started(&mut self, d: HashMap<String, OwnedValue>) {
        let Some(iface_path) = dict_path(&d, "interface_object") else { return };
        let role = dict_str(&d, "role").unwrap_or_default();
        let ifname = self.wpa.ifname(&iface_path).await.unwrap_or_default();
        let peer = match self.current_peer.clone() {
            Some(p) => {
                let info = self.peer(&p).await;
                Some((p, info))
            }
            None => None,
        };
        info!(%ifname, %role, "P2P group started");
        self.finding = false;
        self.forming = None;
        if let Some((p, _)) = &peer {
            // approvals cover one connection
            self.approvals.remove(p);
        }
        if self.nm_active.is_none() {
            wpa::nm_unmanage(&self.wpa.conn, &ifname).await;
        }

        let internal = self.internal.clone();
        let go = role.eq_ignore_ascii_case("GO");
        let subnets = self.opts.go_subnets.clone();
        let host = self.identity.host_name.clone();
        let eapol_ip = (dict_ip(&d, "IpAddr"), dict_ip(&d, "IpAddrMask"), dict_ip(&d, "IpAddrGo"));
        let ifn = ifname.clone();
        let skip = self.opts.skip_netconfig;
        let managed = self.nm.clone().zip(self.nm_active.as_ref().map(|(a, _)| a.clone()));
        let task = tokio::spawn(async move {
            let r = if let Some((nm, active)) = managed {
                nm.peer_address(&active, go, eapol_ip.2).await.map(|ip| {
                    let _ = internal.send(Internal::GroupReady { peer_ip: IpAddr::V4(ip) });
                })
            } else if go {
                if skip { Err("address setup skipped (test mode)".into()) } else { run_go(ifn, subnets, internal.clone()).await }
            } else {
                run_client(ifn, host, eapol_ip, skip, internal.clone()).await
            };
            if let Err(reason) = r {
                let _ = internal.send(Internal::GroupFailed { reason });
            }
        });
        self.group = Some(Group { iface_path, ifname, peer, task: Some(task), connected: false });
    }
}

/// Turns Wi-Fi power saving off on the group link (needs CAP_NET_ADMIN).
fn reduce_latency(ifname: &str) {
    match netlink::disable_wifi_power_save(ifname) {
        Ok(()) => info!(%ifname, "Wi-Fi power saving turned off on the group link"),
        Err(e) if e.raw_os_error() == Some(libc::EPERM) => {
            warn!(%ifname, "Wi-Fi power saving stays on, which adds lag: run scripts/setup-linux.sh to grant cap_net_admin")
        }
        // not applicable when we are the group owner
        Err(e) => debug!(%ifname, "could not change Wi-Fi power saving: {e}"),
    }
}

async fn run_go(ifname: String, subnets: Vec<Ipv4Addr>, internal: mpsc::UnboundedSender<Internal>) -> Result<(), String> {
    let used = netlink::local_ipv4_addresses();
    let ip = subnets
        .iter()
        .copied()
        .find(|s| !used.iter().any(|u| u.octets()[..3] == s.octets()[..3]))
        .ok_or("no free private subnet for the Wi-Fi Direct group")?;
    netlink::add_ipv4_address(&ifname, ip, 24).map_err(|e| format!("cannot set {ip}/24 on {ifname}: {e}"))?;
    info!(%ifname, %ip, "group owner address set; serving DHCP");
    let (ltx, mut lrx) = mpsc::unbounded_channel();
    let server = tokio::spawn(dhcp::runner::serve(ifname.clone(), DhcpServer::for_group(ip), ltx));
    let first = tokio::time::timeout(Duration::from_secs(60), lrx.recv()).await;
    match first {
        Ok(Some(lease)) => {
            let _ = internal.send(Internal::GroupReady { peer_ip: IpAddr::V4(lease.ip) });
        }
        _ => {
            server.abort();
            return Err("the source never asked for an IP address (DHCP)".into());
        }
    }
    // keep serving renewals for the life of the group
    while lrx.recv().await.is_some() {}
    match server.await {
        Ok(Err(e)) => Err(format!("DHCP server stopped: {e}")),
        _ => Ok(()),
    }
}

async fn run_client(
    ifname: String,
    host: String,
    eapol: (Option<Ipv4Addr>, Option<Ipv4Addr>, Option<Ipv4Addr>),
    skip_netconfig: bool,
    internal: mpsc::UnboundedSender<Internal>,
) -> Result<(), String> {
    // wpa_supplicant may already have an address for us (IP allocation in EAPOL-Key)
    let (ip, prefix, go_ip) = match eapol {
        (Some(ip), mask, Some(go)) => (ip, mask.map_or(24, |m| u32::from(m).count_ones() as u8), go),
        _ => {
            let mac = netlink::mac_address(&ifname).ok_or("cannot read the group interface MAC address")?;
            let lease = dhcp::runner::obtain(&ifname, mac, &host, Duration::from_secs(30))
                .await
                .map_err(|e| format!("DHCP from the group owner failed: {e}"))?;
            (lease.ip, lease.prefix, lease.router.unwrap_or(lease.server))
        }
    };
    if !skip_netconfig {
        netlink::add_ipv4_address(&ifname, ip, prefix).map_err(|e| format!("cannot set {ip}/{prefix} on {ifname}: {e}"))?;
    }
    info!(%ifname, %ip, %go_ip, "joined source's group as client");
    let _ = internal.send(Internal::GroupReady { peer_ip: IpAddr::V4(go_ip) });
    std::future::pending::<()>().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wps_pins_have_valid_checksums() {
        // known-good PIN from the WPS spec examples
        assert_eq!(wps_checksum(1234567), 0);
        for _ in 0..50 {
            let pin = generate_wps_pin();
            assert_eq!(pin.len(), 8);
            let n: u32 = pin[..7].parse().unwrap();
            assert_eq!(pin[7..].parse::<u32>().unwrap(), wps_checksum(n));
        }
    }

    #[test]
    fn password_ids() {
        assert_eq!(method_for_password_id(4).1, "pbc");
        assert_eq!(method_for_password_id(5), (PairingMethod::EnterPin, "keypad"));
        assert!(matches!(method_for_password_id(1).0, PairingMethod::DisplayPin(p) if p.len() == 8));
    }
}
