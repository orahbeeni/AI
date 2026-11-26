//! The receiver window.

mod keys;
pub mod video;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Key, Pos2, Rect, RichText, Vec2};
use wfd_core::uibc::{InputEvent, MouseButton};
use wfd_transport::{PairingMethod, PairingReply, TransportKind, TransportState};

use crate::config::{Decoder, MaxVideo, PointerStyle, Settings};
use crate::engine::{EngineCmd, EngineHandle, Phase, Shared};
use crate::share::{download_folder, human_size};
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
    show_settings: bool,
    show_files: bool,
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
        let mut style = (*cc.egui_ctx.global_style()).clone();
        style.visuals = egui::Visuals::dark();
        cc.egui_ctx.set_global_style(style);
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
            show_settings: false,
            show_files: false,
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
        self.show_settings = true;
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

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.snapshot.receiver_name).strong().size(16.0));
            ui.separator();
            for (label, (kind, state)) in &self.snapshot.transports {
                if *kind == TransportKind::Direct {
                    continue;
                }
                let (color, text) = match state {
                    TransportState::Ready { .. } => (Color32::from_rgb(80, 200, 120), "ready"),
                    TransportState::Busy { .. } | TransportState::Starting => (Color32::from_rgb(230, 190, 60), "busy"),
                    TransportState::Unavailable { .. } => (Color32::from_rgb(150, 150, 150), "off"),
                    TransportState::Error { .. } => (Color32::from_rgb(230, 90, 80), "error"),
                };
                let detail = match state {
                    TransportState::Ready { detail } | TransportState::Busy { detail } => detail.clone(),
                    TransportState::Unavailable { reason } => reason.clone(),
                    TransportState::Error { message } => message.clone(),
                    TransportState::Starting => "starting".into(),
                };
                status_dot(ui, color);
                ui.label(format!("{label}: {text}")).on_hover_text(detail);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.show_settings = !self.show_settings;
                }
                if ui.button("Full screen").on_hover_text("F11 or double-click the picture").clicked() {
                    self.toggle_fullscreen(ui.ctx());
                }
                if ui.selectable_label(self.settings.show_overlay, "Stats").on_hover_text("Stats overlay (F3)").clicked() {
                    self.settings.show_overlay = !self.settings.show_overlay;
                    self.settings.save();
                }
            });
        });
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        let s = &self.snapshot.session;
        ui.horizontal(|ui| {
            let kind = s.kind.map(|k| k.label()).unwrap_or("");
            let status = match s.phase {
                Phase::Playing => format!("Showing {} ({kind})", s.peer_name),
                Phase::Paused => format!("{} paused", s.peer_name),
                Phase::Negotiating => format!("Negotiating with {}…", s.peer_name),
                Phase::Connecting => format!("Connecting to {}…", s.peer_name),
                Phase::Idle => "Waiting for a device".into(),
            };
            ui.label(status);
            if let (Phase::Playing, Some(t)) = (s.phase, s.since) {
                let secs = t.elapsed().as_secs();
                ui.label(RichText::new(format!("{}:{:02}", secs / 60, secs % 60)).weak());
            }
            if let Some(v) = &s.video {
                ui.label(RichText::new(format!("· {v}")).weak());
            }
            if let Some(u) = &s.uibc {
                let hint = if u.relative_mouse {
                    if self.captured { "mouse locked: Ctrl+Alt releases" } else { "click the picture to control" }
                } else {
                    "mouse and keyboard go to the device"
                };
                ui.label(RichText::new(format!("· input: {hint}")).weak()).on_hover_text(&u.description);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if s.phase != Phase::Idle && ui.button("Disconnect").clicked() {
                    self.engine.send(EngineCmd::Disconnect);
                }
                if self.snapshot.share_url.is_some()
                    && ui.selectable_label(self.show_files, "Files").on_hover_text("Share files with the device").clicked()
                {
                    self.show_files = !self.show_files;
                }
                if s.phase == Phase::Playing
                    && ui.button("Refresh").on_hover_text("Refresh the picture (asks the device for a key frame)").clicked()
                {
                    self.engine.send(EngineCmd::RequestIdr);
                }
                let mut vol = self.settings.volume as f32;
                if ui.add(egui::Slider::new(&mut vol, 0.0..=1.5).show_value(false)).changed() {
                    self.settings.volume = vol as f64;
                    self.engine.send(EngineCmd::SetVolume(vol as f64));
                    self.settings.save();
                }
                let icon = if self.settings.muted { "Unmute" } else { "Mute" };
                if ui.button(icon).clicked() {
                    self.settings.muted = !self.settings.muted;
                    self.engine.send(EngineCmd::SetMuted(self.settings.muted));
                    self.settings.save();
                }
            });
        });
    }

    fn idle_screen(&mut self, ui: &mut egui::Ui) {
        let s = self.snapshot.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height().min(600.0) * 0.08);
                let (title, sub) = match s.session.phase {
                    Phase::Connecting => (format!("Connecting to {}…", s.session.peer_name), "Opening the screen-sharing session"),
                    Phase::Negotiating | Phase::Playing | Phase::Paused => {
                        (format!("{} connected", s.session.peer_name), "Waiting for the first picture…")
                    }
                    Phase::Idle => ("Ready to receive".to_string(), "Cast your screen to this window"),
                };
                ui.label(RichText::new(title).size(30.0).strong());
                ui.label(RichText::new(sub).size(16.0).weak());
                ui.add_space(18.0);
                ui.label(RichText::new("This receiver appears as").weak());
                ui.label(RichText::new(&s.receiver_name).size(24.0).color(Color32::from_rgb(120, 190, 255)));
                ui.add_space(18.0);

                let wifi_direct_ok = s.transports.values().any(|(k, st)| {
                    *k == TransportKind::WifiDirect && matches!(st, TransportState::Ready { .. } | TransportState::Busy { .. })
                });
                egui::Frame::group(ui.style()).inner_margin(14.0).show(ui, |ui| {
                    ui.set_max_width(620.0);
                    ui.label(RichText::new("How to connect").strong());
                    ui.label(format!(
                        "Samsung DeX / Android: open Quick Settings > Smart View (or the DeX tile) > choose “{}”.",
                        s.receiver_name
                    ));
                    ui.label(format!("Windows: press Win + K > choose “{}”.", s.receiver_name));
                    if !wifi_direct_ok {
                        ui.label(
                            RichText::new("Phones need Wi-Fi Direct, which isn't active on this computer yet (see below).")
                                .color(Color32::from_rgb(230, 190, 60)),
                        );
                    }
                });
                ui.add_space(10.0);
                egui::Frame::group(ui.style()).inner_margin(14.0).show(ui, |ui| {
                    ui.set_max_width(620.0);
                    ui.label(RichText::new("Connection methods").strong());
                    for (label, (kind, state)) in &s.transports {
                        if *kind == TransportKind::Direct {
                            continue;
                        }
                        let (dot, text) = match state {
                            TransportState::Ready { detail } => (Color32::from_rgb(80, 200, 120), format!("ready — {detail}")),
                            TransportState::Busy { detail } => (Color32::from_rgb(230, 190, 60), detail.clone()),
                            TransportState::Starting => (Color32::from_rgb(230, 190, 60), "starting…".into()),
                            TransportState::Unavailable { reason } => {
                                (Color32::from_rgb(150, 150, 150), format!("not available — {reason}"))
                            }
                            TransportState::Error { message } => (Color32::from_rgb(230, 90, 80), message.clone()),
                        };
                        ui.horizontal_wrapped(|ui| {
                            status_dot(ui, dot);
                            ui.label(RichText::new(label).strong());
                            ui.label(text);
                        });
                    }
                });
                if cfg!(target_os = "macos") {
                    ui.label(
                        RichText::new("Infrastructure Mode (MS-MICE) is active. Wi-Fi Direct is not supported by macOS.")
                            .color(Color32::from_rgb(230, 190, 60)),
                    );
                }
                if let Some(e) = &s.last_error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(format!("Last attempt failed: {e}")).color(Color32::from_rgb(240, 110, 100)));
                }
                ui.add_space(16.0);
                if s.sim_running {
                    if ui.button("Stop the simulated phone").clicked() {
                        self.engine.send(EngineCmd::StopSimulation);
                    }
                } else if s.session.phase == Phase::Idle && ui.button("Test with a simulated phone").clicked() {
                    self.engine.send(EngineCmd::StartSimulation);
                }
                ui.add_space(16.0);
                egui::CollapsingHeader::new("Activity").default_open(false).show(ui, |ui| {
                    if ui.small_button("Copy log").clicked() {
                        let text: String = s.log.iter().map(|(t, m)| format!("{:>5}s ago  {m}\n", t.elapsed().as_secs())).collect();
                        ui.ctx().copy_text(text);
                    }
                    for (t, m) in s.log.iter().rev().take(40) {
                        ui.label(RichText::new(format!("{:>4}s ago  {m}", t.elapsed().as_secs())).small().weak());
                    }
                });
            });
        });
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
            self.overlay(ui.painter(), rect);
        }
        if let Some(t) = self.fullscreen_hint
            && t.elapsed() < Duration::from_secs(3)
        {
            ui.painter().text(
                rect.center_top() + Vec2::new(0.0, 30.0),
                Align2::CENTER_TOP,
                "Press F11 to leave full screen",
                FontId::proportional(18.0),
                Color32::from_white_alpha(200),
            );
        }
    }

    fn overlay(&self, painter: &egui::Painter, rect: Rect) {
        let m = self.snapshot.media.clone().unwrap_or_default();
        let s = &self.snapshot.session;
        let lines = [
            format!("decoder   {} · {}", m.decoder, if self.renderer.is_gpu() { "GPU YUV shader" } else { "CPU convert" }),
            format!("video     {}x{} · decode {:.1} fps · display {:.1} fps", m.width, m.height, m.fps, self.render_fps),
            format!("network   {:.1} Mbit/s · lost packets {}", m.bitrate_kbps / 1000.0, m.lost_packets),
            format!(
                "latency   decode-to-display {:.0} ms · jitter buffer {} ms · {}",
                self.frame_age_ms,
                self.settings.jitter_ms,
                if self.settings.av_sync { "A/V sync" } else { "low-latency" }
            ),
            format!("stream    {} · {}", s.video.as_deref().unwrap_or("-"), s.audio.as_deref().unwrap_or("no audio")),
            format!("input     {}", s.uibc.as_ref().map_or("off".to_string(), |u| u.description.clone())),
        ];
        let font = FontId::monospace(13.0);
        let pos = rect.min + Vec2::new(10.0, 10.0);
        let galleys: Vec<_> =
            lines.iter().map(|l| painter.layout_no_wrap(l.clone(), font.clone(), Color32::from_rgb(200, 255, 200))).collect();
        let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 16.0;
        let line_h = galleys.first().map_or(17.0, |g| g.size().y + 2.0);
        painter.rect_filled(
            Rect::from_min_size(pos, Vec2::new(width, line_h * lines.len() as f32 + 12.0)),
            6.0,
            Color32::from_black_alpha(190),
        );
        for (i, g) in galleys.into_iter().enumerate() {
            painter.galley(pos + Vec2::new(8.0, 6.0 + line_h * i as f32), g, Color32::WHITE);
        }
    }

    fn pairing_modal(&mut self, ctx: &egui::Context) {
        let Some(p) = self.snapshot.prompts.first().cloned() else { return };
        let mut reply = None;
        egui::Modal::new(egui::Id::NULL.with(("pair", p.id))).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(format!("Allow “{}” to show its screen?", p.device_name));
            ui.label(RichText::new(format!("via {}", p.kind)).weak());
            ui.add_space(8.0);
            match &p.method {
                PairingMethod::PushButton => {
                    ui.label("The device asked to connect.");
                }
                PairingMethod::DisplayPin(pin) => {
                    ui.label(format!("Enter this PIN on {}:", p.device_name));
                    let spaced = if pin.len() == 8 { format!("{} {}", &pin[..4], &pin[4..]) } else { pin.clone() };
                    ui.label(RichText::new(spaced).size(34.0).monospace().strong());
                }
                PairingMethod::EnterPin => {
                    ui.label(format!("Type the PIN shown on {}:", p.device_name));
                    let r = ui.add(egui::TextEdit::singleline(&mut self.pin_entry).hint_text("PIN").desired_width(160.0));
                    r.request_focus();
                }
            }
            ui.add_space(6.0);
            if p.method == PairingMethod::PushButton {
                ui.checkbox(&mut self.always_allow, "Always allow without asking");
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let can_allow = p.method != PairingMethod::EnterPin || self.pin_entry.trim().len() >= 4;
                if ui.add_enabled(can_allow, egui::Button::new(RichText::new("Allow").strong())).clicked()
                    || (can_allow && ui.input(|i| i.key_pressed(Key::Enter)))
                {
                    reply = Some(match p.method {
                        PairingMethod::EnterPin => PairingReply::AcceptWithPin(self.pin_entry.trim().to_string()),
                        _ => PairingReply::Accept,
                    });
                }
                if ui.button("Deny").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    reply = Some(PairingReply::Reject);
                }
            });
        });
        if let Some(r) = reply {
            if self.always_allow && r == PairingReply::Accept {
                self.settings.auto_accept = true;
                self.settings.save();
                self.engine.send(EngineCmd::UpdateSettings(self.settings.clone()));
            }
            self.pin_entry.clear();
            self.always_allow = false;
            self.engine.send(EngineCmd::PairingReply { id: p.id, reply: r });
            // hide it right away; the engine confirms on its next update
            self.snapshot.prompts.retain(|x| x.id != p.id);
        }
    }

    /// Adds files dropped on the window to the list the device can download.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        self.show_files = true;
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

    fn files_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_files;
        egui::Window::new("Files")
            .open(&mut open)
            .collapsible(true)
            .resizable(false)
            .default_width(380.0)
            .anchor(Align2::RIGHT_BOTTOM, [-12.0, -44.0])
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                if let Some(url) = &self.snapshot.share_url {
                    ui.label("On the device, open its web browser and go to:");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(url).size(22.0).monospace().strong());
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(url.clone());
                        }
                    });
                    ui.label(RichText::new("Download files there, or send files back to this computer.").small().weak());
                }
                if let Some(p) = &self.share_problem {
                    ui.label(RichText::new(p).color(Color32::from_rgb(230, 190, 60)));
                }
                ui.separator();
                ui.label(RichText::new("Shared with the device").strong());
                let files = self.snapshot.shared_files.lock().unwrap().files().to_vec();
                if files.is_empty() {
                    ui.label(RichText::new("Drag files onto this window.").weak());
                }
                egui::ScrollArea::vertical().id_salt("shared").max_height(180.0).show(ui, |ui| {
                    for f in files {
                        ui.horizontal(|ui| {
                            if ui.small_button("Remove").clicked() {
                                self.snapshot.shared_files.lock().unwrap().remove(f.id);
                            }
                            ui.add(egui::Label::new(format!("{}  ({})", f.name, human_size(f.size))).truncate());
                        });
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Received from the device").strong());
                    if ui.small_button("Open folder").clicked() {
                        open_folder(&download_folder());
                    }
                });
                if self.snapshot.received.is_empty() {
                    ui.label(RichText::new("Saved in Downloads/Miracast.").weak());
                }
                egui::ScrollArea::vertical().id_salt("received").max_height(120.0).show(ui, |ui| {
                    for p in self.snapshot.received.iter().rev() {
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.add(egui::Label::new(name).truncate());
                    }
                });
            });
        self.show_files = open;
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        let before = self.settings.clone();
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .anchor(Align2::RIGHT_TOP, [-12.0, 40.0])
            .show(ctx, |ui| {
                egui::Grid::new("settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Receiver name");
                    ui.text_edit_singleline(&mut self.settings.name);
                    ui.end_row();
                    ui.label("Wi-Fi Direct");
                    ui.checkbox(&mut self.settings.wifi_direct, "phones, DeX, Windows");
                    ui.end_row();
                    ui.label("Infrastructure");
                    ui.checkbox(&mut self.settings.infrastructure, "MS-MICE over your network (Windows)");
                    ui.end_row();
                    ui.label("Connections");
                    ui.checkbox(&mut self.settings.auto_accept, "allow without asking");
                    ui.end_row();
                    ui.label("Mouse & keyboard");
                    ui.checkbox(&mut self.settings.uibc, "send to the device (UIBC)");
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut self.settings.capture_on_click, "click locks the mouse to the picture");
                    ui.end_row();
                    ui.label("Mouse pointer");
                    egui::ComboBox::from_id_salt("pointer").selected_text(self.settings.pointer.label()).show_ui(ui, |ui| {
                        for p in PointerStyle::ALL {
                            ui.selectable_value(&mut self.settings.pointer, p, p.label());
                        }
                    });
                    ui.end_row();
                    ui.label("File sharing");
                    ui.checkbox(&mut self.settings.file_share, "exchange files with the connected device");
                    ui.end_row();
                    ui.label("Max quality");
                    egui::ComboBox::from_id_salt("maxv").selected_text(self.settings.max_video.label()).show_ui(ui, |ui| {
                        for m in [MaxVideo::P1080p60, MaxVideo::P1080p30, MaxVideo::P720p60, MaxVideo::P720p30] {
                            ui.selectable_value(&mut self.settings.max_video, m, m.label());
                        }
                    });
                    ui.end_row();
                    ui.label("Video decoder");
                    egui::ComboBox::from_id_salt("dec").selected_text(format!("{:?}", self.settings.decoder)).show_ui(ui, |ui| {
                        for d in [Decoder::Auto, Decoder::Hardware, Decoder::Software] {
                            ui.selectable_value(&mut self.settings.decoder, d, format!("{d:?}"));
                        }
                    });
                    ui.end_row();
                    ui.label("Playback");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.settings.av_sync, false, "lowest latency");
                        ui.radio_value(&mut self.settings.av_sync, true, "smooth (A/V sync)");
                    });
                    ui.end_row();
                    ui.label("Network buffer");
                    ui.add(egui::Slider::new(&mut self.settings.jitter_ms, 20..=300).suffix(" ms"));
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.label(RichText::new(format!("Decoders found: {}", wfd_media::available_decoders().join(", "))).small().weak());
                if let Some(n) = &self.renderer_note {
                    ui.label(RichText::new(format!("Video drawing: CPU fallback ({n})")).small().weak());
                }
                if self.restart_needed {
                    ui.label(
                        RichText::new("Name and connection types apply after restarting discovery.").color(Color32::from_rgb(230, 190, 60)),
                    );
                    if ui.button("Restart discovery now").clicked() {
                        self.engine.send(EngineCmd::RestartTransports);
                        self.restart_needed = false;
                    }
                }
                ui.label(RichText::new("Quality, decoder and playback apply to the next connection.").small().weak());
            });
        self.show_settings = open;
        let s = &self.settings;
        if s.name != before.name
            || s.wifi_direct != before.wifi_direct
            || s.infrastructure != before.infrastructure
            || s.uibc != before.uibc
        {
            self.restart_needed = true;
        }
        if s.name != before.name
            || s.wifi_direct != before.wifi_direct
            || s.infrastructure != before.infrastructure
            || s.auto_accept != before.auto_accept
            || s.uibc != before.uibc
            || s.capture_on_click != before.capture_on_click
            || s.pointer != before.pointer
            || s.file_share != before.file_share
            || s.max_video != before.max_video
            || s.decoder != before.decoder
            || s.av_sync != before.av_sync
            || s.jitter_ms != before.jitter_ms
        {
            self.settings.save();
            self.engine.send(EngineCmd::UpdateSettings(self.settings.clone()));
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

fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(12.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), 4.5, color);
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
        if self.fullscreen && !self.forwarding && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.toggle_fullscreen(&ctx);
        }

        let show_video = playing && self.renderer.has_frame();
        if !self.fullscreen {
            egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
            if session.phase != Phase::Idle {
                egui::Panel::bottom("bottom").show(ui, |ui| self.bottom_bar(ui));
            }
        }
        let frame = if show_video { egui::Frame::NONE.fill(Color32::BLACK) } else { egui::Frame::central_panel(ui.style()) };
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if show_video {
                self.video_view(ui);
            } else {
                self.video_rect = None;
                self.idle_screen(ui);
            }
        });
        self.take_dropped_files(&ctx);
        if self.show_files {
            self.files_window(&ctx);
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let text = match &self.snapshot.share_url {
                Some(_) => format!("Drop to share with {}", self.snapshot.session.peer_name),
                None => "Connect a device to share files".into(),
            };
            let area = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::NULL.with("drop")));
            painter.rect_filled(area, 0.0, Color32::from_black_alpha(180));
            painter.text(area.center(), Align2::CENTER_CENTER, text, FontId::proportional(28.0), Color32::WHITE);
        }
        self.pairing_modal(&ctx);
        if self.show_settings {
            self.settings_window(&ctx);
        }
        // keys go to the device only while the picture is up and nothing in
        // our own UI wants them
        self.forwarding = show_video
            && self.snapshot.input.is_some()
            && self.snapshot.prompts.is_empty()
            && !self.show_settings
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
