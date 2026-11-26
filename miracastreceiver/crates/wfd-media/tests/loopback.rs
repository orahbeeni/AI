//! Streams the test source over localhost RTP into the receiver and checks
//! that video frames and audio come out the other side.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wfd_media::{DecoderChoice, FrameSlot, MediaEvent, MediaReceiver, ReceiverConfig, TestSource, TestSourceConfig, free_udp_port};

fn run(decoder: DecoderChoice, width: u32, height: u32, fps: u32) -> (wfd_media::MediaStats, Vec<MediaEvent>, Option<Vec<u8>>) {
    let port = free_udp_port(0);
    let frames = FrameSlot::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let mut cfg = ReceiverConfig::new(port);
    cfg.decoder = decoder;
    cfg.audio_sink = Some("fakesink".into());
    let rx = MediaReceiver::start(cfg, frames.clone(), Arc::new(move |e| ev.lock().unwrap().push(e)), Arc::new(|| {})).unwrap();
    let src = TestSource::start(&TestSourceConfig { port, width, height, fps, pattern: "smpte".into(), ..Default::default() }).unwrap();

    let start = Instant::now();
    let mut rgba = None;
    while start.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(100));
        if rx.stats().frames >= (fps as u64 * 2) {
            if let Some(f) = frames.take() {
                rgba = f.to_rgba();
            }
            break;
        }
        if start.elapsed() > Duration::from_secs(1) {
            src.force_keyframe();
        }
    }
    // let the stats thread compute a full-second fps sample
    std::thread::sleep(Duration::from_millis(1100));
    let stats = rx.stats();
    drop(src);
    rx.stop();
    let evs = events.lock().unwrap().clone();
    (stats, evs, rgba)
}

#[test]
fn software_decode_720p30_with_audio() {
    let (stats, events, rgba) = run(DecoderChoice::Software, 1280, 720, 30);
    eprintln!("{stats:?}\n{events:?}");
    assert!(stats.frames >= 60, "decoded {} frames", stats.frames);
    assert_eq!((stats.width, stats.height), (1280, 720));
    assert!(stats.fps > 20.0, "fps {}", stats.fps);
    assert!(stats.bitrate_kbps > 100.0);
    assert!(stats.decoder == "avdec_h264" || stats.decoder == "openh264dec");
    assert!(events.contains(&MediaEvent::FirstFrame { width: 1280, height: 720 }));
    assert!(events.iter().any(|e| matches!(e, MediaEvent::AudioStarted { caps } if caps.contains("audio/mpeg"))), "{events:?}");
    assert!(!events.iter().any(|e| matches!(e, MediaEvent::Error(_))), "{events:?}");

    // SMPTE bars: the top-left bar is light grey/white-ish, a later bar is
    // strongly coloured -> the picture really decoded and converted.
    let rgba = rgba.expect("frame");
    let px = |x: usize, y: usize| &rgba[(y * 1280 + x) * 4..(y * 1280 + x) * 4 + 3];
    let a = px(80, 360);
    // bars: white, yellow, cyan, green, magenta, red, blue
    let magenta = px(80 + 1280 / 7 * 4, 360);
    let red = px(80 + 1280 / 7 * 5, 360);
    eprintln!("bar0={a:?} magenta={magenta:?} red={red:?}");
    assert!(a.iter().all(|c| *c > 150), "first bar should be bright: {a:?}");
    assert!(magenta[0] > 150 && magenta[1] < 60 && magenta[2] > 150, "fifth bar should be magenta: {magenta:?}");
    assert!(red[0] > 150 && red[1] < 60 && red[2] < 60, "sixth bar should be red: {red:?}");
}

#[test]
fn auto_decoder_1080p60() {
    let (stats, events, _) = run(DecoderChoice::Auto, 1920, 1080, 60);
    eprintln!("decoder={} {stats:?}", stats.decoder);
    assert!(stats.frames >= 100, "decoded {} frames with {}", stats.frames, stats.decoder);
    assert_eq!((stats.width, stats.height), (1920, 1080));
    assert!(stats.fps > 45.0, "fps {} with {}", stats.fps, stats.decoder);
    assert!(!events.iter().any(|e| matches!(e, MediaEvent::Error(_))), "{events:?}");
}
