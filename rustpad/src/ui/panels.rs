//! Side panels (workspace tree, function list, document map) and the search results panel.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Vec2};
use regex::Regex;

use crate::app::App;
use crate::cmd::Cmd;
use crate::lang;

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Default)]
pub struct Tree {
    cache: HashMap<PathBuf, (f64, Vec<Entry>)>,
}

impl Tree {
    fn entries(&mut self, dir: &PathBuf, now: f64) -> Vec<Entry> {
        if let Some((t, v)) = self.cache.get(dir) {
            if now - *t < 3.0 {
                return v.clone();
            }
        }
        let mut v: Vec<Entry> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| {
                        let path = e.path();
                        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false) || path.is_dir();
                        Entry { name: e.file_name().to_string_lossy().into_owned(), path, is_dir }
                    })
                    .filter(|e| !(e.name == ".git" && e.is_dir))
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        self.cache.insert(dir.clone(), (now, v.clone()));
        v
    }
}

impl App {
    pub fn workspace_panel(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        let _ = cmds;
        ui.horizontal(|ui| {
            ui.strong("Folder as Workspace");
            if ui.small_button("＋").on_hover_text("Add folder").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.add_workspace_root(&dir);
                }
            }
        });
        ui.separator();
        let roots = self.settings.workspace_roots.clone();
        let mut remove: Option<usize> = None;
        let mut open: Option<PathBuf> = None;
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            for (i, r) in roots.iter().enumerate() {
                let root = PathBuf::from(r);
                let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.clone());
                let resp = egui::CollapsingHeader::new(RichText::new(name).strong()).id_salt(("ws-root", r)).default_open(true).show(ui, |ui| {
                    draw_dir(&mut self.tree, ui, &root, self.time, &mut open);
                });
                resp.header_response.context_menu(|ui| {
                    if ui.button("Remove folder from workspace").clicked() {
                        remove = Some(i);
                        ui.close();
                    }
                });
            }
            if roots.is_empty() {
                ui.label("Use File ▸ Open Folder as Workspace…");
            }
        });
        if let Some(i) = remove {
            self.settings.workspace_roots.remove(i);
        }
        if let Some(p) = open {
            self.open_path(&p);
        }
    }

    pub fn function_list_panel(&mut self, ui: &mut egui::Ui) {
        ui.strong("Function List");
        ui.separator();
        let (id, ver, lang_id) = {
            let d = self.doc_ref();
            (d.id, d.version, d.lang)
        };
        // collect a finished background scan
        if let Some((jid, jver, rx)) = &self.fn_job {
            match rx.try_recv() {
                Ok(items) => {
                    self.fn_list_cache = Some((*jid, *jver, items));
                    self.fn_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(60)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.fn_job = None,
            }
        }
        let stale = !matches!(&self.fn_list_cache, Some((i, v, _)) if *i == id && *v == ver);
        if stale && self.fn_job.is_none() {
            if self.time >= self.fn_list_next {
                // scan off the UI thread; while typing, at most a few scans per second
                self.fn_list_next = self.time + 0.4;
                let rope = self.doc_ref().rope.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(symbols_of(&rope, lang_id));
                });
                self.fn_job = Some((id, ver, rx));
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
            }
        }
        let mut jump: Option<usize> = None;
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
        match &self.fn_list_cache {
            Some((i, _, items)) if *i == id => {
                if items.is_empty() {
                    ui.label(RichText::new("(no symbols)").weak());
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, items.len(), |ui, range| {
                    for (line, name) in &items[range] {
                        if ui.selectable_label(false, name).clicked() {
                            jump = Some(*line);
                        }
                    }
                });
            }
            _ => {
                ui.label(RichText::new("…").weak());
            }
        }
        if let Some(l) = jump {
            self.goto_line(l + 1);
            self.want_editor_focus = true;
        }
    }

    pub fn results_panel(&mut self, ui: &mut egui::Ui) {
        let mut clear = false;
        let mut close = false;
        let mut jump: Option<(Option<PathBuf>, String, usize, (usize, usize))> = None;
        let theme = self.theme().clone();
        ui.horizontal(|ui| {
            ui.strong("Search results");
            if self.fif_running {
                ui.spinner();
                ui.label(format!("{} files scanned", self.fif_progress));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✖").on_hover_text("Close panel").clicked() {
                    close = true;
                }
                if ui.small_button("Clear").clicked() {
                    clear = true;
                }
            });
        });
        ui.separator();
        let mono = FontId::monospace(12.5);
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            for (si, set) in self.results.iter_mut().enumerate() {
                let hdr = egui::CollapsingHeader::new(RichText::new(&set.title).strong()).id_salt(("rs", si)).open(if si == 0 { None } else { Some(!set.collapsed) }).default_open(si == 0);
                hdr.show(ui, |ui| {
                    for (fi, f) in set.files.iter().enumerate() {
                        let total: usize = f.hits.iter().map(|h| h.ranges.len()).sum();
                        let title = format!("{}  ({} hit{})", f.name, total, if total == 1 { "" } else { "s" });
                        egui::CollapsingHeader::new(RichText::new(title).color(theme.fg.c())).id_salt(("rf", si, fi)).default_open(true).show(ui, |ui| {
                            for h in &f.hits {
                                let mut job = egui::text::LayoutJob::default();
                                job.append(&format!("Line {:>5}: ", h.line + 1), 0.0, egui::TextFormat { font_id: mono.clone(), color: theme.gutter_fg.c(), ..Default::default() });
                                let chars: Vec<char> = h.text.chars().collect();
                                let mut pos = 0;
                                for &(a, b) in &h.ranges {
                                    let a = a.min(chars.len());
                                    let b = b.min(chars.len()).max(a);
                                    if a > pos {
                                        job.append(&chars[pos..a].iter().collect::<String>(), 0.0, egui::TextFormat { font_id: mono.clone(), color: theme.fg.c(), ..Default::default() });
                                    }
                                    if b > a {
                                        job.append(
                                            &chars[a..b].iter().collect::<String>(),
                                            0.0,
                                            egui::TextFormat { font_id: mono.clone(), color: theme.fg.c(), background: theme.find_highlight.a(160), ..Default::default() },
                                        );
                                    }
                                    pos = b.max(pos);
                                }
                                if pos < chars.len() {
                                    job.append(&chars[pos..].iter().collect::<String>(), 0.0, egui::TextFormat { font_id: mono.clone(), color: theme.fg.c(), ..Default::default() });
                                }
                                let r = ui.add(egui::Label::new(job).sense(Sense::click()));
                                if r.clicked() {
                                    let first = h.ranges.first().copied().unwrap_or((0, 0));
                                    jump = Some((f.path.clone(), f.name.clone(), h.line, first));
                                }
                            }
                        });
                    }
                });
            }
        });
        if clear {
            self.results.clear();
        }
        if close {
            self.show_results = false;
        }
        if let Some((path, name, line, cols)) = jump {
            self.goto_result(path.as_deref(), &name, line, cols);
        }
    }

    /// Scaled-down overview of the document with a viewport indicator.
    pub fn doc_map(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme().clone();
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme.gutter_bg.c());
        let pitch = 2.5f32;
        let max_lines = (rect.height() / pitch) as usize;
        let page = self.ed.view_rows.1.max(1) as usize;
        let (cur_row, total_rows) = {
            let d = self.doc_ref();
            (d.scroll_row as usize, d.rowmap.total_rows() as usize)
        };
        // window of lines to show, centered on the viewport
        let first = if total_rows <= max_lines { 0 } else { (cur_row + page / 2).saturating_sub(max_lines / 2).min(total_rows - max_lines) };
        let doc = self.doc();
        let n_lines = doc.len_lines();
        let mut spans = Vec::new();
        let mut shown = 0;
        let mut line = doc.rowmap.locate(first as u64).0;
        let mut y = rect.top();
        while line < n_lines && shown < max_lines {
            if doc.is_hidden(line) {
                line += 1;
                continue;
            }
            let text = doc.line_spans(line, &mut spans);
            if text.len() < 4000 {
                let x0 = rect.left() + 4.0;
                let mut col = 0usize;
                let mut bi = 0usize;
                let mut si = 0usize;
                for ch in text.chars() {
                    let w = if ch == '\t' { 4 } else { 1 };
                    if !ch.is_whitespace() && col < (rect.width() as usize - 8) {
                        while si < spans.len() && spans[si].end <= bi {
                            si += 1;
                        }
                        let st = if si < spans.len() && spans[si].start <= bi { spans[si].style } else { lang::Style::Default };
                        let c = theme.style_color(st);
                        painter.rect_filled(Rect::from_min_size(Pos2::new(x0 + col as f32, y), Vec2::new(1.0, pitch - 0.8)), 0.0, c.gamma_multiply(0.9));
                    }
                    col += w;
                    bi += ch.len_utf8();
                }
            }
            y += pitch;
            shown += 1;
            line += 1;
        }
        // viewport indicator
        let vy0 = rect.top() + (cur_row.saturating_sub(first)) as f32 * pitch;
        let vh = (page as f32 * pitch).min(rect.height());
        let view = Rect::from_min_size(Pos2::new(rect.left(), vy0), Vec2::new(rect.width(), vh));
        painter.rect_filled(view, 0.0, Color32::from_rgba_unmultiplied(120, 160, 255, 40));
        painter.rect_stroke(view, 0.0, egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 160, 255, 140)), egui::StrokeKind::Inside);
        if resp.dragged() || resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let row = first as f32 + (p.y - rect.top()) / pitch - page as f32 / 2.0;
                self.doc().scroll_request = Some(row.max(0.0));
            }
        }
    }
}

fn draw_dir(tree: &mut Tree, ui: &mut egui::Ui, dir: &PathBuf, now: f64, open: &mut Option<PathBuf>) {
    let entries = tree.entries(dir, now);
    for e in entries {
        if e.is_dir {
            egui::CollapsingHeader::new(&e.name).id_salt(("ws-dir", &e.path)).show(ui, |ui| {
                draw_dir(tree, ui, &e.path, now, open);
            });
        } else if ui.selectable_label(false, &e.name).clicked() {
            *open = Some(e.path.clone());
        }
    }
}

fn symbols_of(rope: &ropey::Rope, lang_id: usize) -> Vec<(usize, String)> {
    let def = lang::LANGS[lang_id.min(lang::LANGS.len() - 1)];
    if def.symbols.is_empty() || rope.len_lines() > 300_000 {
        return Vec::new();
    }
    let Ok(re) = Regex::new(def.symbols) else { return Vec::new() };
    let mut out = Vec::new();
    for l in 0..rope.len_lines() {
        let slice = rope.line(l);
        if slice.len_bytes() > 400 {
            continue;
        }
        let text = slice.to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        if let Some(c) = re.captures(text) {
            let name = (1..c.len()).find_map(|i| c.get(i)).map(|m| m.as_str().trim().to_string());
            if let Some(n) = name.filter(|n| !n.is_empty()) {
                out.push((l, n));
            }
        }
        if out.len() > 20_000 {
            break;
        }
    }
    out
}
