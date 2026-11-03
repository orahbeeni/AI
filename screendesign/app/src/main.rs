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
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("Sketchframe crashed.\n\n{info}");
        if std::thread::current().name() == Some("main") {
            fail(&msg);
        }
        let _ = std::fs::create_dir_all(model::data_dir());
        let _ = std::fs::write(model::data_dir().join("error.log"), msg);
    }));

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
        .with_title("Sketchframe")
        .with_app_id("sketchframe");
    let viewport = match app_icon() {
        Some(icon) => viewport.with_icon(icon),
        None => viewport,
    };
    // Graphics: the GPU through wgpu first, then WARP (Windows' own software DirectX 12, for
    // GPUs wgpu skips or drivers that fail), then OpenGL. `--renderer wgpu|warp|glow|dx12|vulkan|gl`
    // (or SKETCHFRAME_RENDERER) forces one; dx12, vulkan and gl pick a wgpu backend.
    let forced = value("--renderer").map(|p| p.to_string_lossy().into_owned()).or_else(|| std::env::var("SKETCHFRAME_RENDERER").ok());
    let order = match forced.as_deref() {
        Some("glow") => vec![Gfx::Glow],
        Some("warp") => vec![Gfx::Warp],
        Some(b @ ("dx12" | "vulkan" | "gl")) => {
            std::env::set_var("WGPU_BACKEND", b);
            vec![Gfx::Wgpu]
        }
        Some("wgpu") => vec![Gfx::Wgpu],
        _ if cfg!(windows) => vec![Gfx::Wgpu, Gfx::Warp, Gfx::Glow],
        _ => vec![Gfx::Wgpu, Gfx::Glow],
    };
    let mut errors = vec![];
    for gfx in order {
        let open = open.clone();
        let shot = shot.clone();
        match eframe::run_native("Sketchframe", gfx.options(viewport.clone()), Box::new(move |cc| Ok(Box::new(app::App::new(cc, open, shot))))) {
            Ok(()) => return,
            Err(e) => {
                errors.push(format!("{gfx:?}: {e}"));
                log_startup(&format!("{gfx:?} failed: {e}"));
            }
        }
    }
    fail(&format!("Sketchframe could not open its window.\n\n{}", errors.join("\n")));
}

#[derive(Clone, Copy, Debug)]
enum Gfx {
    Wgpu,
    Warp,
    Glow,
}

impl Gfx {
    fn options(self, viewport: egui::ViewportBuilder) -> eframe::NativeOptions {
        use eframe::{egui_wgpu, wgpu};
        let mut o = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, ..Default::default() };
        match self {
            // On Windows wgpu's OpenGL backend can start yet show only black (VMware's SVGA3D
            // driver does), so leave it out there; without a DirectX 12 or Vulkan GPU, wgpu then
            // lands on WARP.
            Gfx::Wgpu if cfg!(windows) => {
                let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
                setup.instance_descriptor.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY);
                o.wgpu_options.wgpu_setup = setup.into();
            }
            Gfx::Wgpu => {}
            Gfx::Glow => o.renderer = eframe::Renderer::Glow,
            Gfx::Warp => {
                let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
                setup.instance_descriptor.backends = wgpu::Backends::DX12;
                setup.native_adapter_selector = Some(std::sync::Arc::new(|adapters, surface| {
                    for a in adapters {
                        let i = a.get_info();
                        log_startup(&format!("dx12 adapter: {} ({:?})", i.name, i.device_type));
                    }
                    adapters
                        .iter()
                        .find(|a| a.get_info().device_type == wgpu::DeviceType::Cpu && surface.is_none_or(|s| a.is_surface_supported(s)))
                        .cloned()
                        .ok_or_else(|| "no WARP (software DirectX 12) adapter".to_string())
                }));
                o.wgpu_options.wgpu_setup = setup.into();
            }
        }
        o
    }
}

/// Appends a line to startup.log in the data folder, to see which graphics path a machine used.
pub fn log_startup(line: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(model::data_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(model::data_dir().join("startup.log")) {
        let _ = writeln!(f, "{line}");
    }
}

/// Windows GUI builds have no console, so write problems to a log file and show a message box.
fn fail(msg: &str) -> ! {
    let log = model::data_dir().join("error.log");
    let _ = std::fs::create_dir_all(model::data_dir());
    let _ = std::fs::write(&log, msg);
    eprintln!("{msg}");
    #[cfg(windows)]
    message_box(&format!("{msg}\n\nDetails were saved to {}", log.display()));
    std::process::exit(1);
}

#[cfg(windows)]
fn message_box(text: &str) {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: *mut u8, text: *const u16, caption: *const u16, flags: u32) -> i32;
    }
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide("Sketchframe").as_ptr(), 0x10) };
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
