//! Native Wi-Fi Direct Miracast sink on Windows through WinRT:
//! `WiFiDirectAdvertisementPublisher` advertises us with our WFD IE and a
//! "display" device type; `WiFiDirectConnectionListener` receives the
//! source's connection request; after pairing, `WiFiDirectDevice` gives the
//! source's IP (Windows runs DHCP on the group itself).
//!
//! All WinRT calls run on one dedicated thread with blocking `join()`, so no
//! WinRT object crosses into tokio futures.
//!
//! Untested on real hardware: this compiles for Windows targets, but has not
//! been run against a phone yet.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};
use windows::Devices::Enumeration::{
    DeviceInformationCustomPairing, DevicePairingKinds, DevicePairingProtectionLevel, DevicePairingRequestedEventArgs,
    DevicePairingResultStatus,
};
use windows::Devices::WiFiDirect::{
    WiFiDirectAdvertisementListenStateDiscoverability, WiFiDirectAdvertisementPublisher, WiFiDirectConnectionListener,
    WiFiDirectConnectionParameters, WiFiDirectConnectionRequestedEventArgs, WiFiDirectConnectionStatus, WiFiDirectDevice,
    WiFiDirectInformationElement,
};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataWriter, IBuffer};
use windows::core::{HSTRING, Ref};

use crate::{
    BoxFuture, DiscoveryTransport, PairingMethod, PairingReply, PairingRequest, PeerId, ReceiverIdentity, TransportCommand, TransportError,
    TransportEvent, TransportKind, TransportState,
};

const KIND: TransportKind = TransportKind::WifiDirect;

pub struct WindowsP2p {
    identity: ReceiverIdentity,
}

impl WindowsP2p {
    pub fn new(identity: ReceiverIdentity) -> Self {
        Self { identity }
    }
}

fn buffer(bytes: &[u8]) -> windows::core::Result<IBuffer> {
    let w = DataWriter::new()?;
    w.WriteBytes(bytes)?;
    w.DetachBuffer()
}

fn ask(events: &mpsc::UnboundedSender<TransportEvent>, peer: &PeerId, name: &str, method: PairingMethod) -> PairingReply {
    let (tx, rx) = oneshot::channel();
    let _ = events.send(TransportEvent::PairingRequest(PairingRequest {
        kind: KIND,
        peer: peer.clone(),
        device_name: name.to_string(),
        method,
        reply: tx,
    }));
    rx.blocking_recv().unwrap_or(PairingReply::Reject)
}

type Devices = Arc<Mutex<HashMap<PeerId, WiFiDirectDevice>>>;

fn handle_request(
    args: &WiFiDirectConnectionRequestedEventArgs,
    events: &mpsc::UnboundedSender<TransportEvent>,
    devices: &Devices,
) -> windows::core::Result<()> {
    let request = args.GetConnectionRequest()?;
    let info = request.DeviceInformation()?;
    let name = info.Name()?.to_string();
    let id = info.Id()?;
    let peer = PeerId(id.to_string());
    info!(%name, "Wi-Fi Direct connection request");

    if ask(events, &peer, &name, PairingMethod::PushButton) == PairingReply::Reject {
        let _ = request.Close();
        return Ok(());
    }

    let pairing = info.Pairing()?;
    if !pairing.IsPaired()? {
        let custom: DeviceInformationCustomPairing = pairing.Custom()?;
        let ev = events.clone();
        let p = peer.clone();
        let n = name.clone();
        custom.PairingRequested(&TypedEventHandler::new(
            move |_: Ref<DeviceInformationCustomPairing>, a: Ref<DevicePairingRequestedEventArgs>| {
                let a = a.ok()?;
                let kind = a.PairingKind()?;
                if kind == DevicePairingKinds::ProvidePin {
                    // declining = not calling Accept; Windows then fails the pairing
                    if let PairingReply::AcceptWithPin(pin) = ask(&ev, &p, &n, PairingMethod::EnterPin) {
                        a.AcceptWithPin(&HSTRING::from(pin))?;
                    }
                } else if kind == DevicePairingKinds::DisplayPin {
                    let pin = a.Pin()?.to_string();
                    let _ = ev.send(TransportEvent::Status {
                        kind: KIND,
                        state: TransportState::Busy { detail: format!("enter PIN {pin} on {n}") },
                    });
                    a.Accept()?;
                } else {
                    a.Accept()?;
                }
                Ok(())
            },
        ))?;
        let params = WiFiDirectConnectionParameters::new()?;
        params.SetGroupOwnerIntent(15)?;
        let kinds =
            DevicePairingKinds(DevicePairingKinds::ConfirmOnly.0 | DevicePairingKinds::DisplayPin.0 | DevicePairingKinds::ProvidePin.0);
        let result = custom.PairWithProtectionLevelAndSettingsAsync(kinds, DevicePairingProtectionLevel::Default, &params)?.join()?;
        let st = result.Status()?;
        if st != DevicePairingResultStatus::Paired && st != DevicePairingResultStatus::AlreadyPaired {
            warn!(status = st.0, "Wi-Fi Direct pairing failed");
            return Ok(());
        }
    }

    let device = WiFiDirectDevice::FromIdAsync(&id)?.join()?;
    let pairs = device.GetConnectionEndpointPairs()?;
    let remote = pairs.GetAt(0)?.RemoteHostName()?.CanonicalName()?.to_string();
    let Ok(peer_ip) = remote.parse::<IpAddr>() else {
        warn!(%remote, "could not parse source address");
        return Ok(());
    };
    {
        let ev = events.clone();
        let p = peer.clone();
        let devs = devices.clone();
        device.ConnectionStatusChanged(&TypedEventHandler::new(move |d: Ref<WiFiDirectDevice>, _: Ref<windows::core::IInspectable>| {
            if d.ok()?.ConnectionStatus()? == WiFiDirectConnectionStatus::Disconnected {
                devs.lock().unwrap().remove(&p);
                let _ =
                    ev.send(TransportEvent::PeerDisconnected { kind: KIND, peer: p.clone(), reason: "Wi-Fi Direct link closed".into() });
            }
            Ok(())
        }))?;
    }
    devices.lock().unwrap().insert(peer.clone(), device);
    let _ = events.send(TransportEvent::PeerConnected {
        kind: KIND,
        peer,
        peer_ip,
        rtsp_port: wfd_core::ie::DEFAULT_RTSP_PORT,
        device_name: name,
    });
    Ok(())
}

fn winrt_thread(
    identity: ReceiverIdentity,
    events: mpsc::UnboundedSender<TransportEvent>,
    commands: std::sync::mpsc::Receiver<TransportCommand>,
) -> windows::core::Result<()> {
    let publisher = WiFiDirectAdvertisementPublisher::new()?;
    let adv = publisher.Advertisement()?;
    adv.SetListenStateDiscoverability(WiFiDirectAdvertisementListenStateDiscoverability::Normal)?;
    let ie = WiFiDirectInformationElement::new()?;
    ie.SetOui(&buffer(&wfd_core::ie::WFA_OUI)?)?;
    ie.SetOuiType(wfd_core::ie::WFD_OUI_TYPE)?;
    ie.SetValue(&buffer(&wfd_core::ie::sink_subelements(wfd_core::ie::DEFAULT_RTSP_PORT, identity.uibc))?)?;
    adv.InformationElements()?.Append(&ie)?;

    let devices: Devices = Arc::new(Mutex::new(HashMap::new()));
    let listener = WiFiDirectConnectionListener::new()?;
    {
        let events = events.clone();
        let devices = devices.clone();
        listener.ConnectionRequested(&TypedEventHandler::new(
            move |_: Ref<WiFiDirectConnectionListener>, args: Ref<WiFiDirectConnectionRequestedEventArgs>| {
                if let Err(e) = handle_request(args.ok()?, &events, &devices) {
                    warn!("Wi-Fi Direct request failed: {e}");
                }
                Ok(())
            },
        ))?;
    }
    publisher.Start()?;
    let _ = events.send(TransportEvent::Status {
        kind: KIND,
        state: TransportState::Ready { detail: format!("discoverable as \"{}\"", identity.friendly_name) },
    });
    while let Ok(cmd) = commands.recv() {
        match cmd {
            TransportCommand::SessionEnded { peer } => {
                if let Some(d) = devices.lock().unwrap().remove(&peer) {
                    let _ = d.Close();
                }
            }
            TransportCommand::Shutdown => break,
        }
    }
    let _ = publisher.Stop();
    drop(listener);
    devices.lock().unwrap().clear();
    Ok(())
}

impl DiscoveryTransport for WindowsP2p {
    fn kind(&self) -> TransportKind {
        KIND
    }

    fn run(
        self: Box<Self>,
        events: mpsc::UnboundedSender<TransportEvent>,
        mut commands: mpsc::UnboundedReceiver<TransportCommand>,
    ) -> BoxFuture<'static, Result<(), TransportError>> {
        Box::pin(async move {
            let (ctx, crx) = std::sync::mpsc::channel();
            let (done_tx, done_rx) = oneshot::channel();
            let ev = events.clone();
            let identity = self.identity;
            std::thread::Builder::new()
                .name("wfd-winrt".into())
                .spawn(move || {
                    let r = winrt_thread(identity, ev.clone(), crx).map_err(|e| e.to_string());
                    if let Err(e) = &r {
                        let _ = ev.send(TransportEvent::Status {
                            kind: KIND,
                            state: TransportState::Unavailable { reason: format!("Wi-Fi Direct (WinRT) failed: {e}") },
                        });
                    }
                    let _ = done_tx.send(r);
                })
                .map_err(TransportError::Io)?;
            let mut done = done_rx;
            loop {
                tokio::select! {
                    r = &mut done => return r.unwrap_or(Ok(())).map_err(TransportError::Unavailable),
                    cmd = commands.recv() => {
                        let shutdown = matches!(cmd, None | Some(TransportCommand::Shutdown));
                        let _ = ctx.send(cmd.unwrap_or(TransportCommand::Shutdown));
                        if shutdown {
                            return done.await.unwrap_or(Ok(())).map_err(TransportError::Unavailable);
                        }
                    }
                }
            }
        })
    }
}
