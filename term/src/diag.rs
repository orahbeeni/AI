//! A small log (`term.log` in the settings folder) for what is otherwise invisible: what the GPU
//! surface reported, long gaps between frames, and panics in background threads.

use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui_wgpu::SurfaceErrorAction;
use eframe::wgpu::CurrentSurfaceTexture;

use crate::config::config_dir;

static LAST_FRAME: Mutex<Option<Instant>> = Mutex::new(None);
/// When a background thread last asked for a repaint (output arrived).
static REPAINT_ASKED: Mutex<Option<Instant>> = Mutex::new(None);
static SEEN: [AtomicU64; 5] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
/// Frames still to describe in detail after one that came after a long quiet spell.
static DETAIL: AtomicU64 = AtomicU64::new(0);

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
    let _ = std::thread::Builder::new().name("watchdog".into()).spawn(watchdog);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        log(&format!("PANIC in thread {:?}: {info}", thread.name().unwrap_or("?")));
        previous(info);
    }));
}

/// A background thread asked for a repaint; the watchdog expects a frame to follow.
pub fn repaint_requested() {
    *REPAINT_ASKED.lock().unwrap() = Some(Instant::now());
}

/// Notices a window that stops redrawing: output keeps asking for repaints but no frame ever runs.
/// (Native Wayland can do this for good when the compositor loses a frame callback.)
fn watchdog() {
    let mut stalled = false;
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let asked = *REPAINT_ASKED.lock().unwrap();
        let frame = *LAST_FRAME.lock().unwrap();
        let waiting = asked.is_some_and(|a| a.elapsed() > Duration::from_secs(3) && frame.is_none_or(|f| f < a));
        if waiting && !stalled {
            stalled = true;
            log(&format!(
                "STALL: a repaint was asked for {} s ago but no frame has run since; the window is not redrawing (hidden, or a lost Wayland frame callback)",
                asked.unwrap().elapsed().as_secs()
            ));
        } else if !waiting && stalled {
            stalled = false;
            log("recovered: frames are running again");
        }
    }
}

/// Called at the start of every frame. A frame after a long quiet spell is logged, and so are the
/// next few in detail (window size, focus, occlusion, pointer), so a freeze after leaving the
/// window idle shows whether frames resume, what they see, and whether the window thinks it is hidden.
pub fn frame(ctx: &egui::Context) {
    let now = Instant::now();
    let gap = {
        let mut last = LAST_FRAME.lock().unwrap();
        let gap = last.map(|prev| now.duration_since(prev));
        *last = Some(now);
        gap
    };
    let detail = |what: &str| {
        let (events, pointer, vp, size, ppp) = ctx.input(|i| {
            let vp = i.viewport();
            (
                i.events.len(),
                i.pointer.latest_pos().map(|p| format!("({:.0},{:.0})", p.x, p.y)).unwrap_or_else(|| "none".into()),
                format!(
                    "focused={:?} occluded={:?} minimized={:?} maximized={:?} fullscreen={:?}",
                    vp.focused, vp.occluded, vp.minimized, vp.maximized, vp.fullscreen
                ),
                i.content_rect().size(),
                i.pixels_per_point(),
            )
        });
        log(&format!(
            "{what}: {events} input events, pointer {pointer}, {vp}, size {:.0}x{:.0} @{ppp}x, wants_pointer={} using_pointer={}",
            size.x,
            size.y,
            ctx.egui_wants_pointer_input(),
            ctx.egui_is_using_pointer()
        ));
    };
    if gap.is_some_and(|g| g > Duration::from_secs(300)) {
        log(&format!("frame after {} s without one", gap.unwrap().as_secs()));
        DETAIL.store(12, Ordering::Relaxed);
    }
    let left = DETAIL.load(Ordering::Relaxed);
    if left > 0 {
        DETAIL.store(left - 1, Ordering::Relaxed);
        detail(&format!("  frame {}", 13 - left));
    }
}

/// What to do when the GPU surface did not hand over a frame. The defaults skip most failures
/// without asking for another frame, which would leave the window frozen on its last picture.
pub fn surface_status(status: &CurrentSurfaceTexture) -> SurfaceErrorAction {
    let (slot, name, action) = match status {
        // The compositor changed the surface (resize, scale, output).
        CurrentSurfaceTexture::Outdated => (0, "Outdated", SurfaceErrorAction::Reconfigure),
        CurrentSurfaceTexture::Lost => (1, "Lost", SurfaceErrorAction::RecreateSurface),
        // Reported as hidden or minimised: nothing is drawn, and only the next event repaints.
        // Logged, because a window wrongly believed hidden would look exactly like a freeze.
        CurrentSurfaceTexture::Occluded => (4, "Occluded", SurfaceErrorAction::SkipFrame),
        // No frame in time: rebuild the swapchain and try again straight away.
        CurrentSurfaceTexture::Timeout => (2, "Timeout", SurfaceErrorAction::Reconfigure),
        _ => (3, "Validation or other", SurfaceErrorAction::RecreateSurface),
    };
    // Log the first of each kind, then every hundredth, so a failure loop cannot fill the disk.
    let n = SEEN[slot].fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n % 100 == 0 {
        log(&format!("GPU surface: {name} (#{n})"));
    }
    if n > 3 && slot != 4 {
        // Failing repeatedly: do not spin.
        std::thread::sleep(Duration::from_millis(100));
    }
    action
}
