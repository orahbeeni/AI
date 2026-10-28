//! Operating-system specific input capture, input injection, screen layout and clipboard
//! change detection.

use crate::geom::Screen;
use crate::proto::Button;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

/// Synthesises input on this machine (client role). Positions are in the coordinate space of
/// the `Screen` last passed to `set_screen`.
pub trait Injector: Send {
    fn set_screen(&mut self, screen: &Screen);
    fn motion(&mut self, x: f64, y: f64);
    fn button(&mut self, button: Button, down: bool);
    /// 1/120ths of a notch, positive `dy` = up, positive `dx` = right.
    fn wheel(&mut self, dx: i32, dy: i32);
    fn key(&mut self, code: u16, down: bool);
    /// Whether this OS generates key repeat itself for held injected keys. If not, the
    /// server's repeats are injected.
    fn os_repeats_keys(&self) -> bool;
}

/// Reports clipboard changes cheaply so the clipboard is only read when it changed.
pub trait ClipboardWatch: Send {
    /// A number that changes whenever the clipboard does, or `None` if this platform cannot
    /// tell (then the contents are compared instead).
    fn change_count(&mut self) -> Option<u64>;
}

/// Monitor layout that is used when detection fails.
pub fn fallback_screen() -> Screen {
    Screen::single(1920, 1080)
}
