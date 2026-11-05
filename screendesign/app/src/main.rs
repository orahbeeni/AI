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
mod images;
mod inspector;
mod mcp;
mod model;
mod pen;
mod picture;
mod render;

use eframe::egui;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let file = value("--file");
    start_log();
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
    // Graphics. A driver that crashes takes the whole process down, so on Windows a small launcher
    // starts the real app as a child with one graphics mode and tries the next when it dies while
    // starting (see `supervise`). `--renderer dx12|vulkan|gl|warp|glow|wgpu` (or SKETCHFRAME_RENDERER)
    // forces one mode and skips the launcher.
    let forced = value("--renderer").map(|p| p.to_string_lossy().into_owned()).or_else(|| std::env::var("SKETCHFRAME_RENDERER").ok());
    if cfg!(windows) && forced.is_none() && shot.is_none() {
        supervise(&args);
    }
    let order = match forced.as_deref().and_then(Gfx::parse) {
        Some(g) => vec![g],
        None if cfg!(windows) => vec![Gfx::Dx12, Gfx::Vulkan, Gfx::Warp, Gfx::Glow],
        None => vec![Gfx::Auto, Gfx::Glow],
    };
    let mut errors = vec![];
    for gfx in order {
        let open = open.clone();
        let shot = shot.clone();
        if std::env::var("SKETCHFRAME_TEST_CRASH").map(|v| v.split(',').any(|n| n == gfx.name())).unwrap_or(false) {
            // test hook: pretend this graphics mode crashes the driver
            log_startup(&format!("test: crashing on purpose in {gfx:?}"));
            unsafe { std::ptr::null_mut::<u8>().write_volatile(1) };
        }
        match eframe::run_native("Sketchframe", gfx.options(viewport.clone()), Box::new(move |cc| Ok(Box::new(app::App::new(cc, open, shot))))) {
            Ok(()) => {
                log_startup("window closed normally");
                return;
            }
            Err(e) => {
                errors.push(format!("{gfx:?}: {e}"));
                log_startup(&format!("{gfx:?} failed: {e}"));
            }
        }
    }
    fail(&format!("Sketchframe could not open its window.\n\n{}", errors.join("\n")));
}

/// Graphics modes. Dx12, Vulkan and Gl are wgpu backends, Warp is Windows' own software DirectX 12,
/// Glow is the separate OpenGL renderer and Auto lets wgpu choose.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Gfx {
    Auto,
    Dx12,
    Vulkan,
    Gl,
    Warp,
    Glow,
}

impl Gfx {
    fn name(self) -> &'static str {
        match self {
            Gfx::Auto => "wgpu",
            Gfx::Dx12 => "dx12",
            Gfx::Vulkan => "vulkan",
            Gfx::Gl => "gl",
            Gfx::Warp => "warp",
            Gfx::Glow => "glow",
        }
    }

    fn parse(s: &str) -> Option<Gfx> {
        [Gfx::Auto, Gfx::Dx12, Gfx::Vulkan, Gfx::Gl, Gfx::Warp, Gfx::Glow].into_iter().find(|g| g.name() == s)
    }

    fn options(self, viewport: egui::ViewportBuilder) -> eframe::NativeOptions {
        use eframe::{egui_wgpu, wgpu};
        let mut o = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, ..Default::default() };
        let backends = match self {
            Gfx::Auto => return o,
            Gfx::Glow => {
                o.renderer = eframe::Renderer::Glow;
                return o;
            }
            Gfx::Dx12 | Gfx::Warp => wgpu::Backends::DX12,
            Gfx::Vulkan => wgpu::Backends::VULKAN,
            Gfx::Gl => wgpu::Backends::GL,
        };
        let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        setup.instance_descriptor.backends = backends;
        if self == Gfx::Warp {
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
        }
        o.wgpu_options.wgpu_setup = setup.into();
        o
    }
}

/// How long a start may take before a crash no longer counts as "could not start".
const START_SECS: f32 = 20.0;

/// Windows launcher: runs this program again as a child with one graphics mode. A graphics driver
/// that crashes (as Intel's Vulkan driver does on some PCs) only kills the child, and the next mode
/// is tried. The mode that worked is remembered in renderer.txt and tried first next time.
fn supervise(args: &[String]) -> ! {
    let Ok(exe) = std::env::current_exe() else { fail("can't find the program's own file") };
    let memory = model::data_dir().join("renderer.txt");
    let mut order = vec![Gfx::Dx12, Gfx::Vulkan, Gfx::Warp, Gfx::Glow];
    if let Some(g) = std::fs::read_to_string(&memory).ok().and_then(|s| Gfx::parse(s.trim())) {
        order.retain(|o| *o != g);
        order.insert(0, g);
    }
    let mut failures = vec![];
    for g in order {
        let started = std::time::Instant::now();
        let child = std::process::Command::new(&exe).args(args).arg("--renderer").arg(g.name()).env("SKETCHFRAME_CHILD", "1").spawn();
        let mut code = None;
        let mut stayed = false;
        if let Ok(mut child) = child {
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        code = status.code();
                        break;
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
                if !stayed && started.elapsed().as_secs_f32() >= START_SECS {
                    // it has been running long enough: this mode works on this PC
                    stayed = true;
                    let _ = std::fs::write(&memory, g.name());
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
        log_startup(&format!("{} ended with {code:?} after {:.1}s", g.name(), started.elapsed().as_secs_f32()));
        if code == Some(0) || stayed {
            let _ = std::fs::write(&memory, g.name());
            std::process::exit(code.unwrap_or(0));
        }
        failures.push(format!("{}: {}", g.name(), code.map(|c| format!("exit code {c:#x}")).unwrap_or_else(|| "did not run".into())));
    }
    fail(&format!("Sketchframe could not open its window in any graphics mode.\n\n{}", failures.join("\n")));
}

/// First lines of startup.log: which version started, so a silent failure still leaves a trace.
fn start_log() {
    let log = model::data_dir().join("startup.log");
    if std::fs::metadata(&log).map(|m| m.len() > 50_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&log);
    }
    log_startup(&format!("--- start {} {}", env!("CARGO_PKG_VERSION"), std::env::args().skip(1).collect::<Vec<_>>().join(" ")));
    #[cfg(windows)]
    install_crash_log();
}

/// Windows: a crash that is not a Rust panic (access violation, stack overflow…) writes crash.log.
#[cfg(windows)]
fn install_crash_log() {
    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        next: *const ExceptionRecord,
        address: *const u8,
    }
    #[repr(C)]
    struct ExceptionPointers {
        record: *const ExceptionRecord,
        context: *const u8,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn SetUnhandledExceptionFilter(f: Option<unsafe extern "system" fn(*const ExceptionPointers) -> i32>) -> *const u8;
        fn GetModuleHandleW(name: *const u16) -> *const u8;
    }
    unsafe extern "system" fn filter(p: *const ExceptionPointers) -> i32 {
        let rec = &*(*p).record;
        let base = GetModuleHandleW(std::ptr::null()) as usize;
        let msg = format!(
            "Sketchframe {} crashed: exception {:#010x} at {:#x} (module offset {:#x})",
            env!("CARGO_PKG_VERSION"),
            rec.code,
            rec.address as usize,
            (rec.address as usize).wrapping_sub(base)
        );
        let _ = std::fs::create_dir_all(model::data_dir());
        let _ = std::fs::write(model::data_dir().join("crash.log"), &msg);
        log_startup(&msg);
        1
    }
    unsafe { SetUnhandledExceptionFilter(Some(filter)) };
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
    if std::env::var_os("SKETCHFRAME_CHILD").is_none() {
        message_box(&format!("{msg}\n\nDetails were saved to {}", log.display()));
    }
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
