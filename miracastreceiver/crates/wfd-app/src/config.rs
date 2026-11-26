//! Settings saved between runs (JSON in the platform config folder).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Decoder {
    #[default]
    Auto,
    Software,
    Hardware,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MaxVideo {
    #[default]
    P1080p60,
    P1080p30,
    P720p60,
    P720p30,
}

impl MaxVideo {
    pub fn label(self) -> &'static str {
        match self {
            MaxVideo::P1080p60 => "1080p 60 fps",
            MaxVideo::P1080p30 => "1080p 30 fps",
            MaxVideo::P720p60 => "720p 60 fps",
            MaxVideo::P720p30 => "720p 30 fps",
        }
    }

    pub fn limits(self) -> wfd_core::video::SinkVideoLimits {
        let (max_width, max_height, max_fps) = match self {
            MaxVideo::P1080p60 => (1920, 1080, 60),
            MaxVideo::P1080p30 => (1920, 1080, 30),
            MaxVideo::P720p60 => (1280, 720, 60),
            MaxVideo::P720p30 => (1280, 720, 30),
        };
        wfd_core::video::SinkVideoLimits { max_width, max_height, max_fps }
    }
}

/// The mouse pointer shown over the picture while it isn't locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PointerStyle {
    #[default]
    Arrow,
    Hand,
    Crosshair,
    Hidden,
}

impl PointerStyle {
    pub const ALL: [PointerStyle; 4] = [PointerStyle::Arrow, PointerStyle::Hand, PointerStyle::Crosshair, PointerStyle::Hidden];

    pub fn label(self) -> &'static str {
        match self {
            PointerStyle::Arrow => "Arrow",
            PointerStyle::Hand => "Hand",
            PointerStyle::Crosshair => "Crosshair",
            PointerStyle::Hidden => "Hidden (only the device's)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub name: String,
    pub wifi_direct: bool,
    pub infrastructure: bool,
    /// Accept push-button connections without asking.
    pub auto_accept: bool,
    pub decoder: Decoder,
    /// false = lowest latency (show frames as they decode).
    pub av_sync: bool,
    pub jitter_ms: u32,
    pub max_video: MaxVideo,
    pub uibc: bool,
    pub volume: f64,
    pub muted: bool,
    pub show_overlay: bool,
    /// Click in the picture to lock the mouse to it (relative HID mouse).
    pub capture_on_click: bool,
    pub pointer: PointerStyle,
    /// Let the connected device exchange files through its web browser.
    pub file_share: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: wfd_transport::net::default_friendly_name(),
            wifi_direct: true,
            infrastructure: true,
            auto_accept: false,
            decoder: Decoder::Auto,
            av_sync: false,
            jitter_ms: 60,
            max_video: MaxVideo::P1080p60,
            uibc: true,
            volume: 1.0,
            muted: false,
            show_overlay: false,
            capture_on_click: true,
            pointer: PointerStyle::Arrow,
            file_share: true,
        }
    }
}

fn config_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("MIRACAST_RECEIVER_CONFIG_DIR") {
        return Some(PathBuf::from(d));
    }
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.map(|b| b.join("miracast-receiver"))
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = Self::path() else { return };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }
}
