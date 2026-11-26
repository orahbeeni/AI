//! GStreamer media for a Wi-Fi Display sink: receives RTP/MPEG-TS, decodes
//! H.264 (hardware when available) and LPCM/AAC, hands I420 frames to the GUI
//! and plays audio on the system output.

mod frame;
mod receiver;
mod test_source;

pub use frame::{ColorMatrix, DecodedFrame, FrameSlot, Plane};
pub use receiver::{
    DecoderChoice, EventFn, MediaError, MediaEvent, MediaReceiver, MediaStats, NotifyFn, ReceiverConfig, available_decoders, init,
    pick_decoder,
};
pub use test_source::{TestSource, TestSourceConfig};

/// Finds a free UDP port for RTP (tries `preferred` first; 0 = any).
pub fn free_udp_port(preferred: u16) -> u16 {
    if preferred != 0 && std::net::UdpSocket::bind(("0.0.0.0", preferred)).is_ok() {
        return preferred;
    }
    std::net::UdpSocket::bind(("0.0.0.0", 0)).and_then(|s| s.local_addr()).map(|a| a.port()).unwrap_or(19000)
}
