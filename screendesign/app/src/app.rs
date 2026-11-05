//! The editor: window layout, files, undo, live reload and the side panels.

use crate::catalog;
use crate::draw;
use crate::model::{self, Control, Project};
use crate::pen::{Pen, R};
use crate::render;
use eframe::egui::{self, Color32, Key, Modifiers, Pos2, RichText, Sense, Vec2};
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(PartialEq, Clone, Copy)]
pub enum Mode {
    Edit,
    Present,
    Flow,
}

pub enum Drag {
    None,
    Move { start: Pos2, orig: Vec<(u64, f32, f32)>, bbox: R },
    Resize { id: u64, handle: usize, orig: R, start: Pos2 },
    Marquee { start: Pos2, add: bool },
}

pub struct App {
    pub proj: Project,
    pub path: PathBuf,
    pub named: bool,
    pub cur: usize,
    pub sel: Vec<u64>,
    pub zoom: f32,
    pub pan: Vec2,
    pub sketch: bool,
    pub dark_ui: bool,
    pub grid: bool,
    pub snap: bool,
    pub grid_size: f32,
    pub show_frame: bool,
    pub undo: Vec<Project>,
    pub redo: Vec<Project>,
    pub skip_record: bool,
    pub last_change: f64,
    pub drag: Drag,
    pub editing: Option<u64>,
    pub clip: Vec<Control>,
    /// Pictures used by `clip`, so they paste into other projects too.
    pub clip_images: std::collections::BTreeMap<String, crate::images::ImgRef>,
    /// Fingerprint of the picture last put on the system clipboard by Copy.
    pub clip_image: Option<u64>,
    /// A picture was just pasted by the paste event, so the Ctrl+V release must not paste it again.
    pub image_pasted: bool,
    pub last_cmd: f64,
    pub bg_tool: Option<crate::picture::BgTool>,
    pub quick: String,
    pub focus_quick: bool,
    pub lib_cat: usize,
    pub lib_drag: Option<String>,
    pub mode: Mode,
    pub history: Vec<usize>,
    pub dirty_at: Option<f64>,
    pub disk_mtime: Option<SystemTime>,
    pub last_poll: f64,
    pub status: String,
    pub show_mcp: bool,
    pub show_versions: bool,
    pub show_help: bool,
    pub version_name: String,
    pub guides: Vec<(bool, f32)>,
    pub fit_next: bool,
    pub canvas_rect: egui::Rect,
    pub shot: Option<PathBuf>,
    pub frames: u32,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, file: Option<PathBuf>, shot: Option<PathBuf>) -> App {
        render::install_fonts(&cc.egui_ctx);
        let file_given = file.is_some();
        if let Some(rs) = &cc.wgpu_render_state {
            let i = rs.adapter.get_info();
            crate::log_startup(&format!("wgpu {:?}: {} ({:?}, driver {} {})", i.backend, i.name, i.device_type, i.driver, i.driver_info));
        } else if let Some(gl) = &cc.gl {
            use eframe::glow::HasContext;
            let (v, r) = unsafe { (gl.get_parameter_string(eframe::glow::VERSION), gl.get_parameter_string(eframe::glow::RENDERER)) };
            crate::log_startup(&format!("glow: {r} (OpenGL {v})"));
        }
        let (proj, path, named, status) = if shot.is_some() && file.is_some() {
            // a picture of this project (the last screen, first picture selected), for testing
            let f = file.clone().unwrap_or_default();
            match Project::load(&f) {
                Ok(p) => (p, std::env::temp_dir().join("sketchframe-shot.json"), false, "Ready".to_string()),
                Err(e) => (model::sample(), std::env::temp_dir().join("sketchframe-shot.json"), false, e),
            }
        } else if shot.is_some() {
            (model::sample(), std::env::temp_dir().join("sketchframe-shot.json"), false, "Ready".to_string())
        } else if let Some(f) = file {
            match Project::load(&f) {
                Ok(p) => (p, f, true, "Opened".to_string()),
                Err(e) => (model::sample(), model::autosave_path(), false, e),
            }
        } else {
            let p = model::current_file();
            match Project::load(&p) {
                Ok(proj) => {
                    let named = p != model::autosave_path();
                    (proj, p, named, "Welcome back".to_string())
                }
                Err(_) => (model::sample(), model::autosave_path(), false, "A sample project to start from".to_string()),
            }
        };
        let mut app = App {
            proj,
            path,
            named,
            cur: 0,
            sel: vec![],
            zoom: 1.0,
            pan: Vec2::ZERO,
            sketch: true,
            dark_ui: false,
            grid: false,
            snap: true,
            grid_size: 8.0,
            show_frame: true,
            undo: vec![],
            redo: vec![],
            skip_record: false,
            last_change: 0.0,
            drag: Drag::None,
            editing: None,
            clip: vec![],
            clip_images: Default::default(),
            clip_image: None,
            image_pasted: false,
            last_cmd: -10.0,
            bg_tool: None,
            quick: String::new(),
            focus_quick: false,
            lib_cat: 0,
            lib_drag: None,
            mode: Mode::Edit,
            history: vec![],
            dirty_at: None,
            disk_mtime: None,
            last_poll: 0.0,
            status,
            show_mcp: false,
            show_versions: false,
            show_help: false,
            version_name: String::new(),
            guides: vec![],
            fit_next: true,
            canvas_rect: egui::Rect::NOTHING,
            shot,
            frames: 0,
        };
        if app.shot.is_some() && file_given {
            app.cur = app.proj.screens.len() - 1;
            if let Some(c) = app.proj.screens[app.cur].controls.iter().find(|c| c.kind == "picture") {
                app.sel = vec![c.id];
            }
        } else if app.shot.is_some() {
            // show a phone screen with something selected so the picture shows the editor at work
            app.cur = 1;
            if let Some(c) = app.proj.screens[1].controls.iter().find(|c| c.kind == "card") {
                app.sel = vec![c.id];
            }
            match std::env::var("SKETCHFRAME_SHOT_MODE").as_deref() {
                Ok("flow") => app.mode = Mode::Flow,
                Ok("present") => app.mode = Mode::Present,
                _ => {}
            }
        } else {
            if !app.path.exists() {
                let _ = app.proj.save(&app.path);
            }
            model::remember_current(&app.path);
            app.disk_mtime = mtime(&app.path);
        }
        app
    }

    pub fn screen(&self) -> &model::Screen {
        &self.proj.screens[self.cur]
    }

    pub fn screen_mut(&mut self) -> &mut model::Screen {
        &mut self.proj.screens[self.cur]
    }

    pub fn selected(&self) -> Vec<&Control> {
        self.screen().controls.iter().filter(|c| self.sel.contains(&c.id)).collect()
    }

    pub fn open_screen(&mut self, i: usize) {
        if i < self.proj.screens.len() && i != self.cur {
            self.cur = i;
            self.sel.clear();
            self.editing = None;
            self.fit_next = true;
        }
    }

    // ------------------------------------------------------------ undo

    fn record(&mut self, before: Project, now: f64, ctx: &egui::Context) {
        let gesture = ctx.input(|i| i.pointer.any_down()) || ctx.egui_wants_keyboard_input();
        if !(gesture && now - self.last_change < 0.8) {
            self.undo.push(before);
            if self.undo.len() > 150 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.last_change = now;
        self.dirty_at = Some(now);
    }

    pub fn do_undo(&mut self) {
        if let Some(p) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.proj, p));
            self.after_swap("Undone");
        }
    }

    pub fn do_redo(&mut self) {
        if let Some(p) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.proj, p));
            self.after_swap("Redone");
        }
    }

    fn after_swap(&mut self, msg: &str) {
        self.skip_record = true;
        self.dirty_at = Some(0.0);
        self.cur = self.cur.min(self.proj.screens.len() - 1);
        let ids: Vec<u64> = self.screen().controls.iter().map(|c| c.id).collect();
        self.sel.retain(|id| ids.contains(id));
        self.status = msg.into();
    }

    // ------------------------------------------------------------ files

    fn save_now(&mut self) {
        if self.shot.is_some() {
            return;
        }
        match self.proj.save(&self.path) {
            Ok(()) => self.disk_mtime = mtime(&self.path),
            Err(e) => self.status = e,
        }
    }

    fn poll_disk(&mut self, now: f64) {
        if self.shot.is_some() || now - self.last_poll < 0.7 {
            return;
        }
        self.last_poll = now;
        let m = mtime(&self.path);
        if m.is_some() && m != self.disk_mtime && self.dirty_at.is_none() && !matches!(self.drag, Drag::Move { .. } | Drag::Resize { .. }) {
            if let Ok(p) = Project::load(&self.path) {
                if p != self.proj {
                    let old = std::mem::replace(&mut self.proj, p);
                    self.undo.push(old);
                    self.redo.clear();
                    self.skip_record = true;
                    self.cur = self.cur.min(self.proj.screens.len() - 1);
                    let ids: Vec<u64> = self.screen().controls.iter().map(|c| c.id).collect();
                    self.sel.retain(|id| ids.contains(id));
                    self.status = "Updated from outside (AI assistant) — Undo brings back the previous version".into();
                }
            }
            self.disk_mtime = m;
        }
    }

    pub fn new_project(&mut self) {
        self.proj = Project::empty();
        self.path = model::autosave_path();
        self.named = false;
        self.cur = 0;
        self.sel.clear();
        self.fit_next = true;
        model::remember_current(&self.path);
        self.save_now();
        self.status = "New project".into();
    }

    pub fn open_dialog(&mut self) {
        if let Some(f) = rfd::FileDialog::new().add_filter("Sketchframe project", &["json"]).pick_file() {
            match Project::load(&f) {
                Ok(p) => {
                    self.proj = p;
                    self.path = f;
                    self.named = true;
                    self.cur = 0;
                    self.sel.clear();
                    self.fit_next = true;
                    self.skip_record = true;
                    model::remember_current(&self.path);
                    self.disk_mtime = mtime(&self.path);
                    self.status = format!("Opened {}", self.path.display());
                }
                Err(e) => self.status = e,
            }
        }
    }

    pub fn save_as_dialog(&mut self) {
        let name = format!("{}.sketch.json", self.proj.name.replace(['/', '\\'], "-"));
        if let Some(f) = rfd::FileDialog::new().add_filter("Sketchframe project", &["json"]).set_file_name(name).save_file() {
            self.path = f;
            self.named = true;
            model::remember_current(&self.path);
            self.save_now();
            self.status = format!("Saved to {}", self.path.display());
        }
    }

    fn export(&mut self, what: &str) {
        let base = self.screen().name.replace(['/', '\\'], "-");
        let result: Result<String, String> = match what {
            "png" | "svg" => {
                let Some(f) = rfd::FileDialog::new().set_file_name(format!("{base}.{what}")).save_file() else { return };
                let data = if what == "png" {
                    render::screen_png(&self.proj, self.cur, self.sketch, self.show_frame, 2.0)
                } else {
                    Ok(render::screen_svg(&self.proj, self.cur, self.sketch, self.show_frame).into_bytes())
                };
                data.and_then(|d| std::fs::write(&f, d).map_err(|e| e.to_string())).map(|_| f.display().to_string())
            }
            "html" => {
                let Some(f) = rfd::FileDialog::new().set_file_name(format!("{}-prototype.html", self.proj.name)).save_file() else { return };
                std::fs::write(&f, render::prototype_html(&self.proj, self.sketch)).map_err(|e| e.to_string()).map(|_| f.display().to_string())
            }
            "all" => {
                let Some(dir) = rfd::FileDialog::new().pick_folder() else { return };
                let mut n = 0;
                let mut err = None;
                for i in 0..self.proj.screens.len() {
                    let name = format!("{:02}-{}.png", i + 1, self.proj.screens[i].name.replace(['/', '\\'], "-"));
                    match render::screen_png(&self.proj, i, self.sketch, self.show_frame, 2.0) {
                        Ok(d) => {
                            if std::fs::write(dir.join(name), d).is_ok() {
                                n += 1;
                            }
                        }
                        Err(e) => err = Some(e),
                    }
                }
                match err {
                    Some(e) => Err(e),
                    None => Ok(format!("{n} pictures in {}", dir.display())),
                }
            }
            _ => return,
        };
        self.status = match result {
            Ok(s) => format!("Exported {s}"),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    // ------------------------------------------------------------ editing helpers

    pub fn add_control(&mut self, kind: &str, at: Option<Pos2>) {
        let center = at.unwrap_or_else(|| {
            let c = self.canvas_rect.center();
            self.to_model(c)
        });
        let mut c = self.proj.make_control(kind, 0.0, 0.0);
        if kind == "component" {
            c.component = self.proj.screens.iter().find(|s| s.is_component && s.id != self.screen().id).map(|s| s.id.clone());
            if let Some(s) = c.component.as_deref().and_then(|k| self.proj.screen(k)) {
                c.w = s.w;
                c.h = s.h;
            }
        }
        let snap = |v: f32, g: f32| (v / g).round() * g;
        c.x = snap(center.x - c.w / 2.0, self.grid_size);
        c.y = snap(center.y - c.h / 2.0, self.grid_size);
        let id = c.id;
        self.screen_mut().controls.push(c);
        self.sel = vec![id];
        self.status = format!("Added {}", catalog::kind(kind).map(|k| k.name).unwrap_or(kind));
        if kind == "picture" {
            self.choose_picture_for(id);
        }
    }

    pub fn delete_selection(&mut self) {
        let sel = self.sel.clone();
        self.screen_mut().controls.retain(|c| !sel.contains(&c.id) || c.locked);
        self.sel.clear();
        self.editing = None;
    }

    /// Duplicate the selection (without touching the system clipboard).
    pub fn duplicate(&mut self) {
        let items: Vec<Control> = self.selected().into_iter().cloned().collect();
        self.paste_items(items);
    }

    /// Paste: a picture on the clipboard, parts copied from Sketchframe, or text (which becomes a label).
    /// `text` is what the paste event carried; None when pasting from a menu.
    pub fn paste(&mut self, text: Option<String>) {
        if self.try_paste_image() {
            return;
        }
        let mut items = self.clip.clone();
        if let Some(t) = &text {
            if let Ok(p) = serde_json::from_str::<crate::picture::ClipPayload>(t) {
                self.proj.images.extend(p.images);
                items = p.controls;
            } else if let Ok(v) = serde_json::from_str::<Vec<Control>>(t) {
                items = v;
            } else if !t.trim().is_empty() {
                // plain text becomes a label or paragraph
                let kind = if t.contains('\n') || t.len() > 60 { "paragraph" } else { "label" };
                self.add_control(kind, None);
                let s = t.trim().to_string();
                if let Some(id) = self.sel.first().copied() {
                    if let Some(c) = self.screen_mut().controls.iter_mut().find(|c| c.id == id) {
                        c.text = s;
                    }
                }
                return;
            }
        } else {
            self.proj.images.extend(self.clip_images.clone());
        }
        self.paste_items(items);
    }

    pub fn paste_items(&mut self, items: Vec<Control>) {
        if items.is_empty() {
            return;
        }
        let mut groups = std::collections::HashMap::new();
        let mut new_ids = vec![];
        for mut c in items {
            c.id = self.proj.new_id();
            c.x += 16.0;
            c.y += 16.0;
            c.locked = false;
            if let Some(g) = c.group {
                let ng = *groups.entry(g).or_insert_with(|| self.proj.new_id());
                c.group = Some(ng);
            }
            new_ids.push(c.id);
            self.screen_mut().controls.push(c);
        }
        self.clip = self.selected_after(&new_ids);
        self.sel = new_ids;
    }

    fn selected_after(&self, ids: &[u64]) -> Vec<Control> {
        self.screen().controls.iter().filter(|c| ids.contains(&c.id)).cloned().collect()
    }

    pub fn group(&mut self) {
        if self.sel.len() < 2 {
            return;
        }
        let g = self.proj.new_id();
        let sel = self.sel.clone();
        for c in self.screen_mut().controls.iter_mut().filter(|c| sel.contains(&c.id)) {
            c.group = Some(g);
        }
        self.status = "Grouped".into();
    }

    pub fn ungroup(&mut self) {
        let sel = self.sel.clone();
        for c in self.screen_mut().controls.iter_mut().filter(|c| sel.contains(&c.id)) {
            c.group = None;
        }
        self.status = "Ungrouped".into();
    }

    pub fn toggle_lock(&mut self) {
        let sel = self.sel.clone();
        let lock = !self.selected().iter().all(|c| c.locked);
        for c in self.screen_mut().controls.iter_mut().filter(|c| sel.contains(&c.id)) {
            c.locked = lock;
        }
        self.status = if lock { "Locked".into() } else { "Unlocked".into() };
    }

    /// Moves the selection in the drawing order: +1 forward, -1 backward, 2 front, -2 back.
    pub fn arrange(&mut self, how: i32) {
        let sel = self.sel.clone();
        let cs = &mut self.screen_mut().controls;
        match how {
            2 | -2 => {
                let (mut picked, mut rest): (Vec<Control>, Vec<Control>) = cs.drain(..).partition(|c| sel.contains(&c.id));
                if how == 2 {
                    rest.append(&mut picked);
                    *cs = rest;
                } else {
                    picked.append(&mut rest);
                    *cs = picked;
                }
            }
            1 => {
                for i in (0..cs.len().saturating_sub(1)).rev() {
                    if sel.contains(&cs[i].id) && !sel.contains(&cs[i + 1].id) {
                        cs.swap(i, i + 1);
                    }
                }
            }
            _ => {
                for i in 1..cs.len() {
                    if sel.contains(&cs[i].id) && !sel.contains(&cs[i - 1].id) {
                        cs.swap(i, i - 1);
                    }
                }
            }
        }
    }

    /// Select every control in the same group as `id`.
    pub fn with_group(&self, id: u64) -> Vec<u64> {
        let g = self.screen().controls.iter().find(|c| c.id == id).and_then(|c| c.group);
        match g {
            Some(g) => self.screen().controls.iter().filter(|c| c.group == Some(g)).map(|c| c.id).collect(),
            None => vec![id],
        }
    }

    pub fn duplicate_screen(&mut self, i: usize) {
        let mut s = self.proj.screens[i].clone();
        let mut n = self.proj.screens.len() + 1;
        while self.proj.screens.iter().any(|x| x.id == format!("s{n}")) {
            n += 1;
        }
        s.id = format!("s{n}");
        s.name = format!("{} copy", s.name);
        for c in s.controls.iter_mut() {
            c.id = self.proj.new_id();
        }
        self.proj.screens.insert(i + 1, s);
        self.open_screen(i + 1);
    }

    pub fn delete_screen(&mut self, i: usize) {
        if self.proj.screens.len() > 1 {
            self.proj.screens.remove(i);
            self.cur = self.cur.min(self.proj.screens.len() - 1);
            self.sel.clear();
            self.fit_next = true;
        }
    }

    // ------------------------------------------------------------ keys

    fn keys(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let events = ctx.input(|i| i.events.clone());
        for e in &events {
            match e {
                egui::Event::Copy if !typing && self.mode == Mode::Edit => self.copy(ctx),
                egui::Event::Cut if !typing && self.mode == Mode::Edit => {
                    self.copy(ctx);
                    self.delete_selection();
                }
                egui::Event::Paste(t) if !typing && self.mode == Mode::Edit => self.paste(Some(t.clone())),
                _ => {}
            }
        }
        self.watch_paste_key(ctx, typing);
        let cmd = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, k));
        let cmd_shift = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, k));
        let plain = |k: Key| !typing && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        if cmd_shift(Key::Z) || cmd(Key::Y) {
            if !typing {
                self.do_redo();
            }
        }
        if cmd(Key::Z) && !typing {
            self.do_undo();
        }
        if cmd(Key::S) {
            if self.named {
                self.save_now();
                self.status = format!("Saved {}", self.path.display());
            } else {
                self.save_as_dialog();
            }
        }
        if cmd(Key::O) {
            self.open_dialog();
        }
        if cmd(Key::K) {
            self.focus_quick = true;
        }
        if cmd(Key::Enter) || plain(Key::F5) {
            self.mode = if self.mode == Mode::Present { Mode::Edit } else { Mode::Present };
            self.history.clear();
        }
        if self.mode == Mode::Present {
            if plain(Key::Escape) {
                self.mode = Mode::Edit;
            }
            if plain(Key::ArrowLeft) || plain(Key::Backspace) {
                if let Some(p) = self.history.pop() {
                    self.cur = p;
                }
            }
            if plain(Key::ArrowRight) {
                let next = (self.cur + 1..self.proj.screens.len()).find(|i| !self.proj.screens[*i].is_component);
                if let Some(n) = next {
                    self.history.push(self.cur);
                    self.cur = n;
                }
            }
            return;
        }
        if self.mode == Mode::Flow {
            if plain(Key::Escape) {
                self.mode = Mode::Edit;
            }
            return;
        }
        if typing {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.editing = None;
                ctx.memory_mut(|m| m.stop_text_input());
            }
            return;
        }
        if cmd(Key::A) {
            self.sel = self.screen().controls.iter().filter(|c| !c.locked && !c.hidden).map(|c| c.id).collect();
        }
        if cmd(Key::D) {
            self.duplicate();
        }
        if cmd_shift(Key::G) {
            self.ungroup();
        } else if cmd(Key::G) {
            self.group();
        }
        if cmd(Key::L) {
            self.toggle_lock();
        }
        if cmd(Key::Equals) || cmd(Key::Plus) {
            self.zoom_by(1.25, None);
        }
        if cmd(Key::Minus) {
            self.zoom_by(0.8, None);
        }
        if cmd(Key::Num0) {
            self.fit_next = true;
        }
        if cmd(Key::Num1) {
            self.zoom_by(1.0 / self.zoom, None);
        }
        if plain(Key::Delete) || plain(Key::Backspace) {
            self.delete_selection();
        }
        if plain(Key::Escape) {
            self.sel.clear();
            self.editing = None;
        }
        if plain(Key::Slash) {
            self.focus_quick = true;
        }
        if plain(Key::CloseBracket) {
            self.arrange(1);
        }
        if plain(Key::OpenBracket) {
            self.arrange(-1);
        }
        if plain(Key::Enter) || plain(Key::F2) {
            if self.sel.len() == 1 {
                self.editing = Some(self.sel[0]);
            }
        }
        let step = if ctx.input(|i| i.modifiers.shift) { 10.0 } else { 1.0 };
        let mut d = Vec2::ZERO;
        for (k, v) in [(Key::ArrowLeft, Vec2::new(-step, 0.0)), (Key::ArrowRight, Vec2::new(step, 0.0)), (Key::ArrowUp, Vec2::new(0.0, -step)), (Key::ArrowDown, Vec2::new(0.0, step))] {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k) || i.consume_key(Modifiers::SHIFT, k)) {
                d += v;
            }
        }
        if d != Vec2::ZERO {
            let sel = self.sel.clone();
            for c in self.screen_mut().controls.iter_mut().filter(|c| sel.contains(&c.id) && !c.locked) {
                c.x += d.x;
                c.y += d.y;
            }
        }
    }

    // ------------------------------------------------------------ screenshot for checks

    fn screenshot_hook(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot.clone() else { return };
        self.frames += 1;
        ctx.request_repaint();
        if self.frames == 120 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(file) = std::fs::File::create(&path) {
                let mut enc = png::Encoder::new(file, image.width() as u32, image.height() as u32);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                if let Ok(mut w) = enc.write_header() {
                    let _ = w.write_image_data(image.as_raw());
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.frames > 400 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    // ------------------------------------------------------------ panels

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New project").clicked() {
                    self.new_project();
                }
                if ui.button("Open…  ⌘O").clicked() {
                    self.open_dialog();
                }
                if ui.button("Insert picture…").on_hover_text("PNG, JPEG, GIF, WebP, BMP, ICO, TIFF or SVG. You can also paste or drop pictures on the screen.").clicked() {
                    self.insert_pictures_dialog();
                }
                if ui.button("Save as…  ⌘S").clicked() {
                    self.save_as_dialog();
                }
                ui.separator();
                if ui.button("Export screen as PNG…").clicked() {
                    self.export("png");
                }
                if ui.button("Export screen as SVG…").clicked() {
                    self.export("svg");
                }
                if ui.button("Export all screens as PNG…").clicked() {
                    self.export("all");
                }
                if ui.button("Export clickable prototype (HTML)…").clicked() {
                    self.export("html");
                }
                ui.separator();
                if ui.button("Version history…").clicked() {
                    self.show_versions = true;
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo  ⌘Z")).clicked() {
                    self.do_undo();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo  ⇧⌘Z")).clicked() {
                    self.do_redo();
                }
                ui.separator();
                let ctx = ui.ctx().clone();
                if ui.add_enabled(!self.sel.is_empty(), egui::Button::new("Copy  ⌘C")).clicked() {
                    self.copy(&ctx);
                }
                if ui.button("Paste  ⌘V").clicked() {
                    self.paste(None);
                }
                if ui.button("Duplicate  ⌘D").clicked() {
                    self.duplicate();
                }
                if ui.button("Delete  ⌫").clicked() {
                    self.delete_selection();
                }
                if ui.button("Select all  ⌘A").clicked() {
                    self.sel = self.screen().controls.iter().filter(|c| !c.locked).map(|c| c.id).collect();
                }
                ui.separator();
                if ui.button("Group  ⌘G").clicked() {
                    self.group();
                }
                if ui.button("Ungroup  ⇧⌘G").clicked() {
                    self.ungroup();
                }
                if ui.button("Lock / unlock  ⌘L").clicked() {
                    self.toggle_lock();
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.sketch, "Hand-drawn look");
                ui.checkbox(&mut self.show_frame, "Device frames");
                ui.checkbox(&mut self.grid, "Grid");
                ui.checkbox(&mut self.snap, "Snap to grid and guides");
                ui.add(egui::Slider::new(&mut self.grid_size, 4.0..=40.0).step_by(1.0).text("Grid size"));
                ui.checkbox(&mut self.dark_ui, "Dark app theme");
                ui.separator();
                if ui.button("Fit screen  ⌘0").clicked() {
                    self.fit_next = true;
                }
                if ui.button("Actual size  ⌘1").clicked() {
                    self.zoom_by(1.0 / self.zoom, None);
                }
                if ui.button("Flow map (all screens and links)").clicked() {
                    self.mode = Mode::Flow;
                }
                if ui.button("Present  ⌘↩").clicked() {
                    self.mode = Mode::Present;
                    self.history.clear();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Connect an AI assistant (MCP)…").clicked() {
                    self.show_mcp = true;
                }
                if ui.button("Keyboard shortcuts and text tricks…").clicked() {
                    self.show_help = true;
                }
            });
            ui.separator();
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo")).on_hover_text("Undo").clicked() {
                self.do_undo();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo")).on_hover_text("Redo").clicked() {
                self.do_redo();
            }
            ui.separator();
            self.quick_add(ui);
            ui.separator();
            for (m, label) in [(Mode::Edit, "✏ Edit"), (Mode::Flow, "🗺 Flow"), (Mode::Present, "▶ Present")] {
                if ui.selectable_label(self.mode == m, label).clicked() {
                    self.mode = m;
                    self.history.clear();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("AI assistant").on_hover_text("Let an AI assistant design screens with you").clicked() {
                    self.show_mcp = true;
                }
                let look = if self.sketch { "Look: Sketch" } else { "Look: Clean" };
                if ui.button(look).on_hover_text("Switch between the hand-drawn and the clean look").clicked() {
                    self.sketch = !self.sketch;
                }
                if ui.button(format!("{:.0}%", self.zoom * 100.0)).on_hover_text("Fit the screen in the window").clicked() {
                    self.fit_next = true;
                }
            });
        });
    }

    fn quick_add(&mut self, ui: &mut egui::Ui) {
        let id = egui::Id::new("quick_add");
        let resp = ui.add(egui::TextEdit::singleline(&mut self.quick).id(id).hint_text("Quick add…  ⌘K").desired_width(190.0));
        if self.focus_quick {
            resp.request_focus();
            self.focus_quick = false;
        }
        let q = self.quick.trim().to_lowercase();
        if q.is_empty() {
            return;
        }
        let hits: Vec<&catalog::Kind> = catalog::KINDS
            .iter()
            .filter(|k| k.name.to_lowercase().contains(&q) || k.id.contains(&q) || k.cat.to_lowercase().contains(&q))
            .take(10)
            .collect();
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if enter {
            if let Some(k) = hits.first() {
                let kid = k.id;
                self.add_control(kid, None);
            }
            self.quick.clear();
            return;
        }
        let mut pick = None;
        egui::Area::new(id.with("pop")).order(egui::Order::Foreground).fixed_pos(resp.rect.left_bottom() + Vec2::new(0.0, 4.0)).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(220.0);
                if hits.is_empty() {
                    ui.weak("Nothing matches");
                }
                for (i, k) in hits.iter().enumerate() {
                    let label = if i == 0 { format!("{}  ↩", k.name) } else { k.name.to_string() };
                    if ui.selectable_label(i == 0, label).on_hover_text(k.cat).clicked() {
                        pick = Some(k.id);
                    }
                }
            });
        });
        if let Some(k) = pick {
            self.add_control(k, None);
            self.quick.clear();
        }
    }

    fn library(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Parts").strong());
            for (i, c) in catalog::CATS.iter().enumerate() {
                if ui.selectable_label(self.lib_cat == i, *c).clicked() {
                    self.lib_cat = i;
                }
            }
        });
        let cat = catalog::CATS[self.lib_cat];
        let kinds: Vec<&catalog::Kind> = catalog::KINDS.iter().filter(|k| k.cat == cat).collect();
        egui::ScrollArea::horizontal().id_salt("lib_scroll").show(ui, |ui| {
            ui.horizontal(|ui| {
                for k in kinds {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(92.0, 78.0), Sense::click_and_drag());
                    let hovered = resp.hovered();
                    let bg = if hovered { ui.visuals().widgets.hovered.bg_fill } else { ui.visuals().extreme_bg_color };
                    ui.painter().rect_filled(rect, 6.0, bg);
                    let ctx = ui.ctx().clone();
                    let m = render::egui_measure(&ctx, self.sketch);
                    let mut pen = Pen::new(self.sketch, false, &m);
                    let area = R::new(rect.min.x + 6.0, rect.min.y + 6.0, rect.width() - 12.0, rect.height() - 26.0);
                    draw::preview(&mut pen, &self.proj, k.id, area);
                    render::paint(ui.painter(), &pen.out, Pos2::ZERO, 1.0, self.sketch);
                    ui.painter().text(
                        Pos2::new(rect.center().x, rect.max.y - 9.0),
                        egui::Align2::CENTER_CENTER,
                        k.name,
                        egui::FontId::proportional(10.5),
                        ui.visuals().text_color(),
                    );
                    let resp = resp.on_hover_text(format!("{}\n{}", k.name, if k.help.is_empty() { "Click to add, or drag onto the screen" } else { k.help }));
                    if resp.drag_started() {
                        self.lib_drag = Some(k.id.to_string());
                    }
                    if resp.clicked() {
                        self.add_control(k.id, None);
                    }
                }
            });
        });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Screens");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("+", |ui| {
                    ui.label(RichText::new("New screen for…").weak());
                    for d in catalog::DEVICES {
                        if ui.button(d.name).clicked() {
                            let n = self.proj.screens.iter().filter(|s| !s.is_component).count() + 1;
                            self.proj.add_screen(&format!("Screen {n}"), d.id);
                            let i = self.proj.screens.len() - 1;
                            self.open_screen(i);
                        }
                    }
                })
                .response
                .on_hover_text("Add a screen");
            });
        });
        let mut open = None;
        let mut action: Option<(&str, usize)> = None;
        egui::ScrollArea::vertical().id_salt("screens").max_height(ui.available_height() * 0.5).show(ui, |ui| {
            for comp in [false, true] {
                if comp {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Components").strong()).on_hover_text("Reusable pieces: edit one and every copy updates");
                        if ui.small_button("+").on_hover_text("New component").clicked() {
                            let n = self.proj.screens.iter().filter(|s| s.is_component).count() + 1;
                            self.proj.add_screen(&format!("Component {n}"), "none");
                            let s = self.proj.screens.last_mut().unwrap();
                            s.is_component = true;
                            s.w = 360.0;
                            s.h = 120.0;
                            open = Some(self.proj.screens.len() - 1);
                        }
                    });
                }
                for i in 0..self.proj.screens.len() {
                    if self.proj.screens[i].is_component != comp {
                        continue;
                    }
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::click());
                    let on = i == self.cur;
                    let v = ui.visuals();
                    let bg = if on { v.selection.bg_fill.gamma_multiply(0.45) } else if resp.hovered() { v.widgets.hovered.bg_fill } else { Color32::TRANSPARENT };
                    ui.painter().rect_filled(rect, 6.0, bg);
                    let th = egui::Rect::from_min_size(rect.min + Vec2::new(6.0, 4.0), Vec2::new(54.0, 44.0));
                    ui.painter().rect_filled(th, 3.0, Color32::from_gray(236));
                    let ctx = ui.ctx().clone();
                    let m = render::egui_measure(&ctx, self.sketch);
                    let mut pen = Pen::new(self.sketch, false, &m);
                    draw::thumbnail(&mut pen, &self.proj, i, R::new(th.min.x + 2.0, th.min.y + 2.0, th.width() - 4.0, th.height() - 4.0));
                    render::paint(ui.painter(), &pen.out, Pos2::ZERO, 1.0, self.sketch);
                    let s = &self.proj.screens[i];
                    let txt = v.text_color();
                    ui.painter().text(Pos2::new(th.max.x + 8.0, rect.min.y + 18.0), egui::Align2::LEFT_CENTER, &s.name, egui::FontId::proportional(13.5), txt);
                    let dev = catalog::device(&s.device).map(|d| d.name.split(" (").next().unwrap_or(d.name)).unwrap_or("Custom");
                    ui.painter().text(
                        Pos2::new(th.max.x + 8.0, rect.min.y + 36.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{dev} · {}", s.controls.len()),
                        egui::FontId::proportional(11.0),
                        v.weak_text_color(),
                    );
                    if resp.clicked() {
                        open = Some(i);
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Open").clicked() {
                            action = Some(("open", i));
                        }
                        if ui.button("Duplicate").clicked() {
                            action = Some(("dup", i));
                        }
                        if ui.button("Move up").clicked() {
                            action = Some(("up", i));
                        }
                        if ui.button("Move down").clicked() {
                            action = Some(("down", i));
                        }
                        if ui.button(if comp { "Make it a normal screen" } else { "Make it a component" }).clicked() {
                            action = Some(("comp", i));
                        }
                        if ui.add_enabled(self.proj.screens.len() > 1, egui::Button::new("Delete")).clicked() {
                            action = Some(("del", i));
                        }
                    });
                }
            }
        });
        if let Some(i) = open {
            self.open_screen(i);
        }
        match action {
            Some(("open", i)) => self.open_screen(i),
            Some(("dup", i)) => self.duplicate_screen(i),
            Some(("del", i)) => self.delete_screen(i),
            Some(("up", i)) if i > 0 => {
                self.proj.screens.swap(i, i - 1);
                if self.cur == i {
                    self.cur -= 1;
                } else if self.cur == i - 1 {
                    self.cur += 1;
                }
            }
            Some(("down", i)) if i + 1 < self.proj.screens.len() => {
                self.proj.screens.swap(i, i + 1);
                if self.cur == i {
                    self.cur += 1;
                } else if self.cur == i + 1 {
                    self.cur -= 1;
                }
            }
            Some(("comp", i)) => {
                let s = &mut self.proj.screens[i];
                s.is_component = !s.is_component;
            }
            _ => {}
        }
        ui.separator();
        ui.heading("Layers");
        egui::ScrollArea::vertical().id_salt("layers").show(ui, |ui| {
            let n = self.screen().controls.len();
            if n == 0 {
                ui.weak("Nothing here yet. Click a part above, or drag it onto the screen.");
            }
            let mut click: Option<(u64, bool)> = None;
            for i in (0..n).rev() {
                let c = self.screen().controls[i].clone();
                let on = self.sel.contains(&c.id);
                ui.horizontal(|ui| {
                    let mut hidden = c.hidden;
                    if ui.small_button(if hidden { "–" } else { "👁" }).on_hover_text("Show / hide").clicked() {
                        hidden = !hidden;
                        self.screen_mut().controls[i].hidden = hidden;
                    }
                    if ui.small_button(if c.locked { "🔒" } else { "🔓" }).on_hover_text("Lock / unlock").clicked() {
                        self.screen_mut().controls[i].locked = !c.locked;
                    }
                    let mut name = c.layer_name();
                    if c.group.is_some() {
                        name = format!("Group · {name}");
                    }
                    if c.link.is_some() {
                        name.push_str("  🔗");
                    }
                    let label = if c.hidden { RichText::new(name).weak() } else { RichText::new(name) };
                    if ui.selectable_label(on, label).clicked() {
                        click = Some((c.id, ui.input(|i| i.modifiers.shift || i.modifiers.command)));
                    }
                });
            }
            if let Some((id, add)) = click {
                if add {
                    if let Some(p) = self.sel.iter().position(|x| *x == id) {
                        self.sel.remove(p);
                    } else {
                        self.sel.push(id);
                    }
                } else {
                    self.sel = vec![id];
                }
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(&self.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let file = if self.named { self.path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default() } else { "Autosaved".into() };
                ui.weak(format!("Saved: {file}")).on_hover_text(self.path.display().to_string());
                ui.separator();
                let s = self.screen();
                ui.weak(format!("{} · {:.0}×{:.0}", s.name, s.w, s.h));
                if let [c] = self.selected().as_slice() {
                    ui.separator();
                    ui.weak(format!("x {:.0}  y {:.0}  w {:.0}  h {:.0}", c.x, c.y, c.w, c.h));
                } else if self.sel.len() > 1 {
                    ui.separator();
                    ui.weak(format!("{} selected", self.sel.len()));
                }
            });
        });
    }

    fn windows(&mut self, ctx: &egui::Context) {
        if self.show_mcp {
            let mut open = true;
            egui::Window::new("Connect an AI assistant").open(&mut open).default_width(560.0).show(ctx, |ui| {
                ui.label("This app has a built-in MCP server. AI assistants that support MCP (Claude Desktop, Claude Code, editors with AI) can then create screens, place parts, link screens together and look at the result. You see every change here as it happens, and Undo takes it back.");
                ui.add_space(6.0);
                let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "sketchframe".into());
                let cfg = format!(
                    "{{\n  \"mcpServers\": {{\n    \"sketchframe\": {{\n      \"command\": \"{}\",\n      \"args\": [\"--mcp\"]\n    }}\n  }}\n}}",
                    exe.replace('\\', "\\\\")
                );
                ui.label(RichText::new("1. Add this to the assistant's MCP settings:").strong());
                let mut c = cfg.clone();
                ui.add(egui::TextEdit::multiline(&mut c).code_editor().desired_width(f32::INFINITY));
                if ui.button("Copy settings").clicked() {
                    ui.ctx().copy_text(cfg);
                    self.status = "MCP settings copied".into();
                }
                let cli = format!("claude mcp add sketchframe -- \"{exe}\" --mcp");
                ui.label(RichText::new("Or for Claude Code, run in a terminal:").strong());
                let mut c2 = cli.clone();
                ui.add(egui::TextEdit::singleline(&mut c2).code_editor().desired_width(f32::INFINITY));
                if ui.button("Copy command").clicked() {
                    ui.ctx().copy_text(cli);
                }
                ui.add_space(6.0);
                ui.label(RichText::new("2. Ask it things like:").strong());
                ui.label("• \"Design a sign-up flow for a running app: welcome, sign up, pick goals, home.\"\n• \"Add a settings screen and link the gear icon to it.\"\n• \"Make a dark version of the home screen for a watch.\"");
                ui.add_space(6.0);
                ui.weak(format!("The assistant edits the project you have open: {}", self.path.display()));
            });
            self.show_mcp = open;
        }
        if self.show_help {
            let mut open = true;
            egui::Window::new("Shortcuts and text tricks").open(&mut open).default_width(520.0).show(ctx, |ui| {
                egui::Grid::new("keys").striped(true).show(ui, |ui| {
                    for (k, v) in [
                        ("⌘K or /", "Quick add a part by name"),
                        ("Double-click / Enter", "Edit a part's text in place"),
                        ("Drag empty space", "Select several parts"),
                        ("Shift-click", "Add to the selection"),
                        ("Arrows (Shift = 10)", "Nudge"),
                        ("⌘D", "Duplicate"),
                        ("⌘C ⌘X ⌘V", "Copy, cut, paste (pasting text makes a label, pasting a picture adds it)"),
                        ("⌘G / ⇧⌘G", "Group / ungroup"),
                        ("⌘L", "Lock"),
                        ("[ and ]", "Send backward / bring forward"),
                        ("⌘Z / ⇧⌘Z", "Undo / redo"),
                        ("Scroll, middle-drag, Space-drag", "Move around"),
                        ("⌘ + scroll or pinch", "Zoom"),
                        ("⌘0 / ⌘1", "Fit / actual size"),
                        ("⌘Enter or F5", "Present (click links, ← back, Esc to stop)"),
                    ] {
                        ui.label(RichText::new(k).strong());
                        ui.label(v);
                        ui.end_row();
                    }
                });
                ui.separator();
                ui.label(RichText::new("Text tricks").strong());
                ui.label("Lists, tabs, menus and tables read their text:\n• one item per line (or commas for tabs, chips, nav bars)\n• * marks the active item, - marks a disabled one\n• :home Home puts the home icon before the word\n• tables: rows on lines, cells split by commas\n• a tab character adds a right-hand detail (shortcuts)");
            });
            self.show_help = open;
        }
        if self.show_versions {
            let mut open = true;
            let mut restore = None;
            let mut delete = None;
            egui::Window::new("Version history").open(&mut open).default_width(380.0).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.version_name).hint_text("Name this version").desired_width(200.0));
                    if ui.button("Save version").clicked() {
                        let name = if self.version_name.trim().is_empty() { format!("Version {}", self.proj.versions.len() + 1) } else { self.version_name.trim().to_string() };
                        let screens = self.proj.screens.clone();
                        self.proj.versions.push(model::Snapshot { name, time: model::now_secs(), screens });
                        self.version_name.clear();
                    }
                });
                ui.separator();
                if self.proj.versions.is_empty() {
                    ui.weak("No saved versions yet.");
                }
                for (i, v) in self.proj.versions.iter().enumerate().rev() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}  ·  {} screens  ·  {}", v.name, v.screens.len(), ago(v.time)));
                        if ui.small_button("Restore").clicked() {
                            restore = Some(i);
                        }
                        if ui.small_button("Delete").clicked() {
                            delete = Some(i);
                        }
                    });
                }
            });
            if let Some(i) = restore {
                self.proj.screens = self.proj.versions[i].screens.clone();
                self.cur = 0;
                self.sel.clear();
                self.fit_next = true;
                self.status = "Version restored — Undo to go back".into();
            }
            if let Some(i) = delete {
                self.proj.versions.remove(i);
            }
            self.show_versions = open;
        }
    }
}

fn ago(t: u64) -> String {
    let d = model::now_secs().saturating_sub(t);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86399 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86400),
    }
}

pub fn mtime(p: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        ctx.set_visuals(if self.dark_ui { egui::Visuals::dark() } else { egui::Visuals::light() });
        let before = self.proj.clone();
        self.screenshot_hook(&ctx);
        self.poll_disk(now);
        self.keys(&ctx);

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        match self.mode {
            Mode::Edit => {
                egui::Panel::top("library").show(ui, |ui| self.library(ui));
                egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
                egui::Panel::left("screens").default_size(236.0).resizable(true).show(ui, |ui| self.left_panel(ui));
                egui::Panel::right("inspector").default_size(286.0).resizable(true).show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
                });
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.canvas(ui));
            }
            Mode::Present => {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.present(ui));
            }
            Mode::Flow => {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.flow(ui));
            }
        }
        self.handle_drops(&ctx);
        self.bg_tool_window(&ctx);
        self.windows(&ctx);

        if self.cur >= self.proj.screens.len() {
            self.cur = self.proj.screens.len() - 1;
        }
        if self.skip_record {
            self.skip_record = false;
        } else if self.proj != before {
            self.record(before, now, &ctx);
        }
        if let Some(t) = self.dirty_at {
            if now - t > 0.5 && !ctx.input(|i| i.pointer.any_down()) {
                self.dirty_at = None;
                self.save_now();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(900));
    }
}
