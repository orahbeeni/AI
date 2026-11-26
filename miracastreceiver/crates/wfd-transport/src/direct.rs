//! "Transport" for a source whose address is already known: emits a single
//! `PeerConnected` (used by the simulator and `connect <ip:port>`).

use std::net::SocketAddr;

use tokio::sync::mpsc;

use crate::{BoxFuture, DiscoveryTransport, PeerId, TransportCommand, TransportError, TransportEvent, TransportKind, TransportState};

pub struct DirectTransport {
    pub addr: SocketAddr,
    pub device_name: String,
}

impl DiscoveryTransport for DirectTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Direct
    }

    fn run(
        self: Box<Self>,
        events: mpsc::UnboundedSender<TransportEvent>,
        mut commands: mpsc::UnboundedReceiver<TransportCommand>,
    ) -> BoxFuture<'static, Result<(), TransportError>> {
        Box::pin(async move {
            let _ = events.send(TransportEvent::Status {
                kind: TransportKind::Direct,
                state: TransportState::Ready { detail: self.addr.to_string() },
            });
            let peer = PeerId(format!("direct:{}", self.addr));
            let _ = events.send(TransportEvent::PeerConnected {
                kind: TransportKind::Direct,
                peer,
                peer_ip: self.addr.ip(),
                rtsp_port: self.addr.port(),
                device_name: self.device_name,
            });
            while let Some(cmd) = commands.recv().await {
                if cmd == TransportCommand::Shutdown {
                    break;
                }
            }
            Ok(())
        })
    }
}
