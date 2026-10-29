//! The window's contents.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Stroke, Ui, vec2};

use super::Command;
use super::files::{self, FilesView};
use super::layout::{self, DragState, Machine, Placed};
use crate::autostart;
use crate::clipnet::human;
use crate::config::{Config, Identity, Mode, Paths, Peer, Peers, Role};
use crate::engine::Engine;
use crate::geom::{Screen, Side};
use crate::platform;
use crate::proto::Os;
use crate::status::{Hub, LinkState, PairPrompt, Status};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tab {
    Overview,
    Files,
    Settings,
    Log,
}

/// What the window needs from the process around it.
pub struct AppCtx {
    pub engine: Arc<Mutex<Engine>>,
    pub hub: Arc<Hub>,
    pub paths: Paths,
    pub commands: Option<Arc<Mutex<Receiver<Command>>>>,
    /// Closing the window leaves wifishare running in the tray.
    pub has_tray: bool,
    /// Set when the user asked to quit (as opposed to just closing the window).
    pub quit: Arc<AtomicBool>,
    pub start_minimized: bool,
}

pub struct App {
    ctx: AppCtx,
    draft: Config,
    saved: Config,
    tab: Tab,
    peers: Vec<Peer>,
    peers_read: Instant,
    my_screen: Screen,
    key: String,
    pair_target: String,
    pairing: Option<String>,
    autostart: bool,
    drag: DragState,
    files_view: FilesView,
    settings_error: Option<String>,
    icon: Option<egui::TextureHandle>,
    first_frame: bool,
    opened: Instant,
}

const GREEN: Color32 = Color32::from_rgb(46, 160, 90);
const AMBER: Color32 = Color32::from_rgb(214, 140, 30);
const RED: Color32 = Color32::from_rgb(210, 70, 60);
const GREY: Color32 = Color32::from_rgb(130, 130, 130);

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, ctx: AppCtx) -> App {
        let cfg = Config::load(&ctx.paths).unwrap_or_default();
        let key = Identity::load_or_create(&ctx.paths).map(|id| id.fingerprint()).unwrap_or_default();
        let my_screen = platform::screen().unwrap_or_else(|_| platform::fallback_screen());
        cc.egui_ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = vec2(8.0, 6.0);
            style.spacing.button_padding = vec2(10.0, 4.0);
        });
        let mut app = App {
            ctx,
            draft: cfg.clone(),
            saved: cfg,
            tab: Tab::Overview,
            peers: Vec::new(),
            peers_read: Instant::now() - Duration::from_secs(60),
            my_screen,
            key,
            pair_target: String::new(),
            pairing: None,
            autostart: autostart::is_enabled(),
            drag: DragState::default(),
            files_view: FilesView::new(),
            settings_error: None,
            icon: None,
            first_frame: true,
            opened: Instant::now(),
        };
        app.refresh_peers(true);
        app
    }

    #[cfg(test)]
    pub(super) fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    #[cfg(test)]
    pub(super) fn files_view(&mut self) -> &mut FilesView {
        &mut self.files_view
    }

    fn refresh_peers(&mut self, force: bool) {
        if force || self.peers_read.elapsed() > Duration::from_secs(2) {
            self.peers = Peers::load(&self.ctx.paths).map(|p| p.peers).unwrap_or_default();
            self.peers_read = Instant::now();
        }
    }

    fn engine(&self) -> std::sync::MutexGuard<'_, Engine> {
        self.ctx.engine.lock().unwrap()
    }

    fn mode(&self) -> Option<Mode> {
        self.saved.mode
    }

    /// Saves `cfg` as the config file and makes it the new baseline.
    fn persist(&mut self, cfg: Config) -> bool {
        match cfg.save(&self.ctx.paths) {
            Ok(()) => {
                self.saved = cfg;
                self.settings_error = None;
                true
            }
            Err(e) => {
                self.settings_error = Some(format!("Could not save settings: {e:#}"));
                false
            }
        }
    }

    fn set_mode(&mut self, mode: Mode) {
        if self.saved.mode == Some(mode) {
            return;
        }
        let mut cfg = self.saved.clone();
        cfg.mode = Some(mode);
        self.draft.mode = Some(mode);
        if self.persist(cfg) {
            self.engine().start(mode);
        }
    }

    fn handle_commands(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.ctx.commands else { return };
        let Ok(rx) = rx.try_lock() else { return };
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Command::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                Command::Quit => {
                    self.ctx.quit.store(true, Ordering::SeqCst);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn header(&mut self, ui: &mut Ui, st: &Status) {
        let icon = self
            .icon
            .get_or_insert_with(|| {
                let img = egui::ColorImage::from_rgba_unmultiplied([64, 64], &crate::icon::rgba(64));
                ui.ctx().load_texture("app-icon", img, egui::TextureOptions::LINEAR)
            })
            .clone();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.image((icon.id(), vec2(30.0, 30.0)));
            ui.vertical(|ui| {
                ui.label(RichText::new("wifishare").strong().size(18.0));
                ui.label(RichText::new(format!("{}  ·  key {}", self.saved.name, short_key(&self.key))).size(12.0).weak());
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if st.running {
                    if ui.button("Stop").on_hover_text("Stop sharing until you start it again").clicked() {
                        self.engine().stop();
                    }
                } else if let Some(mode) = self.mode() {
                    if ui.add(egui::Button::new(RichText::new("Start").strong())).clicked() {
                        self.engine().start(mode);
                    }
                }
                let (text, color) = status_pill(st, self.mode());
                pill(ui, &text, color);
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 8.0) / 2.0;
            for (mode, title, sub) in [
                (Mode::Server, "Share this computer", "Its keyboard, mouse and clipboard control the others"),
                (Mode::Client, "Be controlled", "Use the keyboard and mouse of another computer"),
            ] {
                let selected = self.mode() == Some(mode);
                let sub_color = if selected { ui.visuals().selection.stroke.color } else { ui.visuals().text_color() };
                let mut job = egui::text::LayoutJob::default();
                RichText::new(format!("{title}\n")).strong().size(15.0).append_to(
                    &mut job,
                    ui.style(),
                    egui::FontSelection::Default,
                    Align::LEFT,
                );
                RichText::new(sub).size(12.0).color(sub_color.gamma_multiply(0.85)).append_to(
                    &mut job,
                    ui.style(),
                    egui::FontSelection::Default,
                    Align::LEFT,
                );
                let button = egui::Button::new(job).selected(selected).min_size(vec2(width, 52.0)).corner_radius(CornerRadius::same(8));
                if ui.add(button).clicked() {
                    self.set_mode(mode);
                }
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (tab, name) in [(Tab::Overview, "Overview"), (Tab::Files, "Files"), (Tab::Settings, "Settings"), (Tab::Log, "Log")] {
                let selected = self.tab == tab;
                let text = if selected { RichText::new(name).strong() } else { RichText::new(name) };
                let resp = ui.add(egui::Button::new(text).frame(false));
                if selected {
                    let r = resp.rect;
                    ui.painter().line_segment(
                        [r.left_bottom() + vec2(0.0, 3.0), r.right_bottom() + vec2(0.0, 3.0)],
                        Stroke::new(2.0, ui.visuals().selection.bg_fill),
                    );
                }
                if resp.clicked() {
                    self.tab = tab;
                }
                ui.add_space(8.0);
            }
        });
        ui.add_space(2.0);
    }

    fn overview(&mut self, ui: &mut Ui, st: &Status) {
        if let Some(e) = &st.error {
            banner(ui, RED, "Sharing stopped", e);
        }
        for w in &st.warnings {
            let action = warning_action(w);
            banner(ui, AMBER, "Needs attention", w);
            if let Some((label, url)) = action
                && ui.button(label).clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
        match self.mode() {
            None => {
                card(ui, "Get started", |ui| {
                    ui.label("Pick what this computer does above:");
                    ui.label("•  Share this computer: on the machine whose keyboard and mouse you use (your Mac).");
                    ui.label("•  Be controlled: on the other machines.");
                    ui.label(
                        "Then pair them once: on the controlled computer, choose the server from the list and compare the 6-digit codes.",
                    );
                });
            }
            Some(Mode::Server) => {
                self.server_overview(ui, st);
                self.clipboard_card(ui, st);
                self.sound_card(ui, st);
            }
            Some(Mode::Client) => {
                self.client_overview(ui, st);
                self.clipboard_card(ui, st);
                self.sound_card(ui, st);
            }
        }
    }

    fn server_overview(&mut self, ui: &mut Ui, st: &Status) {
        let clients: Vec<Peer> = self.peers.iter().filter(|p| p.role == Role::Client).cloned().collect();
        let screen = st.local_screen.clone().unwrap_or_else(|| self.my_screen.clone());
        card(ui, "Screens", |ui| {
            ui.label(RichText::new("Drag a computer to the edge of this screen where it physically sits.").weak());
            let server = Machine { name: self.saved.name.clone(), screen, connected: true, active: false };
            let mut placed: Vec<Placed> = clients
                .iter()
                .map(|p| {
                    let live = st.clients.iter().find(|c| c.name.eq_ignore_ascii_case(&p.name));
                    Placed {
                        machine: Machine {
                            name: p.name.clone(),
                            screen: live.map(|c| c.screen.clone()).unwrap_or_else(|| Screen::single(1920, 1080)),
                            connected: live.is_some(),
                            active: st.active.as_deref() == Some(p.name.as_str()),
                        },
                        side: live.map(|c| c.side).unwrap_or_else(|| self.side_for(&p.name)),
                    }
                })
                .collect();
            // Connected machines that are not (or no longer) in the peers file still get drawn.
            for c in &st.clients {
                if !placed.iter().any(|p| p.machine.name.eq_ignore_ascii_case(&c.name)) {
                    placed.push(Placed {
                        machine: Machine {
                            name: c.name.clone(),
                            screen: c.screen.clone(),
                            connected: true,
                            active: st.active.as_deref() == Some(c.name.as_str()),
                        },
                        side: c.side,
                    });
                }
            }
            if let Some((name, side)) = layout::canvas(ui, &server, &placed, &mut self.drag, 230.0) {
                self.move_client(&name, side);
            }
        });
        card(ui, "Computers", |ui| {
            if clients.is_empty() {
                ui.label("No computers paired yet.");
                ui.label(
                    RichText::new(format!(
                        "On the other computer, open wifishare, choose \"Be controlled\" and pick \"{}\" from the servers it finds.",
                        self.saved.name
                    ))
                    .weak(),
                );
                if !st.running {
                    ui.label(RichText::new("This computer has to be running (press Start) to be found.").weak());
                }
            }
            let mut forget = None;
            let mut set_side = None;
            for p in &clients {
                let live = st.clients.iter().find(|c| c.name.eq_ignore_ascii_case(&p.name));
                ui.horizontal(|ui| {
                    dot(ui, if live.is_some() { GREEN } else { GREY });
                    ui.label(RichText::new(&p.name).strong());
                    let detail = match live {
                        Some(c) => {
                            let mut parts = vec![os_name(c.os).to_string(), c.addr.to_string(), format!("for {}", span(c.since.elapsed()))];
                            if let Some(ms) = c.rtt.get() {
                                parts.push(format!("{ms} ms"));
                            }
                            if st.active.as_deref() == Some(c.name.as_str()) {
                                parts.push("has the cursor".into());
                            }
                            parts.join("  ·  ")
                        }
                        None => "not connected".into(),
                    };
                    ui.label(RichText::new(detail).weak());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Forget").on_hover_text("Unpair: it will have to pair again to connect").clicked() {
                            forget = Some(p.name.clone());
                        }
                        let current = live.map(|c| c.side).unwrap_or_else(|| self.side_for(&p.name));
                        let mut side = current;
                        egui::ComboBox::from_id_salt(("side", &p.name)).selected_text(side_label(side)).width(90.0).show_ui(ui, |ui| {
                            for s in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
                                ui.selectable_value(&mut side, s, side_label(s));
                            }
                        });
                        if side != current {
                            set_side = Some((p.name.clone(), side));
                        }
                    });
                });
            }
            if let Some((name, side)) = set_side {
                self.move_client(&name, side);
            }
            if let Some(name) = forget {
                self.forget(&name);
            }
        });
    }

    fn side_for(&self, name: &str) -> Side {
        self.saved.layout_for(name).map(|l| l.side).unwrap_or(self.saved.server.default_side)
    }

    fn move_client(&mut self, name: &str, side: Side) {
        let mut cfg = self.saved.clone();
        cfg.set_side(name, side);
        self.draft.server.clients = cfg.server.clients.clone();
        if self.persist(cfg) {
            self.engine().reload();
            log::info!("{name} is now on the {side} edge");
        }
    }

    fn forget(&mut self, name: &str) {
        let Ok(mut peers) = Peers::load(&self.ctx.paths) else { return };
        let was_server = peers.peers.iter().any(|p| p.name == name && p.role == Role::Server);
        peers.peers.retain(|p| p.name != name);
        if peers.save(&self.ctx.paths).is_ok() {
            log::info!("forgot {name}");
            self.refresh_peers(true);
            if was_server && let Some(mode) = self.mode() {
                self.engine().start(mode);
            }
        }
    }

    fn client_overview(&mut self, ui: &mut Ui, st: &Status) {
        let server = self.peers.iter().rev().find(|p| p.role == Role::Server).cloned();
        card(ui, "Connection", |ui| {
            let (headline, detail, color) = match (&st.link, st.running) {
                (_, false) => ("Not running".to_string(), "Press Start to connect.".to_string(), GREY),
                (LinkState::Connected { server, addr, since, rtt }, _) => {
                    let ms = rtt.get().map(|m| format!("  ·  {m} ms")).unwrap_or_default();
                    (format!("Connected to {server}"), format!("{}  ·  for {}{ms}", addr.ip(), span(since.elapsed())), GREEN)
                }
                (LinkState::Searching { server, since, error }, _) => {
                    let mut d = format!("Trying for {}. It reconnects by itself as soon as {server} is reachable.", span(since.elapsed()));
                    if let Some(e) = error {
                        d.push_str(&format!("\nLast attempt: {e}"));
                    }
                    (format!("Looking for {server}…"), d, AMBER)
                }
                (LinkState::Unpaired, _) => ("Not paired yet".to_string(), "Pick your server below to pair with it.".to_string(), AMBER),
                (LinkState::Idle, _) => ("Starting…".to_string(), String::new(), GREY),
            };
            ui.horizontal(|ui| {
                dot(ui, color);
                ui.label(RichText::new(headline).size(16.0).strong());
            });
            if !detail.is_empty() {
                ui.label(RichText::new(detail).weak());
            }
        });

        card(ui, "Servers on this network", |ui| {
            if st.discovered.is_empty() {
                ui.label(RichText::new(if st.running { "Listening for servers…" } else { "Start to look for servers." }).weak());
            }
            let mut pair = None;
            for d in &st.discovered {
                ui.horizontal(|ui| {
                    let paired = self.peers.iter().any(|p| p.key == hex::encode(&d.key));
                    dot(ui, if paired { GREEN } else { GREY });
                    ui.label(RichText::new(&d.name).strong());
                    ui.label(RichText::new(d.addr.ip().to_string()).weak());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if paired {
                            ui.label(RichText::new("paired").color(GREEN));
                        } else if ui.add_enabled(self.pairing.is_none(), egui::Button::new("Pair")).clicked() {
                            pair = Some(d.addr.ip().to_string());
                        }
                    });
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Or by address:");
                ui.add(egui::TextEdit::singleline(&mut self.pair_target).hint_text("192.168.1.25 or host name").desired_width(200.0));
                let ready = !self.pair_target.trim().is_empty() && self.pairing.is_none();
                if ui.add_enabled(ready, egui::Button::new("Pair")).clicked() {
                    pair = Some(self.pair_target.trim().to_string());
                }
            });
            if let Some(target) = &self.pairing {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label(format!("Pairing with {target}… approve it on both computers."));
                });
            }
            if let Some(target) = pair {
                self.pairing = Some(target.clone());
                self.engine().pair(target);
            }
        });

        if let Some(s) = server {
            card(ui, "Paired server", |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&s.name).strong());
                    let key = hex::decode(&s.key).map(|k| crate::config::fingerprint(&k)).unwrap_or_default();
                    ui.label(RichText::new(format!("key {}", short_key(&key))).weak());
                    if let Some(a) = &s.last_addr {
                        ui.label(RichText::new(format!("last at {a}")).weak());
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Forget").clicked() {
                            self.forget(&s.name);
                        }
                    });
                });
            });
        }
    }

    fn clipboard_card(&mut self, ui: &mut Ui, st: &Status) {
        card(ui, "Clipboard", |ui| {
            if !self.saved.clipboard.enabled {
                ui.label(RichText::new("Clipboard sharing is off (Settings).").weak());
                return;
            }
            ui.label(RichText::new("Copy on one computer, paste on the other: text, images and files.").weak());
            match &st.clipboard {
                Some(ev) => {
                    ui.label(format!("{}  ·  {}", ev.text, ago(ev.at.elapsed())));
                }
                None => {
                    ui.label(RichText::new("Nothing copied yet.").weak());
                }
            }
            for t in &st.transfers {
                let done = t.done.load(Ordering::Relaxed);
                let frac = if t.total == 0 { 1.0 } else { done as f32 / t.total as f32 };
                ui.label(&t.label);
                ui.add(egui::ProgressBar::new(frac).text(format!("{} of {}", human(done), human(t.total))));
            }
        });
    }

    fn sound_card(&mut self, ui: &mut Ui, st: &Status) {
        let peer = match self.mode() {
            Some(Mode::Client) => self.peers.iter().rev().find(|p| p.role == Role::Server).map(|p| p.name.clone()),
            _ => None,
        }
        .unwrap_or_else(|| "the other computer".into());
        let mut cfg = self.saved.clone();
        let mut changed = false;
        card(ui, "Sound", |ui| {
            if crate::audio::can_send() {
                changed |= ui.checkbox(&mut cfg.audio.send, format!("Play this computer's sound on {peer}")).changed();
                if cfg.audio.send {
                    ui.indent("play_here", |ui| {
                        changed |= ui.checkbox(&mut cfg.audio.play_here, "Keep playing it here too").changed();
                    });
                }
            }
            changed |= ui.checkbox(&mut cfg.audio.receive, "Play sound that other computers send here").changed();
            for a in &st.audio {
                ui.horizontal(|ui| {
                    dot(ui, GREEN);
                    let what = if a.sending {
                        format!("Sending this computer's sound to {}", a.peer)
                    } else {
                        format!("Playing sound from {}", a.peer)
                    };
                    ui.label(what);
                    let mut detail = format!("for {}", span(a.since.elapsed()));
                    let (packets, lost) = (a.packets.load(Ordering::Relaxed), a.lost.load(Ordering::Relaxed));
                    if !a.sending && lost > 0 {
                        detail.push_str(&format!("  ·  {:.1}% lost", 100.0 * lost as f64 / (packets + lost).max(1) as f64));
                    }
                    ui.label(RichText::new(detail).weak());
                });
            }
            if cfg.audio.send && !st.audio.iter().any(|a| a.sending) {
                ui.label(
                    RichText::new(if st.running {
                        format!("Starts when {peer} is connected.")
                    } else {
                        "Starts when sharing is running.".into()
                    })
                    .weak(),
                );
            }
        });
        if changed {
            self.draft.audio = cfg.audio.clone();
            if self.persist(cfg) {
                self.engine().reload();
            }
        }
    }

    fn files_tab(&mut self, ui: &mut Ui, st: &Status) {
        let peers: Vec<String> = match (&st.link, self.mode()) {
            (LinkState::Connected { server, .. }, Some(Mode::Client)) => vec![server.clone()],
            (_, Some(Mode::Server)) => st.clients.iter().map(|c| c.name.clone()).collect(),
            _ => Vec::new(),
        };
        let cx = files::Ctx { files: self.ctx.hub.files(), cfg: &self.saved, peers: &peers };
        if let Some(cfg) = self.files_view.show(ui, &cx) {
            self.draft.files = cfg.files.clone();
            self.persist(cfg);
        }
    }

    fn settings(&mut self, ui: &mut Ui) {
        let d = &mut self.draft;
        section(ui, "This computer", |ui| {
            egui::Grid::new("me").num_columns(2).spacing(vec2(16.0, 8.0)).show(ui, |ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(220.0));
                ui.end_row();
                ui.label("Port");
                ui.add(egui::DragValue::new(&mut d.port).range(1024..=65534));
                ui.end_row();
            });
        });
        section(ui, "When sharing this computer", |ui| {
            egui::Grid::new("server").num_columns(2).spacing(vec2(16.0, 8.0)).show(ui, |ui| {
                ui.label("New computers go to the");
                egui::ComboBox::from_id_salt("default_side").selected_text(side_label(d.server.default_side)).show_ui(ui, |ui| {
                    for s in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
                        ui.selectable_value(&mut d.server.default_side, s, side_label(s));
                    }
                });
                ui.end_row();
                ui.label("Cmd and Ctrl");
                ui.checkbox(&mut d.server.swap_cmd_ctrl, "Swap them between a Mac and a PC (Cmd+C copies on Linux)");
                ui.end_row();
                for c in &mut d.server.clients {
                    ui.label(format!("Pointer speed on {}", c.name));
                    ui.add(egui::Slider::new(&mut c.speed, 0.25..=4.0).logarithmic(true));
                    ui.end_row();
                }
            });
        });
        section(ui, "When being controlled", |ui| {
            egui::Grid::new("client").num_columns(2).spacing(vec2(16.0, 8.0)).show(ui, |ui| {
                ui.label("Server address");
                let mut addr = d.client.server.clone().unwrap_or_default();
                if ui.add(egui::TextEdit::singleline(&mut addr).hint_text("optional, found automatically").desired_width(220.0)).changed() {
                    d.client.server = Some(addr.trim().to_string()).filter(|a| !a.is_empty());
                }
                ui.end_row();
                ui.label("Scroll speed");
                let mut speed = d.client.scroll_speed.unwrap_or(1.0);
                if ui.add(egui::Slider::new(&mut speed, 0.25..=4.0).logarithmic(true)).changed() {
                    d.client.scroll_speed = Some(speed);
                }
                ui.end_row();
            });
        });
        section(ui, "Clipboard", |ui| {
            ui.checkbox(&mut d.clipboard.enabled, "Share the clipboard");
            egui::Grid::new("clip").num_columns(2).spacing(vec2(16.0, 8.0)).show(ui, |ui| {
                ui.label("Send copied files up to");
                ui.add(egui::DragValue::new(&mut d.clipboard.max_files_mb).range(1..=1_000_000).suffix(" MB"));
                ui.end_row();
                ui.label("Text up to");
                ui.add(egui::DragValue::new(&mut d.clipboard.max_text_mb).range(1..=1024).suffix(" MB"));
                ui.end_row();
                ui.label("Images up to");
                ui.add(egui::DragValue::new(&mut d.clipboard.max_image_mb).range(1..=4096).suffix(" MB"));
                ui.end_row();
            });
        });
        section(ui, "Sound", |ui| {
            egui::Grid::new("sound").num_columns(2).spacing(vec2(16.0, 8.0)).show(ui, |ui| {
                ui.label("Buffer when playing sound from another computer");
                ui.add(egui::DragValue::new(&mut d.audio.buffer_ms).range(10..=500).suffix(" ms"))
                    .on_hover_text("More rides out bad Wi-Fi; less keeps the sound closer to the picture");
                ui.end_row();
            });
        });
        section(ui, "Connection", |ui| {
            egui::Grid::new("net").num_columns(2).spacing(vec2(16.0, 8.0)).show(ui, |ui| {
                ui.label("Heartbeat every");
                ui.add(egui::DragValue::new(&mut d.network.heartbeat_ms).range(200..=10_000).suffix(" ms"));
                ui.end_row();
                ui.label("Reconnect after silence of");
                ui.add(egui::DragValue::new(&mut d.network.timeout_ms).range(1000..=60_000).suffix(" ms"));
                ui.end_row();
            });
        });
        section(ui, "Startup", |ui| {
            if ui.checkbox(&mut self.autostart, "Start wifishare when I log in").changed() {
                if let Err(e) = autostart::set_enabled(self.autostart) {
                    self.settings_error = Some(format!("Could not change the login item: {e:#}"));
                    self.autostart = autostart::is_enabled();
                }
            }
            let d = &mut self.draft;
            ui.checkbox(&mut d.gui.start_sharing, "Start sharing as soon as wifishare opens");
            if self.ctx.has_tray {
                ui.checkbox(&mut d.gui.keep_running, "Keep running in the tray when the window is closed");
            }
            if cfg!(target_os = "linux")
                && !autostart::launcher_installed()
                && ui.button("Add wifishare to the applications menu").clicked()
            {
                if let Err(e) = autostart::install_launcher() {
                    self.settings_error = Some(format!("Could not add the launcher: {e:#}"));
                }
            }
        });

        ui.add_space(8.0);
        if let Some(e) = &self.settings_error {
            ui.colored_label(RED, e);
        }
        let dirty = self.draft_differs();
        ui.horizontal(|ui| {
            if ui.add_enabled(dirty, egui::Button::new(RichText::new("Save").strong())).clicked() {
                let cfg = self.draft.clone();
                if self.persist(cfg) {
                    if self.ctx.hub.snapshot().running
                        && let Some(mode) = self.mode()
                    {
                        self.engine().start(mode);
                    }
                    self.ctx.hub.update(|s| s.notice = Some(("Settings saved".into(), Instant::now())));
                }
            }
            if ui.add_enabled(dirty, egui::Button::new("Discard changes")).clicked() {
                self.draft = self.saved.clone();
            }
            if dirty {
                ui.label(RichText::new("Saving restarts sharing (a second or two).").weak());
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new(format!("Settings file: {}", self.ctx.paths.config_file().display())).small().weak());
        ui.label(RichText::new(format!("This computer's key: {}", self.key)).small().weak());
    }

    fn draft_differs(&self) -> bool {
        toml::to_string(&self.draft).ok() != toml::to_string(&self.saved).ok()
    }

    fn log_view(&mut self, ui: &mut Ui) {
        let lines = crate::logger::recent();
        ui.horizontal(|ui| {
            if ui.button("Copy all").clicked() {
                ui.ctx().copy_text(lines.join("\n"));
            }
            ui.label(RichText::new(format!("{} lines", lines.len())).weak());
        });
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for line in &lines {
                let color = if line.contains(" WARN ") || line.contains(" ERROR ") { AMBER } else { ui.visuals().text_color() };
                ui.label(RichText::new(line).monospace().size(12.0).color(color));
            }
        });
    }

    fn pair_dialog(&mut self, ctx: &egui::Context) {
        let Some(p) = self.ctx.hub.pair_prompts().into_iter().next() else { return };
        egui::Modal::new(egui::Id::new(("pair", p.id))).show(ctx, |ui| {
            ui.set_width(380.0);
            pair_contents(ui, &p, &self.ctx.hub);
        });
    }

    fn notice(&mut self, ctx: &egui::Context, st: &Status) {
        let Some((text, at)) = &st.notice else { return };
        if self.pairing.is_some() && (text.starts_with("Paired") || text.starts_with("Pairing failed")) {
            self.pairing = None;
            self.refresh_peers(true);
        }
        if at.elapsed() > Duration::from_secs(6) {
            return;
        }
        egui::Area::new(egui::Id::new("notice")).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -18.0)).show(ctx, |ui| {
            Frame::popup(ui.style()).inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
                ui.label(RichText::new(text).strong());
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if std::mem::take(&mut self.first_frame) && self.ctx.start_minimized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.handle_commands(&ctx);
        // For checking the close-to-tray cycle without clicking: close after a few seconds.
        if std::env::var_os("WIFISHARE_GUI_SELFTEST").is_some() && self.opened.elapsed() > Duration::from_secs(4) {
            log::info!("selftest: closing the window");
            self.opened = Instant::now() + Duration::from_secs(3600);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.refresh_peers(false);
        let st = self.ctx.hub.snapshot();

        egui::Panel::top("header")
            .frame(Frame::side_top_panel(ui.style()).inner_margin(Margin::symmetric(16, 6)))
            .show(ui, |ui| self.header(ui, &st));
        egui::CentralPanel::default().frame(Frame::central_panel(ui.style()).inner_margin(Margin::symmetric(16, 10))).show(ui, |ui| {
            match self.tab {
                Tab::Overview => {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.overview(ui, &st));
                }
                Tab::Files => self.files_tab(ui, &st),
                Tab::Settings => {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.settings(ui));
                }
                Tab::Log => self.log_view(ui),
            }
        });
        self.pair_dialog(&ctx);
        self.notice(&ctx, &st);
        let copying = self.ctx.hub.files().is_some_and(|f| f.jobs().iter().any(|j| !j.finished()));
        ctx.request_repaint_after(if st.transfers.is_empty() && !copying { Duration::from_secs(1) } else { Duration::from_millis(100) });
    }
}

fn pair_contents(ui: &mut Ui, p: &PairPrompt, hub: &Hub) {
    let title = if p.incoming { format!("Pair with {}?", p.peer) } else { "Check the code".to_string() };
    ui.heading(title);
    ui.add_space(4.0);
    if p.incoming {
        ui.label(format!("{} ({}, {}) wants to use this computer's keyboard, mouse and clipboard.", p.peer, os_name(p.os), p.addr));
    } else {
        ui.label(format!("Pairing with {} at {}.", p.peer, p.addr));
    }
    ui.add_space(10.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(&p.code).size(34.0).strong().monospace());
    });
    ui.add_space(10.0);
    ui.label(
        RichText::new(if p.incoming {
            "Allow it only if the other computer shows exactly the same code."
        } else {
            "The server shows a code too. If it is the same, confirm here and approve it there."
        })
        .weak(),
    );
    ui.add_space(10.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let yes = if p.incoming { "Allow" } else { "The codes match" };
        if ui.add(egui::Button::new(RichText::new(yes).strong())).clicked() {
            hub.answer_pair(p.id, true);
        }
        if ui.button(if p.incoming { "Reject" } else { "Cancel" }).clicked() {
            hub.answer_pair(p.id, false);
        }
    });
}

fn status_pill(st: &Status, mode: Option<Mode>) -> (String, Color32) {
    if !st.running {
        return if st.error.is_some() { ("Stopped (error)".into(), RED) } else { ("Stopped".into(), GREY) };
    }
    match mode {
        Some(Mode::Server) if !st.capturing => ("Clipboard only".into(), AMBER),
        Some(Mode::Server) => match st.clients.len() {
            0 => ("Waiting for computers".into(), AMBER),
            1 => (format!("Sharing with {}", st.clients[0].name), GREEN),
            n => (format!("Sharing with {n} computers"), GREEN),
        },
        Some(Mode::Client) => match &st.link {
            LinkState::Connected { server, .. } => (format!("Connected to {server}"), GREEN),
            LinkState::Searching { server, .. } => (format!("Looking for {server}"), AMBER),
            LinkState::Unpaired => ("Not paired".into(), AMBER),
            LinkState::Idle => ("Starting".into(), GREY),
        },
        None => ("Stopped".into(), GREY),
    }
}

fn pill(ui: &mut Ui, text: &str, color: Color32) {
    Frame::new()
        .fill(color.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0, color))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                dot(ui, color);
                ui.label(RichText::new(text).color(ui.visuals().strong_text_color()));
            });
        });
}

fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

fn card(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    Frame::group(ui.style()).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong().size(15.0));
        ui.add_space(4.0);
        body(ui);
    });
    ui.add_space(8.0);
}

fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).strong().size(15.0));
    ui.add_space(2.0);
    ui.indent(title, body);
    ui.add_space(6.0);
    ui.separator();
}

fn banner(ui: &mut Ui, color: Color32, title: &str, text: &str) {
    Frame::new()
        .fill(color.gamma_multiply(0.15))
        .stroke(Stroke::new(1.0, color))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().color(color));
            ui.label(text);
        });
    ui.add_space(8.0);
}

/// A button that helps fix a warning, where the platform has a settings page for it.
fn warning_action(w: &str) -> Option<(&'static str, &'static str)> {
    if cfg!(target_os = "macos") && w.contains("Accessibility") {
        return Some(("Open Accessibility settings", "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"));
    }
    None
}

fn side_label(s: Side) -> &'static str {
    match s {
        Side::Left => "Left",
        Side::Right => "Right",
        Side::Top => "Above",
        Side::Bottom => "Below",
    }
}

fn os_name(os: Os) -> &'static str {
    match os {
        Os::Linux => "Linux",
        Os::MacOs => "macOS",
        Os::Windows => "Windows",
        Os::Other => "other",
    }
}

fn short_key(fp: &str) -> String {
    fp.split(':').take(4).collect::<Vec<_>>().join(":")
}

fn span(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min", s / 60),
        _ => format!("{} h {} min", s / 3600, (s % 3600) / 60),
    }
}

fn ago(d: Duration) -> String {
    if d.as_secs() < 3 { "just now".into() } else { format!("{} ago", span(d)) }
}
