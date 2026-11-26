//! Runs the Linux Wi-Fi Direct transport against a mock wpa_supplicant on a
//! private D-Bus daemon. Checks the exact D-Bus types we write (WFD IEs,
//! P2PDeviceConfig, Connect arguments), the pairing flow, group start/finish
//! and that settings are restored on shutdown. Signals are emitted on the
//! *parent* interface path, as wpa_supplicant does for a dedicated P2P device.
#![cfg(all(target_os = "linux", feature = "linux-p2p"))]

use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use wfd_transport::linux_p2p::{LinuxP2p, P2pOptions};
use wfd_transport::{DiscoveryTransport, PairingMethod, PairingReply, ReceiverIdentity, TransportCommand, TransportEvent, TransportState};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const WLAN: &str = "/fi/w1/wpa_supplicant1/Interfaces/1";
const P2PDEV: &str = "/fi/w1/wpa_supplicant1/Interfaces/2";
const GROUP_IF: &str = "/fi/w1/wpa_supplicant1/Interfaces/3";
const PEER: &str = "/fi/w1/wpa_supplicant1/Interfaces/1/Peers/021122334455";
const P2P_IFACE: &str = "fi.w1.wpa_supplicant1.Interface.P2PDevice";

#[derive(Default, Debug)]
struct State {
    wfd_ies: Vec<u8>,
    config: HashMap<String, OwnedValue>,
    config_signatures: HashMap<String, String>,
    config_methods: String,
    calls: Vec<(String, String)>,
    connect_args: Vec<HashMap<String, String>>,
}

type Shared = Arc<Mutex<State>>;

struct Root(Shared);
#[zbus::interface(name = "fi.w1.wpa_supplicant1")]
impl Root {
    #[zbus(property)]
    fn capabilities(&self) -> Vec<String> {
        vec!["ap".into(), "p2p".into(), "interworking".into()]
    }
    #[zbus(property)]
    fn interfaces(&self) -> Vec<OwnedObjectPath> {
        vec![ObjectPath::try_from(WLAN).unwrap().into(), ObjectPath::try_from(P2PDEV).unwrap().into()]
    }
    #[zbus(property, name = "WFDIEs")]
    fn wfd_ies(&self) -> Vec<u8> {
        self.0.lock().unwrap().wfd_ies.clone()
    }
    #[zbus(property, name = "WFDIEs")]
    fn set_wfd_ies(&mut self, v: Vec<u8>) {
        self.0.lock().unwrap().wfd_ies = v;
    }
}

struct Iface(String);
#[zbus::interface(name = "fi.w1.wpa_supplicant1.Interface")]
impl Iface {
    #[zbus(property)]
    fn ifname(&self) -> String {
        self.0.clone()
    }
}

struct Wps(Shared);
#[zbus::interface(name = "fi.w1.wpa_supplicant1.Interface.WPS")]
impl Wps {
    #[zbus(property)]
    fn config_methods(&self) -> String {
        self.0.lock().unwrap().config_methods.clone()
    }
    #[zbus(property)]
    fn set_config_methods(&mut self, v: String) {
        self.0.lock().unwrap().config_methods = v;
    }
}

struct P2p(Shared, &'static str);
#[zbus::interface(name = "fi.w1.wpa_supplicant1.Interface.P2PDevice")]
impl P2p {
    #[zbus(property, name = "P2PDeviceConfig")]
    fn p2p_device_config(&self) -> HashMap<String, OwnedValue> {
        self.0.lock().unwrap().config.iter().map(|(k, v)| (k.clone(), v.try_clone().unwrap())).collect()
    }
    #[zbus(property, name = "P2PDeviceConfig")]
    fn set_p2p_device_config(&mut self, v: HashMap<String, OwnedValue>) {
        let mut s = self.0.lock().unwrap();
        for (k, val) in v {
            s.config_signatures.insert(k.clone(), val.value_signature().to_string());
            s.config.insert(k, val);
        }
    }
    fn find(&self, args: HashMap<String, OwnedValue>) {
        let t: String = args.get("DiscoveryType").map(|v| String::try_from(v.try_clone().unwrap()).unwrap()).unwrap_or_default();
        self.0.lock().unwrap().calls.push(("Find".into(), t));
    }
    fn stop_find(&self) {
        self.0.lock().unwrap().calls.push(("StopFind".into(), self.1.into()));
    }
    fn connect(&self, args: HashMap<String, OwnedValue>) -> String {
        let flat = args.iter().map(|(k, v)| (k.clone(), format!("{}:{:?}", v.value_signature(), **v))).collect();
        let mut s = self.0.lock().unwrap();
        s.calls.push(("Connect".into(), self.1.into()));
        s.connect_args.push(flat);
        String::new()
    }
    fn reject_peer(&self, peer: OwnedObjectPath) {
        self.0.lock().unwrap().calls.push(("RejectPeer".into(), peer.to_string()));
    }
    fn disconnect(&self) {
        self.0.lock().unwrap().calls.push(("Disconnect".into(), self.1.into()));
    }
}

struct Peer;
#[zbus::interface(name = "fi.w1.wpa_supplicant1.Peer")]
impl Peer {
    #[zbus(property)]
    fn device_name(&self) -> String {
        "Galaxy S24".into()
    }
    #[zbus(property)]
    fn device_address(&self) -> Vec<u8> {
        vec![0x02, 0x11, 0x22, 0x33, 0x44, 0x55]
    }
    #[zbus(property, name = "IEs")]
    fn ies(&self) -> Vec<u8> {
        // WFD source, session available, RTSP port 7239
        vec![0x00, 0x00, 0x06, 0x00, 0x10, 0x1c, 0x47, 0x00, 0x32]
    }
}

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn start_bus() -> (Daemon, String) {
    let mut child = Command::new("dbus-daemon")
        .args(["--config-file=/usr/share/dbus-1/session.conf", "--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("dbus-daemon");
    let mut out = child.stdout.take().unwrap();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    use std::io::Read;
    while out.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {
        buf.push(byte[0]);
    }
    (Daemon(child), String::from_utf8(buf).unwrap())
}

async fn next(rx: &mut mpsc::UnboundedReceiver<TransportEvent>) -> TransportEvent {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("event in time").expect("open");
        eprintln!("event: {ev:?}");
        // skip intermediate Busy statuses
        if matches!(&ev, TransportEvent::Status { state: TransportState::Busy { .. }, .. }) {
            continue;
        }
        return ev;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn full_p2p_flow_against_mock_wpa_supplicant() {
    let _ = tracing_subscriber::fmt().with_env_filter("wfd_transport=debug").with_test_writer().try_init();
    let (_daemon, addr) = start_bus();
    // SAFETY: this test binary runs a single test; nothing else reads env concurrently.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &addr);
        std::env::set_var("WFD_WPA_BUS", "session");
    }
    let state: Shared = Arc::new(Mutex::new(State::default()));
    {
        let mut s = state.lock().unwrap();
        s.config.insert("DeviceName".into(), OwnedValue::try_from(Value::from("old-name")).unwrap());
        s.config_methods = "display".into();
    }
    let mock = zbus::connection::Builder::address(addr.as_str())
        .unwrap()
        .name("fi.w1.wpa_supplicant1")
        .unwrap()
        .serve_at("/fi/w1/wpa_supplicant1", Root(state.clone()))
        .unwrap()
        .serve_at(WLAN, Iface("wlan0".into()))
        .unwrap()
        .serve_at(WLAN, P2p(state.clone(), "wlan0"))
        .unwrap()
        .serve_at(P2PDEV, Iface("p2p-dev-wlan0".into()))
        .unwrap()
        .serve_at(P2PDEV, P2p(state.clone(), "p2p-dev-wlan0"))
        .unwrap()
        .serve_at(P2PDEV, Wps(state.clone()))
        .unwrap()
        .serve_at(GROUP_IF, Iface("p2p-wlan0-0".into()))
        .unwrap()
        .serve_at(GROUP_IF, P2p(state.clone(), "p2p-wlan0-0"))
        .unwrap()
        .serve_at(PEER, Peer)
        .unwrap()
        .build()
        .await
        .unwrap();

    let mut identity = ReceiverIdentity::new("Desk Display");
    identity.host_name = "deskpc".into();
    let t = LinuxP2p::new(identity, P2pOptions { skip_netconfig: true, ..Default::default() });
    let (etx, mut erx) = mpsc::unbounded_channel();
    let (ctx, crx) = mpsc::unbounded_channel();
    let task = tokio::spawn(Box::new(t).run(etx, crx));

    assert!(matches!(next(&mut erx).await, TransportEvent::Status { state: TransportState::Starting, .. }));
    let ev = next(&mut erx).await;
    let TransportEvent::Status { state: TransportState::Ready { detail }, .. } = ev else { panic!("{ev:?}") };
    assert!(detail.contains("p2p-dev-wlan0"), "{detail}");

    {
        let s = state.lock().unwrap();
        assert_eq!(s.wfd_ies, wfd_core::ie::sink_subelements(7236, true), "WFD IEs");
        assert_eq!(s.config_signatures.get("DeviceName").map(String::as_str), Some("s"));
        assert_eq!(s.config_signatures.get("PrimaryDeviceType").map(String::as_str), Some("ay"));
        assert_eq!(s.config_signatures.get("GOIntent").map(String::as_str), Some("u"));
        assert_eq!(s.config_signatures.get("PersistentReconnect").map(String::as_str), Some("b"));
        assert_eq!(s.config_signatures.get("VendorExtension").map(String::as_str), Some("aay"));
        let name: String = s.config["DeviceName"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(name, "Desk Display");
        let ty: Vec<u8> = s.config["PrimaryDeviceType"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(ty, vec![0, 7, 0, 0x50, 0xF2, 4, 0, 1]);
        let ve: Vec<Vec<u8>> = s.config["VendorExtension"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(&ve[0][..3], &[0x00, 0x01, 0x37], "MS-MICE vendor extension");
        assert!(ve[0].windows(6).any(|w| w == b"deskpc"));
        assert_eq!(s.config_methods, "virtual_push_button");
        assert!(s.calls.contains(&("Find".into(), "social".into())));
    }

    // phone starts GO negotiation with push-button, intent 0
    let peer = ObjectPath::try_from(PEER).unwrap();
    mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "GONegotiationRequest", &(peer.clone(), 4u16, 0u8)).await.unwrap();
    let TransportEvent::PairingRequest(req) = next(&mut erx).await else { panic!() };
    assert_eq!(req.device_name, "Galaxy S24");
    assert_eq!(req.method, PairingMethod::PushButton);
    assert_eq!(req.peer.0, "02:11:22:33:44:55");
    req.reply.send(PairingReply::Accept).unwrap();

    // the retry of the same request must not prompt again
    tokio::time::sleep(Duration::from_millis(300)).await;
    {
        let s = state.lock().unwrap();
        let c = s.connect_args.last().expect("Connect called");
        assert!(c["peer"].starts_with("o:") && c["peer"].contains(PEER), "{}", c["peer"]);
        assert_eq!(c["wps_method"], "s:Str(\"pbc\")");
        assert_eq!(c["go_intent"], "i:I32(15)");
        assert_eq!(c["authorize_only"], "b:Bool(false)");
        assert_eq!(c["persistent"], "b:Bool(false)");
    }

    // group up as P2P client with IP allocated in EAPOL
    let mut d: HashMap<&str, Value> = HashMap::new();
    d.insert("interface_object", Value::from(ObjectPath::try_from(GROUP_IF).unwrap()));
    d.insert("role", Value::from("client"));
    d.insert("group_object", Value::from(ObjectPath::try_from("/fi/w1/wpa_supplicant1/Groups/x").unwrap()));
    d.insert("IpAddr", Value::from(vec![192u8, 168, 49, 20]));
    d.insert("IpAddrMask", Value::from(vec![255u8, 255, 255, 0]));
    d.insert("IpAddrGo", Value::from(vec![192u8, 168, 49, 1]));
    mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "GroupStarted", &(d,)).await.unwrap();
    let ev = next(&mut erx).await;
    let TransportEvent::PeerConnected { peer_ip, rtsp_port, device_name, peer, .. } = ev else { panic!("{ev:?}") };
    assert_eq!(peer_ip.to_string(), "192.168.49.1");
    assert_eq!(rtsp_port, 7239, "port from the source's WFD IE");
    assert_eq!(device_name, "Galaxy S24");

    // a second phone while busy is refused
    mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "GONegotiationRequest", &(peer_path_other(), 4u16, 0u8)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(state.lock().unwrap().calls.iter().any(|(c, p)| c == "RejectPeer" && p.ends_with("Peers/0a0b0c0d0e0f")));

    // app ends the session -> group Disconnect on the group interface
    ctx.send(TransportCommand::SessionEnded { peer: peer.clone() }).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(state.lock().unwrap().calls.contains(&("Disconnect".into(), "p2p-wlan0-0".into())));
    let mut f: HashMap<&str, Value> = HashMap::new();
    f.insert("interface_object", Value::from(ObjectPath::try_from(GROUP_IF).unwrap()));
    f.insert("role", Value::from("client"));
    mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "GroupFinished", &(f,)).await.unwrap();
    let ev = next(&mut erx).await;
    assert!(matches!(ev, TransportEvent::PeerDisconnected { .. }), "{ev:?}");

    // PIN flow: phone displays a PIN (password id 5) and insists on GO (intent 15)
    mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "GONegotiationRequest", &(ObjectPath::try_from(PEER).unwrap(), 5u16, 15u8))
        .await
        .unwrap();
    let ev = next(&mut erx).await;
    let ev = if matches!(ev, TransportEvent::Status { .. }) { next(&mut erx).await } else { ev };
    let TransportEvent::PairingRequest(req) = ev else { panic!("{ev:?}") };
    assert_eq!(req.method, PairingMethod::EnterPin);
    req.reply.send(PairingReply::AcceptWithPin("12345670".into())).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    {
        let s = state.lock().unwrap();
        let c = s.connect_args.last().unwrap();
        assert_eq!(c["wps_method"], "s:Str(\"keypad\")");
        assert_eq!(c["pin"], "s:Str(\"12345670\")");
        assert_eq!(c["go_intent"], "i:I32(0)", "peer insists on GO -> we yield");
    }

    mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "GroupFormationFailure", &("failed",)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The order a real phone uses: provision discovery, then GO negotiation
    // while the user is still looking at the prompt. After Allow we must start
    // the negotiation ourselves.
    let peer = ObjectPath::try_from(PEER).unwrap();
    mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "ProvisionDiscoveryPBCRequest", &(peer.clone(),)).await.unwrap();
    let ev = next(&mut erx).await;
    let ev = if matches!(ev, TransportEvent::Status { .. }) { next(&mut erx).await } else { ev };
    let TransportEvent::PairingRequest(req) = ev else { panic!("{ev:?}") };
    mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "GONegotiationRequest", &(peer, 4u16, 13u8)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let before = state.lock().unwrap().connect_args.len();
    req.reply.send(PairingReply::Accept).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    {
        let s = state.lock().unwrap();
        assert_eq!(s.connect_args.len(), before + 1);
        let c = s.connect_args.last().unwrap();
        assert_eq!(c["wps_method"], "s:Str(\"pbc\")");
        assert_eq!(c["go_intent"], "i:I32(15)");
        assert_eq!(c["authorize_only"], "b:Bool(false)");
    }
    while let Ok(ev) = erx.try_recv() {
        assert!(!matches!(ev, TransportEvent::PairingRequest(_)), "asked twice: {ev:?}");
    }

    // shutdown restores what we changed
    ctx.send(TransportCommand::Shutdown).unwrap();
    task.await.unwrap().unwrap();
    let native_connects = {
        let s = state.lock().unwrap();
        assert!(s.wfd_ies.is_empty(), "WFD IEs restored");
        let name: String = s.config["DeviceName"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(name, "old-name");
        assert_eq!(s.config_methods, "display");
        assert!(s.calls.iter().any(|(c, _)| c == "StopFind"));
        s.connect_args.len()
    };

    through_network_manager(&mock, &state, native_connects).await;
}

/// With NetworkManager managing the P2P device, it must make the connection:
/// it deletes groups it didn't start.
async fn through_network_manager(mock: &zbus::Connection, state: &Shared, native_connects: usize) {
    let nm: NmShared = Default::default();
    nm.lock().unwrap().active_state = 1;
    let server = mock.object_server();
    server.at(NM_PATH, NmRoot(nm.clone())).await.unwrap();
    server.at(NM_DEVICE, NmDevice).await.unwrap();
    server.at(NM_ACTIVE, NmActive(nm.clone())).await.unwrap();
    mock.request_name("org.freedesktop.NetworkManager").await.unwrap();

    let t = LinuxP2p::new(ReceiverIdentity::new("Desk Display"), P2pOptions { skip_netconfig: true, ..Default::default() });
    let (etx, mut erx) = mpsc::unbounded_channel();
    let (ctx, crx) = mpsc::unbounded_channel();
    let task = tokio::spawn(Box::new(t).run(etx, crx));
    assert!(matches!(next(&mut erx).await, TransportEvent::Status { state: TransportState::Starting, .. }));
    assert!(matches!(next(&mut erx).await, TransportEvent::Status { state: TransportState::Ready { .. }, .. }));

    let peer = ObjectPath::try_from(PEER).unwrap();
    let body = (peer.clone(),);
    let ask = || mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "ProvisionDiscoveryPBCRequest", &body);
    ask().await.unwrap();
    let TransportEvent::PairingRequest(req) = next(&mut erx).await else { panic!() };
    mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "GONegotiationRequest", &(peer.clone(), 4u16, 13u8)).await.unwrap();
    req.reply.send(PairingReply::Accept).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    // the phone repeats its request while we connect
    ask().await.unwrap();
    ask().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    while let Ok(ev) = erx.try_recv() {
        assert!(!matches!(ev, TransportEvent::PairingRequest(_)), "asked twice: {ev:?}");
    }
    {
        let n = nm.lock().unwrap();
        assert_eq!(n.activations.len(), 1);
        let settings = &n.activations[0];
        assert!(settings["connection.type"].contains("wifi-p2p"));
        assert!(settings["wifi-p2p.peer"].contains("02:11:22:33:44:55"));
        assert!(settings.contains_key("wifi-p2p.wfd-ies"));
        assert!(settings["ipv4.never-default"].contains("true"));
        assert_eq!(state.lock().unwrap().connect_args.len(), native_connects, "wpa Connect is left to NetworkManager");
    }

    let mut d: HashMap<&str, Value> = HashMap::new();
    d.insert("interface_object", Value::from(ObjectPath::try_from(GROUP_IF).unwrap()));
    d.insert("role", Value::from("client"));
    d.insert("IpAddrGo", Value::from(vec![192u8, 168, 49, 1]));
    mock.emit_signal(None::<&str>, WLAN, P2P_IFACE, "GroupStarted", &(d,)).await.unwrap();
    nm.lock().unwrap().active_state = 2;
    let ev = next(&mut erx).await;
    let TransportEvent::PeerConnected { peer_ip, peer, .. } = ev else { panic!("{ev:?}") };
    assert_eq!(peer_ip.to_string(), "192.168.49.1");

    ctx.send(TransportCommand::SessionEnded { peer }).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(nm.lock().unwrap().deactivated, 1);
    nm.lock().unwrap().active_state = 4;
    let ev = next(&mut erx).await;
    assert!(matches!(ev, TransportEvent::PeerDisconnected { .. }), "{ev:?}");

    ctx.send(TransportCommand::Shutdown).unwrap();
    task.await.unwrap().unwrap();
}

const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_DEVICE: &str = "/org/freedesktop/NetworkManager/Devices/4";
const NM_ACTIVE: &str = "/org/freedesktop/NetworkManager/ActiveConnection/7";

#[derive(Default)]
struct NmState {
    activations: Vec<HashMap<String, String>>,
    deactivated: usize,
    active_state: u32,
}

type NmShared = Arc<Mutex<NmState>>;

fn path(p: &str) -> OwnedObjectPath {
    ObjectPath::try_from(p).unwrap().into()
}

struct NmRoot(NmShared);
#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl NmRoot {
    fn get_device_by_ip_iface(&self, iface: String) -> zbus::fdo::Result<OwnedObjectPath> {
        if iface == "p2p-dev-wlan0" { Ok(path(NM_DEVICE)) } else { Err(zbus::fdo::Error::Failed("no such device".into())) }
    }
    fn add_and_activate_connection2(
        &self,
        settings: HashMap<String, HashMap<String, OwnedValue>>,
        _device: OwnedObjectPath,
        _specific: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
    ) -> (OwnedObjectPath, OwnedObjectPath, HashMap<String, OwnedValue>) {
        assert!(options.contains_key("persist"));
        let flat = settings.iter().flat_map(|(s, m)| m.iter().map(move |(k, v)| (format!("{s}.{k}"), format!("{:?}", **v)))).collect();
        self.0.lock().unwrap().activations.push(flat);
        (path("/org/freedesktop/NetworkManager/Settings/1"), path(NM_ACTIVE), HashMap::new())
    }
    fn deactivate_connection(&self, _active: OwnedObjectPath) {
        self.0.lock().unwrap().deactivated += 1;
    }
}

struct NmDevice;
#[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
impl NmDevice {
    #[zbus(property)]
    fn device_type(&self) -> u32 {
        30
    }
    #[zbus(property)]
    fn managed(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn ip_interface(&self) -> String {
        "p2p-wlan0-0".into()
    }
    #[zbus(property)]
    fn ip4_config(&self) -> OwnedObjectPath {
        path("/")
    }
    #[zbus(property)]
    fn dhcp4_config(&self) -> OwnedObjectPath {
        path("/")
    }
}

struct NmActive(NmShared);
#[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl NmActive {
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.0.lock().unwrap().active_state
    }
}

fn peer_path_other() -> ObjectPath<'static> {
    ObjectPath::try_from("/fi/w1/wpa_supplicant1/Interfaces/1/Peers/0a0b0c0d0e0f").unwrap()
}
