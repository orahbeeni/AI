//! The receiver window.

mod chrome;
mod drawer;
mod home;
mod icons;
mod keys;
mod overlays;
mod theme;
pub mod video;
mod widgets;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use egui::{Color32, Key, Pos2, Rect, Vec2};
use wfd_core::uibc::{InputEvent, MouseButton};

use crate::config::{PointerStyle, Settings};
use crate::engine::{EngineHandle, Phase, Shared};
use video::VideoRenderer;

/// Exit code reported by `--selftest` runs.
pub static SELFTEST_EXIT: AtomicI32 = AtomicI32::new(0);

pub struct SelfTest {
    pub screenshot: Option<PathBuf>,
    pub started: Instant,
    injected_at: Option<Instant>,
    shot_requested: bool,
}

impl SelfTest {
    pub fn new(screenshot: Option<PathBuf>) -> Self {
        Self { screenshot, started: Instant::now(), injected_at: None, shot_requested: false }
    }
}

/// Take a screenshot of whatever is on screen after a delay, then exit
/// (used to check the UI without a desktop).
pub struct Shot {
    pub path: PathBuf,
    pub after: Duration,
    pub started: Instant,
    requested: bool,
}

impl Shot {
    pub fn new(path: PathBuf, after: Duration) -> Self {
        Self { path, after, started: Instant::now(), requested: false }
    }
}

/// What the side panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drawer {
    Settings,
    Files,
    Activity,
}

pub struct ReceiverApp {
    pub shot: Option<Shot>,
    engine: EngineHandle,
    settings: Settings,
    renderer: VideoRenderer,
    renderer_note: Option<String>,
    gl: Option<Arc<egui_glow::glow::Context>>,
    frame_size: (u32, u32),
    frame_age_ms: f32,
    render_frames: u32,
    render_fps: f32,
    render_tick: Instant,
    pin_entry: String,
    always_allow: bool,
    drawer: Option<Drawer>,
    /// Why the last dropped item couldn't be shared.
    share_problem: Option<String>,
    restart_needed: bool,
    // input forwarding
    video_rect: Option<Rect>,
    forwarding: bool,
    input_space: (u32, u32),
    relative_mouse: bool,
    captured: bool,
    last_pointer: Option<Pos2>,
    mods: u8,
    wheel: Vec2,
    fullscreen: bool,
    fullscreen_hint: Option<Instant>,
    snapshot: Shared,
    selftest: Option<SelfTest>,
}

impl ReceiverApp {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: EngineHandle, settings: Settings, selftest: Option<SelfTest>) -> Self {
        theme::install(&cc.egui_ctx);
        let (renderer, note) = VideoRenderer::new(cc.gl.as_ref());
        if let Some(n) = &note {
            tracing::warn!("using CPU video conversion: {n}");
        }
        Self {
            shot: None,
            engine,
            settings,
            renderer,
            renderer_note: note,
            gl: cc.gl.clone(),
            frame_size: (0, 0),
            frame_age_ms: 0.0,
            render_frames: 0,
            render_fps: 0.0,
            render_tick: Instant::now(),
            pin_entry: String::new(),
            always_allow: false,
            drawer: None,
            share_problem: None,
            restart_needed: false,
            video_rect: None,
            forwarding: false,
            input_space: (1920, 1080),
            relative_mouse: false,
            captured: false,
            last_pointer: None,
            mods: 0,
            wheel: Vec2::ZERO,
            fullscreen: false,
            fullscreen_hint: None,
            snapshot: Shared::default(),
            selftest,
        }
    }

    pub fn open_settings(&mut self) {
        self.drawer = Some(Drawer::Settings);
    }

    fn toggle_drawer(&mut self, drawer: Drawer) {
        self.drawer = (self.drawer != Some(drawer)).then_some(drawer);
    }

    fn send_input(&self, ev: InputEvent) {
        if let Some(tx) = &self.snapshot.input {
            let _ = tx.send(ev);
        }
    }

    fn to_video(&self, p: Pos2) -> Option<(u16, u16)> {
        let r = self.video_rect?;
        let (w, h) = self.input_space;
        let x = ((p.x - r.min.x) / r.width() * w as f32).clamp(0.0, w as f32 - 1.0);
        let y = ((p.y - r.min.y) / r.height() * h as f32).clamp(0.0, h as f32 - 1.0);
        Some((x as u16, y as u16))
    }

    fn scale(&self) -> f32 {
        self.video_rect.map_or(1.0, |r| self.input_space.0 as f32 / r.width().max(1.0))
    }

    fn set_capture(&mut self, ctx: &egui::Context, on: bool) {
        if self.captured == on {
            return;
        }
        self.captured = on;
        let grab = if on {
            if cfg!(target_os = "macos") { egui::viewport::CursorGrab::Locked } else { egui::viewport::CursorGrab::Confined }
        } else {
            egui::viewport::CursorGrab::None
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(grab));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(!on));
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        if self.fullscreen {
            self.fullscreen_hint = Some(Instant::now());
        }
    }

    /// Input forwarding happens here, before egui sees the events, so key
    /// presses meant for the phone never move egui's focus.
    fn forward_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let mut keep = Vec::with_capacity(raw.events.len());
        let mut want_release = false;
        for ev in raw.events.drain(..) {
            // Global shortcuts first
            if let egui::Event::Key { key: Key::F11, pressed: true, .. } = ev {
                self.toggle_fullscreen(ctx);
                continue;
            }
            if !self.forwarding {
                keep.push(ev);
                continue;
            }
            // not when one of our own windows covers that part of the picture
            let over_video = |p: Pos2, r: Option<Rect>| {
                r.is_some_and(|r| r.contains(p)) && ctx.layer_id_at(p).is_none_or(|l| l.order == egui::Order::Background)
            };
            match &ev {
                egui::Event::PointerMoved(p) => {
                    if !self.captured && over_video(*p, self.video_rect) {
                        let (dx, dy) = match self.last_pointer {
                            Some(l) => (((p.x - l.x) * self.scale()).round() as i32, ((p.y - l.y) * self.scale()).round() as i32),
                            None => (0, 0),
                        };
                        if let Some((x, y)) = self.to_video(*p) {
                            self.send_input(InputEvent::PointerMove { x, y, dx, dy });
                        }
                    }
                    self.last_pointer = Some(*p);
                    keep.push(ev);
                }
                egui::Event::MouseMoved(d) if self.captured => {
                    let s = self.scale().max(0.5);
                    self.send_input(InputEvent::PointerMove { x: 0, y: 0, dx: (d.x * s).round() as i32, dy: (d.y * s).round() as i32 });
                }
                egui::Event::PointerButton { pos, button, pressed, .. } if self.captured || over_video(*pos, self.video_rect) => {
                    let b = match button {
                        egui::PointerButton::Primary => Some(MouseButton::Left),
                        egui::PointerButton::Secondary => Some(MouseButton::Right),
                        egui::PointerButton::Middle => Some(MouseButton::Middle),
                        _ => None,
                    };
                    if *pressed && !self.captured && self.relative_mouse && self.settings.capture_on_click {
                        self.set_capture(ctx, true);
                    }
                    if let (Some(b), Some((x, y))) = (b, self.to_video(*pos)) {
                        self.send_input(InputEvent::Button { button: b, pressed: *pressed, x, y });
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. }
                    if self.captured || self.last_pointer.is_some_and(|p| over_video(p, self.video_rect)) =>
                {
                    let per_notch = match unit {
                        egui::MouseWheelUnit::Point => 50.0,
                        egui::MouseWheelUnit::Line => 1.0,
                        egui::MouseWheelUnit::Page => 1.0 / 3.0,
                    };
                    self.wheel += *delta / per_notch;
                    let (nx, ny) = (self.wheel.x.trunc(), self.wheel.y.trunc());
                    if nx != 0.0 || ny != 0.0 {
                        self.wheel -= Vec2::new(nx, ny);
                        self.send_input(InputEvent::Wheel { dx: nx as i32, dy: ny as i32 });
                    }
                }
                egui::Event::Key { key, physical_key, pressed, modifiers, .. } => {
                    let key = physical_key.unwrap_or(*key);
                    if let Some(bit) = keys::modifier_bit(key) {
                        if *pressed {
                            self.mods |= bit
                        } else {
                            self.mods &= !bit
                        }
                        self.send_input(InputEvent::Modifiers(self.mods));
                    } else if let Some(usage) = keys::hid_usage(key) {
                        // platforms without separate modifier key events
                        let m = self.mods | keys::modifiers_from_egui(*modifiers);
                        self.send_input(InputEvent::Key { usage, pressed: *pressed, modifiers: m });
                    }
                    if modifiers.ctrl && modifiers.alt && self.captured {
                        want_release = true;
                    }
                }
                egui::Event::Text(_) | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) => {}
                egui::Event::WindowFocused(false) => {
                    self.send_input(InputEvent::ReleaseAll);
                    self.mods = 0;
                    want_release = true;
                    keep.push(ev);
                }
                _ => keep.push(ev),
            }
        }
        raw.events = keep;
        if want_release {
            self.set_capture(ctx, false);
            self.send_input(InputEvent::ReleaseAll);
            self.mods = 0;
        }
    }

    fn take_frame(&mut self, ctx: &egui::Context) {
        if let Some(f) = self.engine.frames.take() {
            self.frame_size = (f.width, f.height);
            self.frame_age_ms = f.decoded_at.elapsed().as_secs_f32() * 1000.0;
            self.renderer.set_frame(ctx, f);
            self.render_frames += 1;
        }
        let dt = self.render_tick.elapsed();
        if dt >= Duration::from_secs(1) {
            self.render_fps = self.render_frames as f32 / dt.as_secs_f32();
            self.render_frames = 0;
            self.render_tick = Instant::now();
        }
    }

    fn video_view(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_rect_before_wrap();
        let (_, resp) = ui.allocate_exact_size(avail.size(), egui::Sense::click());
        ui.painter().rect_filled(avail, 0.0, Color32::BLACK);
        let (fw, fh) = self.frame_size;
        if fw == 0 || fh == 0 {
            return;
        }
        // letterbox
        let aspect = fw as f32 / fh as f32;
        let mut size = avail.size();
        if size.x / size.y > aspect {
            size.x = size.y * aspect;
        } else {
            size.y = size.x / aspect;
        }
        let rect = Rect::from_center_size(avail.center(), size);
        self.video_rect = Some(rect);
        self.renderer.paint(ui.painter(), rect);

        if resp.double_clicked() && !self.forwarding {
            self.toggle_fullscreen(ui.ctx());
        }
        if self.forwarding && resp.hovered() && !self.captured {
            ui.ctx().set_cursor_icon(match self.settings.pointer {
                PointerStyle::Arrow => egui::CursorIcon::Default,
                PointerStyle::Hand => egui::CursorIcon::PointingHand,
                PointerStyle::Crosshair => egui::CursorIcon::Crosshair,
                PointerStyle::Hidden => egui::CursorIcon::None,
            });
        }
        if self.settings.show_overlay {
            self.stats_overlay(ui, rect);
        }
        self.fullscreen_hint(ui, rect);
    }

    /// Adds files dropped on the window to the list the device can download.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        self.drawer = Some(Drawer::Files);
        if self.snapshot.share_url.is_none() {
            self.share_problem = Some(if self.settings.file_share {
                "Connect a device first, then drop the files again.".into()
            } else {
                "File sharing is turned off in Settings.".into()
            });
            return;
        }
        let mut list = self.snapshot.shared_files.lock().unwrap();
        self.share_problem = None;
        for f in dropped {
            if let Err(e) = list.add(f.path()) {
                self.share_problem = Some(e);
            }
        }
    }

    fn run_selftest(&mut self, ctx: &egui::Context) {
        let Some(t) = self.selftest.as_mut() else { return };
        let snap = &self.snapshot;
        let frames = snap.media.as_ref().map_or(0, |m| m.frames);
        if t.started.elapsed() > Duration::from_secs(40) {
            eprintln!("SELFTEST FAIL: timed out (phase {:?}, frames {frames})", snap.session.phase);
            SELFTEST_EXIT.store(1, Ordering::SeqCst);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if snap.session.phase != Phase::Playing || frames < 45 || !self.renderer.has_frame() {
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        }
        // drive the input back-channel like a user would
        if t.injected_at.is_none() && snap.input.is_some() {
            if let Some(tx) = &snap.input {
                let (w, h) = self.input_space;
                let _ = tx.send(InputEvent::PointerMove { x: (w / 2) as u16, y: (h / 2) as u16, dx: 40, dy: -20 });
                let _ = tx.send(InputEvent::Button { button: MouseButton::Left, pressed: true, x: (w / 2) as u16, y: (h / 2) as u16 });
                let _ = tx.send(InputEvent::Button { button: MouseButton::Left, pressed: false, x: (w / 2) as u16, y: (h / 2) as u16 });
                let _ = tx.send(InputEvent::Key { usage: 0x0B, pressed: true, modifiers: 0x02 }); // Shift+H
                let _ = tx.send(InputEvent::Key { usage: 0x0B, pressed: false, modifiers: 0x02 });
            }
            t.injected_at = Some(Instant::now());
        }
        let settled = t.injected_at.is_some_and(|i| i.elapsed() > Duration::from_millis(1500)) || snap.input.is_none();
        if settled && !t.shot_requested {
            t.shot_requested = true;
            let path = t.screenshot.clone();
            let m = snap.media.clone().unwrap_or_default();
            let inputs = snap.sim_inputs.as_ref().map(|i| i.lock().unwrap().clone()).unwrap_or_default();
            eprintln!(
                "SELFTEST: playing {} · {}x{} · decoder {} · {:.1} fps · display {:.1} fps · renderer {}",
                snap.session.video.as_deref().unwrap_or("?"),
                m.width,
                m.height,
                m.decoder,
                m.fps,
                self.render_fps,
                if self.renderer.is_gpu() { "gpu" } else { "cpu" }
            );
            eprintln!("SELFTEST: input back-channel: {}", snap.session.uibc.as_ref().map_or("off".into(), |u| u.description.clone()));
            for i in &inputs {
                eprintln!("SELFTEST: simulated phone received: {i}");
            }
            let ok_input = snap.input.is_none() || inputs.iter().any(|i| i.contains("keyboard") || i.contains("key"));
            if !ok_input {
                eprintln!("SELFTEST FAIL: no input reached the simulated phone");
                SELFTEST_EXIT.store(2, Ordering::SeqCst);
            }
            let ctx2 = ctx.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                if let Some(p) = path {
                    match save_png(&p, &img) {
                        Ok(()) => eprintln!("SELFTEST: screenshot saved to {}", p.display()),
                        Err(e) => eprintln!("SELFTEST: screenshot failed: {e}"),
                    }
                }
                ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx2.request_repaint();
            })));
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

/// Shows a folder in the system's file manager.
fn open_folder(path: &std::path::Path) {
    let _ = std::fs::create_dir_all(path);
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    match std::process::Command::new(program).arg(path).spawn() {
        // reap it when it exits
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => tracing::warn!("cannot open {}: {e}", path.display()),
    }
}

pub fn save_png(path: &std::path::Path, img: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    w.write_image_data(&bytes).map_err(|e| e.to_string())
}

impl eframe::App for ReceiverApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.forward_input(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.snapshot = self.engine.snapshot();
        self.take_frame(&ctx);
        let session = self.snapshot.session.clone();
        let playing = matches!(session.phase, Phase::Playing | Phase::Paused);
        if !playing {
            if self.renderer.has_frame() {
                self.renderer.clear();
                self.frame_size = (0, 0);
            }
            if self.captured {
                self.set_capture(&ctx, false);
            }
        }
        if let Some(u) = &session.uibc {
            self.input_space = (u.width, u.height);
            self.relative_mouse = u.relative_mouse;
        } else if self.captured {
            self.set_capture(&ctx, false);
        }
        if ctx.input(|i| i.key_pressed(Key::F3)) && !self.forwarding {
            self.settings.show_overlay = !self.settings.show_overlay;
            self.settings.save();
        }
        // (an open drop-down list takes Escape for itself)
        if !self.forwarding
            && self.snapshot.prompts.is_empty()
            && !egui::Popup::is_any_open(&ctx)
            && ctx.input(|i| i.key_pressed(Key::Escape))
        {
            // Escape closes the side panel first, then leaves full screen
            if self.drawer.is_some() {
                self.drawer = None;
            } else if self.fullscreen {
                self.toggle_fullscreen(&ctx);
            }
        }
        self.take_dropped_files(&ctx);

        let show_video = playing && self.renderer.has_frame();
        if !self.fullscreen {
            egui::Panel::top("top").frame(chrome::bar_frame()).show(ui, |ui| self.header(ui));
            if session.phase != Phase::Idle {
                egui::Panel::bottom("bottom").frame(chrome::bar_frame()).show(ui, |ui| self.session_controls(ui));
            }
        }
        if let Some(drawer) = self.drawer {
            // never wider than the window can spare
            let width = drawer::WIDTH.min(ctx.content_rect().width() * 0.85);
            egui::Panel::right("drawer")
                .resizable(false)
                .exact_size(width)
                .frame(egui::Frame::new().fill(theme::SURFACE))
                .show(ui, |ui| self.drawer_contents(ui, drawer));
        }
        let fill = if show_video { Color32::BLACK } else { theme::BG };
        egui::CentralPanel::default().frame(egui::Frame::new().fill(fill)).show(ui, |ui| {
            if show_video {
                self.video_view(ui);
            } else {
                self.video_rect = None;
                self.home(ui);
            }
        });
        self.drop_overlay(&ctx);
        self.pairing_modal(&ctx);
        // keys go to the device only while the picture is up and nothing in
        // our own UI wants them
        self.forwarding = show_video
            && self.snapshot.input.is_some()
            && self.snapshot.prompts.is_empty()
            && self.drawer != Some(Drawer::Settings)
            && ctx.memory(|m| m.focused().is_none());

        self.run_selftest(&ctx);
        if let Some(shot) = self.shot.as_mut() {
            if !shot.requested && shot.started.elapsed() >= shot.after {
                shot.requested = true;
                let path = shot.path.clone();
                let ctx2 = ctx.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                    match save_png(&path, &img) {
                        Ok(()) => eprintln!("screenshot saved to {}", path.display()),
                        Err(e) => eprintln!("screenshot failed: {e}"),
                    }
                    ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
                    ctx2.request_repaint();
                })));
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if !playing {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, gl: Option<&egui_glow::glow::Context>) {
        self.renderer.destroy(gl.or(self.gl.as_deref()));
        self.engine.shutdown();
    }
}
