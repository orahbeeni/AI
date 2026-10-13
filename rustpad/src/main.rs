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

/// The window / task-bar icon: the RustPad mascot, embedded as a PNG.
fn icon() -> egui::IconData {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!("../assets/app_icon.png").as_slice()));
    let mut reader = decoder.read_info().expect("embedded icon is a valid PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("icon size")];
    let info = reader.next_frame(&mut buf).expect("embedded icon decodes");
    buf.truncate(info.buffer_size());
    egui::IconData { rgba: buf, width: info.width, height: info.height }
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
        .with_app_id("rustpad")
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
