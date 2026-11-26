//! Wi-Fi Display (Miracast) sink protocol core.
//!
//! Everything here is pure Rust and transport-agnostic:
//! - [`rtsp`]: RTSP/1.0 message model and incremental parser
//! - [`params`], [`video`], [`audio`]: WFD parameter encoding (M3/M4)
//! - [`ie`]: WFD Information Element subelements for Wi-Fi Direct discovery
//! - [`uibc`]: User Input Back Channel capability negotiation and packets
//! - [`session`]: the sink state machine (M1-M8, M13 IDR, M16 keep-alive)
//! - [`source`]: a simulated source used for tests and the app's demo mode
//! - [`driver`]: tokio drivers that run either session over TCP

pub mod audio;
pub mod ie;
pub mod params;
pub mod rtsp;
pub mod session;
pub mod source;
pub mod uibc;
pub mod video;

#[cfg(feature = "tokio")]
pub mod driver;

pub use session::{Negotiated, Output, SessionEvent, SinkConfig, SinkSession, SinkState};
