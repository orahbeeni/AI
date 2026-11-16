#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod ui;

use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mut screenshot = None;
    let mut files = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--screenshot" {
            screenshot = args.next().map(PathBuf::from);
        } else {
            files.push(PathBuf::from(arg));
        }
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Cutline")
        .with_app_id("cutline")
        .with_inner_size([1360.0, 860.0])
        .with_min_inner_size([1060.0, 680.0])
        .with_icon(app_icon())
        .with_drag_and_drop(true);
    if screenshot.is_some() {
        viewport = viewport.with_active(false);
        #[cfg(windows)]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }

    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        persist_window: screenshot.is_none(),
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    if screenshot.is_some() {
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            builder.with_activate_ignoring_other_apps(false);
        }));
    }

    eframe::run_native(
        "Cutline",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, files, screenshot)))),
    )
}

/// A small film frame, drawn here so there's no image file to ship.
fn app_icon() -> egui::IconData {
    const SIZE: u32 = 64;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let edge = x.min(SIZE - 1 - x).min(y).min(SIZE - 1 - y);
            let t = (x + y) as f32 / (2 * SIZE) as f32;
            let body = [(110.0 - 40.0 * t) as u8, (123.0 + 70.0 * t) as u8, (255.0 - 40.0 * t) as u8, 255];
            let hole = (6..12).contains(&y) || (52..58).contains(&y);
            let pixel = if edge < 2 {
                [0, 0, 0, 0]
            } else if hole && (x / 9) % 2 == 0 {
                [16, 18, 22, 255]
            } else {
                body
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    egui::IconData { rgba, width: SIZE, height: SIZE }
}
