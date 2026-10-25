//! A small log (`term.log` in the settings folder) for what is otherwise invisible: what the GPU
//! surface reported, long gaps between frames, and panics in background threads.

use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use eframe::egui_wgpu::SurfaceErrorAction;
use eframe::wgpu::CurrentSurfaceTexture;

use crate::config::config_dir;

static LAST_FRAME: Mutex<Option<Instant>> = Mutex::new(None);
static SEEN: [AtomicU64; 4] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

pub fn log(msg: &str) {
    let path = config_dir().join("term.log");
    let _ = std::fs::create_dir_all(config_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {msg}", crate::record::timestamp());
    }
}

/// Call once at startup: keeps the log small and records panics from background threads, which
/// would otherwise vanish silently (a terminal whose output thread died just stops updating).
pub fn init() {
    let path = config_dir().join("term.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 512 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    log(&format!("start term {}", env!("CARGO_PKG_VERSION")));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        log(&format!("PANIC in thread {:?}: {info}", thread.name().unwrap_or("?")));
        previous(info);
    }));
}

/// Called at the start of every frame with the number of input events in it. A frame after a long
/// quiet spell is logged, so a freeze after leaving the window idle shows up as "frames did / did not resume".
pub fn frame(events: usize) {
    let now = Instant::now();
    let mut last = LAST_FRAME.lock().unwrap();
    if let Some(prev) = *last
        && now.duration_since(prev) > Duration::from_secs(300)
    {
        log(&format!("frame after {} s without one ({events} input events in it)", now.duration_since(prev).as_secs()));
    }
    *last = Some(now);
}

/// What to do when the GPU surface did not hand over a frame. The defaults skip most failures
/// without asking for another frame, which would leave the window frozen on its last picture.
pub fn surface_status(status: &CurrentSurfaceTexture) -> SurfaceErrorAction {
    let (slot, name, action) = match status {
        // The compositor changed the surface (resize, scale, output).
        CurrentSurfaceTexture::Outdated => (0, "Outdated", SurfaceErrorAction::Reconfigure),
        CurrentSurfaceTexture::Lost => (1, "Lost", SurfaceErrorAction::RecreateSurface),
        // Hidden or minimised: nothing to draw, and the next event repaints.
        CurrentSurfaceTexture::Occluded => return SurfaceErrorAction::SkipFrame,
        // No frame in time: rebuild the swapchain and try again straight away.
        CurrentSurfaceTexture::Timeout => (2, "Timeout", SurfaceErrorAction::Reconfigure),
        _ => (3, "Validation or other", SurfaceErrorAction::RecreateSurface),
    };
    // Log the first of each kind, then every hundredth, so a failure loop cannot fill the disk.
    let n = SEEN[slot].fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n % 100 == 0 {
        log(&format!("GPU surface: {name} (#{n})"));
    }
    if n > 3 {
        // Failing repeatedly: do not spin.
        std::thread::sleep(Duration::from_millis(100));
    }
    action
}
