//! The receiver engine: runs the discovery transports, the RTSP session, the
//! media pipeline and the UIBC client on a tokio runtime, and publishes a
//! snapshot ([`Shared`]) that the UI reads every frame.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use wfd_core::driver::{SinkCommand, run_sink};
use wfd_core::session::{Negotiated, SessionEvent, SinkConfig};
use wfd_core::uibc::{InputEvent, UibcCapability, UibcEncoder};
use wfd_core::video::VideoFormats;
use wfd_media::{DecoderChoice, FrameSlot, MediaEvent, MediaReceiver, MediaStats, ReceiverConfig, free_udp_port};
use wfd_transport::direct::DirectTransport;
use wfd_transport::{
    DiscoveryTransport, PairingMethod, PairingReply, PeerId, ReceiverIdentity, TransportCommand, TransportEvent, TransportKind,
    TransportState,
};

use crate::config::{Decoder, Settings};
use crate::share::{self, ShareServer};
use crate::sim::{InputLog, SimOptions, SimPhone};

pub type Repaint = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug)]
pub enum EngineCmd {
    PairingReply {
        id: u64,
        reply: PairingReply,
    },
    Disconnect,
    StartSimulation,
    StopSimulation,
    SetVolume(f64),
    SetMuted(bool),
    RequestIdr,
    /// New settings (take effect for the next session / media start).
    UpdateSettings(Settings),
    /// Re-advertise with the current name and transport choices.
    RestartTransports,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Connecting,
    Negotiating,
    Playing,
    Paused,
}

#[derive(Debug, Clone)]
pub struct PairingPrompt {
    pub id: u64,
    pub kind: TransportKind,
    pub device_name: String,
    pub method: PairingMethod,
}

#[derive(Debug, Clone)]
pub struct UibcView {
    pub description: String,
    pub relative_mouse: bool,
    /// Coordinate space (negotiated stream size).
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct SessionView {
    pub phase: Phase,
    pub kind: Option<TransportKind>,
    pub peer_name: String,
    pub video: Option<String>,
    pub video_size: Option<(u32, u32)>,
    pub audio: Option<String>,
    pub uibc: Option<UibcView>,
    pub since: Option<Instant>,
}

impl Default for SessionView {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            kind: None,
            peer_name: String::new(),
            video: None,
            video_size: None,
            audio: None,
            uibc: None,
            since: None,
        }
    }
}

/// What the UI shows. Cloned out under a short lock each frame.
#[derive(Clone, Default)]
pub struct Shared {
    pub receiver_name: String,
    pub transports: BTreeMap<String, (TransportKind, TransportState)>,
    pub session: SessionView,
    pub prompts: Vec<PairingPrompt>,
    pub log: VecDeque<(Instant, String)>,
    pub last_error: Option<String>,
    pub sim_running: bool,
    pub media: Option<MediaStats>,
    /// Send input here while UIBC is connected.
    pub input: Option<mpsc::UnboundedSender<InputEvent>>,
    pub sim_inputs: Option<InputLog>,
    /// Where the connected device finds the file-sharing page.
    pub share_url: Option<String>,
    /// Files offered to the device (the UI adds and removes them).
    pub shared_files: share::Files,
    /// Files the device sent in this session.
    pub received: Vec<std::path::PathBuf>,
}

impl Shared {
    fn log(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        info!("{msg}");
        self.log.push_back((Instant::now(), msg));
        while self.log.len() > 200 {
            self.log.pop_front();
        }
    }
}

#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub settings: Settings,
    /// Connect straight to a source's RTSP address (no discovery).
    pub connect: Option<SocketAddr>,
    pub rtp_port: u16,
    pub simulate_on_start: bool,
    /// Override the audio sink ("fakesink" for headless tests).
    pub audio_sink: Option<String>,
}

pub struct EngineHandle {
    pub shared: Arc<Mutex<Shared>>,
    pub frames: FrameSlot,
    tx: mpsc::UnboundedSender<EngineCmd>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl EngineHandle {
    pub fn start(opts: EngineOptions, repaint: Repaint) -> Self {
        let shared = Arc::new(Mutex::new(Shared { receiver_name: opts.settings.name.clone(), ..Default::default() }));
        let frames = FrameSlot::new();
        let (tx, rx) = mpsc::unbounded_channel();
        let thread = {
            let shared = shared.clone();
            let frames = frames.clone();
            std::thread::Builder::new()
                .name("wfd-engine".into())
                .spawn(move || {
                    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(3).enable_all().build().expect("tokio runtime");
                    rt.block_on(Engine::run(Engine::new(opts, shared, frames, repaint), rx));
                    rt.shutdown_timeout(Duration::from_secs(2));
                })
                .expect("engine thread")
        };
        Self { shared, frames, tx, thread: Some(thread) }
    }

    pub fn send(&self, cmd: EngineCmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn snapshot(&self) -> Shared {
        self.shared.lock().unwrap().clone()
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(EngineCmd::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct ActiveSession {
    id: u64,
    kind: TransportKind,
    peer: PeerId,
    peer_ip: IpAddr,
    commands: mpsc::UnboundedSender<SinkCommand>,
    media: Option<MediaReceiver>,
    negotiated: Option<Negotiated>,
    uibc_task: Option<tokio::task::JoinHandle<()>>,
    playing_since: Option<Instant>,
    last_idr: Instant,
}

struct Engine {
    opts: EngineOptions,
    shared: Arc<Mutex<Shared>>,
    frames: FrameSlot,
    repaint: Repaint,
    transports: HashMap<TransportKind, mpsc::UnboundedSender<TransportCommand>>,
    transport_tasks: Vec<tokio::task::JoinHandle<()>>,
    session: Option<ActiveSession>,
    next_session: u64,
    pairing: HashMap<u64, oneshot::Sender<PairingReply>>,
    next_prompt: u64,
    sim: Option<SimPhone>,
    sev_tx: mpsc::UnboundedSender<(u64, SessionEvent)>,
    mev_tx: mpsc::UnboundedSender<(u64, MediaEvent)>,
    tev_tx: mpsc::UnboundedSender<TransportEvent>,
    volume: f64,
    muted: bool,
    share: Option<ShareServer>,
}

impl Engine {
    fn new(opts: EngineOptions, shared: Arc<Mutex<Shared>>, frames: FrameSlot, repaint: Repaint) -> (Self, Channels) {
        let (sev_tx, sev_rx) = mpsc::unbounded_channel();
        let (mev_tx, mev_rx) = mpsc::unbounded_channel();
        let (tev_tx, tev_rx) = mpsc::unbounded_channel();
        let volume = opts.settings.volume;
        let muted = opts.settings.muted;
        (
            Self {
                opts,
                shared,
                frames,
                repaint,
                transports: HashMap::new(),
                transport_tasks: Vec::new(),
                session: None,
                next_session: 1,
                pairing: HashMap::new(),
                next_prompt: 1,
                sim: None,
                sev_tx,
                mev_tx,
                tev_tx,
                volume,
                muted,
                share: None,
            },
            Channels { sev_rx, mev_rx, tev_rx },
        )
    }

    fn update(&self, f: impl FnOnce(&mut Shared)) {
        f(&mut self.shared.lock().unwrap());
        (self.repaint)();
    }

    fn log(&self, msg: impl Into<String>) {
        let msg = msg.into();
        self.update(|s| s.log(msg));
    }

    fn spawn_transport(&mut self, t: Box<dyn DiscoveryTransport>) {
        let kind = t.kind();
        let (ctx, crx) = mpsc::unbounded_channel();
        self.transports.insert(kind, ctx);
        let events = self.tev_tx.clone();
        self.transport_tasks.push(tokio::spawn(async move {
            if let Err(e) = t.run(events, crx).await {
                warn!(%kind, "transport stopped: {e}");
            }
        }));
    }

    fn start_transports(&mut self) {
        let s = &self.opts.settings;
        let identity = ReceiverIdentity { uibc: s.uibc, advertise_mice: s.infrastructure, ..ReceiverIdentity::new(&s.name) };
        let wifi_direct = s.wifi_direct;
        let infrastructure = s.infrastructure;
        if wifi_direct {
            match (wfd_transport::wifi_direct_support(), wfd_transport::wifi_direct_transport(&identity)) {
                (Ok(()), Some(t)) => self.spawn_transport(t),
                (Err(reason), _) => self.set_transport(TransportKind::WifiDirect, TransportState::Unavailable { reason: reason.into() }),
                (Ok(()), None) => {}
            }
        }
        if infrastructure && let Some(t) = wfd_transport::infrastructure_transport(&identity) {
            self.spawn_transport(t);
        }
        if let Some(addr) = self.opts.connect {
            self.spawn_transport(Box::new(DirectTransport { addr, device_name: format!("Source at {addr}") }));
        }
    }

    fn set_transport(&self, kind: TransportKind, state: TransportState) {
        self.update(|s| {
            s.transports.insert(kind.label().to_string(), (kind, state));
        });
    }

    async fn run(self_and_channels: (Self, Channels), mut cmds: mpsc::UnboundedReceiver<EngineCmd>) {
        let (mut eng, mut ch) = self_and_channels;
        eng.start_transports();
        if eng.opts.simulate_on_start {
            eng.start_simulation().await;
        }
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                Some(ev) = ch.tev_rx.recv() => eng.on_transport_event(ev).await,
                Some((id, ev)) = ch.sev_rx.recv() => eng.on_session_event(id, ev).await,
                Some((id, ev)) = ch.mev_rx.recv() => eng.on_media_event(id, ev),
                cmd = cmds.recv() => match cmd {
                    None | Some(EngineCmd::Shutdown) => break,
                    Some(c) => eng.on_command(c).await,
                },
                _ = tick.tick() => eng.on_tick(),
            }
        }
        eng.shutdown().await;
    }

    async fn shutdown(&mut self) {
        if let Some(sim) = self.sim.take() {
            sim.stop();
        }
        if let Some(s) = &self.session {
            let _ = s.commands.send(SinkCommand::Teardown);
            // give M8 a moment to go out
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        self.end_session("receiver closed");
        for t in self.transports.values() {
            let _ = t.send(TransportCommand::Shutdown);
        }
        for t in self.transport_tasks.drain(..) {
            let _ = tokio::time::timeout(Duration::from_secs(3), t).await;
        }
    }

    async fn on_command(&mut self, cmd: EngineCmd) {
        match cmd {
            EngineCmd::PairingReply { id, reply } => {
                if let Some(tx) = self.pairing.remove(&id) {
                    let _ = tx.send(reply);
                }
                self.update(|s| s.prompts.retain(|p| p.id != id));
            }
            EngineCmd::Disconnect => {
                if let Some(s) = &self.session {
                    let _ = s.commands.send(SinkCommand::Teardown);
                }
            }
            EngineCmd::StartSimulation => self.start_simulation().await,
            EngineCmd::StopSimulation => {
                if let Some(sim) = self.sim.take() {
                    sim.stop();
                }
                self.update(|s| {
                    s.sim_running = false;
                    s.sim_inputs = None;
                });
            }
            EngineCmd::SetVolume(v) => {
                self.volume = v;
                if let Some(m) = self.session.as_ref().and_then(|s| s.media.as_ref()) {
                    m.set_volume(v);
                }
            }
            EngineCmd::SetMuted(m) => {
                self.muted = m;
                if let Some(media) = self.session.as_ref().and_then(|s| s.media.as_ref()) {
                    media.set_muted(m);
                }
            }
            EngineCmd::RequestIdr => self.request_idr(true),
            EngineCmd::UpdateSettings(st) => {
                self.opts.settings = st;
                if !self.opts.settings.file_share {
                    self.stop_share();
                } else if self.session.as_ref().is_some_and(|s| s.playing_since.is_some()) {
                    self.start_share().await;
                }
            }
            EngineCmd::RestartTransports => {
                for t in self.transports.values() {
                    let _ = t.send(TransportCommand::Shutdown);
                }
                self.transports.clear();
                for t in self.transport_tasks.drain(..) {
                    let _ = tokio::time::timeout(Duration::from_secs(3), t).await;
                }
                let name = self.opts.settings.name.clone();
                self.update(|s| {
                    s.transports.clear();
                    s.receiver_name = name;
                });
                self.start_transports();
                self.log("discovery restarted");
            }
            EngineCmd::Shutdown => {}
        }
    }

    async fn start_simulation(&mut self) {
        if self.sim.is_some() {
            return;
        }
        let shared = self.shared.clone();
        let repaint = self.repaint.clone();
        let log = Arc::new(move |m: String| {
            shared.lock().unwrap().log(m);
            repaint();
        });
        let limits = self.opts.settings.max_video.limits();
        let opts = SimOptions { max_width: limits.max_width, max_height: limits.max_height, max_fps: limits.max_fps.min(30), uibc: true };
        match SimPhone::start(opts, log).await {
            Ok(sim) => {
                let addr = sim.rtsp_addr;
                let inputs = sim.inputs.clone();
                self.sim = Some(sim);
                self.update(|s| {
                    s.sim_running = true;
                    s.sim_inputs = Some(inputs);
                });
                self.log(format!("simulated phone listening on {addr}"));
                self.start_session(TransportKind::Direct, PeerId("simulator".into()), addr.ip(), addr.port(), "Simulated phone".into());
            }
            Err(e) => self.log(format!("cannot start the simulated phone: {e}")),
        }
    }

    async fn on_transport_event(&mut self, ev: TransportEvent) {
        match ev {
            TransportEvent::Status { kind, state } => {
                let changed = self.shared.lock().unwrap().transports.get(kind.label()).map(|(_, s)| s != &state).unwrap_or(true);
                if changed {
                    match &state {
                        TransportState::Ready { detail } => self.log(format!("{kind}: ready ({detail})")),
                        TransportState::Unavailable { reason } => self.log(format!("{kind}: unavailable: {reason}")),
                        TransportState::Error { message } => self.log(format!("{kind}: {message}")),
                        TransportState::Busy { detail } => self.log(format!("{kind}: {detail}")),
                        TransportState::Starting => {}
                    }
                }
                self.set_transport(kind, state);
            }
            TransportEvent::PairingRequest(req) => {
                let auto = self.opts.settings.auto_accept && !matches!(req.method, PairingMethod::EnterPin);
                if auto {
                    if let PairingMethod::DisplayPin(pin) = &req.method {
                        self.log(format!("{} connecting: enter PIN {pin} on it", req.device_name));
                    } else {
                        self.log(format!("{} connecting (auto-accepted)", req.device_name));
                    }
                    let _ = req.reply.send(PairingReply::Accept);
                    return;
                }
                let id = self.next_prompt;
                self.next_prompt += 1;
                let prompt = PairingPrompt { id, kind: req.kind, device_name: req.device_name.clone(), method: req.method.clone() };
                self.pairing.insert(id, req.reply);
                self.log(format!("{} wants to connect ({})", prompt.device_name, prompt.kind));
                self.update(|s| s.prompts.push(prompt));
            }
            TransportEvent::PeerConnected { kind, peer, peer_ip, rtsp_port, device_name } => {
                self.start_session(kind, peer, peer_ip, rtsp_port, device_name);
            }
            TransportEvent::PeerDisconnected { peer, reason, .. } => {
                if self.session.as_ref().is_some_and(|s| s.peer == peer) {
                    self.log(format!("source disconnected: {reason}"));
                    if let Some(s) = &self.session {
                        let _ = s.commands.send(SinkCommand::Teardown);
                    }
                    self.end_session(&reason);
                }
            }
        }
    }

    fn start_session(&mut self, kind: TransportKind, peer: PeerId, peer_ip: IpAddr, rtsp_port: u16, name: String) {
        if let Some(old) = &self.session {
            let _ = old.commands.send(SinkCommand::Teardown);
            self.end_session("replaced by a new connection");
        }
        let id = self.next_session;
        self.next_session += 1;
        let s = &self.opts.settings;
        let rtp_port = free_udp_port(self.opts.rtp_port);
        let mut cfg = SinkConfig::new(s.name.clone(), rtp_port);
        cfg.video = VideoFormats::sink_capabilities(s.max_video.limits());
        if !s.uibc {
            cfg.uibc = None;
        }
        cfg.source_host = peer_ip.to_string();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let sev = self.sev_tx.clone();
        let addr = SocketAddr::new(peer_ip, rtsp_port);
        self.log(format!("connecting to {name} at {addr} ({kind})"));
        tokio::spawn(async move {
            // The source's RTSP server can take a moment after the link is up.
            let deadline = Instant::now() + Duration::from_secs(20);
            let stream = loop {
                match tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(addr)).await {
                    Ok(Ok(s)) => break s,
                    Ok(Err(e)) if Instant::now() < deadline => debug!("RTSP connect to {addr}: {e}; retrying"),
                    Err(_) if Instant::now() < deadline => {}
                    other => {
                        let reason = match other {
                            Ok(Err(e)) => format!("cannot reach the source's RTSP server at {addr}: {e}"),
                            _ => format!("timed out connecting to {addr}"),
                        };
                        let _ = sev.send((id, SessionEvent::Closed { reason }));
                        return;
                    }
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
            };
            let (etx, mut erx) = mpsc::unbounded_channel();
            let fwd = {
                let sev = sev.clone();
                tokio::spawn(async move {
                    while let Some(e) = erx.recv().await {
                        let _ = sev.send((id, e));
                    }
                })
            };
            if let Err(e) = run_sink(stream, cfg, etx, cmd_rx).await {
                let _ = sev.send((id, SessionEvent::Closed { reason: format!("RTSP connection error: {e}") }));
            }
            let _ = fwd.await;
        });
        self.session = Some(ActiveSession {
            id,
            kind,
            peer,
            peer_ip,
            commands: cmd_tx,
            media: None,
            negotiated: None,
            uibc_task: None,
            playing_since: None,
            last_idr: Instant::now(),
        });
        self.update(|s| {
            s.session = SessionView {
                phase: Phase::Connecting,
                kind: Some(kind),
                peer_name: name,
                since: Some(Instant::now()),
                ..Default::default()
            };
            s.last_error = None;
        });
    }

    fn end_session(&mut self, reason: &str) {
        let Some(mut s) = self.session.take() else { return };
        if let Some(t) = s.uibc_task.take() {
            t.abort();
        }
        if let Some(m) = s.media.take() {
            m.stop();
        }
        self.frames.clear();
        if let Some(t) = self.transports.get(&s.kind) {
            let _ = t.send(TransportCommand::SessionEnded { peer: s.peer.clone() });
        }
        self.stop_share();
        if s.kind == TransportKind::Direct && s.peer.0 == "simulator" {
            self.sim = None;
        }
        let reason = reason.to_string();
        let was_playing = s.playing_since.is_some();
        self.update(|sh| {
            sh.log(format!("session ended: {reason}"));
            if !was_playing && !reason.contains("closed by receiver") && !reason.contains("teardown") {
                sh.last_error = Some(reason.clone());
            }
            sh.session = SessionView::default();
            sh.media = None;
            sh.input = None;
            if s.peer.0 == "simulator" {
                sh.sim_running = false;
            }
        });
    }

    fn request_idr(&mut self, force: bool) {
        if let Some(s) = self.session.as_mut()
            && (force || s.last_idr.elapsed() > Duration::from_millis(1000))
        {
            s.last_idr = Instant::now();
            let _ = s.commands.send(SinkCommand::RequestIdr);
        }
    }

    async fn on_session_event(&mut self, id: u64, ev: SessionEvent) {
        if self.session.as_ref().map(|s| s.id) != Some(id) {
            return; // stale
        }
        match ev {
            SessionEvent::Negotiated(n) => {
                let video = n.video.map(|v| v.to_string());
                let size = n.video.map(|v| (v.width, v.height));
                let audio = n.audio.map(|a| a.to_string());
                self.log(format!(
                    "negotiated video {}, audio {}{}",
                    video.as_deref().unwrap_or("none"),
                    audio.as_deref().unwrap_or("none"),
                    if n.uibc_enabled { ", input back-channel on" } else { "" }
                ));
                let restart_uibc = self.session.as_ref().is_some_and(|s| s.playing_since.is_some());
                if let Some(s) = self.session.as_mut() {
                    s.negotiated = Some(n);
                }
                self.update(|s| {
                    if s.session.phase == Phase::Connecting {
                        s.session.phase = Phase::Negotiating;
                    }
                    s.session.video = video;
                    s.session.video_size = size;
                    s.session.audio = audio;
                });
                if restart_uibc {
                    self.start_uibc();
                }
            }
            SessionEvent::PrepareMedia { rtp_port } => self.start_media(rtp_port),
            SessionEvent::Playing { .. } => {
                if let Some(s) = self.session.as_mut() {
                    s.playing_since = Some(Instant::now());
                }
                self.update(|s| s.session.phase = Phase::Playing);
                self.start_uibc();
                self.start_share().await;
            }
            SessionEvent::Paused => self.update(|s| s.session.phase = Phase::Paused),
            SessionEvent::UibcSetting { enabled } => {
                if let Some(n) = self.session.as_mut().and_then(|s| s.negotiated.as_mut()) {
                    n.uibc_enabled = enabled;
                }
                self.start_uibc();
            }
            SessionEvent::Closed { reason } => self.end_session(&reason),
            SessionEvent::Warning(w) => self.log(format!("RTSP: {w}")),
        }
    }

    /// Serves the file-sharing page on the link to the connected device.
    async fn start_share(&mut self) {
        let Some(peer) = self.session.as_ref().map(|s| s.peer_ip) else { return };
        if self.share.is_some() || !self.opts.settings.file_share {
            return;
        }
        // the local address on the route to the device (nothing is sent)
        let local = std::net::UdpSocket::bind(("0.0.0.0", 0)).and_then(|s| {
            s.connect((peer, 9))?;
            s.local_addr()
        });
        let files = self.shared.lock().unwrap().shared_files.clone();
        let shared = self.shared.clone();
        let repaint = self.repaint.clone();
        let on_received: share::OnReceived = Arc::new(move |path, size| {
            {
                let mut s = shared.lock().unwrap();
                s.log(format!("received {} ({})", path.display(), share::human_size(size)));
                s.received.push(path);
            }
            repaint();
        });
        let result = match local {
            Ok(local) => ShareServer::start(local.ip(), peer, files, share::download_folder(), on_received).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(server) => {
                let url = server.url.clone();
                self.share = Some(server);
                self.log(format!("file sharing at {url}"));
                self.update(|s| s.share_url = Some(url));
            }
            Err(e) => self.log(format!("file sharing unavailable: {e}")),
        }
    }

    fn stop_share(&mut self) {
        if self.share.take().is_some() {
            self.update(|s| {
                s.share_url = None;
                // the next device must not see files meant for this one
                s.shared_files.lock().unwrap().clear();
                s.received.clear();
            });
        }
    }

    fn start_media(&mut self, rtp_port: u16) {
        let Some(s) = self.session.as_mut() else { return };
        if s.media.is_some() {
            return;
        }
        let st = &self.opts.settings;
        let cfg = ReceiverConfig {
            rtp_port,
            jitter_latency_ms: st.jitter_ms,
            av_sync: st.av_sync,
            decoder: match st.decoder {
                Decoder::Auto => DecoderChoice::Auto,
                Decoder::Software => DecoderChoice::Software,
                Decoder::Hardware => DecoderChoice::Hardware,
            },
            enable_audio: true,
            volume: self.volume,
            muted: self.muted,
            audio_sink: self.opts.audio_sink.clone(),
        };
        let id = s.id;
        let mev = self.mev_tx.clone();
        let repaint = self.repaint.clone();
        match MediaReceiver::start(
            cfg,
            self.frames.clone(),
            Arc::new(move |e| {
                let _ = mev.send((id, e));
            }),
            Arc::new(move || repaint()),
        ) {
            Ok(m) => {
                s.media = Some(m);
            }
            Err(e) => {
                let msg = format!("cannot start video/audio: {e}");
                self.update(|s| s.last_error = Some(msg.clone()));
                self.log(msg);
            }
        }
    }

    fn start_uibc(&mut self) {
        let enabled_by_user = self.opts.settings.uibc;
        let Some(s) = self.session.as_mut() else { return };
        if let Some(t) = s.uibc_task.take() {
            t.abort();
        }
        let Some(n) = &s.negotiated else { return };
        let (Some(cap), true, true) = (n.uibc.clone(), n.uibc_enabled, enabled_by_user) else {
            self.update(|sh| {
                sh.session.uibc = None;
                sh.input = None;
            });
            return;
        };
        let Some(port) = cap.port else { return };
        let addr = SocketAddr::new(s.peer_ip, port);
        let encoder = UibcEncoder::new(&cap);
        let (w, h) = n.video.map(|v| (v.width, v.height)).unwrap_or((1920, 1080));
        let view = UibcView { description: encoder.describe(), relative_mouse: encoder.uses_relative_mouse(), width: w, height: h };
        let (tx, rx) = mpsc::unbounded_channel();
        let shared = self.shared.clone();
        let repaint = self.repaint.clone();
        s.uibc_task = Some(tokio::spawn(uibc_client(addr, cap, encoder, rx, shared, repaint)));
        self.update(|sh| {
            sh.session.uibc = Some(view.clone());
            sh.input = Some(tx);
        });
    }

    fn on_media_event(&mut self, id: u64, ev: MediaEvent) {
        if self.session.as_ref().map(|s| s.id) != Some(id) {
            return;
        }
        match ev {
            MediaEvent::FirstFrame { width, height } => self.log(format!("video started {width}x{height}")),
            MediaEvent::FormatChanged { width, height } => self.log(format!("video size changed to {width}x{height}")),
            MediaEvent::AudioStarted { caps } => self.log(format!("audio started ({caps})")),
            MediaEvent::PacketLoss => self.request_idr(false),
            MediaEvent::Error(e) => {
                self.log(format!("media error: {e}"));
                self.request_idr(false);
            }
            MediaEvent::Warning(w) => debug!("media warning: {w}"),
        }
    }

    fn on_tick(&mut self) {
        // drop prompts whose transport gave up waiting
        let expired: Vec<u64> = self.pairing.iter().filter(|(_, tx)| tx.is_closed()).map(|(id, _)| *id).collect();
        for id in &expired {
            self.pairing.remove(id);
        }
        let stats = self.session.as_ref().and_then(|s| s.media.as_ref()).map(|m| m.stats());
        // no picture yet, or picture stalled: ask for a key frame
        let mut want_idr = false;
        if let (Some(s), Some(st)) = (&self.session, &stats)
            && let Some(since) = s.playing_since
        {
            let stalled = st.last_frame_at.map_or(since.elapsed() > Duration::from_millis(1500), |t| t.elapsed() > Duration::from_secs(2));
            want_idr = stalled;
        }
        if want_idr {
            self.request_idr(false);
        }
        self.update(|s| {
            s.prompts.retain(|p| !expired.contains(&p.id));
            s.media = stats;
        });
    }
}

struct Channels {
    sev_rx: mpsc::UnboundedReceiver<(u64, SessionEvent)>,
    mev_rx: mpsc::UnboundedReceiver<(u64, MediaEvent)>,
    tev_rx: mpsc::UnboundedReceiver<TransportEvent>,
}

async fn uibc_client(
    addr: SocketAddr,
    cap: UibcCapability,
    mut encoder: UibcEncoder,
    mut rx: mpsc::UnboundedReceiver<InputEvent>,
    shared: Arc<Mutex<Shared>>,
    repaint: Repaint,
) {
    let mut stream = None;
    for _ in 0..10 {
        match tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(addr)).await {
            Ok(Ok(s)) => {
                stream = Some(s);
                break;
            }
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    let Some(mut stream) = stream else {
        shared.lock().unwrap().log(format!("input back-channel: cannot connect to {addr}"));
        repaint();
        return;
    };
    let _ = stream.set_nodelay(true);
    shared.lock().unwrap().log(format!("input back-channel connected ({}; {cap})", encoder.describe()));
    repaint();
    for p in encoder.initial_packets() {
        if stream.write_all(&p).await.is_err() {
            return;
        }
    }
    while let Some(ev) = rx.recv().await {
        let packets = encoder.encode(&ev);
        if packets.is_empty() {
            continue;
        }
        let bytes: Vec<u8> = packets.concat();
        if let Err(e) = stream.write_all(&bytes).await {
            shared.lock().unwrap().log(format!("input back-channel closed: {e}"));
            repaint();
            return;
        }
    }
}
