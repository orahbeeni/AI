//! The sink media pipeline:
//!
//! ```text
//! udpsrc -> rtpjitterbuffer -> rtpmp2tdepay -> tsdemux -+-> queue -> h264parse -> <decoder> -> videoconvert -> appsink (I420)
//!                                                       +-> queue -> decodebin -> audioconvert -> audioresample -> volume -> autoaudiosink
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::frame::{DecodedFrame, FrameSlot};

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("GStreamer init failed: {0}")]
    Init(String),
    #[error("missing GStreamer element '{0}' (install the GStreamer plugins packages)")]
    MissingElement(String),
    #[error("no usable H.264 decoder found")]
    NoDecoder,
    #[error("pipeline error: {0}")]
    Pipeline(String),
}

pub fn init() -> Result<(), MediaError> {
    static INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    INIT.get_or_init(|| gst::init().map_err(|e| e.to_string())).clone().map_err(MediaError::Init)
}

pub(crate) fn make(factory: &str) -> Result<gst::Element, MediaError> {
    gst::ElementFactory::make(factory).build().map_err(|_| MediaError::MissingElement(factory.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecoderChoice {
    /// Hardware decoder when one initialises, otherwise software.
    #[default]
    Auto,
    Software,
    Hardware,
}

const HW_DECODERS: &[&str] = &[
    "vah264dec",
    "vaapih264dec",
    "nvh264dec",
    "d3d11h264dec",
    "d3d12h264dec",
    "vtdec_hw",
    "vtdec",
    "msdkh264dec",
    "v4l2slh264dec",
    "v4l2h264dec",
];
const SW_DECODERS: &[&str] = &["avdec_h264", "openh264dec"];

/// Names of H.264 decoders present on this system, hardware first.
pub fn available_decoders() -> Vec<String> {
    if init().is_err() {
        return Vec::new();
    }
    HW_DECODERS.iter().chain(SW_DECODERS).filter(|n| gst::ElementFactory::find(n).is_some()).map(|s| s.to_string()).collect()
}

/// Creates the first decoder that reaches READY (hardware ones fail there
/// when the driver/device is missing).
pub fn pick_decoder(choice: DecoderChoice) -> Result<(gst::Element, String), MediaError> {
    init()?;
    let order: Vec<&str> = match choice {
        DecoderChoice::Auto => HW_DECODERS.iter().chain(SW_DECODERS).copied().collect(),
        DecoderChoice::Hardware => HW_DECODERS.to_vec(),
        DecoderChoice::Software => SW_DECODERS.to_vec(),
    };
    for name in order {
        let Ok(el) = gst::ElementFactory::make(name).build() else { continue };
        if el.set_state(gst::State::Ready).is_err() {
            debug!(decoder = name, "decoder failed to initialise, skipping");
            let _ = el.set_state(gst::State::Null);
            continue;
        }
        let _ = el.set_state(gst::State::Null);
        // low-latency tuning
        match name {
            "avdec_h264" => {
                el.set_property_from_str("thread-type", "slice");
                el.set_property("output-corrupt", false);
            }
            "nvh264dec" => el.set_property("max-display-delay", 0i32),
            _ => {}
        }
        return Ok((el, name.to_string()));
    }
    Err(MediaError::NoDecoder)
}

#[derive(Debug, Clone)]
pub struct ReceiverConfig {
    pub rtp_port: u16,
    /// rtpjitterbuffer latency.
    pub jitter_latency_ms: u32,
    /// true = present by timestamp (smoother, adds latency);
    /// false = show/play as soon as decoded (lowest latency).
    pub av_sync: bool,
    pub decoder: DecoderChoice,
    pub enable_audio: bool,
    pub volume: f64,
    pub muted: bool,
    /// Override the audio sink (e.g. "fakesink" in tests).
    pub audio_sink: Option<String>,
}

impl ReceiverConfig {
    pub fn new(rtp_port: u16) -> Self {
        Self {
            rtp_port,
            jitter_latency_ms: 60,
            av_sync: false,
            decoder: DecoderChoice::Auto,
            enable_audio: true,
            volume: 1.0,
            muted: false,
            audio_sink: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MediaEvent {
    FirstFrame {
        width: u32,
        height: u32,
    },
    FormatChanged {
        width: u32,
        height: u32,
    },
    AudioStarted {
        caps: String,
    },
    /// RTP packets lost: the picture is corrupt until the next IDR.
    PacketLoss,
    Error(String),
    Warning(String),
}

#[derive(Debug, Clone, Default)]
pub struct MediaStats {
    pub decoder: String,
    pub width: u32,
    pub height: u32,
    pub frames: u64,
    pub fps: f32,
    pub bitrate_kbps: f32,
    pub lost_packets: u64,
    pub last_frame_at: Option<Instant>,
    pub audio: Option<String>,
}

pub type EventFn = Arc<dyn Fn(MediaEvent) + Send + Sync>;
pub type NotifyFn = Arc<dyn Fn() + Send + Sync>;

pub struct MediaReceiver {
    pipeline: gst::Pipeline,
    volume: Arc<Mutex<Option<gst::Element>>>,
    stats: Arc<Mutex<MediaStats>>,
    stop: Arc<AtomicBool>,
    bus_thread: Option<JoinHandle<()>>,
}

impl MediaReceiver {
    /// Builds and starts the pipeline. `on_frame` runs on the streaming
    /// thread for every decoded frame (use it to wake the GUI).
    pub fn start(cfg: ReceiverConfig, frames: FrameSlot, on_event: EventFn, on_frame: NotifyFn) -> Result<Self, MediaError> {
        init()?;
        let pipeline = gst::Pipeline::with_name("wfd-receiver");

        let udpsrc = make("udpsrc")?;
        udpsrc.set_property("port", cfg.rtp_port as i32);
        udpsrc.set_property("buffer-size", 4 * 1024 * 1024i32);
        udpsrc.set_property(
            "caps",
            gst::Caps::builder("application/x-rtp")
                .field("media", "video")
                .field("clock-rate", 90000i32)
                .field("encoding-name", "MP2T")
                .build(),
        );
        let jitter = make("rtpjitterbuffer")?;
        jitter.set_property("latency", cfg.jitter_latency_ms);
        // Never true: in MP2T-over-RTP the RTP timestamp follows whichever PES
        // (audio or video) the muxer last wrote, so it jumps back and forth by
        // the A/V offset. drop-on-latency then discards whole IDR bursts and
        // the picture never starts.
        jitter.set_property("drop-on-latency", false);
        jitter.set_property("do-lost", true);
        let depay = make("rtpmp2tdepay")?;
        let demux = make("tsdemux")?;
        demux.set_property("latency", cfg.jitter_latency_ms as i32);

        pipeline.add_many([&udpsrc, &jitter, &depay, &demux]).map_err(|e| MediaError::Pipeline(e.to_string()))?;
        gst::Element::link_many([&udpsrc, &jitter, &depay, &demux]).map_err(|e| MediaError::Pipeline(e.to_string()))?;

        let stats = Arc::new(Mutex::new(MediaStats::default()));
        let bytes = Arc::new(AtomicU64::new(0));

        // bitrate: count bytes leaving udpsrc
        {
            let bytes = bytes.clone();
            udpsrc.static_pad("src").unwrap().add_probe(gst::PadProbeType::BUFFER, move |_, info| {
                if let Some(gst::PadProbeData::Buffer(b)) = &info.data {
                    bytes.fetch_add(b.size() as u64, Ordering::Relaxed);
                }
                gst::PadProbeReturn::Ok
            });
        }
        // packet loss: rtpjitterbuffer sends GstRTPPacketLost downstream
        {
            let stats = stats.clone();
            let on_event = on_event.clone();
            jitter.static_pad("src").unwrap().add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
                if let Some(gst::PadProbeData::Event(ev)) = &info.data
                    && ev.has_name("GstRTPPacketLost")
                {
                    stats.lock().unwrap().lost_packets += 1;
                    on_event(MediaEvent::PacketLoss);
                }
                gst::PadProbeReturn::Ok
            });
        }

        let volume_slot: Arc<Mutex<Option<gst::Element>>> = Arc::new(Mutex::new(None));
        let video_linked = Arc::new(AtomicBool::new(false));
        let audio_linked = Arc::new(AtomicBool::new(false));

        // tsdemux exposes pads once it sees the PMT
        {
            let pipeline_weak = pipeline.downgrade();
            let cfg = cfg.clone();
            let stats = stats.clone();
            let on_event = on_event.clone();
            let volume_slot = volume_slot.clone();
            demux.connect_pad_added(move |_, pad| {
                let Some(pipeline) = pipeline_weak.upgrade() else { return };
                let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
                let Some(s) = caps.structure(0) else { return };
                let name = s.name().to_string();
                let result = if name == "video/x-h264" {
                    if video_linked.swap(true, Ordering::SeqCst) {
                        return;
                    }
                    link_video(&pipeline, pad, &cfg, frames.clone(), stats.clone(), on_event.clone(), on_frame.clone())
                } else if name.starts_with("audio/") && cfg.enable_audio {
                    if audio_linked.swap(true, Ordering::SeqCst) {
                        return;
                    }
                    link_audio(&pipeline, pad, &cfg, &volume_slot, stats.clone(), on_event.clone())
                } else {
                    debug!(caps = %name, "ignoring tsdemux pad");
                    link_fakesink(&pipeline, pad)
                };
                if let Err(e) = result {
                    on_event(MediaEvent::Error(format!("linking {name}: {e}")));
                }
            });
        }

        pipeline.set_state(gst::State::Playing).map_err(|e| MediaError::Pipeline(format!("cannot start: {e}")))?;
        info!(port = cfg.rtp_port, "media receiver listening");

        let stop = Arc::new(AtomicBool::new(false));
        let bus_thread = {
            let bus = pipeline.bus().unwrap();
            let stop = stop.clone();
            let stats = stats.clone();
            std::thread::Builder::new()
                .name("wfd-media-bus".into())
                .spawn(move || bus_loop(bus, stop, stats, bytes, on_event))
                .map_err(|e| MediaError::Pipeline(e.to_string()))?
        };

        Ok(Self { pipeline, volume: volume_slot, stats, stop, bus_thread: Some(bus_thread) })
    }

    pub fn stats(&self) -> MediaStats {
        self.stats.lock().unwrap().clone()
    }

    pub fn set_volume(&self, v: f64) {
        if let Some(vol) = self.volume.lock().unwrap().as_ref() {
            vol.set_property("volume", v.clamp(0.0, 4.0));
        }
    }

    pub fn set_muted(&self, muted: bool) {
        if let Some(vol) = self.volume.lock().unwrap().as_ref() {
            vol.set_property("mute", muted);
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(t) = self.bus_thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for MediaReceiver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn add_chain(pipeline: &gst::Pipeline, pad: &gst::Pad, chain: &[&gst::Element]) -> Result<(), String> {
    pipeline.add_many(chain.iter().copied()).map_err(|e| e.to_string())?;
    gst::Element::link_many(chain.iter().copied()).map_err(|e| e.to_string())?;
    for el in chain {
        el.sync_state_with_parent().map_err(|e| e.to_string())?;
    }
    let sink = chain[0].static_pad("sink").ok_or("chain has no sink pad")?;
    pad.link(&sink).map_err(|e| format!("{e:?}"))?;
    Ok(())
}

fn link_fakesink(pipeline: &gst::Pipeline, pad: &gst::Pad) -> Result<(), String> {
    let q = make("queue").map_err(|e| e.to_string())?;
    let sink = make("fakesink").map_err(|e| e.to_string())?;
    sink.set_property("sync", false);
    add_chain(pipeline, pad, &[&q, &sink])
}

fn low_latency_queue() -> Result<gst::Element, String> {
    let q = make("queue").map_err(|e| e.to_string())?;
    q.set_property("max-size-buffers", 0u32);
    q.set_property("max-size-bytes", 0u32);
    q.set_property("max-size-time", 500_000_000u64);
    Ok(q)
}

fn link_video(
    pipeline: &gst::Pipeline,
    pad: &gst::Pad,
    cfg: &ReceiverConfig,
    frames: FrameSlot,
    stats: Arc<Mutex<MediaStats>>,
    on_event: EventFn,
    on_frame: NotifyFn,
) -> Result<(), String> {
    let queue = low_latency_queue()?;
    let parse = make("h264parse").map_err(|e| e.to_string())?;
    let (decoder, decoder_name) = pick_decoder(cfg.decoder).map_err(|e| e.to_string())?;
    let convert = make("videoconvert").map_err(|e| e.to_string())?;
    let appsink = gst_app::AppSink::builder()
        .caps(&gst::Caps::builder("video/x-raw").field("format", "I420").build())
        .max_buffers(2)
        .drop(true)
        .sync(cfg.av_sync)
        .build();
    if cfg.av_sync {
        appsink.set_property("max-lateness", 40_000_000i64);
    }
    info!(decoder = %decoder_name, "video decoder selected");
    stats.lock().unwrap().decoder = decoder_name;

    let mut index = 0u64;
    let mut last_caps: Option<(gst::Caps, gst_video::VideoInfo)> = None;
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let Some(caps) = sample.caps() else { return Ok(gst::FlowSuccess::Ok) };
                let info = match &last_caps {
                    Some((c, i)) if c.as_ref() == caps => i.clone(),
                    _ => {
                        let Ok(i) = gst_video::VideoInfo::from_caps(caps) else { return Err(gst::FlowError::NotNegotiated) };
                        let (w, h) = (i.width(), i.height());
                        let first = last_caps.is_none();
                        last_caps = Some((caps.to_owned(), i.clone()));
                        {
                            let mut s = stats.lock().unwrap();
                            s.width = w;
                            s.height = h;
                        }
                        on_event(if first {
                            MediaEvent::FirstFrame { width: w, height: h }
                        } else {
                            MediaEvent::FormatChanged { width: w, height: h }
                        });
                        i
                    }
                };
                let Some(buffer) = sample.buffer_owned() else { return Ok(gst::FlowSuccess::Ok) };
                index += 1;
                frames.put(DecodedFrame::new(index, buffer, info));
                {
                    let mut s = stats.lock().unwrap();
                    s.frames = index;
                    s.last_frame_at = Some(Instant::now());
                }
                on_frame();
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    let appsink: gst::Element = appsink.upcast();
    add_chain(pipeline, pad, &[&queue, &parse, &decoder, &convert, &appsink])
}

fn link_audio(
    pipeline: &gst::Pipeline,
    pad: &gst::Pad,
    cfg: &ReceiverConfig,
    volume_slot: &Arc<Mutex<Option<gst::Element>>>,
    stats: Arc<Mutex<MediaStats>>,
    on_event: EventFn,
) -> Result<(), String> {
    let caps = pad.current_caps().map(|c| c.to_string()).unwrap_or_default();
    let queue = low_latency_queue()?;
    let decodebin = make("decodebin").map_err(|e| e.to_string())?;
    let convert = make("audioconvert").map_err(|e| e.to_string())?;
    let resample = make("audioresample").map_err(|e| e.to_string())?;
    let volume = make("volume").map_err(|e| e.to_string())?;
    volume.set_property("volume", cfg.volume);
    volume.set_property("mute", cfg.muted);
    let sink = make(cfg.audio_sink.as_deref().unwrap_or("autoaudiosink")).map_err(|e| e.to_string())?;
    configure_audio_sink(&sink, cfg.av_sync);

    pipeline.add_many([&queue, &decodebin, &convert, &resample, &volume, &sink]).map_err(|e| e.to_string())?;
    queue.link(&decodebin).map_err(|e| e.to_string())?;
    gst::Element::link_many([&convert, &resample, &volume, &sink]).map_err(|e| e.to_string())?;
    {
        let convert = convert.clone();
        decodebin.connect_pad_added(move |_, dpad| {
            let is_audio = dpad
                .current_caps()
                .unwrap_or_else(|| dpad.query_caps(None))
                .structure(0)
                .is_some_and(|s| s.name().starts_with("audio/x-raw"));
            if is_audio
                && let Some(sinkpad) = convert.static_pad("sink")
                && !sinkpad.is_linked()
            {
                let _ = dpad.link(&sinkpad);
            }
        });
    }
    for el in [&queue, &decodebin, &convert, &resample, &volume, &sink] {
        el.sync_state_with_parent().map_err(|e| e.to_string())?;
    }
    pad.link(&queue.static_pad("sink").unwrap()).map_err(|e| format!("{e:?}"))?;
    *volume_slot.lock().unwrap() = Some(volume);
    stats.lock().unwrap().audio = Some(short_caps(&caps));
    on_event(MediaEvent::AudioStarted { caps: short_caps(&caps) });
    Ok(())
}

fn short_caps(caps: &str) -> String {
    caps.split(',').next().unwrap_or(caps).to_string()
}

/// Low-latency settings for the audio sink (and for the real sink inside
/// autoaudiosink once it picks one).
fn configure_audio_sink(sink: &gst::Element, sync: bool) {
    fn tune(el: &gst::Element, sync: bool) {
        if el.has_property("sync") {
            el.set_property("sync", sync);
        }
        if el.has_property("buffer-time") {
            el.set_property("buffer-time", 60_000i64);
        }
        if el.has_property("latency-time") {
            el.set_property("latency-time", 10_000i64);
        }
    }
    tune(sink, sync);
    if let Some(bin) = sink.dynamic_cast_ref::<gst::Bin>() {
        bin.connect_deep_element_added(move |_, _, el| {
            if el.is::<gst::Bin>() {
                return;
            }
            if el.has_property("buffer-time") || el.has_property("sync") {
                tune(el, sync);
            }
        });
    }
}

fn bus_loop(bus: gst::Bus, stop: Arc<AtomicBool>, stats: Arc<Mutex<MediaStats>>, bytes: Arc<AtomicU64>, on_event: EventFn) {
    let mut last_tick = Instant::now();
    let mut last_frames = 0u64;
    let mut last_bytes = 0u64;
    let mut ticks = 0u32;
    while !stop.load(Ordering::SeqCst) {
        if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(200)) {
            match msg.view() {
                gst::MessageView::Error(e) => {
                    let src = e.src().map(|s| s.path_string().to_string()).unwrap_or_default();
                    warn!(%src, error = %e.error(), debug = ?e.debug(), "pipeline error");
                    on_event(MediaEvent::Error(format!("{src}: {}", e.error())));
                }
                gst::MessageView::Warning(w) => {
                    debug!(warning = %w.error(), "pipeline warning");
                    on_event(MediaEvent::Warning(w.error().to_string()));
                }
                _ => {}
            }
        }
        let dt = last_tick.elapsed();
        if dt >= Duration::from_secs(1) {
            let b = bytes.load(Ordering::Relaxed);
            let mut s = stats.lock().unwrap();
            let secs = dt.as_secs_f32();
            s.fps = (s.frames - last_frames) as f32 / secs;
            s.bitrate_kbps = (b - last_bytes) as f32 * 8.0 / 1000.0 / secs;
            last_frames = s.frames;
            last_bytes = b;
            last_tick = Instant::now();
            ticks += 1;
            if ticks % 5 == 0 {
                debug!(fps = s.fps, kbit_s = s.bitrate_kbps, frames = s.frames, lost_packets = s.lost_packets, "video stats");
            }
        }
    }
}
