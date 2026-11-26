//! Sans-IO Wi-Fi Display *source*, used to test the sink and to drive the
//! app's "Simulate phone" mode. It behaves like an Android/Windows source:
//! it opens with M1, queries capabilities (M3), picks a mode (M4), triggers
//! SETUP (M5), answers SETUP/PLAY/TEARDOWN and sends M16 keep-alives.

use std::time::{Duration, Instant};

use crate::audio::{AudioCodec, AudioCodecs, AudioFormat, AudioMode};
use crate::params::{self, ClientRtpPorts, Transport};
use crate::rtsp::{Message, Request, Response, WFD_REQUIRE};
use crate::uibc::UibcCapability;
use crate::video::{H264Codec, ResolutionTable, VideoFormats, VideoMode, level, profile};

/// The parameter names a typical source asks for in M3.
pub const M3_PARAMETERS: &[&str] = &[
    "wfd_video_formats",
    "wfd_audio_codecs",
    "wfd_client_rtp_ports",
    "wfd_content_protection",
    "wfd_display_edid",
    "wfd_coupled_sink",
    "wfd_uibc_capability",
    "wfd_standby_resume_capability",
    "wfd_connector_type",
    "wfd_idr_request_capability",
    "intel_friendly_name",
    "microsoft_cursor",
];

#[derive(Debug, Clone)]
pub struct SourceConfig {
    /// Host used in the presentation URL.
    pub host: String,
    pub max_width: u32,
    pub max_height: u32,
    pub max_fps: u32,
    pub prefer_aac: bool,
    /// When set and the sink offers UIBC, enable it on this TCP port.
    pub uibc_port: Option<u16>,
    pub session_timeout: u64,
    pub keepalive_interval: Duration,
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            max_width: 1280,
            max_height: 720,
            max_fps: 30,
            prefer_aac: true,
            uibc_port: None,
            session_timeout: 30,
            keepalive_interval: Duration::from_secs(25),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourceEvent {
    SinkCapabilities(Vec<(String, String)>),
    Negotiated {
        video: VideoMode,
        audio: Option<AudioMode>,
        uibc: Option<UibcCapability>,
    },
    /// Sink sent SETUP: stream to this UDP port once PLAY arrives.
    Setup {
        sink_rtp_port: u16,
    },
    Play,
    Pause,
    IdrRequested,
    Closed {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourceOutput {
    Send(Message),
    Event(SourceEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    M1,
    M3,
    M4,
    Trigger,
    KeepAlive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceState {
    Idle,
    WaitM2,
    Querying,
    Selecting,
    Triggered,
    Ready,
    Playing,
    Paused,
    Closed,
}

pub struct SourceSession {
    cfg: SourceConfig,
    state: SourceState,
    next_cseq: u32,
    pending: Vec<(u32, Pending)>,
    session_id: String,
    sink_rtp_port: u16,
    last_keepalive: Instant,
}

impl SourceSession {
    pub fn new(cfg: SourceConfig, now: Instant) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let session_id = format!("{:08X}", nanos ^ 0x6B8B_4567 ^ std::process::id());
        Self { cfg, state: SourceState::Idle, next_cseq: 1, pending: Vec::new(), session_id, sink_rtp_port: 0, last_keepalive: now }
    }

    pub fn state(&self) -> SourceState {
        self.state
    }

    fn url(&self) -> String {
        format!("rtsp://{}/wfd1.0/streamid=0", self.cfg.host)
    }

    fn request(&mut self, method: &str, uri: &str, kind: Pending) -> Request {
        let c = self.next_cseq;
        self.next_cseq += 1;
        self.pending.push((c, kind));
        Request::new(method, uri, c)
    }

    /// Call once the sink's TCP connection is accepted: sends M1.
    pub fn start(&mut self) -> Vec<SourceOutput> {
        self.state = SourceState::WaitM2;
        let m1 = self.request("OPTIONS", "*", Pending::M1).with_header("Require", WFD_REQUIRE);
        vec![SourceOutput::Send(m1.into())]
    }

    /// Source-initiated teardown (M5 with TEARDOWN trigger).
    pub fn trigger_teardown(&mut self) -> Vec<SourceOutput> {
        if matches!(self.state, SourceState::Closed | SourceState::Idle) {
            return Vec::new();
        }
        let r = self
            .request("SET_PARAMETER", "rtsp://localhost/wfd1.0", Pending::Trigger)
            .with_body("text/parameters", "wfd_trigger_method: TEARDOWN\r\n");
        vec![SourceOutput::Send(r.into())]
    }

    pub fn next_keepalive(&self) -> Option<Instant> {
        matches!(self.state, SourceState::Playing | SourceState::Paused).then(|| self.last_keepalive + self.cfg.keepalive_interval)
    }

    pub fn on_timer(&mut self, now: Instant) -> Vec<SourceOutput> {
        match self.next_keepalive() {
            Some(t) if now >= t => {
                self.last_keepalive = now;
                let mut r = self.request("GET_PARAMETER", "rtsp://localhost/wfd1.0", Pending::KeepAlive);
                r.headers.set("Session", self.session_id.clone());
                vec![SourceOutput::Send(r.into())]
            }
            _ => Vec::new(),
        }
    }

    pub fn handle(&mut self, msg: Message, now: Instant) -> Vec<SourceOutput> {
        match msg {
            Message::Request(r) => self.handle_request(r, now),
            Message::Response(r) => self.handle_response(r),
        }
    }

    fn handle_request(&mut self, req: Request, now: Instant) -> Vec<SourceOutput> {
        let mut out = Vec::new();
        match req.method.as_str() {
            "OPTIONS" => {
                out.push(SourceOutput::Send(
                    Response::ok_for(&req)
                        .with_header("Public", "org.wfa.wfd1.0, SETUP, TEARDOWN, PLAY, PAUSE, GET_PARAMETER, SET_PARAMETER")
                        .into(),
                ));
                if self.state == SourceState::WaitM2 {
                    let m3 = self
                        .request("GET_PARAMETER", "rtsp://localhost/wfd1.0", Pending::M3)
                        .with_body("text/parameters", params::format_names(M3_PARAMETERS));
                    out.push(SourceOutput::Send(m3.into()));
                    self.state = SourceState::Querying;
                }
            }
            "SETUP" => {
                let t = Transport::parse(req.headers.get("Transport").unwrap_or(""));
                let port = t.client_port.unwrap_or(self.sink_rtp_port);
                self.sink_rtp_port = port;
                out.push(SourceOutput::Send(
                    Response::ok_for(&req)
                        .with_header("Session", format!("{};timeout={}", self.session_id, self.cfg.session_timeout))
                        .with_header("Transport", format!("RTP/AVP/UDP;unicast;client_port={port};server_port=50000"))
                        .into(),
                ));
                self.state = SourceState::Ready;
                out.push(SourceOutput::Event(SourceEvent::Setup { sink_rtp_port: port }));
            }
            "PLAY" => {
                out.push(SourceOutput::Send(Response::ok_for(&req).with_header("Session", self.session_id.clone()).into()));
                self.state = SourceState::Playing;
                self.last_keepalive = now;
                out.push(SourceOutput::Event(SourceEvent::Play));
            }
            "PAUSE" => {
                out.push(SourceOutput::Send(Response::ok_for(&req).with_header("Session", self.session_id.clone()).into()));
                self.state = SourceState::Paused;
                out.push(SourceOutput::Event(SourceEvent::Pause));
            }
            "TEARDOWN" => {
                out.push(SourceOutput::Send(Response::ok_for(&req).with_header("Session", self.session_id.clone()).into()));
                self.state = SourceState::Closed;
                out.push(SourceOutput::Event(SourceEvent::Closed { reason: "sink sent TEARDOWN".into() }));
            }
            "SET_PARAMETER" => {
                out.push(SourceOutput::Send(Response::ok_for(&req).into()));
                if req.body_str().contains("wfd_idr_request") {
                    out.push(SourceOutput::Event(SourceEvent::IdrRequested));
                }
            }
            "GET_PARAMETER" => out.push(SourceOutput::Send(Response::ok_for(&req).into())),
            _ => out.push(SourceOutput::Send(Response::new(405, req.cseq()).into())),
        }
        out
    }

    fn handle_response(&mut self, resp: Response) -> Vec<SourceOutput> {
        let Some(pos) = self.pending.iter().position(|(c, _)| Some(*c) == resp.cseq()) else {
            return Vec::new();
        };
        let (_, kind) = self.pending.remove(pos);
        if !resp.is_success() {
            self.state = SourceState::Closed;
            return vec![SourceOutput::Event(SourceEvent::Closed { reason: format!("sink rejected {kind:?}: {}", resp.status) })];
        }
        match kind {
            Pending::M1 | Pending::KeepAlive => Vec::new(),
            Pending::M3 => {
                let caps = params::parse_params(resp.body_str());
                let mut out = vec![SourceOutput::Event(SourceEvent::SinkCapabilities(caps.clone()))];
                match self.select(&caps) {
                    Ok((body, video, audio, uibc)) => {
                        let m4 = self.request("SET_PARAMETER", "rtsp://localhost/wfd1.0", Pending::M4).with_body("text/parameters", body);
                        out.push(SourceOutput::Event(SourceEvent::Negotiated { video, audio, uibc }));
                        out.push(SourceOutput::Send(m4.into()));
                        self.state = SourceState::Selecting;
                    }
                    Err(reason) => {
                        self.state = SourceState::Closed;
                        out.push(SourceOutput::Event(SourceEvent::Closed { reason }));
                    }
                }
                out
            }
            Pending::M4 => {
                let m5 = self
                    .request("SET_PARAMETER", "rtsp://localhost/wfd1.0", Pending::Trigger)
                    .with_body("text/parameters", "wfd_trigger_method: SETUP\r\n");
                self.state = SourceState::Triggered;
                vec![SourceOutput::Send(m5.into())]
            }
            Pending::Trigger => Vec::new(),
        }
    }

    /// Builds the M4 body from the sink's M3 answer.
    #[allow(clippy::type_complexity)]
    fn select(&mut self, caps: &[(String, String)]) -> Result<(String, VideoMode, Option<AudioMode>, Option<UibcCapability>), String> {
        let vf = params::get(caps, "wfd_video_formats").ok_or("sink did not report wfd_video_formats")?;
        let vf = VideoFormats::parse(vf).map_err(|e| e.to_string())?;
        let codec: &H264Codec = vf.codecs.first().ok_or("no codecs")?;
        let video = codec
            .modes()
            .into_iter()
            .filter(|m| !m.interlaced && m.width <= self.cfg.max_width && m.height <= self.cfg.max_height && m.fps <= self.cfg.max_fps)
            .filter(|m| m.table != ResolutionTable::Hh)
            .max_by_key(|m| (m.width * m.height, m.fps))
            .ok_or("no common video mode")?;
        let prof = if codec.profile & profile::CHP != 0 { profile::CHP } else { profile::CBP };
        let lvl = if video.width * video.height * video.fps > 1280 * 720 * 30 { level::L4_2 } else { level::L3_1 };

        let sink_audio = params::get(caps, "wfd_audio_codecs").and_then(AudioCodecs::parse).unwrap_or_default();
        let pick = |f: AudioFormat| sink_audio.supports(f).map(|c| (f, c.modes));
        let audio_choice = if self.cfg.prefer_aac {
            pick(AudioFormat::Aac).or_else(|| pick(AudioFormat::Lpcm))
        } else {
            pick(AudioFormat::Lpcm).or_else(|| pick(AudioFormat::Aac))
        };
        let audio_codecs = match audio_choice {
            // choose 48 kHz stereo: LPCM bit 1, AAC bit 0
            Some((AudioFormat::Lpcm, modes)) => {
                let bit = if modes & 0b10 != 0 { 0b10 } else { 0b01 };
                AudioCodecs(vec![AudioCodec { format: AudioFormat::Lpcm, modes: bit, latency: 0 }])
            }
            Some((f, _)) => AudioCodecs(vec![AudioCodec { format: f, modes: 0b1, latency: 0 }]),
            None => AudioCodecs::none(),
        };

        let sink_port = params::get(caps, "wfd_client_rtp_ports")
            .and_then(ClientRtpPorts::parse)
            .map(|p| p.rtp_port0)
            .ok_or("sink did not report wfd_client_rtp_ports")?;
        self.sink_rtp_port = sink_port;

        let mut body: Vec<(String, String)> = vec![
            ("wfd_video_formats".into(), VideoFormats::selection(video, prof, lvl).to_string()),
            ("wfd_audio_codecs".into(), audio_codecs.to_string()),
            ("wfd_presentation_URL".into(), format!("{} none", self.url())),
            ("wfd_client_rtp_ports".into(), ClientRtpPorts::udp(sink_port).to_string()),
        ];
        let mut uibc = None;
        if let (Some(port), Some(offer)) = (self.cfg.uibc_port, params::get(caps, "wfd_uibc_capability").and_then(UibcCapability::parse)) {
            let mut chosen = offer.clone();
            chosen.port = Some(port);
            body.push(("wfd_uibc_capability".into(), chosen.to_string()));
            body.push(("wfd_uibc_setting".into(), "enable".into()));
            uibc = Some(chosen);
        }
        Ok((params::format_params(&body), video, audio_codecs.selected_mode(), uibc))
    }
}
