#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod encode;
mod ffmpeg;
mod player;
mod timeline;

use eframe::egui;

/// The app icon (Dock, taskbar, window), embedded in the binary.
fn icon() -> egui::IconData {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!("../assets/icon.png").as_slice()));
    let mut reader = decoder.read_info().expect("embedded icon is a valid PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("icon size")];
    let info = reader.next_frame(&mut buf).expect("embedded icon decodes");
    buf.truncate(info.buffer_size());
    egui::IconData { rgba: buf, width: info.width, height: info.height }
}

fn main() -> eframe::Result {
    let mut file = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("video {}\n\nUsage: video [FILE]\n\nConvert, cut and crop a video, and keep it under a file size.", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            // macOS passes this when launched from Finder.
            s if s.starts_with("-psn_") => {}
            s => file = Some(std::path::PathBuf::from(s)),
        }
    }

    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1000.0, 920.0])
        .with_min_inner_size([640.0, 620.0])
        .with_title("Video")
        .with_app_id("video")
        .with_icon(icon())
        .with_drag_and_drop(true);
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Video", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, file)))))
}
