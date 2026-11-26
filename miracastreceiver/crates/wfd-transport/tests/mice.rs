//! MS-MICE end to end: a fake Windows source talks to the TCP 7250 server,
//! and the `_display._tcp` record is discovered over real multicast.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use wfd_transport::mice::{MiceMessage, MiceTransport, command, tlv, utf16le_encode};
use wfd_transport::{DiscoveryTransport, PairingReply, ReceiverIdentity, TransportCommand, TransportEvent, TransportKind, TransportState};

const SOURCE_ID: [u8; 16] = [0x91, 0xF4, 0xAB, 0xE9, 0xEF, 0xF5, 0x46, 0x4A, 0xAE, 0xE2, 0x69, 0x72, 0x2A, 0xED, 0x11, 0xB5];

async fn next(rx: &mut mpsc::UnboundedReceiver<TransportEvent>) -> TransportEvent {
    tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("event in time").expect("open")
}

async fn start() -> (u16, mpsc::UnboundedReceiver<TransportEvent>, mpsc::UnboundedSender<TransportCommand>) {
    let mut t = MiceTransport::new(ReceiverIdentity::new("Test Receiver"), 0);
    t.register_mdns = false;
    let (etx, mut erx) = mpsc::unbounded_channel();
    let (ctx, crx) = mpsc::unbounded_channel();
    tokio::spawn(Box::new(t).run(etx, crx));
    assert!(matches!(next(&mut erx).await, TransportEvent::Status { state: TransportState::Starting, .. }));
    let TransportEvent::Status { state: TransportState::Ready { detail }, .. } = next(&mut erx).await else { panic!() };
    let port: u16 = detail.split_whitespace().nth(1).unwrap().parse().unwrap();
    (port, erx, ctx)
}

fn source_ready(rtsp_port: u16) -> Vec<u8> {
    MiceMessage::new(command::SOURCE_READY)
        .with(tlv::FRIENDLY_NAME, utf16le_encode("Dummy1-Kabylake"))
        .with(tlv::RTSP_PORT, rtsp_port.to_be_bytes().to_vec())
        .with(tlv::SOURCE_ID, SOURCE_ID.to_vec())
        .encode()
}

#[tokio::test]
async fn source_ready_connect_and_stop_projection() {
    let (port, mut events, _cmds) = start().await;
    let mut src = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    // send it in two pieces to exercise framing
    let msg = source_ready(7236);
    src.write_all(&msg[..10]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    src.write_all(&msg[10..]).await.unwrap();

    let TransportEvent::PairingRequest(req) = next(&mut events).await else { panic!("expected pairing request") };
    assert_eq!(req.device_name, "Dummy1-Kabylake");
    req.reply.send(PairingReply::Accept).unwrap();
    let TransportEvent::PeerConnected { kind, peer_ip, rtsp_port, device_name, .. } = next(&mut events).await else { panic!() };
    assert_eq!(kind, TransportKind::Infrastructure);
    assert_eq!(peer_ip.to_string(), "127.0.0.1");
    assert_eq!(rtsp_port, 7236);
    assert_eq!(device_name, "Dummy1-Kabylake");

    src.write_all(&MiceMessage::stop_projection("Dummy1-Kabylake", &SOURCE_ID).encode()).await.unwrap();
    let TransportEvent::PeerDisconnected { reason, .. } = next(&mut events).await else { panic!() };
    assert!(reason.contains("stopped projecting"), "{reason}");
}

#[tokio::test]
async fn receiver_ends_session_with_stop_projection() {
    let (port, mut events, cmds) = start().await;
    let mut src = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    src.write_all(&source_ready(7777)).await.unwrap();
    let TransportEvent::PairingRequest(req) = next(&mut events).await else { panic!() };
    req.reply.send(PairingReply::Accept).unwrap();
    let TransportEvent::PeerConnected { peer, rtsp_port, .. } = next(&mut events).await else { panic!() };
    assert_eq!(rtsp_port, 7777);

    cmds.send(TransportCommand::SessionEnded { peer }).unwrap();
    let mut buf = vec![0u8; 256];
    let n = tokio::time::timeout(Duration::from_secs(3), src.read(&mut buf)).await.unwrap().unwrap();
    let m = MiceMessage::decode(&buf[..n]).unwrap();
    assert_eq!(m.command, command::STOP_PROJECTION);
    assert_eq!(m.friendly_name().as_deref(), Some("Test Receiver"));
    assert_eq!(m.source_id(), Some(&SOURCE_ID[..]));
}

#[tokio::test]
async fn rejected_and_unsupported_sources_are_closed() {
    let (port, mut events, _cmds) = start().await;
    // rejected by the user -> receives STOP_PROJECTION, no PeerConnected
    let mut src = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    src.write_all(&source_ready(7236)).await.unwrap();
    let TransportEvent::PairingRequest(req) = next(&mut events).await else { panic!() };
    req.reply.send(PairingReply::Reject).unwrap();
    let mut buf = vec![0u8; 256];
    let n = tokio::time::timeout(Duration::from_secs(3), src.read(&mut buf)).await.unwrap().unwrap();
    assert_eq!(MiceMessage::decode(&buf[..n]).unwrap().command, command::STOP_PROJECTION);

    // a source asking for DTLS is disconnected (we don't advertise it)
    let mut src = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let hs = MiceMessage::new(command::SECURITY_HANDSHAKE).with(tlv::SECURITY_TOKEN, vec![0x16, 0xfe, 0xfd]).encode();
    src.write_all(&hs).await.unwrap();
    let n = tokio::time::timeout(Duration::from_secs(3), src.read(&mut buf)).await.unwrap().unwrap();
    assert_eq!(n, 0, "connection closed");
    assert!(tokio::time::timeout(Duration::from_millis(300), events.recv()).await.is_err(), "no events for it");
}

/// Registers through the system mDNS responder (Avahi on Linux) and finds the
/// record with an independent mDNS browser over multicast.
#[tokio::test]
async fn display_service_is_discoverable_over_mdns() {
    let instance = format!("WFD Test {}", std::process::id());
    let reg =
        wfd_transport::mice::register_display_service(&instance, 7250, "{0D5D1F0C-0000-4000-8000-000000000001}").await.expect("register");
    eprintln!("registered via {}", reg.backend);

    let browser = mdns_sd::ServiceDaemon::new().unwrap();
    let rx = browser.browse("_display._tcp.local.").unwrap();
    let found = tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while let Ok(ev) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            if let mdns_sd::ServiceEvent::ServiceResolved(s) = ev
                && s.fullname.starts_with(&instance.replace(' ', "\\032")) | s.fullname.starts_with(&instance)
            {
                return Some((s.port, s.get_property_val_str("container_id").map(str::to_string), s.host.clone(), s.addresses.len()));
            }
        }
        None
    })
    .await
    .unwrap();
    let _ = browser.shutdown();
    drop(reg);
    let (port, cid, host, naddr) = found.expect("our _display._tcp instance was not discovered");
    eprintln!("found port={port} container_id={cid:?} host={host} addresses={naddr}");
    assert_eq!(port, 7250);
    assert_eq!(cid.as_deref(), Some("{0D5D1F0C-0000-4000-8000-000000000001}"));
    assert!(naddr > 0, "resolved to an address");
}
