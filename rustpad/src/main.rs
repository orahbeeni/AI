#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod cmd;
mod debug;
mod doc;
mod editor;
mod encoding;
mod fold;
#[cfg(test)]
mod fuzz_tests;
mod hash;
mod icons;
mod fonts;
mod lang;
mod lexer;
mod ops;
mod rowmap;
mod search;
mod session;
mod settings;
mod theme;
mod ui;
mod util;

use eframe::egui;

/// A small procedurally drawn window icon (a notepad page with text lines).
fn icon() -> egui::IconData {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let i = (y * n + x) * 4;
            let inside = x >= 8 && x < 56 && y >= 4 && y < 60;
            if !inside {
                continue;
            }
            let corner = (x < 11 || x >= 53) && (y < 7 || y >= 57);
            if corner {
                continue;
            }
            let header = y < 16;
            let line = y >= 22 && (y - 22) % 9 < 3 && y < 56 && x >= 14 && x < 50 - (((y - 22) / 9) as usize % 3) * 8;
            let (r, g, b) = if header {
                (0xd9, 0x6a, 0x1a)
            } else if line {
                (0x55, 0x5f, 0x70)
            } else {
                (0xf7, 0xf7, 0xf2)
            };
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = 255;
        }
    }
    egui::IconData { rgba, width: n as u32, height: n as u32 }
}

fn main() -> eframe::Result {
    // The embedded web view (wry) can only attach as a child window under X11, so on Linux run through
    // XWayland when a Wayland session also offers an X display.
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_some() && std::env::var_os("RUSTPAD_WAYLAND").is_none() {
        // SAFETY: nothing else has started yet, so no other thread can be reading the environment.
        unsafe {
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let settings = settings::Settings::load();
    let size = settings.window_size.unwrap_or([1100.0, 720.0]);
    let mut vp = egui::ViewportBuilder::default()
        .with_inner_size(size)
        .with_min_inner_size([480.0, 320.0])
        .with_title("RustPad")
        .with_icon(icon())
        .with_drag_and_drop(true);
    if let Some(p) = settings.window_pos {
        vp = vp.with_position(p);
    }
    if settings.maximized {
        vp = vp.with_maximized(true);
    }
    let options = eframe::NativeOptions { viewport: vp, ..Default::default() };
    eframe::run_native("RustPad", options, Box::new(|cc| Ok(Box::new(app::App::new(cc, args)))))
}
