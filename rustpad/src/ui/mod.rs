//! Top-level UI composition and the eframe `App` implementation.

pub mod dialogs;
pub mod menus;
pub mod panels;

use eframe::egui::{self, Key};

use crate::app::App;
use crate::cmd::{self, Cmd};
use crate::editor::{self, Params};

impl App {
    pub fn select_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.active = i;
            self.want_editor_focus = true;
            self.tab_scroll_to_active = true;
        }
    }

    /// Commands that must not steal Ctrl+key from a focused text field.
    fn text_field_conflict(c: &Cmd) -> bool {
        matches!(
            c,
            Cmd::SelectAll | Cmd::Undo | Cmd::Redo | Cmd::DuplicateLine | Cmd::CutLine | Cmd::Unindent | Cmd::Case(_) | Cmd::TransposeLine | Cmd::DeleteLine | Cmd::JoinLines | Cmd::SplitLines
        )
    }

    fn collect_shortcuts(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        let editor_id = self.ed.id;
        let focused = ctx.memory(|m| m.focused());
        let text_focus = focused.is_some() && focused != Some(editor_id);
        let modal = self.pending.is_some();
        if modal {
            return;
        }
        ctx.input_mut(|i| {
            cmd::with_shortcuts(|list| {
                for s in list {
                    if text_focus && Self::text_field_conflict(&s.cmd) {
                        continue;
                    }
                    if i.consume_key(s.mods, s.key) {
                        cmds.push(s.cmd.clone());
                    }
                }
            });
        });
    }

    fn draw_editor_panes(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        let theme_idx = self.themes.iter().position(|t| t.name == self.settings.theme).unwrap_or(0);
        let time = self.time;
        if !self.split {
            let i = self.active.min(self.tabs.len() - 1);
            if self.tabs[i].html_view {
                self.webview_rect = Some(ui.available_rect_before_wrap());
                ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::hover());
                return;
            }
            let theme = &self.themes[theme_idx];
            let p = Params { theme, set: &self.settings, time };
            let out = editor::show(ui, &mut self.tabs[i], &mut self.ed, &p);
            cmds.extend(out);
            return;
        }
        let full = ui.available_rect_before_wrap();
        let (r_a, r_b) = if self.split_vertical {
            let mid = full.center().x;
            (egui::Rect::from_min_max(full.min, egui::pos2(mid - 1.0, full.max.y)), egui::Rect::from_min_max(egui::pos2(mid + 1.0, full.min.y), full.max))
        } else {
            let mid = full.center().y;
            (egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, mid - 1.0)), egui::Rect::from_min_max(egui::pos2(full.min.x, mid + 1.0), full.max))
        };
        let other_rect = if self.pane_swapped { r_a } else { r_b };
        // which pane did the user click?
        if ui.input(|i| i.pointer.primary_pressed()) {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                if other_rect.contains(p) && self.view2 < self.tabs.len() {
                    // swap roles
                    std::mem::swap(&mut self.active, &mut self.view2);
                    std::mem::swap(&mut self.ed, &mut self.ed2);
                    self.pane_swapped = !self.pane_swapped;
                    self.ed.focus_request = true;
                }
            }
        }
        let theme = &self.themes[theme_idx];
        let active = self.active.min(self.tabs.len() - 1);
        let v2 = self.view2.min(self.tabs.len() - 1);
        let (rect_main, rect_other) = if self.pane_swapped { (r_b, r_a) } else { (r_a, r_b) };
        {
            let mut ui1 = ui.new_child(egui::UiBuilder::new().max_rect(rect_main));
            let p = Params { theme, set: &self.settings, time };
            let out = editor::show(&mut ui1, &mut self.tabs[active], &mut self.ed, &p);
            cmds.extend(out);
        }
        {
            let mut ui2 = ui.new_child(egui::UiBuilder::new().max_rect(rect_other));
            let p = Params { theme, set: &self.settings, time };
            // the unfocused pane must not consume keyboard input
            let _ = editor::show(&mut ui2, &mut self.tabs[v2], &mut self.ed2, &p);
        }
        let sep = if self.split_vertical {
            egui::Rect::from_min_max(egui::pos2(r_a.max.x, full.min.y), egui::pos2(r_b.min.x, full.max.y))
        } else {
            egui::Rect::from_min_max(egui::pos2(full.min.x, r_a.max.y), egui::pos2(full.max.x, r_b.min.y))
        };
        ui.painter().rect_filled(sep, 0.0, theme.guide.c());
        ui.allocate_rect(full, egui::Sense::hover());
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let started = std::time::Instant::now();
        self.webview_rect = None;
        #[cfg(target_os = "linux")]
        if self.webview.is_some() {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
        self.ui_inner(ui);
        self.sync_webview(frame, ui.ctx().pixels_per_point());
        if let Some(th) = std::env::var("RP_TRACE").ok().and_then(|v| v.parse::<f64>().ok().or(Some(25.0))) {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            if ms > th {
                eprintln!("[trace] frame {} took {:.1} ms", self.frame_count, ms);
            }
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(s) = self.script.as_mut() {
            s.inject(raw_input);
        }
    }

    fn on_exit(&mut self) {
        self.settings.save();
    }
}

impl App {
    /// Show the web view over the editor area when an HTML view tab is active, hide it otherwise.
    fn sync_webview(&mut self, frame: &eframe::Frame, _ppp: f32) {
        let Some(rect) = self.webview_rect else {
            if let Some(w) = &self.webview {
                let _ = w.set_visible(false);
            }
            return;
        };
        let url = self.tabs[self.active.min(self.tabs.len() - 1)].path.as_ref().map(|p| format!("file://{}", p.display())).unwrap_or_default();
        if self.webview.is_none() || self.webview_url != url {
            self.webview = None;
            #[cfg(target_os = "linux")]
            if gtk::init().is_err() {
                self.set_status("Cannot create web view: GTK init failed".to_string());
                let i = self.active.min(self.tabs.len() - 1);
                self.tabs[i].html_view = false;
                return;
            }
            match wry::WebViewBuilder::new().with_url(&url).build_as_child(frame) {
                Ok(w) => self.webview = Some(w),
                Err(e) => {
                    self.set_status(format!("Cannot create web view: {e}"));
                    #[cfg(target_os = "linux")]
                    if let Some(p) = &self.tabs[self.active.min(self.tabs.len() - 1)].path {
                        crate::app::open_with_default(&p.to_string_lossy());
                    }
                    let i = self.active.min(self.tabs.len() - 1);
                    self.tabs[i].html_view = false;
                    return;
                }
            }
            self.webview_url = url;
        }
        if let Some(w) = &self.webview {
            let bounds = wry::Rect {
                position: wry::dpi::LogicalPosition::new(rect.min.x as f64, rect.min.y as f64).into(),
                size: wry::dpi::LogicalSize::new(rect.width() as f64, rect.height() as f64).into(),
            };
            let _ = w.set_bounds(bounds);
            let _ = w.set_visible(true);
        }
    }

    fn ui_inner(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.time = ctx.input(|i| i.time);

        // window close
        if ctx.input(|i| i.viewport().close_requested()) && !self.should_exit {
            if self.settings.restore_session || !self.tabs.iter().any(|d| d.is_modified()) {
                self.should_exit = true;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                let ids = self.tabs.iter().map(|d| d.id).collect();
                self.close_tabs(ids, true);
            }
        }
        if self.should_exit {
            self.finish_and_close(&ctx);
            return;
        }

        // dropped files
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            if p.is_dir() {
                self.add_workspace_root(&p);
            } else {
                self.open_path(&p);
            }
        }

        // keep doc options in sync with settings
        let opts = crate::doc::EditOpts {
            tab_width: self.settings.tab_width,
            use_spaces: self.settings.use_spaces,
            auto_indent: self.settings.auto_indent,
            auto_close: self.settings.auto_close,
            auto_close_tags: self.settings.auto_close_tags,
        };
        for d in &mut self.tabs {
            d.opts = opts;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }

        let mut cmds: Vec<Cmd> = Vec::new();
        if self.dialogs.map_capture.is_some() {
            // the shortcut mapper is waiting for a key combination: swallow all key input
            let captured = ctx.input_mut(|i| {
                let mut found = None;
                i.events.retain(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => {
                        found = Some((*key, *modifiers));
                        false
                    }
                    egui::Event::Key { .. } | egui::Event::Text(_) => false,
                    _ => true,
                });
                found
            });
            self.dialogs.map_captured = captured;
        } else {
            self.collect_shortcuts(&ctx, &mut cmds);
        }

        let fullscreen_clean = self.post_it;
        if !fullscreen_clean {
            egui::Panel::top("menu").show(ui, |ui| {
                self.menu_bar(ui, &mut cmds);
            });
            if self.settings.tool_bar {
                egui::Panel::top("toolbar").show(ui, |ui| {
                    ui.add_space(2.0);
                    self.tool_bar(ui, &mut cmds);
                    ui.add_space(2.0);
                });
            }
            if self.settings.tab_bar {
                egui::Panel::top("tabs").frame(egui::Frame::NONE).show(ui, |ui| {
                    self.tab_bar(ui, &mut cmds);
                });
            }
            if self.settings.status_bar {
                egui::Panel::bottom("status").show(ui, |ui| {
                    self.status_bar(ui, &mut cmds);
                });
            }
        }

        // incremental search bar
        if self.inc_search.is_some() {
            egui::Panel::bottom("incsearch").show(ui, |ui| {
                let mut close = false;
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("Incremental search:");
                    let text = self.inc_search.as_mut().unwrap();
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(260.0));
                    if self.inc_focus {
                        r.request_focus();
                        self.inc_focus = false;
                    }
                    if r.changed() {
                        changed = true;
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        let shift = ui.input(|i| i.modifiers.shift);
                        let mut q = self.settings.find.clone();
                        q.pattern = self.inc_search.clone().unwrap_or_default();
                        q.mode = crate::search::Mode::Normal;
                        self.search_step(q, shift);
                        r.request_focus();
                    }
                    if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                });
                if changed {
                    let mut q = self.settings.find.clone();
                    q.pattern = self.inc_search.clone().unwrap_or_default();
                    q.mode = crate::search::Mode::Normal;
                    // search from the start of the current selection so typing refines the match
                    let d = self.doc();
                    let s = d.primary_sel().start();
                    d.set_caret(s);
                    self.search_step(q, false);
                }
                if close {
                    self.inc_search = None;
                    self.want_editor_focus = true;
                }
            });
        }

        if self.show_results && !fullscreen_clean {
            egui::Panel::bottom("results").resizable(true).default_size(200.0).min_size(80.0).show(ui, |ui| {
                self.results_panel(ui);
            });
        }
        if self.settings.workspace && !fullscreen_clean {
            egui::Panel::left("workspace").resizable(true).default_size(220.0).min_size(120.0).show(ui, |ui| {
                self.workspace_panel(ui, &mut cmds);
            });
        }
        if self.settings.function_list && !fullscreen_clean {
            egui::Panel::right("funclist").resizable(true).default_size(200.0).min_size(100.0).show(ui, |ui| {
                self.function_list_panel(ui);
            });
        }
        if self.settings.doc_map && !fullscreen_clean {
            egui::Panel::right("docmap").resizable(true).default_size(120.0).min_size(60.0).max_size(300.0).frame(egui::Frame::NONE).show(ui, |ui| {
                self.doc_map(ui);
            });
        }

        let theme_bg = self.theme().bg.c();
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme_bg)).show(ui, |ui| {
            if self.want_editor_focus {
                self.ed.focus_request = true;
                self.want_editor_focus = false;
            }
            let t0 = std::time::Instant::now();
            self.draw_editor_panes(ui, &mut cmds);
            if let Some(th) = std::env::var("RP_TRACE").ok().and_then(|v| v.parse::<f64>().ok()) {
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                if ms > th {
                    eprintln!("[trace]   editor took {ms:.2} ms");
                }
            }
        });

        // dialogs
        let dlg = self.show_dialogs(&ctx);
        cmds.extend(dlg);

        // execute collected commands
        for c in cmds {
            let t0 = std::time::Instant::now();
            let label = format!("{c:?}");
            self.exec(&ctx, c);
            if std::env::var_os("RP_TRACE").is_some() && t0.elapsed().as_millis() > 20 {
                eprintln!("[trace]   cmd {} took {} ms", label.chars().take(60).collect::<String>(), t0.elapsed().as_millis());
            }
        }

        let t0 = std::time::Instant::now();
        self.tick(&ctx);
        if std::env::var_os("RP_TRACE").is_some() && t0.elapsed().as_millis() > 20 {
            eprintln!("[trace]   tick took {} ms", t0.elapsed().as_millis());
        }
        self.sync_title(&ctx);
        self.debug_script(&ctx);
    }

}

impl App {
    /// Handle test-script actions that need app state (see `debug.rs`).
    fn debug_script(&mut self, ctx: &egui::Context) {
        let Some(script) = self.script.as_mut() else { return };
        ctx.request_repaint();
        let pending: Vec<String> = std::mem::take(&mut script.pending);
        for step in pending {
            let (op, rest) = step.split_once(' ').unwrap_or((step.as_str(), ""));
            match op {
                "shot" => {
                    if let Some(s) = self.script.as_mut() {
                        s.shot_path = Some(rest.to_string());
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                }
                "dump" => {
                    let t = self.doc_ref().text_string();
                    let _ = std::fs::write(rest, t.as_bytes());
                }
                "state" => {
                    let d = self.doc_ref();
                    let sels: Vec<(usize, usize)> = d.sels.iter().map(|s| (s.anchor, s.head)).collect();
                    let json = serde_json::json!({
                        "caret": d.caret(), "sels": sels, "line": d.line_of(d.caret()) + 1,
                        "modified": d.is_modified(), "tabs": self.tabs.len(), "active": self.active,
                        "lang": crate::lang::name(d.lang), "scroll_row": d.scroll_row,
                        "find_status": self.dialogs.find_status, "find_open": self.dialogs.find_open,
                        "bookmarks": d.bookmarks.iter().collect::<Vec<_>>(),
                        "collapsed": d.collapsed.iter().collect::<Vec<_>>(),
                        "enc": d.enc.label(), "eol": d.eol.label(), "recording": self.recording,
                        "names": self.tabs.iter().map(|t| t.display_name()).collect::<Vec<_>>(),
                        "modified_flags": self.tabs.iter().map(|t| t.is_modified()).collect::<Vec<_>>(),
                        "sel_text": d.selected_text(), "read_only": d.read_only, "split": self.split,
                        "pending": match &self.pending { None => "", Some(crate::app::Pending::Message(_)) => "message", Some(crate::app::Pending::ConfirmClose{..}) => "confirm_close", Some(crate::app::Pending::ExternalChange(_)) => "external_change", Some(crate::app::Pending::Hash{..}) => "hash", Some(_) => "other" },
                        "pending_text": match &self.pending { Some(crate::app::Pending::Hash{text, ..}) => text.clone(), _ => String::new() },
                        "ac": self.ed.ac.as_ref().map(|a| a.items.clone()),
                        "zoom": self.settings.zoom, "wrap": self.settings.word_wrap,
                        "cur_line_text": d.line_text(d.line_of(d.caret())),
                        "results": self.results.iter().map(|r| r.files.iter().map(|f| f.hits.len()).sum::<usize>()).collect::<Vec<_>>(),
                    });
                    let _ = std::fs::write(rest, json.to_string());
                }
                "cmd" => {
                    if let Ok(c) = serde_json::from_str::<Cmd>(rest) {
                        self.exec(ctx, c);
                    }
                }
                "setting" => {
                    if let (Ok(mut cur), Ok(patch)) = (serde_json::to_value(&self.settings), serde_json::from_str::<serde_json::Value>(rest)) {
                        if let (Some(c), Some(p)) = (cur.as_object_mut(), patch.as_object()) {
                            for (k, v) in p {
                                c.insert(k.clone(), v.clone());
                            }
                        }
                        if let Ok(s) = serde_json::from_value(cur) {
                            self.settings = s;
                        }
                    }
                }
                "sh" => {
                    let _ = std::process::Command::new("sh").args(["-c", rest]).status();
                }
                "exit" => std::process::exit(0),
                _ => {}
            }
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let path = self.script.as_mut().and_then(|s| s.shot_path.take());
            if let Some(path) = path {
                let (w, h) = (img.width() as u32, img.height() as u32);
                let mut data = Vec::with_capacity((w * h * 4) as usize);
                for px in &img.pixels {
                    data.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
                }
                if let Ok(file) = std::fs::File::create(&path) {
                    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
                    enc.set_color(png::ColorType::Rgba);
                    enc.set_depth(png::BitDepth::Eight);
                    if let Ok(mut wr) = enc.write_header() {
                        let _ = wr.write_image_data(&data);
                    }
                }
            }
        }
    }

    fn sync_title(&mut self, ctx: &egui::Context) {
        let d = self.doc_ref();
        let name = match &d.path {
            Some(p) => p.to_string_lossy().into_owned(),
            None => d.display_name(),
        };
        let title = format!("{}{} - RustPad", if d.is_modified() { "*" } else { "" }, name);
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn finish_and_close(&mut self, ctx: &egui::Context) {
        if self.settings.restore_session {
            crate::session::save(self, true);
        }
        // remember window geometry
        let (size, pos, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (v.inner_rect.map(|r| r.size()), v.outer_rect.map(|r| r.min), v.maximized)
        });
        if let Some(s) = size {
            self.settings.window_size = Some([s.x, s.y]);
        }
        if let Some(p) = pos {
            self.settings.window_pos = Some([p.x, p.y]);
        }
        self.settings.maximized = maximized.unwrap_or(false);
        self.settings.save();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
