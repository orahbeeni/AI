//! Receive RTP/MPEG-TS on a port and print stats: `cargo run --example rx -- 19000 [secs] [sw|auto]`
use std::sync::Arc;
use std::time::Duration;
use wfd_media::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(19000);
    let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let mut cfg = ReceiverConfig::new(port);
    cfg.decoder = if args.get(3).map(String::as_str) == Some("sw") { DecoderChoice::Software } else { DecoderChoice::Auto };
    cfg.audio_sink = Some("fakesink".into());
    let rx = MediaReceiver::start(cfg, FrameSlot::new(), Arc::new(|e| eprintln!("event: {e:?}")), Arc::new(|| {})).unwrap();
    for _ in 0..secs {
        std::thread::sleep(Duration::from_secs(1));
        eprintln!("{:?}", rx.stats());
    }
}
