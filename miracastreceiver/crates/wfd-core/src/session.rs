//! Sans-IO Wi-Fi Display sink session (the sink is the RTSP TCP client).
//!
//! ```text
//!  source                                   sink
//!    | M1 OPTIONS *                  ->       |
//!    |                               <-  200  |  Public: org.wfa.wfd1.0, GET_PARAMETER, SET_PARAMETER
//!    |                               <- M2 OPTIONS *
//!    | 200 Public: ...               ->       |
//!    | M3 GET_PARAMETER (names)      ->       |
//!    |                               <-  200 + values
//!    | M4 SET_PARAMETER (selection)  ->       |
//!    |                               <-  200  |
//!    | M5 SET_PARAMETER trigger SETUP ->      |
//!    |                               <-  200  |
//!    |                               <- M6 SETUP (client_port)
//!    | 200 Session/Transport         ->       |
//!    |                               <- M7 PLAY
//!    | 200                           ->       |   ... RTP/MPEG-TS on UDP ...
//!    | M16 GET_PARAMETER (empty)     ->       |   keep-alive, every < timeout
//!    |                               <-  200  |
//!    |                               <- M13 SET_PARAMETER wfd_idr_request (on demand)
//!    | M5 trigger TEARDOWN           ->       |
//!    |                               <- M8 TEARDOWN
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tracing::{debug, warn};

use crate::audio::{AudioCodecs, AudioMode};
use crate::params::{self, ClientRtpPorts, Transport};
use crate::rtsp::{Message, Request, Response, WFD_REQUIRE};
use crate::uibc::UibcCapability;
use crate::video::{SinkVideoLimits, VideoFormats, VideoMode};

/// Time allowed between connecting and the source's M1.
const NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(30);
/// Default RTSP session timeout when the source does not state one.
const DEFAULT_SESSION_TIMEOUT: u64 = 60;
/// Extra slack on top of the session timeout before giving up.
const KEEPALIVE_GRACE: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct SinkConfig {
    pub friendly_name: String,
    pub manufacturer: String,
    pub model: String,
    pub version: String,
    /// UDP port we receive RTP on.
    pub rtp_port: u16,
    pub video: VideoFormats,
    pub audio: AudioCodecs,
    /// `None` = don't offer UIBC.
    pub uibc: Option<UibcCapability>,
    /// Used for the SETUP URL when the source gives no presentation URL.
    pub source_host: String,
}

impl SinkConfig {
    pub fn new(friendly_name: impl Into<String>, rtp_port: u16) -> Self {
        Self {
            friendly_name: friendly_name.into(),
            manufacturer: "Open Source".into(),
            model: "Miracast Receiver".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            rtp_port,
            video: VideoFormats::sink_capabilities(SinkVideoLimits::default()),
            audio: AudioCodecs::sink_capabilities(),
            uibc: Some(UibcCapability::sink_default()),
            source_host: "localhost".into(),
        }
    }
}

/// Parameters agreed in M4 (or a later re-negotiation).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Negotiated {
    pub video: Option<VideoMode>,
    pub video_formats: Option<VideoFormats>,
    pub audio: Option<AudioMode>,
    pub presentation_url: Option<String>,
    pub client_rtp_port: Option<u16>,
    pub uibc: Option<UibcCapability>,
    pub uibc_enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// The source finished capability negotiation (M3) and sent its choice (M4).
    Negotiated(Negotiated),
    /// The sink is about to send SETUP: start receiving RTP on this port now
    /// so the first IDR frame isn't lost.
    PrepareMedia {
        rtp_port: u16,
    },
    Playing {
        session_id: String,
    },
    Paused,
    UibcSetting {
        enabled: bool,
    },
    Closed {
        reason: String,
    },
    Warning(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    Send(Message),
    Event(SessionEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Options,
    Setup,
    Play,
    Pause,
    Teardown,
    Idr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkState {
    /// TCP connected, waiting for M1.
    WaitingM1,
    /// M1 answered, M2 sent; waiting for M3/M4.
    Capabilities,
    /// M4 applied; waiting for the SETUP trigger.
    Negotiated,
    SetupSent,
    PlaySent,
    Playing,
    Paused,
    TeardownSent,
    Closed,
}

pub struct SinkSession {
    cfg: SinkConfig,
    state: SinkState,
    next_cseq: u32,
    pending: HashMap<u32, Pending>,
    session_id: Option<String>,
    session_timeout: Duration,
    negotiated: Negotiated,
    last_rx: Instant,
    teardown_deadline: Option<Instant>,
    source_methods: Vec<String>,
}

impl SinkSession {
    pub fn new(cfg: SinkConfig, now: Instant) -> Self {
        Self {
            cfg,
            state: SinkState::WaitingM1,
            next_cseq: 1,
            pending: HashMap::new(),
            session_id: None,
            session_timeout: Duration::from_secs(DEFAULT_SESSION_TIMEOUT),
            negotiated: Negotiated::default(),
            last_rx: now,
            teardown_deadline: None,
            source_methods: Vec::new(),
        }
    }

    pub fn state(&self) -> SinkState {
        self.state
    }

    pub fn negotiated(&self) -> &Negotiated {
        &self.negotiated
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn config(&self) -> &SinkConfig {
        &self.cfg
    }

    fn take_cseq(&mut self) -> u32 {
        let c = self.next_cseq;
        self.next_cseq += 1;
        c
    }

    fn presentation_url(&self) -> String {
        self.negotiated.presentation_url.clone().unwrap_or_else(|| format!("rtsp://{}/wfd1.0/streamid=0", self.cfg.source_host))
    }

    fn request(&mut self, method: &str, uri: &str, kind: Pending) -> Request {
        let cseq = self.take_cseq();
        self.pending.insert(cseq, kind);
        let mut r = Request::new(method, uri, cseq);
        if let Some(id) = &self.session_id
            && kind != Pending::Options
            && kind != Pending::Setup
        {
            r.headers.set("Session", id.clone());
        }
        r
    }

    fn close(&mut self, reason: impl Into<String>) -> Output {
        self.state = SinkState::Closed;
        Output::Event(SessionEvent::Closed { reason: reason.into() })
    }

    /// When the driver should call [`SinkSession::on_timer`] next.
    pub fn deadline(&self) -> Option<Instant> {
        match self.state {
            SinkState::Closed => None,
            SinkState::TeardownSent => self.teardown_deadline,
            SinkState::WaitingM1 | SinkState::Capabilities | SinkState::Negotiated => Some(self.last_rx + NEGOTIATION_TIMEOUT),
            _ => Some(self.last_rx + self.session_timeout + KEEPALIVE_GRACE),
        }
    }

    pub fn on_timer(&mut self, now: Instant) -> Vec<Output> {
        match self.deadline() {
            Some(d) if now >= d => {
                let what = if self.state == SinkState::TeardownSent {
                    "teardown complete (source did not answer)"
                } else if matches!(self.state, SinkState::Playing | SinkState::Paused) {
                    "source stopped sending keep-alives"
                } else {
                    "source stopped responding during negotiation"
                };
                vec![self.close(what)]
            }
            _ => Vec::new(),
        }
    }

    /// Ask the source for an IDR frame (M13), e.g. after packet loss.
    pub fn request_idr(&mut self) -> Vec<Output> {
        if !matches!(self.state, SinkState::Playing) {
            return Vec::new();
        }
        let req =
            self.request("SET_PARAMETER", "rtsp://localhost/wfd1.0", Pending::Idr).with_body("text/parameters", "wfd_idr_request\r\n");
        vec![Output::Send(req.into())]
    }

    /// Sink-initiated teardown (M8).
    pub fn teardown(&mut self, now: Instant) -> Vec<Output> {
        match self.state {
            SinkState::Closed | SinkState::TeardownSent => Vec::new(),
            SinkState::WaitingM1 | SinkState::Capabilities | SinkState::Negotiated => {
                vec![self.close("closed by receiver")]
            }
            _ => {
                let url = self.presentation_url();
                let req = self.request("TEARDOWN", &url, Pending::Teardown);
                self.state = SinkState::TeardownSent;
                self.teardown_deadline = Some(now + Duration::from_secs(5));
                vec![Output::Send(req.into())]
            }
        }
    }

    pub fn pause(&mut self) -> Vec<Output> {
        if self.state != SinkState::Playing {
            return Vec::new();
        }
        let url = self.presentation_url();
        vec![Output::Send(self.request("PAUSE", &url, Pending::Pause).into())]
    }

    pub fn play(&mut self) -> Vec<Output> {
        if self.state != SinkState::Paused {
            return Vec::new();
        }
        let url = self.presentation_url();
        self.state = SinkState::PlaySent;
        vec![Output::Send(self.request("PLAY", &url, Pending::Play).into())]
    }

    /// The transport closed underneath us.
    pub fn on_disconnected(&mut self) -> Vec<Output> {
        if self.state == SinkState::Closed {
            return Vec::new();
        }
        let reason = if self.state == SinkState::TeardownSent { "teardown complete" } else { "connection closed by source" };
        vec![self.close(reason)]
    }

    pub fn handle(&mut self, msg: Message, now: Instant) -> Vec<Output> {
        self.last_rx = now;
        if self.state == SinkState::Closed {
            return Vec::new();
        }
        match msg {
            Message::Request(r) => self.handle_request(r, now),
            Message::Response(r) => self.handle_response(r),
        }
    }

    fn handle_request(&mut self, req: Request, now: Instant) -> Vec<Output> {
        let mut out = Vec::new();
        match req.method.as_str() {
            "OPTIONS" => {
                out.push(Output::Send(Response::ok_for(&req).with_header("Public", "org.wfa.wfd1.0, GET_PARAMETER, SET_PARAMETER").into()));
                if self.state == SinkState::WaitingM1 {
                    let m2 = self.request("OPTIONS", "*", Pending::Options).with_header("Require", WFD_REQUIRE);
                    out.push(Output::Send(m2.into()));
                    self.state = SinkState::Capabilities;
                }
            }
            "GET_PARAMETER" => {
                let names = params::parse_names(req.body_str());
                let mut resp = Response::ok_for(&req);
                if let Some(id) = &self.session_id {
                    resp.headers.set("Session", id.clone());
                }
                if !names.is_empty() {
                    let values: Vec<(String, String)> =
                        names.iter().filter_map(|n| self.parameter_value(n).map(|v| (n.clone(), v))).collect();
                    let skipped: Vec<&String> = names.iter().filter(|n| self.parameter_value(n).is_none()).collect();
                    if !skipped.is_empty() {
                        debug!(?skipped, "M3: not answering unknown parameters");
                    }
                    resp = resp.with_body("text/parameters", params::format_params(&values));
                }
                out.push(Output::Send(resp.into()));
            }
            "SET_PARAMETER" => {
                let p = params::parse_params(req.body_str());
                out.push(Output::Send(Response::ok_for(&req).into()));
                if let Some(trigger) = params::get(&p, "wfd_trigger_method") {
                    out.extend(self.on_trigger(&trigger.trim().to_ascii_uppercase(), now));
                } else {
                    out.extend(self.apply_parameters(&p));
                }
            }
            other => {
                warn!(method = other, "unexpected request from source");
                out.push(Output::Send(Response::new(405, req.cseq()).into()));
            }
        }
        out
    }

    fn on_trigger(&mut self, method: &str, now: Instant) -> Vec<Output> {
        let url = self.presentation_url();
        match method {
            "SETUP" => {
                if matches!(self.state, SinkState::SetupSent | SinkState::PlaySent | SinkState::Playing) {
                    return vec![Output::Event(SessionEvent::Warning("duplicate SETUP trigger ignored".into()))];
                }
                let port = self.negotiated.client_rtp_port.unwrap_or(self.cfg.rtp_port);
                let req =
                    self.request("SETUP", &url, Pending::Setup).with_header("Transport", format!("RTP/AVP/UDP;unicast;client_port={port}"));
                self.state = SinkState::SetupSent;
                vec![Output::Event(SessionEvent::PrepareMedia { rtp_port: port }), Output::Send(req.into())]
            }
            "PLAY" => {
                if self.session_id.is_none() {
                    return vec![Output::Event(SessionEvent::Warning("PLAY trigger before SETUP".into()))];
                }
                self.state = SinkState::PlaySent;
                vec![Output::Send(self.request("PLAY", &url, Pending::Play).into())]
            }
            "PAUSE" => vec![Output::Send(self.request("PAUSE", &url, Pending::Pause).into())],
            "TEARDOWN" => {
                if self.session_id.is_none() {
                    return vec![self.close("source requested teardown")];
                }
                self.state = SinkState::TeardownSent;
                self.teardown_deadline = Some(now + Duration::from_secs(5));
                vec![Output::Send(self.request("TEARDOWN", &url, Pending::Teardown).into())]
            }
            other => vec![Output::Event(SessionEvent::Warning(format!("unknown trigger method {other}")))],
        }
    }

    fn apply_parameters(&mut self, p: &[(String, String)]) -> Vec<Output> {
        let mut out = Vec::new();
        let mut formats_changed = false;
        if let Some(v) = params::get(p, "wfd_video_formats") {
            match VideoFormats::parse(v) {
                Ok(f) => {
                    self.negotiated.video = f.selected_mode();
                    self.negotiated.video_formats = Some(f);
                }
                Err(e) => {
                    // "none" = audio-only session
                    self.negotiated.video = None;
                    self.negotiated.video_formats = None;
                    if !v.trim().eq_ignore_ascii_case("none") {
                        out.push(Output::Event(SessionEvent::Warning(e.to_string())));
                    }
                }
            }
            formats_changed = true;
        }
        if let Some(v) = params::get(p, "wfd_audio_codecs") {
            self.negotiated.audio = AudioCodecs::parse(v).and_then(|a| a.selected_mode());
            formats_changed = true;
        }
        if let Some(v) = params::get(p, "wfd_presentation_URL") {
            let url = v.split_whitespace().next().unwrap_or("");
            if !url.is_empty() && !url.eq_ignore_ascii_case("none") {
                self.negotiated.presentation_url = Some(url.to_string());
            }
        }
        if let Some(v) = params::get(p, "wfd_client_rtp_ports")
            && let Some(ports) = ClientRtpPorts::parse(v)
        {
            self.negotiated.client_rtp_port = Some(ports.rtp_port0);
        }
        if let Some(v) = params::get(p, "wfd_uibc_capability") {
            self.negotiated.uibc = UibcCapability::parse(v);
            formats_changed = true;
        }
        let mut setting_changed = None;
        if let Some(v) = params::get(p, "wfd_uibc_setting") {
            let enabled = v.trim().eq_ignore_ascii_case("enable");
            if enabled != self.negotiated.uibc_enabled {
                setting_changed = Some(enabled);
            }
            self.negotiated.uibc_enabled = enabled;
        }
        if formats_changed {
            if matches!(self.state, SinkState::WaitingM1 | SinkState::Capabilities) {
                self.state = SinkState::Negotiated;
            }
            out.push(Output::Event(SessionEvent::Negotiated(self.negotiated.clone())));
        } else if let Some(enabled) = setting_changed {
            out.push(Output::Event(SessionEvent::UibcSetting { enabled }));
        }
        out
    }

    /// Value for one M3 parameter, `None` = not supported (left out).
    fn parameter_value(&self, name: &str) -> Option<String> {
        let c = &self.cfg;
        let v = match name.to_ascii_lowercase().as_str() {
            "wfd_video_formats" => c.video.to_string(),
            "wfd_audio_codecs" => c.audio.to_string(),
            "wfd_client_rtp_ports" => ClientRtpPorts::udp(c.rtp_port).to_string(),
            "wfd_3d_video_formats" => "none".into(),
            "wfd_content_protection" => "none".into(),
            "wfd_display_edid" => "none".into(),
            "wfd_coupled_sink" => "none".into(),
            "wfd_uibc_capability" => c.uibc.as_ref().map_or("none".into(), |u| u.to_string()),
            "wfd_standby_resume_capability" => "none".into(),
            "wfd_connector_type" => "05".into(),
            "wfd_idr_request_capability" => "1".into(),
            "wfd_i2c" => "none".into(),
            "intel_friendly_name" => c.friendly_name.clone(),
            "intel_sink_manufacturer_name" => c.manufacturer.clone(),
            "intel_sink_model_name" => c.model.clone(),
            "intel_sink_version" => c.version.clone(),
            "intel_sink_device_url" => "none".into(),
            _ => return None,
        };
        Some(v)
    }

    fn handle_response(&mut self, resp: Response) -> Vec<Output> {
        let Some(kind) = resp.cseq().and_then(|c| self.pending.remove(&c)) else {
            return vec![Output::Event(SessionEvent::Warning(format!("unmatched response CSeq {:?}", resp.cseq())))];
        };
        if !resp.is_success() {
            return match kind {
                Pending::Idr | Pending::Options => {
                    vec![Output::Event(SessionEvent::Warning(format!("{kind:?} rejected: {} {}", resp.status, resp.reason)))]
                }
                _ => vec![self.close(format!("source rejected {kind:?}: {} {}", resp.status, resp.reason))],
            };
        }
        match kind {
            Pending::Options => {
                if let Some(p) = resp.headers.get("Public") {
                    self.source_methods = p.split(',').map(|m| m.trim().to_string()).collect();
                }
                Vec::new()
            }
            Pending::Setup => {
                let Some(session) = resp.headers.get("Session") else {
                    return vec![self.close("SETUP response without Session header")];
                };
                let (id, timeout) = params::parse_session_header(session);
                self.session_id = Some(id);
                self.session_timeout = Duration::from_secs(timeout.unwrap_or(DEFAULT_SESSION_TIMEOUT));
                if let Some(t) = resp.headers.get("Transport") {
                    debug!(transport = ?Transport::parse(t), "SETUP accepted");
                }
                let url = self.presentation_url();
                self.state = SinkState::PlaySent;
                vec![Output::Send(self.request("PLAY", &url, Pending::Play).into())]
            }
            Pending::Play => {
                self.state = SinkState::Playing;
                vec![Output::Event(SessionEvent::Playing { session_id: self.session_id.clone().unwrap_or_default() })]
            }
            Pending::Pause => {
                self.state = SinkState::Paused;
                vec![Output::Event(SessionEvent::Paused)]
            }
            Pending::Teardown => vec![self.close("teardown complete")],
            Pending::Idr => Vec::new(),
        }
    }
}
