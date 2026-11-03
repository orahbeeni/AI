#![recursion_limit = "512"]
// No console window behind the app on Windows (pipes for --mcp still work).
#![cfg_attr(windows, windows_subsystem = "windows")]
//! Sketchframe: sketch-style screen design for every modern device, with an MCP server for AI.
//!
//!   sketchframe [project.json]           open the editor
//!   sketchframe --screenshot shot.png    draw ~120 frames, save the window picture, exit
//!   sketchframe --mcp [--file p.json]    run as an MCP server on stdin/stdout
//!   sketchframe --export out.(png|svg|html) [--file p.json] [--screen id]

mod app;
mod canvas;
mod catalog;
mod controls;
mod controls2;
mod draw;
mod icons;
mod inspector;
mod mcp;
mod model;
mod pen;
mod render;

use eframe::egui;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let file = value("--file");

    if args.iter().any(|a| a == "--mcp") {
        mcp::run(file);
        return;
    }
    if let Some(out) = value("--export") {
        std::process::exit(export(&out, file, args.iter().position(|a| a == "--screen").and_then(|i| args.get(i + 1)).cloned()));
    }

    let shot = value("--screenshot");
    let open = file.or_else(|| args.iter().find(|a| !a.starts_with("--") && a.ends_with(".json")).map(PathBuf::from));
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([900.0, 600.0])
        .with_title("Sketchframe");
    let viewport = match app_icon() {
        Some(icon) => viewport.with_icon(icon),
        None => viewport,
    };
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    if let Err(e) = eframe::run_native("Sketchframe", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, open, shot))))) {
        eprintln!("could not open the window: {e}");
        std::process::exit(1);
    }
}

fn app_icon() -> Option<egui::IconData> {
    let bytes: &[u8] = include_bytes!("../windows/icon.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some(egui::IconData { rgba: buf, width: info.width, height: info.height })
}

fn export(out: &PathBuf, file: Option<PathBuf>, screen: Option<String>) -> i32 {
    let p = match file {
        Some(f) => model::Project::load(&f),
        None => model::Project::load(&model::current_file()).or_else(|_| Ok(model::sample())),
    };
    let p = match p {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let idx = screen.as_deref().and_then(|s| p.screen_idx(s)).unwrap_or(0);
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let res = match ext.as_str() {
        "png" => render::screen_png(&p, idx, true, true, 2.0).and_then(|d| std::fs::write(out, d).map_err(|e| e.to_string())),
        "svg" => std::fs::write(out, render::screen_svg(&p, idx, true, true)).map_err(|e| e.to_string()),
        "html" => std::fs::write(out, render::prototype_html(&p, true)).map_err(|e| e.to_string()),
        _ => Err("use a .png, .svg or .html file name".into()),
    };
    match res {
        Ok(()) => {
            println!("wrote {}", out.display());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
