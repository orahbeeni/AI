//! How a Miracast source finds this receiver and how the IP link to it comes up.
//!
//! Each transport implements [`DiscoveryTransport`]: it advertises the sink,
//! asks the app to approve pairing ([`TransportEvent::PairingRequest`]) and
//! reports [`TransportEvent::PeerConnected`] once the source is reachable over
//! IP; the app then opens the RTSP session (`wfd-core`) to
//! `peer_ip:rtsp_port`.
//!
//! | transport | platforms | how |
//! |---|---|---|
//! | [`linux_p2p`] | Linux | wpa_supplicant D-Bus P2P + WFD IEs, GO + embedded DHCP |
//! | [`windows_p2p`] | Windows | WinRT `WiFiDirectAdvertisementPublisher` + listener |
//! | [`mice`] | all | MS-MICE: mDNS `_display._tcp` + TCP 7250 |
//! | [`direct`] | all | connect to a known `ip:port` (tests, simulator) |

use std::fmt;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;

use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

pub mod dhcp;
pub mod direct;
pub mod mice;
pub mod net;

#[cfg(all(target_os = "linux", feature = "linux-p2p"))]
pub mod linux_p2p;

#[cfg(all(windows, feature = "windows-p2p"))]
pub mod windows_p2p;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    /// Native Wi-Fi Direct (Linux wpa_supplicant or Windows WinRT).
    WifiDirect,
    /// Miracast over Infrastructure (MS-MICE).
    Infrastructure,
    /// Direct connection to a known address (simulator, manual connect).
    Direct,
}

impl TransportKind {
    pub fn label(self) -> &'static str {
        match self {
            TransportKind::WifiDirect => "Wi-Fi Direct",
            TransportKind::Infrastructure => "Infrastructure (MS-MICE)",
            TransportKind::Direct => "Direct",
        }
    }
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Transport-specific peer identity (P2P device address, TCP peer, ...).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerId(pub String);

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingMethod {
    /// Push-button: just allow or deny.
    PushButton,
    /// Show this PIN; the user types it on the source.
    DisplayPin(String),
    /// The source shows a PIN; ask the user to type it here.
    EnterPin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingReply {
    Accept,
    AcceptWithPin(String),
    Reject,
}

#[derive(Debug)]
pub struct PairingRequest {
    pub kind: TransportKind,
    pub peer: PeerId,
    pub device_name: String,
    pub method: PairingMethod,
    pub reply: oneshot::Sender<PairingReply>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportState {
    Starting,
    /// Discoverable; `detail` is shown in the UI (e.g. "p2p-dev-wlp4s0").
    Ready {
        detail: String,
    },
    /// Working on a connection (negotiating, DHCP...).
    Busy {
        detail: String,
    },
    /// Not possible on this machine; `reason` explains why and what to do.
    Unavailable {
        reason: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug)]
pub enum TransportEvent {
    Status { kind: TransportKind, state: TransportState },
    PairingRequest(PairingRequest),
    PeerConnected { kind: TransportKind, peer: PeerId, peer_ip: IpAddr, rtsp_port: u16, device_name: String },
    PeerDisconnected { kind: TransportKind, peer: PeerId, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportCommand {
    /// The RTSP session with this peer ended: tear the link down and become
    /// discoverable again.
    SessionEnded { peer: PeerId },
    /// Stop advertising, restore system settings, exit `run`.
    Shutdown,
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("{0}")]
    Unavailable(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// What every transport advertises.
#[derive(Debug, Clone)]
pub struct ReceiverIdentity {
    pub friendly_name: String,
    /// Our host name, for MS-MICE (must not contain '.').
    pub host_name: String,
    /// GUID shown to Windows as the sink's container ID.
    pub container_id: String,
    pub uibc: bool,
    /// Also advertise MS-MICE in Wi-Fi Direct probe responses.
    pub advertise_mice: bool,
}

impl ReceiverIdentity {
    pub fn new(friendly_name: impl Into<String>) -> Self {
        Self {
            friendly_name: friendly_name.into(),
            host_name: net::short_host_name(),
            container_id: format!("{{{}}}", uuid::Uuid::new_v4().to_string().to_uppercase()),
            uibc: true,
            advertise_mice: true,
        }
    }
}

/// A discovery/connection mechanism. `run` owns the transport until
/// [`TransportCommand::Shutdown`] or until the command channel closes.
pub trait DiscoveryTransport: Send {
    fn kind(&self) -> TransportKind;
    fn name(&self) -> String {
        self.kind().label().to_string()
    }
    fn run(
        self: Box<Self>,
        events: mpsc::UnboundedSender<TransportEvent>,
        commands: mpsc::UnboundedReceiver<TransportCommand>,
    ) -> BoxFuture<'static, Result<(), TransportError>>;
}

/// Why native Wi-Fi Direct is or isn't offered on this OS (for the UI).
pub fn wifi_direct_support() -> Result<(), &'static str> {
    if cfg!(target_os = "macos") {
        Err("macOS does not expose Wi-Fi Direct (P2P) to apps, so only Infrastructure mode (MS-MICE) is available.")
    } else if cfg!(all(target_os = "linux", feature = "linux-p2p")) || cfg!(all(windows, feature = "windows-p2p")) {
        Ok(())
    } else {
        Err("Wi-Fi Direct support was not compiled in.")
    }
}

/// The platform's native Wi-Fi Direct transport, if any.
pub fn wifi_direct_transport(identity: &ReceiverIdentity) -> Option<Box<dyn DiscoveryTransport>> {
    #[cfg(all(target_os = "linux", feature = "linux-p2p"))]
    {
        return Some(Box::new(linux_p2p::LinuxP2p::new(identity.clone(), linux_p2p::P2pOptions::default())));
    }
    #[cfg(all(windows, feature = "windows-p2p"))]
    {
        return Some(Box::new(windows_p2p::WindowsP2p::new(identity.clone())));
    }
    #[allow(unreachable_code)]
    {
        let _ = identity;
        None
    }
}

/// MS-MICE transport (all platforms) when compiled in.
pub fn infrastructure_transport(identity: &ReceiverIdentity) -> Option<Box<dyn DiscoveryTransport>> {
    #[cfg(feature = "ms-mice")]
    {
        return Some(Box::new(mice::MiceTransport::new(identity.clone(), mice::MICE_PORT)));
    }
    #[allow(unreachable_code)]
    {
        let _ = identity;
        None
    }
}
