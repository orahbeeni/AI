//! wifishare: share one keyboard and mouse (and the clipboard) between computers on a LAN, and
//! files with phones.
//!
//! The `desktop` feature (on by default) builds everything the computer app needs. Without it,
//! only the networking, pairing and shared-folder parts are built, for the Android app.

// Linux cannot be a server yet, so the input-capture half (switching core, key tables) is only
// exercised by tests there; the macOS and Windows builds use all of it.
#![cfg_attr(target_os = "linux", allow(dead_code))]

#[cfg(feature = "desktop")]
pub mod audio;
#[cfg(feature = "desktop")]
pub mod autostart;
#[cfg(feature = "desktop")]
pub mod client;
#[cfg(feature = "desktop")]
pub mod clipboard;
#[cfg(feature = "desktop")]
pub mod clipnet;
pub mod config;
pub mod discovery;
#[cfg(feature = "desktop")]
pub mod engine;
pub mod files;
pub mod fsutil;
pub mod geom;
#[cfg(feature = "desktop")]
pub mod gui;
#[cfg(feature = "desktop")]
pub mod icon;
#[cfg(feature = "desktop")]
pub mod instance;
#[cfg(feature = "desktop")]
pub mod keymap;
pub mod logger;
pub mod mobile;
pub mod netwatch;
pub mod pair;
pub mod phones;
#[cfg(feature = "desktop")]
pub mod platform;
pub mod proto;
#[cfg(feature = "desktop")]
pub mod server;
pub mod status;
pub mod wire;
