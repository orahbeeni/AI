//! The window: tabs of split terminals, keyboard routing, actions and session handling.

mod dialogs;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use eframe::egui::{self, Align, Color32, CursorIcon, Key, Layout, Modifiers, Rect, RichText, Sense, Stroke, UiBuilder, pos2, vec2};

use crate::Args;
use crate::config::{Combo, Rgb, Settings, TabTitle, Theme};
use crate::input;
use crate::layout::{self, Dir, Nav, Node};
use crate::pane::{Pane, SavedCommand, TermSize, all_text};
use crate::session::{self, LayoutFile, PaneSpec, TabSpec};
use crate::view::{self, Metrics};

pub struct Tab {
    /// Stable identity for the tab's widgets, so dragging survives reordering.
    pub uid: u64,
    pub root: Node<u64>,
    pub focused: u64,
    pub zoomed: Option<u64>,
    pub title: Option<String>,
    /// The saved layout this tab was opened from or saved into.
    pub layout: Option<String>,
    pub activity: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Broadcast {
    Off,
    Tab,
    All,
}

/// Something the UI asked for, carried out after the frame's widgets are drawn.
pub enum Act {
    Action(&'static str),
    Focus(u64),
    Run(u64, String),
    /// Type a command into the terminal without pressing Enter.
    Insert(u64, String),
    EditCommands(u64),
    RenamePane(u64),
    CopyAll(u64),
    SaveOutput(u64),
    Paste(u64, bool),
    ScrollBottom(u64),
    ToggleLock(u64),
    SelectTab(usize),
    MoveTab(usize, usize),
    CloseTab(usize),
    RenameTab(usize),
    OpenLayout(String, bool, bool),
    SetBroadcast(Broadcast),
}

pub struct App {
    ctx: egui::Context,
    s: Settings,
    combos: Vec<(String, Combo)>,
    m: Option<Metrics>,
    panes: HashMap<u64, Pane>,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    clipboard: Option<arboard::Clipboard>,
    broadcast: Broadcast,
    /// Rectangles of the active tab's terminals, from the last frame.
    rects: Vec<(u64, Rect)>,
    acts: Vec<Act>,
    keys: Vec<egui::Event>,
    dlg: dialogs::Dialogs,
    window_title: String,
    window_size: Option<[f32; 2]>,
    fullscreen: bool,
    /// Name of the layout last opened or saved, offered when saving again.
    layout_name: String,
    error: Option<String>,
    /// A short confirmation shown after an action (e.g. an export) finished.
    notice: Option<String>,
    close_window: bool,
    blink_start: Instant,
    #[cfg(debug_assertions)]
    selftest: Option<SelfTest>,
    /// Self-tests: draw the focused terminal's saved-commands menu as if opened.
    #[cfg(any(test, debug_assertions))]
    preview_menu: bool,
}

/// Debug builds only: `TERM_SELFTEST=out.ppm` runs `TERM_SELFTEST_ACTIONS` (comma separated, an
/// action id or `run:<command>`), screenshots the window to a PPM file and quits.
#[cfg(debug_assertions)]
struct SelfTest {
    out: String,
    steps: Vec<String>,
    frame: u64,
    started: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Args) -> App {
        let s = Settings::load();
        crate::fonts::install(&cc.egui_ctx, &s);
        let mut app = App {
            ctx: cc.egui_ctx.clone(),
            combos: s.combos(),
            s,
            m: None,
            panes: HashMap::new(),
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            clipboard: arboard::Clipboard::new().ok(),
            broadcast: Broadcast::Off,
            rects: Vec::new(),
            acts: Vec::new(),
            keys: Vec::new(),
            dlg: dialogs::Dialogs::default(),
            window_title: String::new(),
            window_size: None,
            fullscreen: false,
            layout_name: String::new(),
            error: None,
            notice: None,
            close_window: false,
            blink_start: Instant::now(),
            #[cfg(any(test, debug_assertions))]
            preview_menu: false,
            #[cfg(debug_assertions)]
            selftest: std::env::var("TERM_SELFTEST").ok().map(|out| SelfTest {
                out,
                steps: std::env::var("TERM_SELFTEST_ACTIONS")
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                frame: 0,
                started: Instant::now(),
            }),
        };
        app.apply_visuals();

        let layout_name = args.layout.clone().or_else(|| Some(app.s.startup_layout.clone()).filter(|n| !n.is_empty()));
        let mut opened = false;
        if let Some(name) = layout_name.filter(|_| args.dir.is_none()) {
            match session::load_layout(&name) {
                Some(l) => opened = app.open_layout(l, true, args.run || args.layout.is_none()),
                None => app.error = Some(format!("There is no saved layout called \"{name}\".")),
            }
        }
        if !opened
            && app.s.restore_session
            && !args.fresh
            && args.dir.is_none()
            && let Some(l) = session::load_session().filter(|l| !l.tabs.is_empty())
        {
            let name = l.name.clone();
            opened = app.open_layout(l, true, app.s.run_commands_on_restore);
            app.layout_name = name;
        }
        if !opened {
            app.new_tab(args.dir.clone());
        }
        let explicit = args.layout.is_some() || args.dir.is_some() || !app.s.startup_layout.is_empty();
        if app.s.startup_picker && !explicit && !session::list_layouts().is_empty() {
            app.dlg.open_layouts(true);
        }
        app
    }

    /// Derive the look of every window, menu and widget from the terminal colour scheme.
    fn apply_visuals(&self) {
        let t = &self.s.theme;
        let (bg, fg) = (t.background, t.foreground);
        let dark = bg.luminance() < 0.5;
        let lift = |k: f32| mix(bg, fg, k).color();
        let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };

        v.window_fill = lift(0.05);
        v.panel_fill = lift(0.03);
        v.faint_bg_color = lift(0.08);
        v.extreme_bg_color = if dark { mix(bg, Rgb(0, 0, 0), 0.3).color() } else { mix(bg, Rgb(255, 255, 255), 0.7).color() };
        v.text_edit_bg_color = Some(v.extreme_bg_color);
        v.code_bg_color = lift(0.09);
        v.hyperlink_color = t.accent.color();
        v.window_stroke = Stroke::new(1.0, lift(0.16));
        v.window_corner_radius = egui::CornerRadius::same(10);
        v.menu_corner_radius = egui::CornerRadius::same(8);
        let shadow_alpha = if dark { 140 } else { 45 };
        v.window_shadow = egui::Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(shadow_alpha) };
        v.popup_shadow = egui::Shadow { offset: [0, 6], blur: 16, spread: 0, color: Color32::from_black_alpha(shadow_alpha) };
        // Selected tabs/toggles: a solid accent-tinted fill with readable text.
        let sel = mix(bg, t.accent, 0.7);
        v.selection.bg_fill = sel.color();
        v.selection.stroke = Stroke::new(1.0, if sel.luminance() > 0.55 { Color32::BLACK } else { Color32::WHITE });
        v.text_cursor.stroke = Stroke::new(2.0, t.accent.color());

        let text = mix(bg, fg, 0.92).color();
        let r = egui::CornerRadius::same(4);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = v.window_fill;
        w.noninteractive.weak_bg_fill = v.window_fill;
        w.noninteractive.bg_stroke = Stroke::new(1.0, lift(0.13));
        w.noninteractive.fg_stroke = Stroke::new(1.0, mix(bg, fg, 0.85).color());
        w.inactive.weak_bg_fill = lift(0.11);
        w.inactive.bg_fill = lift(0.15);
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, text);
        w.hovered.weak_bg_fill = lift(0.18);
        w.hovered.bg_fill = lift(0.22);
        w.hovered.bg_stroke = Stroke::new(1.0, mix(bg, t.accent, 0.6).color());
        w.hovered.fg_stroke = Stroke::new(1.5, fg.color());
        w.active.weak_bg_fill = lift(0.24);
        w.active.bg_fill = lift(0.28);
        w.active.bg_stroke = Stroke::new(1.0, t.accent.color());
        w.active.fg_stroke = Stroke::new(1.5, fg.color());
        w.open.weak_bg_fill = lift(0.15);
        w.open.bg_fill = lift(0.15);
        w.open.bg_stroke = Stroke::new(1.0, lift(0.2));
        w.open.fg_stroke = Stroke::new(1.0, fg.color());
        for wv in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            wv.corner_radius = r;
        }

        self.ctx.all_styles_mut(|s| {
            s.spacing.button_padding = vec2(8.0, 4.0);
            s.spacing.item_spacing = vec2(8.0, 6.0);
            s.spacing.interact_size.y = 24.0;
            s.spacing.window_margin = egui::Margin::same(14);
            s.spacing.menu_margin = egui::Margin::same(6);
        });
        // Same look whatever the OS light/dark setting is.
        self.ctx.set_visuals_of(egui::Theme::Dark, v.clone());
        self.ctx.set_visuals_of(egui::Theme::Light, v);
    }

    /// Switch colours only, without saving (used for live preview in Preferences).
    pub fn set_theme(&mut self, theme: Theme) {
        self.s.theme = theme;
        for p in self.panes.values() {
            *p.shared.theme.lock().unwrap() = self.s.theme.clone();
        }
        self.apply_visuals();
        self.ctx.request_repaint();
    }

    pub fn apply_settings(&mut self, new: Settings) {
        let font_changed = new.font_path != self.s.font_path;
        self.s = new;
        if font_changed {
            crate::fonts::install(&self.ctx, &self.s);
        }
        self.combos = self.s.combos();
        for p in self.panes.values() {
            p.apply_settings(&self.s);
        }
        self.apply_visuals();
    }

    fn cell(&self) -> (f32, f32) {
        self.m.as_ref().map(|m| (m.cw, m.ch)).unwrap_or((8.0, 16.0))
    }

    fn meta(&self) -> bool {
        !cfg!(target_os = "macos") || self.s.option_as_meta
    }

    // ----- panes and tabs -------------------------------------------------------------------

    fn spawn(&mut self, dir: Option<PathBuf>, like: Option<u64>) -> Option<u64> {
        let id = self.take_id();
        let size = like.and_then(|l| self.panes.get(&l)).map(|p| p.size).unwrap_or(TermSize { cols: 80, rows: 24 });
        match Pane::spawn(id, &self.s, dir, size, self.cell(), &self.ctx) {
            Ok(p) => {
                self.panes.insert(id, p);
                Some(id)
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    /// Open "Save layout" for tab `tab`, named after the layout that tab belongs to (if any), so
    /// saving again updates it.
    fn open_save_layout(&mut self, tab: usize) {
        let name = self.tabs.get(tab).and_then(|t| t.layout.clone()).unwrap_or_default();
        self.dlg.open_save_layout(name);
    }

    fn export_settings(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export term settings and layouts")
            .add_filter("term settings", &["json"])
            .set_file_name("term-settings.json")
            .save_file()
        else {
            return;
        };
        match crate::transfer::export(&path, &self.s) {
            Ok(n) => {
                let layouts = if n == 1 { "1 layout".to_string() } else { format!("{n} layouts") };
                self.notice = Some(format!("Exported your settings and {layouts} to\n{}", path.display()));
            }
            Err(e) => self.error = Some(format!("Could not export: {e}")),
        }
    }

    fn pick_import(&mut self) {
        let Some(path) =
            rfd::FileDialog::new().set_title("Import term settings and layouts").add_filter("term settings", &["json"]).pick_file()
        else {
            return;
        };
        match crate::transfer::read(&path) {
            Ok(bundle) => self.dlg.open_import(path, bundle),
            Err(e) => self.error = Some(format!("Could not import {}: {e}", path.display())),
        }
    }

    fn take_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id - 1
    }

    fn focused(&self) -> Option<u64> {
        self.tabs.get(self.active).map(|t| t.focused)
    }

    fn focused_dir(&mut self) -> Option<PathBuf> {
        let id = self.focused()?;
        self.panes.get_mut(&id).map(|p| p.current_dir())
    }

    fn new_tab(&mut self, dir: Option<PathBuf>) {
        let dir = dir.or_else(|| self.focused_dir());
        if let Some(id) = self.spawn(dir, self.focused()) {
            let uid = self.take_id();
            self.tabs.push(Tab { uid, root: Node::Leaf(id), focused: id, zoomed: None, title: None, layout: None, activity: false });
            self.active = self.tabs.len() - 1;
        }
    }

    fn split(&mut self, dir: Dir) {
        let Some(target) = self.focused() else { return };
        let cwd = self.focused_dir();
        let Some(id) = self.spawn(cwd, Some(target)) else { return };
        let tab = &mut self.tabs[self.active];
        tab.zoomed = None;
        tab.root.split(&target, dir, id);
        tab.focused = id;
    }

    fn remove_pane(&mut self, id: u64) {
        if let Some(ti) = self.tabs.iter().position(|t| t.root.contains(&id)) {
            let tab = &mut self.tabs[ti];
            if matches!(tab.root, Node::Leaf(l) if l == id) {
                self.tabs.remove(ti);
                if self.active >= self.tabs.len() {
                    self.active = self.tabs.len().saturating_sub(1);
                } else if ti < self.active {
                    self.active -= 1;
                }
            } else {
                let before = tab.root.leaves();
                let idx = before.iter().position(|l| *l == id).unwrap_or(0);
                tab.root.remove(&id);
                let after = tab.root.leaves();
                if tab.focused == id {
                    tab.focused = after[idx.min(after.len() - 1)];
                }
                if tab.zoomed == Some(id) {
                    tab.zoomed = None;
                }
            }
        }
        if let Some(mut p) = self.panes.remove(&id) {
            p.kill();
        }
        if self.tabs.is_empty() {
            self.close_window = true;
        }
    }

    fn close_tab(&mut self, ti: usize) {
        if let Some(tab) = self.tabs.get(ti) {
            for id in tab.root.leaves() {
                self.remove_pane(id);
            }
        }
    }

    fn focus(&mut self, id: u64) {
        if let Some(ti) = self.tabs.iter().position(|t| t.root.contains(&id)) {
            self.active = ti;
            self.tabs[ti].focused = id;
        }
    }

    fn cycle_pane(&mut self, step: i32) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let leaves = tab.root.leaves();
        let i = leaves.iter().position(|l| *l == tab.focused).unwrap_or(0) as i32;
        let n = leaves.len() as i32;
        tab.focused = leaves[((i + step) % n + n) as usize % n as usize];
        if tab.zoomed.is_some() {
            tab.zoomed = Some(tab.focused);
        }
    }

    fn focus_dir(&mut self, nav: Nav) {
        let Some(from) = self.focused() else { return };
        if let Some(to) = layout::neighbour(&self.rects, &from, nav) {
            self.tabs[self.active].focused = to;
        }
    }

    // ----- input ----------------------------------------------------------------------------

    /// Panes that receive typed input: the focused one, or more when broadcasting.
    fn input_targets(&self) -> Vec<u64> {
        let Some(f) = self.focused() else { return Vec::new() };
        match self.broadcast {
            Broadcast::Off => vec![f],
            Broadcast::Tab => self.tabs[self.active].root.leaves(),
            Broadcast::All => self.tabs.iter().flat_map(|t| t.root.leaves()).collect(),
        }
    }

    fn send_input(&mut self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        for id in self.input_targets() {
            if let Some(p) = self.panes.get(&id) {
                if self.s.scroll_on_keystroke {
                    let offset = p.term.lock().grid().display_offset();
                    if offset != 0 || p.is_held() {
                        p.scroll_to_bottom();
                    }
                }
                p.write(bytes.clone());
            }
        }
        self.blink_start = Instant::now();
    }

    fn handle_keys(&mut self) {
        let events = std::mem::take(&mut self.keys);
        let mods = self.ctx.input(|i| i.modifiers);
        for ev in events {
            match ev {
                egui::Event::Key { key, pressed: true, modifiers, .. } => self.on_key(key, modifiers, None),
                egui::Event::Copy => self.on_key(Key::C, mods, None),
                egui::Event::Cut => self.on_key(Key::X, mods, None),
                egui::Event::Paste(text) => self.on_key(Key::V, mods, Some(text)),
                egui::Event::Text(t) => {
                    let alt_meta = mods.alt && !mods.ctrl && self.meta();
                    if !alt_meta && !(cfg!(target_os = "macos") && mods.mac_cmd) {
                        self.send_input(t.into_bytes());
                    }
                }
                egui::Event::Ime(egui::ImeEvent::Commit(t)) => self.send_input(t.into_bytes()),
                _ => {}
            }
        }
    }

    fn on_key(&mut self, key: Key, m: Modifiers, paste: Option<String>) {
        if key == Key::Escape && self.dlg.find.is_some() && m.is_none() {
            self.dlg.find = None;
            return;
        }
        if let Some(action) = self.combos.iter().find(|(_, c)| c.matches(key, m)).map(|(a, _)| a.clone()) {
            match (action.as_str(), paste) {
                ("paste", Some(text)) => self.paste_text(&text),
                _ => self.do_action(&action),
            }
            return;
        }
        let mac = cfg!(target_os = "macos");
        if mac && m.mac_cmd {
            // Familiar macOS line editing; other Cmd keys never reach the shell.
            let seq: &[u8] = match key {
                Key::ArrowLeft => b"\x01",
                Key::ArrowRight => b"\x05",
                Key::Backspace => b"\x15",
                _ => return,
            };
            self.send_input(seq.to_vec());
            return;
        }
        if mac && m.alt && !m.ctrl && !m.shift && !self.s.option_as_meta {
            match key {
                Key::ArrowLeft => return self.send_input(b"\x1bb".to_vec()),
                Key::ArrowRight => return self.send_input(b"\x1bf".to_vec()),
                Key::Backspace => return self.send_input(b"\x1b\x7f".to_vec()),
                _ => {}
            }
        }
        if cfg!(windows) && m.ctrl && m.alt {
            // AltGr: the character arrives as text.
            return;
        }
        let Some(p) = self.focused().and_then(|id| self.panes.get(&id)) else { return };
        let mode = *p.term.lock().mode();
        if let Some(bytes) = input::key_bytes(key, m, mode, self.meta()) {
            self.send_input(bytes);
        }
    }

    // ----- clipboard ------------------------------------------------------------------------

    fn set_clipboard(&mut self, text: &str, primary_only: bool) {
        let Some(c) = self.clipboard.as_mut() else { return };
        #[cfg(target_os = "linux")]
        {
            use arboard::{LinuxClipboardKind, SetExtLinux};
            let _ = c.set().clipboard(LinuxClipboardKind::Primary).text(text.to_owned());
        }
        if !primary_only {
            let _ = c.set_text(text.to_owned());
        }
    }

    fn get_clipboard(&mut self, primary: bool) -> Option<String> {
        let c = self.clipboard.as_mut()?;
        #[cfg(target_os = "linux")]
        if primary {
            use arboard::{GetExtLinux, LinuxClipboardKind};
            return c.get().clipboard(LinuxClipboardKind::Primary).text().ok();
        }
        let _ = primary;
        c.get_text().ok()
    }

    fn paste_text(&mut self, text: &str) {
        let Some(p) = self.focused().and_then(|id| self.panes.get(&id)) else { return };
        let mode = *p.term.lock().mode();
        self.send_input(input::paste_bytes(text, mode));
    }

    fn copy_selection(&mut self) {
        let Some(p) = self.focused().and_then(|id| self.panes.get(&id)) else { return };
        let text = p.term.lock().selection_to_string();
        if let Some(t) = text.filter(|t| !t.is_empty()) {
            self.set_clipboard(&t, false);
        }
    }

    // ----- actions --------------------------------------------------------------------------

    fn do_action(&mut self, action: &str) {
        let fid = self.focused();
        let pane = fid.and_then(|id| self.panes.get(&id));
        match action {
            "copy" => self.copy_selection(),
            "paste" => {
                if let Some(t) = self.get_clipboard(false) {
                    self.paste_text(&t);
                }
            }
            "select_all" => {
                if let Some(p) = pane {
                    let mut t = p.term.lock();
                    let top = Point::new(t.grid().topmost_line(), Column(0));
                    let bottom = Point::new(t.grid().bottommost_line(), t.grid().last_column());
                    let mut sel = Selection::new(SelectionType::Simple, top, Side::Left);
                    sel.update(bottom, Side::Right);
                    t.selection = Some(sel);
                    drop(t);
                    p.set_hold(true);
                }
            }
            "split_horizontal" => self.split(Dir::Col),
            "split_vertical" => self.split(Dir::Row),
            "close_pane" => {
                if let Some(id) = fid {
                    self.remove_pane(id);
                }
            }
            "new_tab" => self.new_tab(None),
            "close_tab" => self.close_tab(self.active),
            "next_tab" if !self.tabs.is_empty() => self.active = (self.active + 1) % self.tabs.len(),
            "prev_tab" if !self.tabs.is_empty() => self.active = (self.active + self.tabs.len() - 1) % self.tabs.len(),
            "next_pane" => self.cycle_pane(1),
            "prev_pane" => self.cycle_pane(-1),
            "focus_up" => self.focus_dir(Nav::Up),
            "focus_down" => self.focus_dir(Nav::Down),
            "focus_left" => self.focus_dir(Nav::Left),
            "focus_right" => self.focus_dir(Nav::Right),
            "resize_up" | "resize_down" | "resize_left" | "resize_right" => {
                let nav = match action {
                    "resize_up" => Nav::Up,
                    "resize_down" => Nav::Down,
                    "resize_left" => Nav::Left,
                    _ => Nav::Right,
                };
                if let (Some(id), Some(tab)) = (fid, self.tabs.get_mut(self.active)) {
                    tab.root.nudge(&id, nav, 0.03);
                }
            }
            "zoom" => {
                if let (Some(id), Some(tab)) = (fid, self.tabs.get_mut(self.active)) {
                    tab.zoomed = if tab.zoomed.is_some() { None } else { Some(id) };
                }
            }
            "equalize" => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.root.equalize();
                }
            }
            "scroll_lock" => {
                if let Some(p) = pane {
                    let on = !p.shared.locked.load(Ordering::Relaxed);
                    p.set_lock(on);
                }
            }
            "scroll_page_up" | "scroll_page_down" | "scroll_top" | "scroll_bottom" => {
                if let Some(p) = pane {
                    let scroll = match action {
                        "scroll_page_up" => Scroll::PageUp,
                        "scroll_page_down" => Scroll::PageDown,
                        "scroll_top" => Scroll::Top,
                        _ => Scroll::Bottom,
                    };
                    let at_bottom = {
                        let mut t = p.term.lock();
                        t.scroll_display(scroll);
                        t.grid().display_offset() == 0
                    };
                    if at_bottom {
                        p.set_lock(false);
                    }
                }
            }
            "find" => {
                if let Some(id) = fid {
                    self.dlg.open_find(id);
                }
            }
            "clear" => {
                if let Some(p) = pane {
                    {
                        let mut t = p.term.lock();
                        t.selection = None;
                        t.grid_mut().clear_history();
                        t.scroll_display(Scroll::Bottom);
                    }
                    p.set_lock(false);
                    // Ask the shell to redraw its prompt on a clean screen.
                    p.write(b"\x0c".to_vec());
                }
            }
            "broadcast" => {
                self.broadcast = match self.broadcast {
                    Broadcast::Off => Broadcast::Tab,
                    Broadcast::Tab => Broadcast::All,
                    Broadcast::All => Broadcast::Off,
                }
            }
            "commands" => {
                if let Some(id) = fid {
                    self.dlg.open_commands(id);
                }
            }
            "save_layout" => self.open_save_layout(self.active),
            "layouts" => self.dlg.open_layouts(false),
            "font_bigger" | "font_smaller" | "font_reset" => {
                let mut s = self.s.clone();
                s.font_size = match action {
                    "font_bigger" => (s.font_size + 1.0).min(72.0),
                    "font_smaller" => (s.font_size - 1.0).max(6.0),
                    _ => Settings::default().font_size,
                };
                self.apply_settings(s);
                self.s.save();
            }
            "fullscreen" => {
                self.fullscreen = !self.fullscreen;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
            "preferences" => self.dlg.open_prefs(&self.s),
            "about" => self.dlg.about = true,
            "export" => self.export_settings(),
            "import" => self.pick_import(),
            "quit" => self.close_window = true,
            _ => {}
        }
    }

    fn run_acts(&mut self) {
        for act in std::mem::take(&mut self.acts) {
            match act {
                Act::Action(a) => self.do_action(a),
                Act::Focus(id) => self.focus(id),
                Act::Run(id, cmd) => {
                    if let Some(p) = self.panes.get(&id) {
                        p.scroll_to_bottom();
                        p.run_command(&cmd);
                    }
                }
                Act::Insert(id, cmd) => {
                    self.focus(id);
                    // Hand the keyboard back to the terminal so the command can be edited and run.
                    self.ctx.memory_mut(|m| m.stop_text_input());
                    if let Some(p) = self.panes.get(&id) {
                        p.scroll_to_bottom();
                        let mode = *p.term.lock().mode();
                        // Bracketed paste where the shell supports it, so nothing runs by accident.
                        p.write(input::paste_bytes(cmd.trim_end_matches(['\r', '\n']), mode));
                    }
                }
                Act::EditCommands(id) => self.dlg.open_commands(id),
                Act::RenamePane(id) => {
                    let name = self.panes.get(&id).map(|p| p.title()).unwrap_or_default();
                    self.dlg.rename_pane = Some((id, name));
                }
                Act::CopyAll(id) => {
                    if let Some(text) = self.panes.get(&id).map(|p| all_text(&p.term.lock())) {
                        self.set_clipboard(&text, false);
                    }
                }
                Act::SaveOutput(id) => {
                    if let Some(p) = self.panes.get(&id) {
                        let text = all_text(&p.term.lock());
                        let name = format!("{}.log", p.title().replace(['/', '\\', ':'], "_"));
                        if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file()
                            && let Err(e) = std::fs::write(&path, text)
                        {
                            self.error = Some(format!("Could not save {}: {e}", path.display()));
                        }
                    }
                }
                Act::Paste(id, primary) => {
                    self.focus(id);
                    if let Some(t) = self.get_clipboard(primary) {
                        self.paste_text(&t);
                    }
                }
                Act::ScrollBottom(id) => {
                    if let Some(p) = self.panes.get(&id) {
                        p.scroll_to_bottom();
                    }
                }
                Act::ToggleLock(id) => {
                    if let Some(p) = self.panes.get(&id) {
                        p.set_lock(!p.shared.locked.load(Ordering::Relaxed));
                    }
                }
                Act::SelectTab(i) => self.active = i.min(self.tabs.len().saturating_sub(1)),
                Act::MoveTab(from, to) if from < self.tabs.len() && to < self.tabs.len() => {
                    let tab = self.tabs.remove(from);
                    self.tabs.insert(to, tab);
                    self.active = to;
                }
                Act::MoveTab(..) => {}
                Act::CloseTab(i) => self.close_tab(i),
                Act::RenameTab(i) => {
                    let title = self.tab_title(i);
                    self.dlg.rename_tab = Some((i, title));
                }
                Act::OpenLayout(name, replace, run) => match session::load_layout(&name) {
                    Some(l) => self.open_saved_layout(l, replace, run),
                    None => self.error = Some(format!("Could not read layout \"{name}\".")),
                },
                Act::SetBroadcast(b) => self.broadcast = b,
            }
        }
    }

    // ----- layouts --------------------------------------------------------------------------

    fn spawn_spec(&mut self, spec: &PaneSpec, run: bool) -> Option<u64> {
        let id = self.spawn(spec.cwd.as_ref().map(PathBuf::from), None)?;
        let p = self.panes.get_mut(&id)?;
        p.name = spec.name.clone();
        p.commands = spec.commands.clone();
        if run {
            p.pending = spec.commands.iter().filter(|c| c.auto_run).map(|c| c.command.clone()).collect();
        }
        Some(id)
    }

    /// Open a saved layout's tabs. Returns false when nothing could be opened.
    fn open_layout(&mut self, l: LayoutFile, replace: bool, run: bool) -> bool {
        if replace {
            self.tabs.clear();
            self.panes.clear();
            self.active = 0;
        }
        let first = self.tabs.len();
        for t in &l.tabs {
            let spawned = t.root.map(&mut |spec| self.spawn_spec(spec, run));
            let Some(root) = prune(spawned) else { continue };
            let leaves = root.leaves();
            let focused = leaves[t.focused.min(leaves.len() - 1)];
            let uid = self.take_id();
            self.tabs.push(Tab { uid, root, focused, zoomed: None, title: t.title.clone(), layout: t.layout.clone(), activity: false });
        }
        if self.tabs.len() == first {
            return false;
        }
        self.active = (first + l.active_tab).min(self.tabs.len() - 1);
        if !l.name.is_empty() && l.name != "session" {
            self.layout_name = l.name.clone();
        }
        true
    }

    /// Open a layout the user picked; its tabs show the layout's name.
    fn open_saved_layout(&mut self, l: LayoutFile, replace: bool, run: bool) {
        let first = if replace { 0 } else { self.tabs.len() };
        let name = l.name.clone();
        self.open_layout(l, replace, run);
        // Files saved before tabs remembered their layout carry no name per tab.
        for t in self.tabs.iter_mut().skip(first) {
            t.layout.get_or_insert_with(|| name.clone());
        }
    }

    fn snapshot(&mut self, name: &str, only_tab: Option<usize>) -> LayoutFile {
        let tabs = &self.tabs;
        let panes = &mut self.panes;
        let mut out = Vec::new();
        for (i, tab) in tabs.iter().enumerate() {
            if only_tab.is_some_and(|o| o != i) {
                continue;
            }
            let leaves = tab.root.leaves();
            let root = tab.root.map(&mut |id| match panes.get_mut(id) {
                Some(p) => {
                    PaneSpec { cwd: Some(p.current_dir().display().to_string()), name: p.name.clone(), commands: p.commands.clone() }
                }
                None => PaneSpec::default(),
            });
            out.push(TabSpec {
                title: tab.title.clone(),
                root,
                focused: leaves.iter().position(|l| *l == tab.focused).unwrap_or(0),
                layout: tab.layout.clone(),
            });
        }
        let active_tab = if only_tab.is_some() { 0 } else { self.active };
        LayoutFile { name: name.to_owned(), tabs: out, active_tab, window_size: self.window_size }
    }

    fn tab_title(&self, i: usize) -> String {
        let Some(tab) = self.tabs.get(i) else { return String::new() };
        if let Some(t) = &tab.title {
            return t.clone();
        }
        let term = self.panes.get(&tab.focused).map(|p| p.title()).unwrap_or_default();
        match (self.s.tab_title, &tab.layout) {
            (TabTitle::Layout, Some(l)) => l.clone(),
            (TabTitle::LayoutAndTerminal, Some(l)) => format!("{l} · {term}"),
            _ => term,
        }
    }

    // ----- per-frame upkeep -----------------------------------------------------------------

    fn housekeeping(&mut self) {
        let dead: Vec<u64> = self.panes.iter().filter(|(_, p)| p.exited()).map(|(id, _)| *id).collect();
        for id in dead {
            self.remove_pane(id);
        }

        let now = Instant::now();
        let mut clip = Vec::new();
        let active_leaves: Vec<u64> = self.tabs.get(self.active).map(|t| t.root.leaves()).unwrap_or_default();
        for (id, p) in self.panes.iter_mut() {
            // Saved commands wait until the shell has drawn its prompt and gone quiet; shells
            // like zsh drop anything typed while they are still starting up.
            if !p.pending.is_empty() {
                let age = p.spawned.elapsed();
                let quiet = age.as_millis() as u64 - p.shared.last_output_ms.load(Ordering::Relaxed).min(age.as_millis() as u64);
                let ready = p.shared.got_output.load(Ordering::Relaxed) && quiet >= 300;
                if ready || age > Duration::from_secs(8) {
                    for c in std::mem::take(&mut p.pending) {
                        p.run_command(&c);
                    }
                } else {
                    self.ctx.request_repaint_after(Duration::from_millis(100));
                }
            }
            if let Some(t) = p.shared.clipboard_store.lock().unwrap().take() {
                clip.push(t);
            }
            if p.shared.bell.swap(false, Ordering::Relaxed) {
                p.bell_until = Some(now + Duration::from_millis(250));
                self.ctx.request_repaint_after(Duration::from_millis(260));
            }
            if active_leaves.contains(id) {
                p.shared.activity.store(false, Ordering::Relaxed);
                p.poll_cwd();
            }
        }
        for t in clip {
            self.set_clipboard(&t, false);
        }
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if i == self.active {
                tab.activity = false;
            } else if tab.root.leaves().iter().any(|id| {
                // A shell drawing its first prompt is not news.
                self.panes
                    .get(id)
                    .is_some_and(|p| p.shared.activity.swap(false, Ordering::Relaxed) && p.spawned.elapsed() > Duration::from_secs(3))
            }) {
                tab.activity = true;
            }
        }

        let title = match self.tabs.len() {
            0 => "term".to_string(),
            _ => format!("{} — term", self.tab_title(self.active)),
        };
        if title != self.window_title {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    // ----- drawing --------------------------------------------------------------------------

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let theme = self.s.theme.clone();
        let dark = theme.background.luminance() < 0.5;
        // The strip sits a little darker (or, for light themes, a little greyer) than the terminals.
        let bar = if dark { mix(theme.background, Rgb(0, 0, 0), 0.35) } else { mix(theme.background, theme.foreground, 0.08) };
        let bar = if bar == theme.background { mix(theme.background, theme.foreground, 0.08) } else { bar };
        const H: f32 = 32.0;
        egui::Panel::top("tabs").exact_size(H).frame(egui::Frame::NONE.fill(bar.color())).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(6.0);
                let n_tabs = self.tabs.len().max(1);
                let room = (ui.available_width() - 120.0).max(100.0);
                let tab_w = (room / n_tabs as f32).clamp(90.0, 220.0);
                let mut rects = Vec::with_capacity(self.tabs.len());
                let mut dragging = None;

                for i in 0..self.tabs.len() {
                    let uid = self.tabs[i].uid;
                    let rect = ui.allocate_space(vec2(tab_w, H)).1;
                    let resp = ui.interact(rect, ui.id().with(("tab", uid)), Sense::click_and_drag());
                    rects.push(rect);
                    let active = i == self.active;
                    // Use the pointer position, not resp.hovered(): the close button sits on top of the
                    // tab, so hovering it would otherwise hide it again before it can be clicked.
                    let hovered = ui.rect_contains_pointer(rect) || resp.dragged();
                    let body = Rect::from_min_max(pos2(rect.min.x + 1.0, rect.min.y + 5.0), pos2(rect.max.x - 1.0, rect.max.y));
                    let radius = egui::CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 };
                    let painter = ui.painter();
                    if active {
                        painter.rect_filled(body, radius, theme.background.color());
                        let line = Rect::from_min_max(body.min + vec2(6.0, 0.0), pos2(body.max.x - 6.0, body.min.y + 2.0));
                        painter.rect_filled(line, 1.0, theme.accent.color());
                    } else if hovered {
                        painter.rect_filled(body, radius, mix(bar, theme.foreground, 0.07).color());
                    }
                    // Thin separator between two quiet tabs.
                    let next_active = i + 1 == self.active;
                    if !active && !next_active && !hovered && i + 1 < self.tabs.len() {
                        let x = body.max.x + 0.5;
                        painter.vline(
                            x,
                            (body.min.y + 7.0)..=(body.max.y - 7.0),
                            Stroke::new(1.0, mix(bar, theme.foreground, 0.18).color()),
                        );
                    }

                    let text_col = if active { theme.foreground } else { mix(bar, theme.foreground, if hovered { 0.85 } else { 0.6 }) };
                    let mut x = body.min.x + 12.0;
                    let cy = body.center().y;
                    if self.tabs[i].activity && !active {
                        painter.circle_filled(pos2(x + 3.0, cy), 3.0, theme.accent.color());
                        x += 12.0;
                    }
                    // Right side: close button (active / hovered) and the terminal count.
                    let mut right = body.max.x - 8.0;
                    let show_close = active || hovered;
                    let close_rect = Rect::from_center_size(pos2(right - 8.0, cy), vec2(18.0, 18.0));
                    if show_close {
                        right = close_rect.min.x - 4.0;
                    }
                    let n = self.tabs[i].root.leaves().len();
                    if n > 1 {
                        let g = painter.layout_no_wrap(n.to_string(), egui::FontId::proportional(10.5), text_col.color());
                        let pill = Rect::from_min_size(pos2(right - g.size().x - 10.0, cy - 8.0), vec2(g.size().x + 10.0, 16.0));
                        painter.rect_filled(pill, 8.0, mix(if active { theme.background } else { bar }, theme.foreground, 0.16).color());
                        painter.galley(pill.center() - g.size() / 2.0, g, text_col.color());
                        right = pill.min.x - 6.0;
                    }
                    let title = self.tab_title(i);
                    let mut job = egui::text::LayoutJob::simple_singleline(title, egui::FontId::proportional(12.5), text_col.color());
                    job.wrap = egui::text::TextWrapping {
                        max_width: (right - x).max(10.0),
                        max_rows: 1,
                        break_anywhere: true,
                        overflow_character: Some('…'),
                    };
                    let g = painter.layout_job(job);
                    painter.galley(pos2(x, cy - g.size().y / 2.0), g, text_col.color());

                    if show_close {
                        let c = ui.interact(close_rect, ui.id().with(("tab_close", uid)), Sense::click());
                        let col = if c.hovered() {
                            ui.painter().circle_filled(close_rect.center(), 9.0, mix(theme.background, theme.foreground, 0.18).color());
                            theme.foreground
                        } else {
                            text_col
                        };
                        let k = 3.5;
                        let ctr = close_rect.center();
                        for (a, b) in [(vec2(-k, -k), vec2(k, k)), (vec2(-k, k), vec2(k, -k))] {
                            ui.painter().line_segment([ctr + a, ctr + b], Stroke::new(1.3, col.color()));
                        }
                        if c.clicked() {
                            self.acts.push(Act::CloseTab(i));
                            continue;
                        }
                        c.on_hover_text("Close tab");
                    }

                    if resp.clicked() || resp.drag_started() {
                        self.acts.push(Act::SelectTab(i));
                    }
                    if resp.double_clicked() {
                        self.acts.push(Act::RenameTab(i));
                    }
                    if resp.middle_clicked() {
                        self.acts.push(Act::CloseTab(i));
                    }
                    if resp.dragged() {
                        dragging = Some(i);
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Rename tab…").clicked() {
                            self.acts.push(Act::RenameTab(i));
                            ui.close();
                        }
                        if ui.button("Save this tab as a layout…").clicked() {
                            self.acts.push(Act::SelectTab(i));
                            self.open_save_layout(i);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Close tab").clicked() {
                            self.acts.push(Act::CloseTab(i));
                            ui.close();
                        }
                    });
                }

                // Drag a tab sideways to reorder.
                if let (Some(from), Some(p)) = (dragging, ui.ctx().pointer_latest_pos()) {
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    if let Some(to) = rects.iter().position(|r| p.x >= r.min.x && p.x < r.max.x)
                        && to != from
                    {
                        self.acts.push(Act::MoveTab(from, to));
                    }
                }

                // New-tab button.
                ui.add_space(4.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(28.0, H), Sense::click());
                let r = Rect::from_center_size(rect.center() + vec2(0.0, 2.0), vec2(24.0, 24.0));
                let col = if resp.hovered() {
                    ui.painter().rect_filled(r, 6.0, mix(bar, theme.foreground, 0.1).color());
                    theme.foreground
                } else {
                    mix(bar, theme.foreground, 0.6)
                };
                let c = r.center();
                ui.painter().line_segment([c - vec2(5.0, 0.0), c + vec2(5.0, 0.0)], Stroke::new(1.5, col.color()));
                ui.painter().line_segment([c - vec2(0.0, 5.0), c + vec2(0.0, 5.0)], Stroke::new(1.5, col.color()));
                if resp.on_hover_text(self.hint("New tab", "new_tab")).clicked() {
                    self.acts.push(Act::Action("new_tab"));
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(6.0);
                    ui.menu_button(RichText::new("☰").size(14.0), |ui| self.main_menu(ui));
                    ui.add_space(8.0);
                    match self.broadcast {
                        Broadcast::Off => {}
                        Broadcast::Tab => {
                            ui.colored_label(Color32::from_rgb(230, 120, 40), "📡 Broadcasting to this tab");
                        }
                        Broadcast::All => {
                            ui.colored_label(Color32::from_rgb(230, 60, 40), "📡 Broadcasting to ALL terminals");
                        }
                    }
                });
            });
        });
    }

    fn hint(&self, label: &str, action: &str) -> String {
        let k = self.s.shortcut_label(action);
        if k.is_empty() { label.to_string() } else { format!("{label}  ({k})") }
    }

    fn item(&mut self, ui: &mut egui::Ui, label: &str, action: &'static str) {
        let k = self.s.shortcut_label(action);
        if ui.add(egui::Button::new(label).shortcut_text(k)).clicked() {
            self.acts.push(Act::Action(action));
            ui.close();
        }
    }

    fn main_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(240.0);
        self.item(ui, "New tab", "new_tab");
        self.item(ui, "Split horizontally", "split_horizontal");
        self.item(ui, "Split vertically", "split_vertical");
        self.item(ui, "Close terminal", "close_pane");
        self.item(ui, "Zoom terminal", "zoom");
        self.item(ui, "Equalise sizes", "equalize");
        ui.separator();
        self.item(ui, "Saved commands…", "commands");
        self.item(ui, "Find…", "find");
        self.item(ui, "Toggle scroll lock", "scroll_lock");
        ui.menu_button("Broadcast input", |ui| {
            for (b, label) in [(Broadcast::Off, "Off"), (Broadcast::Tab, "To this tab"), (Broadcast::All, "To all terminals")] {
                if ui.radio(self.broadcast == b, label).clicked() {
                    self.acts.push(Act::SetBroadcast(b));
                    ui.close();
                }
            }
        });
        ui.separator();
        self.item(ui, "Save layout…", "save_layout");
        self.item(ui, "Layouts…", "layouts");
        let names = session::list_layouts();
        ui.add_enabled_ui(!names.is_empty(), |ui| {
            ui.menu_button("Open layout", |ui| {
                for n in names {
                    if ui.button(&n).clicked() {
                        self.acts.push(Act::OpenLayout(n, false, true));
                        ui.close();
                    }
                }
            });
        });
        ui.separator();
        self.item(ui, "Preferences…", "preferences");
        if ui.button("Export settings & layouts…").clicked() {
            self.acts.push(Act::Action("export"));
            ui.close();
        }
        if ui.button("Import settings & layouts…").clicked() {
            self.acts.push(Act::Action("import"));
            ui.close();
        }
        self.item(ui, "Full screen", "fullscreen");
        if ui.button("About term").clicked() {
            self.acts.push(Act::Action("about"));
            ui.close();
        }
        self.item(ui, "Quit", "quit");
    }

    /// Saved commands in a menu: click runs it, "Insert" (or Shift+click) types it without Enter.
    fn command_items(&mut self, ui: &mut egui::Ui, id: u64, commands: &[SavedCommand]) {
        let shift = ui.input(|i| i.modifiers.shift);
        let theme = &self.s.theme;
        let pill = mix(theme.background, theme.foreground, 0.16).color();
        let weak = mix(theme.background, theme.foreground, 0.55).color();
        // One width for every row so the commands line up, sized to the longest one.
        let font = egui::FontId::monospace(ui.style().text_styles[&egui::TextStyle::Body].size);
        let longest = commands
            .iter()
            .map(|c| ui.painter().layout_no_wrap(c.command.clone(), font.clone(), Color32::WHITE).size().x)
            .fold(0.0, f32::max);
        let width = (longest + 90.0).clamp(240.0, 560.0);
        for c in commands {
            ui.horizontal(|ui| {
                let mut run = egui::Button::new(())
                    .left_text(RichText::new(format!("▶  {}", c.command)).monospace())
                    .min_size(vec2(width, 26.0))
                    .truncate();
                if c.auto_run {
                    run = run.right_text(RichText::new("auto").size(10.5).color(weak));
                }
                let run = ui.add(run);
                let insert = ui.add(
                    egui::Button::new(RichText::new("Insert").size(11.0))
                        .frame(true)
                        .fill(pill)
                        .corner_radius(10)
                        .min_size(vec2(0.0, 20.0)),
                );
                if insert.on_hover_text("Type it into the terminal without pressing Enter (or Shift+click the command)").clicked()
                    || (run.clicked() && shift)
                {
                    self.acts.push(Act::Insert(id, c.command.clone()));
                    ui.close();
                } else if run.on_hover_text("Run (Shift+click to type it without pressing Enter)").clicked() {
                    self.acts.push(Act::Run(id, c.command.clone()));
                    ui.close();
                }
            });
        }
    }

    fn pane_menu(&mut self, ui: &mut egui::Ui, id: u64) {
        ui.set_min_width(240.0);
        let Some(p) = self.panes.get(&id) else { return };
        let has_sel = p.term.lock().selection.as_ref().is_some_and(|s| !s.is_empty());
        let locked = p.shared.locked.load(Ordering::Relaxed);
        let commands: Vec<SavedCommand> = p.commands.clone();
        ui.add_enabled_ui(has_sel, |ui| self.item(ui, "Copy", "copy"));
        self.item(ui, "Paste", "paste");
        self.item(ui, "Select all", "select_all");
        ui.separator();
        ui.menu_button("Saved commands", |ui| {
            ui.set_min_width(220.0);
            if commands.is_empty() {
                ui.weak("No saved commands yet");
            }
            self.command_items(ui, id, &commands);
            ui.separator();
            if ui.button("Edit saved commands…").clicked() {
                self.acts.push(Act::EditCommands(id));
                ui.close();
            }
        });
        if ui.button("Rename terminal…").clicked() {
            self.acts.push(Act::RenamePane(id));
            ui.close();
        }
        ui.separator();
        self.item(ui, "Split horizontally", "split_horizontal");
        self.item(ui, "Split vertically", "split_vertical");
        self.item(ui, "Zoom terminal", "zoom");
        self.item(ui, "New tab", "new_tab");
        ui.separator();
        if ui.button(if locked { "Release scroll lock" } else { "Scroll lock (freeze view)" }).clicked() {
            self.acts.push(Act::ToggleLock(id));
            ui.close();
        }
        self.item(ui, "Find…", "find");
        if ui.button("Copy all output").clicked() {
            self.acts.push(Act::CopyAll(id));
            ui.close();
        }
        if ui.button("Save output to file…").clicked() {
            self.acts.push(Act::SaveOutput(id));
            ui.close();
        }
        self.item(ui, "Clear scrollback", "clear");
        ui.separator();
        ui.menu_button("Window", |ui| self.main_menu(ui));
        ui.separator();
        self.item(ui, "Close terminal", "close_pane");
    }

    fn draw_tab(&mut self, ui: &mut egui::Ui) {
        let area = ui.max_rect();
        let divider_bg = mix(self.s.theme.background, self.s.theme.foreground, 0.18);
        ui.painter().rect_filled(area, 0.0, divider_bg.color());
        let Some(tab) = self.tabs.get(self.active) else { return };
        let gap = self.s.divider_width.max(1.0);
        let mut rects = Vec::new();
        let mut divs = Vec::new();
        match tab.zoomed {
            Some(z) => rects.push((z, area)),
            None => tab.root.layout(area, gap, &mut rects, &mut divs),
        }
        let focused = tab.focused;
        let multi = rects.len() > 1;

        let mut new_ratios = Vec::new();
        for d in &divs {
            let id = ui.id().with(("divider", self.active, d.path.clone()));
            let resp = ui.interact(d.rect.expand(2.0), id, Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(match d.dir {
                    Dir::Row => CursorIcon::ResizeColumn,
                    Dir::Col => CursorIcon::ResizeRow,
                });
                ui.painter().rect_filled(d.rect, 0.0, self.s.theme.accent.color().gamma_multiply(0.7));
            }
            if resp.dragged()
                && let Some(p) = resp.interact_pointer_pos()
            {
                new_ratios.push((d.path.clone(), layout::ratio_for(d, p, gap)));
            }
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            for (path, r) in new_ratios {
                tab.root.set_ratio(&path, r);
            }
        }

        for (id, r) in &rects {
            self.draw_pane(ui, *id, *r, *id == focused, multi);
        }
        self.rects = rects;
    }

    fn draw_pane(&mut self, ui: &mut egui::Ui, id: u64, r: Rect, focused: bool, multi: bool) {
        let Some(m) = self.m.clone() else { return };
        let show_header = self.s.show_titlebars;
        let header_h = if show_header { 20.0 } else { 0.0 };
        let header = Rect::from_min_size(r.min, vec2(r.width(), header_h));
        let body = Rect::from_min_max(pos2(r.min.x, r.min.y + header_h), r.max);
        let pad = self.s.padding;
        let text_rect = Rect::from_min_max(body.min + vec2(pad, pad), body.max - vec2(pad, pad));
        let blink_on = !self.s.cursor_blink || (self.blink_start.elapsed().as_millis() / 530).is_multiple_of(2);
        let in_broadcast = match self.broadcast {
            Broadcast::Off => false,
            Broadcast::Tab | Broadcast::All => true,
        };

        let Some(p) = self.panes.get_mut(&id) else { return };
        let size = m.grid_size(text_rect);
        p.resize(size, (m.cw, m.ch));

        let resp = ui.interact(body, ui.id().with(("pane", id)), Sense::click_and_drag());
        let out = view::mouse(ui, &resp, p, text_rect, &m, &self.s);

        let painter = ui.painter().with_clip_rect(body);
        painter.rect_filled(body, 0.0, self.s.theme.background.color());
        view::draw(&painter, text_rect, p, &self.s, &m, focused, blink_on);
        if !focused && multi && self.s.inactive_dim > 0.0 {
            let a = (self.s.inactive_dim.clamp(0.0, 0.8) * 255.0) as u8;
            let bg = self.s.theme.background;
            painter.rect_filled(body, 0.0, Color32::from_rgba_unmultiplied(bg.0, bg.1, bg.2, a));
        }
        let bell = p.bell_until.is_some_and(|t| Instant::now() < t);
        if bell {
            painter.rect_stroke(body.shrink(1.0), 0.0, Stroke::new(2.0, self.s.theme.accent.color()), egui::StrokeKind::Inside);
        }
        if focused && multi && !show_header {
            painter.rect_stroke(body, 0.0, Stroke::new(1.0, self.s.theme.accent.color()), egui::StrokeKind::Inside);
        }

        let title = p.title();
        let cwd = p.cwd.clone();
        let locked = p.shared.locked.load(Ordering::Relaxed);
        let commands = p.commands.clone();
        let running = p.has_running_job();
        if view::hold_badge(ui, text_rect, p, &self.s) {
            self.acts.push(Act::ScrollBottom(id));
        }

        if out.focus && !focused {
            self.acts.push(Act::Focus(id));
        }
        if out.focus {
            // Focus now so a right-click menu acts on this terminal.
            self.focus(id);
        }
        if let Some(text) = out.copy {
            self.set_clipboard(&text, !self.s.copy_on_select);
        }
        if let Some(primary) = out.paste {
            self.acts.push(Act::Paste(id, primary));
        }
        if out.menu_allowed {
            resp.context_menu(|ui| self.pane_menu(ui, id));
        }

        if show_header {
            self.draw_header(ui, header, id, focused, &title, &cwd, locked, &commands, running, in_broadcast);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_header(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        id: u64,
        focused: bool,
        title: &str,
        cwd: &std::path::Path,
        locked: bool,
        commands: &[SavedCommand],
        running: bool,
        broadcast: bool,
    ) {
        let theme = &self.s.theme;
        let bg = if focused { theme.accent } else { mix(theme.background, theme.foreground, 0.22) };
        let fg = if bg.luminance() > 0.6 { Color32::BLACK } else { Color32::WHITE };
        ui.painter().rect_filled(rect, 0.0, bg.color());
        let resp = ui.interact(rect, ui.id().with(("header", id)), Sense::click());
        if resp.clicked() {
            self.acts.push(Act::Focus(id));
        }
        if resp.double_clicked() {
            self.acts.push(Act::RenamePane(id));
        }

        let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
        child.spacing_mut().item_spacing.x = 4.0;
        child.style_mut().visuals.override_text_color = Some(fg);
        let small = |s: &str| RichText::new(s).size(11.5).color(fg);
        child.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let btn = |ui: &mut egui::Ui, s: &str, tip: &str| ui.add(egui::Button::new(small(s)).frame(false)).on_hover_text(tip);
            if btn(ui, "✕", "Close terminal").clicked() {
                self.acts.push(Act::Focus(id));
                self.acts.push(Act::Action("close_pane"));
            }
            if btn(ui, "⛶", &self.hint("Zoom", "zoom")).clicked() {
                self.acts.push(Act::Focus(id));
                self.acts.push(Act::Action("zoom"));
            }
            if btn(ui, "⬍", &self.hint("Split horizontally", "split_horizontal")).clicked() {
                self.acts.push(Act::Focus(id));
                self.acts.push(Act::Action("split_horizontal"));
            }
            if btn(ui, "⬌", &self.hint("Split vertically", "split_vertical")).clicked() {
                self.acts.push(Act::Focus(id));
                self.acts.push(Act::Action("split_vertical"));
            }
            let lock_tip = if locked { "Scroll lock is on: click to follow output again" } else { "Scroll lock: freeze the view" };
            if btn(ui, if locked { "🔒" } else { "🔓" }, lock_tip).clicked() {
                self.acts.push(Act::ToggleLock(id));
            }
            let cmd_label = if commands.is_empty() { "▶".to_string() } else { format!("▶ {}", commands.len()) };
            // Frameless, like the other title-bar icons.
            ui.style_mut().visuals.button_frame = false;
            ui.menu_button(small(&cmd_label), |ui| {
                ui.set_min_width(220.0);
                if commands.is_empty() {
                    ui.weak("No saved commands yet");
                }
                self.command_items(ui, id, commands);
                ui.separator();
                if ui.button("Edit saved commands…").clicked() {
                    self.acts.push(Act::EditCommands(id));
                    ui.close();
                }
            })
            .response
            .on_hover_text("Saved commands for this terminal");
            if broadcast {
                ui.label(small("📡"));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let mut text = title.to_string();
                if running {
                    text = format!("● {text}");
                }
                ui.add(egui::Label::new(RichText::new(text).size(12.0).color(fg).strong()).truncate().selectable(false));
                let dir = cwd.display().to_string();
                let home = dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_default();
                let dir = if !home.is_empty() && dir.starts_with(&home) { format!("~{}", &dir[home.len()..]) } else { dir };
                ui.add(egui::Label::new(RichText::new(dir).size(11.0).color(fg.gamma_multiply(0.75))).truncate().selectable(false));
            });
        });
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// Drop leaves whose terminal failed to start.
fn prune(n: Node<Option<u64>>) -> Option<Node<u64>> {
    match n {
        Node::Leaf(l) => l.map(Node::Leaf),
        Node::Split { dir, ratio, a, b } => match (prune(*a), prune(*b)) {
            (Some(a), Some(b)) => Some(Node::Split { dir, ratio, a: Box::new(a), b: Box::new(b) }),
            (a, b) => a.or(b),
        },
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // While no text field has focus, keystrokes belong to the terminal: take them before egui
        // can use Tab / Enter / Space / arrows for its own widget focus.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let mut keep = Vec::with_capacity(raw.events.len());
        for ev in raw.events.drain(..) {
            match ev {
                egui::Event::Key { .. }
                | egui::Event::Text(_)
                | egui::Event::Copy
                | egui::Event::Cut
                | egui::Event::Paste(_)
                | egui::Event::Ime(_) => self.keys.push(ev),
                other => keep.push(other),
            }
        }
        raw.events = keep;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.m = Some(Metrics::new(&ctx, &self.s));
        #[cfg(debug_assertions)]
        self.selftest_frame(&ctx);
        if let Some(r) = ctx.input(|i| i.viewport().inner_rect)
            && !self.fullscreen
        {
            self.window_size = Some([r.width(), r.height()]);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_window && self.dlg.wants_confirm_quit(&self.s, self.running_jobs())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }

        self.housekeeping();
        self.handle_keys();
        self.run_acts();

        if self.s.always_show_tabs || self.tabs.len() > 1 {
            self.tab_bar(ui);
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.draw_tab(ui));
        self.dialogs(&ctx);
        #[cfg(any(test, debug_assertions))]
        if self.preview_menu
            && let Some(id) = self.focused()
        {
            let commands = self.panes.get(&id).map(|p| p.commands.clone()).unwrap_or_default();
            egui::Area::new(egui::Id::new("menu_preview")).fixed_pos(pos2(700.0, 60.0)).show(&ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.style_mut().visuals.button_frame = false;
                    self.command_items(ui, id, &commands);
                    ui.separator();
                    let _ = ui.button("Edit saved commands…");
                });
            });
        }
        self.run_acts();

        if self.s.cursor_blink {
            ctx.request_repaint_after(Duration::from_millis(530));
        }
        if self.close_window {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn on_exit(&mut self) {
        #[cfg(debug_assertions)]
        if self.selftest.is_some() {
            return;
        }
        if self.tabs.is_empty() {
            let _ = std::fs::remove_file(crate::config::config_dir().join("session.json"));
            return;
        }
        let name = if self.layout_name.is_empty() { "session".to_string() } else { self.layout_name.clone() };
        let snap = self.snapshot(&name, None);
        session::save_session(&snap);
    }
}

impl App {
    fn running_jobs(&self) -> usize {
        self.panes.values().filter(|p| p.has_running_job()).count()
    }
}

#[cfg(debug_assertions)]
impl App {
    fn selftest_frame(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.selftest.take() else { return };
        st.frame += 1;
        if st.frame == 1 {
            // macOS stops drawing windows that are covered; keep this one in front.
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        ctx.request_repaint_after(Duration::from_millis(50));
        // A step every 600 ms once the shell is up, then a screenshot.
        if !st.steps.is_empty() && st.started.elapsed() > Duration::from_millis(1200) {
            let step = st.steps.remove(0);
            self.script_step(&step);
            st.started = Instant::now() - Duration::from_millis(600);
        } else if st.steps.is_empty() && st.frame < u64::MAX / 2 && st.started.elapsed() > Duration::from_millis(2500) {
            st.frame = u64::MAX / 2;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let mut data = format!("P6\n{} {}\n255\n", img.width(), img.height()).into_bytes();
            for p in &img.pixels {
                data.extend_from_slice(&[p.r(), p.g(), p.b()]);
            }
            let _ = std::fs::write(&st.out, data);
            self.close_window = true;
        }
        self.selftest = Some(st);
    }
}

#[cfg(any(test, debug_assertions))]
impl App {
    #[cfg(test)]
    pub(crate) fn save_dialog_only_tab(&self) -> Option<bool> {
        self.dlg.save_layout.as_ref().map(|s| s.only_tab)
    }

    #[cfg(test)]
    pub(crate) fn focused_commands(&self) -> Vec<(String, bool)> {
        let p = self.focused().and_then(|id| self.panes.get(&id));
        p.map(|p| p.commands.iter().map(|c| (c.command.clone(), c.auto_run)).collect()).unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn tab_uids(&self) -> Vec<u64> {
        self.tabs.iter().map(|t| t.uid).collect()
    }

    /// Scripted input for self-tests: an action id, `run:<command>`, or `select:<row>`.
    pub(crate) fn script_step(&mut self, step: &str) {
        let pane = self.focused().and_then(|id| self.panes.get(&id));
        if let Some(cmd) = step.strip_prefix("run:") {
            if let Some(p) = pane {
                p.run_command(cmd);
            }
        } else if let Some(path) = step.strip_prefix("import_file:") {
            match crate::transfer::read(std::path::Path::new(path)) {
                Ok(b) => self.dlg.open_import(path.into(), b),
                Err(e) => self.error = Some(e),
            }
        } else if step == "menu_preview" {
            self.preview_menu = true;
        } else if let Some(cmd) = step.strip_prefix("insert:") {
            if let Some(id) = self.focused() {
                self.acts.push(Act::Insert(id, cmd.to_string()));
                self.run_acts();
            }
        } else if let Some(cmd) = step.strip_prefix("addcmd:") {
            let (cmd, auto) = match cmd.strip_suffix("!") {
                Some(c) => (c, true),
                None => (cmd, false),
            };
            if let Some(p) = self.focused().and_then(|id| self.panes.get_mut(&id)) {
                p.commands.push(SavedCommand { command: cmd.to_string(), auto_run: auto });
            }
        } else if let Some(name) = step.strip_prefix("set_theme:") {
            if let Some(t) = Theme::presets().into_iter().find(|t| t.name == name) {
                self.set_theme(t);
            }
        } else if let Some(name) = step.strip_prefix("theme:") {
            // Pick a preset in the open Preferences window, as if chosen from the dropdown.
            self.dlg.pick_theme(name);
        } else if let Some(path) = step.strip_prefix("layout:") {
            // Open a layout file with its auto-run commands, replacing everything.
            let l = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok());
            if let Some(l) = l {
                self.open_saved_layout(l, true, true);
            }
        } else if let Some(row) = step.strip_prefix("select:").and_then(|r| r.parse::<i32>().ok()) {
            if let Some(p) = pane {
                let mut t = p.term.lock();
                let line = alacritty_terminal::index::Line(row - t.grid().display_offset() as i32);
                let mut sel = Selection::new(SelectionType::Lines, Point::new(line, Column(0)), Side::Left);
                sel.update(Point::new(line, Column(0)), Side::Left);
                t.selection = Some(sel);
                drop(t);
                p.set_hold(true);
            }
        } else {
            let action: &'static str = Box::leak(step.to_string().into_boxed_str());
            self.do_action(action);
        }
    }
}
