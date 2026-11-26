//! TCP 7250 listener + mDNS registration: the MS-MICE transport.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

use super::{MiceMessage, command, register_display_service};
use crate::{
    BoxFuture, DiscoveryTransport, PairingMethod, PairingReply, PairingRequest, PeerId, ReceiverIdentity, TransportCommand, TransportError,
    TransportEvent, TransportKind, TransportState,
};

/// MS-MICE "Session Establishment Timer" without PIN entry: 30 s, extended
/// here to cover the user answering the Allow/Deny prompt.
const SESSION_ESTABLISHMENT: Duration = Duration::from_secs(60);
const KIND: TransportKind = TransportKind::Infrastructure;

pub struct MiceTransport {
    identity: ReceiverIdentity,
    port: u16,
    /// Skip mDNS (tests).
    pub register_mdns: bool,
}

impl MiceTransport {
    pub fn new(identity: ReceiverIdentity, port: u16) -> Self {
        Self { identity, port, register_mdns: true }
    }
}

impl DiscoveryTransport for MiceTransport {
    fn kind(&self) -> TransportKind {
        KIND
    }

    fn run(
        self: Box<Self>,
        events: mpsc::UnboundedSender<TransportEvent>,
        commands: mpsc::UnboundedReceiver<TransportCommand>,
    ) -> BoxFuture<'static, Result<(), TransportError>> {
        Box::pin(run(*self, events, commands))
    }
}

fn status(events: &mpsc::UnboundedSender<TransportEvent>, state: TransportState) {
    let _ = events.send(TransportEvent::Status { kind: KIND, state });
}

async fn run(
    t: MiceTransport,
    events: mpsc::UnboundedSender<TransportEvent>,
    mut commands: mpsc::UnboundedReceiver<TransportCommand>,
) -> Result<(), TransportError> {
    status(&events, TransportState::Starting);
    let listener = match TcpListener::bind(("0.0.0.0", t.port)).await {
        Ok(l) => l,
        Err(e) => {
            let reason = format!("cannot listen on TCP {}: {e} (is another Miracast receiver running?)", t.port);
            status(&events, TransportState::Unavailable { reason: reason.clone() });
            return Err(TransportError::Unavailable(reason));
        }
    };
    let port = listener.local_addr()?.port();
    let _mdns = if t.register_mdns {
        match register_display_service(&t.identity.friendly_name, port, &t.identity.container_id).await {
            Ok(r) => Some(r),
            Err(e) => {
                warn!("mDNS registration failed: {e}");
                None
            }
        }
    } else {
        None
    };
    let detail = match &_mdns {
        Some(r) => format!("TCP {port} · mDNS via {}", r.backend),
        None => format!("TCP {port} (no mDNS)"),
    };
    info!(port, "MS-MICE listening");
    status(&events, TransportState::Ready { detail });

    // peer -> "stop" sender for the connection task
    let mut active: HashMap<PeerId, oneshot::Sender<()>> = HashMap::new();
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<PeerId>();
    loop {
        tokio::select! {
            acc = listener.accept() => {
                let Ok((stream, addr)) = acc else { continue };
                let peer = PeerId(format!("mice:{addr}"));
                info!(%addr, "MS-MICE connection");
                let (stop_tx, stop_rx) = oneshot::channel();
                active.insert(peer.clone(), stop_tx);
                let events = events.clone();
                let identity = t.identity.clone();
                let done = done_tx.clone();
                tokio::spawn(async move {
                    connection(stream, addr, peer.clone(), identity, events, stop_rx).await;
                    let _ = done.send(peer);
                });
            }
            Some(peer) = done_rx.recv() => {
                active.remove(&peer);
            }
            cmd = commands.recv() => match cmd {
                Some(TransportCommand::SessionEnded { peer }) => {
                    if let Some(stop) = active.remove(&peer) {
                        let _ = stop.send(());
                    }
                }
                Some(TransportCommand::Shutdown) | None => {
                    for (_, stop) in active.drain() {
                        let _ = stop.send(());
                    }
                    // give connection tasks a moment to send STOP_PROJECTION
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    return Ok(());
                }
            }
        }
    }
}

async fn read_message(stream: &mut TcpStream, buf: &mut Vec<u8>) -> std::io::Result<Option<MiceMessage>> {
    loop {
        if let Some(len) = MiceMessage::frame_len(buf)
            && buf.len() >= len
        {
            let frame: Vec<u8> = buf.drain(..len.max(4)).collect();
            return match MiceMessage::decode(&frame) {
                Ok(m) => Ok(Some(m)),
                Err(e) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())),
            };
        }
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    peer: PeerId,
    identity: ReceiverIdentity,
    events: mpsc::UnboundedSender<TransportEvent>,
    mut stop: oneshot::Receiver<()>,
) {
    let mut buf = Vec::new();
    let mut connected = false;
    let mut source_id: Vec<u8> = Vec::new();
    let mut source_name = String::new();
    let establish = tokio::time::sleep(SESSION_ESTABLISHMENT);
    tokio::pin!(establish);

    let reason: String = loop {
        tokio::select! {
            msg = read_message(&mut stream, &mut buf) => {
                let msg = match msg {
                    Ok(Some(m)) => m,
                    Ok(None) => break "source closed the connection".into(),
                    Err(e) => break format!("bad message: {e}"),
                };
                debug!(command = msg.command, %addr, "MS-MICE message");
                match msg.command {
                    command::SOURCE_READY if !connected => {
                        source_name = msg.friendly_name().unwrap_or_else(|| addr.ip().to_string());
                        source_id = msg.source_id().map(<[u8]>::to_vec).unwrap_or_default();
                        let rtsp_port = msg.rtsp_port().unwrap_or(wfd_core::ie::DEFAULT_RTSP_PORT);
                        let (reply_tx, reply_rx) = oneshot::channel();
                        let _ = events.send(TransportEvent::PairingRequest(PairingRequest {
                            kind: KIND,
                            peer: peer.clone(),
                            device_name: source_name.clone(),
                            method: PairingMethod::PushButton,
                            reply: reply_tx,
                        }));
                        let reply = tokio::select! {
                            r = reply_rx => r.unwrap_or(PairingReply::Reject),
                            _ = &mut establish => PairingReply::Reject,
                        };
                        if reply == PairingReply::Reject {
                            let stop = MiceMessage::stop_projection(&identity.friendly_name, &source_id);
                            let _ = stream.write_all(&stop.encode()).await;
                            break "rejected".into();
                        }
                        connected = true;
                        let _ = events.send(TransportEvent::PeerConnected {
                            kind: KIND,
                            peer: peer.clone(),
                            peer_ip: addr.ip(),
                            rtsp_port,
                            device_name: source_name.clone(),
                        });
                    }
                    command::SOURCE_READY => debug!("duplicate SOURCE_READY ignored"),
                    command::STOP_PROJECTION => break format!("{} stopped projecting", if source_name.is_empty() { "source" } else { &source_name }),
                    other => {
                        // SECURITY_HANDSHAKE / SESSION_REQUEST / PIN_*: we advertise
                        // neither encryption nor PIN, so these are unexpected.
                        warn!(command = other, "unsupported MS-MICE message; closing");
                        break format!("source asked for an unsupported MS-MICE feature (command {other})");
                    }
                }
            }
            _ = &mut establish, if !connected => break "session establishment timed out".into(),
            _ = &mut stop => {
                let m = MiceMessage::stop_projection(&identity.friendly_name, &source_id);
                let _ = stream.write_all(&m.encode()).await;
                let _ = stream.shutdown().await;
                // the app ended the session itself: no disconnect event
                return;
            }
        }
    };
    info!(%addr, %reason, "MS-MICE connection closed");
    if connected {
        let _ = events.send(TransportEvent::PeerDisconnected { kind: KIND, peer, reason });
    }
}
