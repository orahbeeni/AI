#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod config;
mod cwd;
mod diag;
mod fonts;
mod input;
mod integration;
mod layout;
mod links;
mod marks;
mod notify;
mod pane;
mod record;
mod search;
mod session;
mod sound;
mod stream;
#[cfg(test)]
mod tests;
mod transfer;
mod view;

use eframe::egui;

fn usage() -> ! {
    println!(
        "term {}\n\nUsage: term [options] [DIR]\n\n  -l, --layout NAME   open a saved layout\n  -r, --run           also run the layout's auto-run commands\n      --fresh         ignore the last session and start with one terminal\n  -h, --help          show this help",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(0)
}

pub struct Args {
    pub layout: Option<String>,
    pub run: bool,
    pub fresh: bool,
    pub dir: Option<std::path::PathBuf>,
}

fn parse_args() -> Args {
    let mut a = Args { layout: None, run: false, fresh: false, dir: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => usage(),
            "-l" | "--layout" => a.layout = it.next(),
            "-r" | "--run" => a.run = true,
            "--fresh" => a.fresh = true,
            // macOS passes this when launched from Finder.
            s if s.starts_with("-psn_") => {}
            s => a.dir = Some(std::path::PathBuf::from(s)),
        }
    }
    a
}

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
    let args = parse_args();
    diag::init();
    let size = session::load_session().and_then(|s| s.window_size).unwrap_or([1000.0, 640.0]);
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(size)
        .with_min_inner_size([320.0, 200.0])
        .with_title("term")
        .with_app_id("term")
        .with_icon(icon());
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    // When the GPU does not hand over a frame, retry and rebuild instead of freezing on the last picture.
    options.wgpu_options.on_surface_status = std::sync::Arc::new(diag::surface_status);
    // Linux: native Wayland can stop redrawing the window for good, so use X11 (XWayland) unless told not to.
    #[cfg(target_os = "linux")]
    {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        if config::Settings::load().force_x11 && set("WAYLAND_DISPLAY") && set("DISPLAY") {
            diag::log("window system: X11 (XWayland); Preferences → General switches to native Wayland");
            options.event_loop_builder = Some(Box::new(|b| {
                use winit::platform::x11::EventLoopBuilderExtX11;
                b.with_x11();
            }));
        } else {
            diag::log("window system: automatic");
        }
    }
    // macOS: the standard app menu quits at once on ⌘Q. Without it, ⌘Q reaches term, which asks
    // first (⌘H and ⌘M, also from that menu, are handled by term too).
    #[cfg(target_os = "macos")]
    {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            b.with_default_menu(false);
        }));
    }
    eframe::run_native("term", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, args)))))
}
