//! A local stand-in for a phone's media: H.264 + AAC in MPEG-TS over RTP/UDP,
//! exactly what a Miracast source sends. Runs in-process; the equivalent
//! `gst-launch-1.0` line is available from [`TestSource::launch_line`].

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_video as gst_video;

use crate::receiver::{MediaError, init};

#[derive(Debug, Clone)]
pub struct TestSourceConfig {
    pub host: String,
    pub port: u16,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub audio: bool,
    /// videotestsrc pattern, e.g. "ball", "smpte", "snow".
    pub pattern: String,
    pub caption: String,
}

impl Default for TestSourceConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 19000,
            width: 1280,
            height: 720,
            fps: 30,
            bitrate_kbps: 6000,
            audio: true,
            pattern: "ball".into(),
            caption: "Simulated phone".into(),
        }
    }
}

pub struct TestSource {
    pipeline: gst::Pipeline,
}

impl TestSource {
    /// The pipeline description (valid `gst-launch-1.0` syntax).
    pub fn launch_line(c: &TestSourceConfig) -> String {
        let caption = c.caption.replace('"', "'");
        let mut s = format!(
            "videotestsrc is-live=true pattern={pattern} ! video/x-raw,width={w},height={h},framerate={fps}/1 \
             ! textoverlay name=caption text=\"{caption}\" valignment=top halignment=left font-desc=\"Sans 24\" \
             ! timeoverlay valignment=bottom halignment=right font-desc=\"Sans 28\" \
             ! videoconvert ! x264enc name=enc tune=zerolatency speed-preset=ultrafast bitrate={br} key-int-max={gop} \
             ! video/x-h264,profile=constrained-baseline ! h264parse config-interval=-1 ! queue \
             ! mpegtsmux name=mux alignment=7 ! rtpmp2tpay pt=33 ! udpsink host={host} port={port} sync=false async=false",
            pattern = c.pattern,
            w = c.width,
            h = c.height,
            fps = c.fps,
            br = c.bitrate_kbps,
            gop = c.fps * 2,
            host = c.host,
            port = c.port,
        );
        if c.audio {
            s.push_str(
                " audiotestsrc is-live=true wave=ticks volume=0.2 ! audioconvert ! audioresample \
                 ! audio/x-raw,rate=48000,channels=2 ! voaacenc bitrate=128000 ! aacparse ! queue ! mux.",
            );
        }
        s
    }

    pub fn start(cfg: &TestSourceConfig) -> Result<Self, MediaError> {
        init()?;
        let pipeline = gst::parse::launch(&Self::launch_line(cfg))
            .map_err(|e| MediaError::Pipeline(e.to_string()))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| MediaError::Pipeline("not a pipeline".into()))?;
        pipeline.set_state(gst::State::Playing).map_err(|e| MediaError::Pipeline(e.to_string()))?;
        Ok(Self { pipeline })
    }

    /// Answer an IDR request (M13).
    pub fn force_keyframe(&self) {
        if let Some(enc) = self.pipeline.by_name("enc")
            && let Some(pad) = enc.static_pad("sink")
        {
            let ev = gst_video::DownstreamForceKeyUnitEvent::builder().all_headers(true).build();
            pad.send_event(ev);
        }
    }

    /// Updates the text drawn on the test picture (shows received input).
    pub fn set_caption(&self, text: &str) {
        if let Some(o) = self.pipeline.by_name("caption") {
            o.set_property("text", text);
        }
    }
}

impl Drop for TestSource {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
