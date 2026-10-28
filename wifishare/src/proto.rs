//! Messages exchanged over an encrypted link (see `wire`).
//!
//! Keys travel as Linux evdev key codes (physical key positions); each platform maps its own
//! codes to and from them in `keymap`. Wheel deltas use 1/120ths of a notch, Linux sign
//! convention: positive `dy` scrolls up, positive `dx` scrolls right.

use serde::{Deserialize, Serialize};

use crate::geom::Screen;

/// Bump when messages change incompatibly; peers with a different version refuse each other.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Other
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Button {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl Button {
    pub fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub version: u32,
    pub name: String,
    pub os: Os,
    /// The sender's monitors; only clients need to send one.
    pub screen: Option<Screen>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Msg {
    Hello(Hello),
    Ping(u64),
    Pong(u64),
    /// Peer is shutting down or refusing the link, with a reason for the log.
    Bye(String),
    /// Pairing: the sender accepts (true) or rejects the code both sides displayed.
    PairConfirm(bool),

    /// Client -> server: its monitor layout changed.
    Screen(Screen),

    /// Server -> client: the cursor moved onto the client at (x, y).
    Enter { x: f64, y: f64 },
    /// Server -> client: the cursor left; release anything still held.
    Leave,
    Motion { x: f64, y: f64 },
    Button { button: Button, down: bool },
    Wheel { dx: i32, dy: i32 },
    Key { code: u16, down: bool, repeat: bool },

    Clip(ClipMsg),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClipContent {
    Text { text: String, html: Option<String> },
    Image { width: u32, height: u32, png: Vec<u8> },
}

/// One entry of a copied file set. `path` is relative and uses `/` separators.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub dir: bool,
    pub exec: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OfferKind {
    /// A postcard-encoded `ClipContent` too large to send inline.
    Blob,
    /// Copied files; their bytes follow in `entries` order.
    Files(Vec<FileEntry>),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ClipMeta {
    pub id: u64,
    /// When it was copied (Unix milliseconds on the copying machine).
    pub at: u64,
    /// Re-sent after a reconnect: apply only if newer than what the receiver has.
    pub replay: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClipMsg {
    /// Small clipboard contents, sent on the control link.
    Inline { meta: ClipMeta, content: ClipContent },
    /// Large contents: the receiver pulls them over a separate bulk connection.
    Offer { meta: ClipMeta, kind: OfferKind, bytes: u64 },
    /// Server -> client: push offer `id` to me over a bulk connection.
    Fetch { id: u64 },
}

/// Messages on a bulk connection (always dialled by the client).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Bulk {
    /// First message from the client. `push`: the client sends offer `id`; otherwise the
    /// server sends its offer `id` to the client.
    Open { id: u64, push: bool },
    Data(Vec<u8>),
    End,
    Error(String),
}
