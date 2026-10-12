//! The conflict editor: a separate window with the two conflicting versions
//! side by side (pick whole blocks or individual lines from either) and the
//! resulting file underneath, which can also be edited by hand.

use crate::detail::spawn;
use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use gitgoing_core::conflict::{self, ConflictFile, Kind, Origin, Region, Resolution, Side};
use gitgoing_core::merge;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

pub enum Outcome {
    Open,
    Cancelled,
    /// The file was resolved and staged.
    Resolved,
}

pub struct ConflictEditor {
    repo: PathBuf,
    file: ConflictFile,
    res: Resolution,
    /// The result edited by hand (disables line selection above).
    manual: Option<String>,
    /// Region index of each conflict, and the longest line, for layout.
    conflict_regions: Vec<usize>,
    max_chars: usize,
    current: usize,
    scroll_to: Option<usize>,
    save_rx: Option<Receiver<Result<(), String>>>,
    error: Option<String>,
}

enum Row {
    Common(usize, usize),
    Header(usize),
    Pair(usize, usize),
}

const OURS: Color32 = Color32::from_rgb(0x4e, 0xa8, 0xde);
const THEIRS: Color32 = Color32::from_rgb(0xe5, 0xc0, 0x7b);
const RED: Color32 = Color32::from_rgb(0xe5, 0x53, 0x4b);

impl ConflictEditor {
    pub fn open(repo: &Path, path: &str) -> Result<Self, String> {
        let file = conflict::load(repo, path)?;
        let res = Resolution::new(file.regions.clone());
        let conflict_regions = file
            .regions
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, Region::Conflict { .. }))
            .map(|(i, _)| i)
            .collect();
        let max_chars = file
            .regions
            .iter()
            .flat_map(|r| match r {
                Region::Common(l) => vec![l],
                Region::Conflict { ours, theirs, .. } => vec![ours, theirs],
            })
            .flatten()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        Ok(ConflictEditor {
            repo: repo.to_path_buf(),
            file,
            res,
            manual: None,
            conflict_regions,
            max_chars,
            current: 0,
            scroll_to: None,
            save_rx: None,
            error: None,
        })
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        if let Some(rx) = &self.save_rx {
            if let Ok(r) = rx.try_recv() {
                self.save_rx = None;
                match r {
                    Ok(()) => return Outcome::Resolved,
                    Err(e) => self.error = Some(e),
                }
            }
        }
        let mut outcome = Outcome::Open;
        let id = egui::ViewportId::from_hash_of(("conflict", &self.file.path));
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("Resolve conflicts: {}", self.file.path))
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([800.0, 500.0]);
        ctx.show_viewport_immediate(id, builder, |ctx, class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                outcome = Outcome::Cancelled;
            }
            let mut cancel = false;
            if class == egui::ViewportClass::Embedded {
                // Backends without real extra windows show it inside the main one.
                egui::Window::new(format!("Resolve conflicts: {}", self.file.path))
                    .default_size([1100.0, 700.0])
                    .show(ctx, |ui| cancel = self.content(ui));
            } else {
                egui::CentralPanel::default().show(ctx, |ui| cancel = self.content(ui));
            }
            if cancel {
                outcome = Outcome::Cancelled;
            }
        });
        outcome
    }

    fn busy(&self) -> bool {
        self.save_rx.is_some()
    }

    /// Run `f` on a background thread; the window closes when it succeeds.
    fn run(&mut self, ctx: &egui::Context, f: impl FnOnce(&Path) -> Result<(), String> + Send + 'static) {
        let repo = self.repo.clone();
        self.error = None;
        self.save_rx = Some(spawn(ctx, move || f(&repo)));
    }

    /// Draw the whole editor. Returns true if the user cancelled.
    fn content(&mut self, ui: &mut egui::Ui) -> bool {
        let mut cancel = false;
        match self.file.kind {
            Kind::Text => self.text_editor(ui, &mut cancel),
            _ => self.whole_file_choice(ui, &mut cancel),
        }
        cancel
    }

    /// Delete/modify and binary conflicts have no lines to pick from.
    fn whole_file_choice(&mut self, ui: &mut egui::Ui, cancel: &mut bool) {
        let ctx = ui.ctx().clone();
        let (ours, theirs) = (self.file.ours_label.clone(), self.file.theirs_label.clone());
        let path = self.file.path.clone();
        ui.add_space(12.0);
        ui.heading(&path);
        ui.add_space(8.0);
        let (text, keep_ours, keep_theirs) = match self.file.kind {
            Kind::DeletedByUs => (
                format!("This file was deleted on your side ({ours}) but changed on {theirs}."),
                ("Delete the file".to_owned(), merge::Side::Ours),
                ("Keep the changed file".to_owned(), merge::Side::Theirs),
            ),
            Kind::DeletedByThem => (
                format!("This file was changed on your side ({ours}) but deleted on {theirs}."),
                ("Keep my changed file".to_owned(), merge::Side::Ours),
                ("Delete the file".to_owned(), merge::Side::Theirs),
            ),
            _ => (
                "This is a binary file, so lines can't be merged. Pick one version.".to_owned(),
                (format!("Use {ours}"), merge::Side::Ours),
                (format!("Use {theirs}"), merge::Side::Theirs),
            ),
        };
        ui.label(text);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            for (label, side) in [keep_ours, keep_theirs] {
                if ui.add_enabled(!self.busy(), egui::Button::new(label)).clicked() {
                    let p = path.clone();
                    self.run(&ctx, move |r| merge::resolve(r, &p, side).map_err(|e| e.message().to_owned()));
                }
            }
            if ui.button("Cancel").clicked() {
                *cancel = true;
            }
        });
        if let Some(e) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
    }

    fn result_lines(&self) -> Vec<(String, Origin)> {
        self.res.result()
    }

    fn result_text(&self) -> String {
        match &self.manual {
            Some(t) => {
                let t = t.replace("\r\n", "\n");
                if self.file.crlf { t.replace('\n', "\r\n") } else { t }
            }
            None => {
                let lines: Vec<String> = self.result_lines().into_iter().map(|(l, _)| l).collect();
                conflict::to_text(&lines, self.file.crlf, self.file.final_newline)
            }
        }
    }

    fn goto(&mut self, c: usize, rows: &[usize]) {
        self.current = c;
        self.scroll_to = rows.get(c).copied();
    }

    fn text_editor(&mut self, ui: &mut egui::Ui, cancel: &mut bool) {
        let ctx = ui.ctx().clone();
        let n = self.res.conflict_count();
        let unresolved = self.res.unresolved_count();
        let interactive = self.manual.is_none();

        // Rows of the side-by-side view, and where each conflict starts.
        let mut rows = Vec::new();
        let mut first_row = Vec::new();
        let mut c = 0;
        for (ri, r) in self.file.regions.iter().enumerate() {
            match r {
                Region::Common(l) => rows.extend((0..l.len()).map(|k| Row::Common(ri, k))),
                Region::Conflict { ours, theirs, .. } => {
                    first_row.push(rows.len());
                    rows.push(Row::Header(c));
                    rows.extend((0..ours.len().max(theirs.len())).map(|k| Row::Pair(c, k)));
                    c += 1;
                }
            }
        }

        egui::TopBottomPanel::top("conflict_top").show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading(&self.file.path);
                ui.add_space(12.0);
                ui.colored_label(OURS, format!("■ {}", self.file.ours_label));
                ui.colored_label(THEIRS, format!("■ {}", self.file.theirs_label));
                ui.weak("Click a line, or a block's header, to include it in the result.");
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(interactive, egui::Button::new("Take all ours")).clicked() {
                    self.res.set_all(true, false);
                }
                if ui.add_enabled(interactive, egui::Button::new("Take all theirs")).clicked() {
                    self.res.set_all(false, true);
                }
                if ui.add_enabled(interactive, egui::Button::new("Take both")).clicked() {
                    self.res.set_all(true, true);
                }
                ui.separator();
                if crate::icons::icon_button(ui, crate::icons::Icon::ArrowUp, false, "Previous conflict", n > 0).clicked() {
                    self.goto(self.current.saturating_sub(1), &first_row);
                }
                if crate::icons::icon_button(ui, crate::icons::Icon::ArrowDown, false, "Next conflict", n > 0).clicked() {
                    self.goto((self.current + 1).min(n.saturating_sub(1)), &first_row);
                }
                ui.label(if n == 0 {
                    "No conflicting lines".to_owned()
                } else {
                    format!("Conflict {} of {}  ·  {} unresolved", self.current + 1, n, unresolved)
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let can_save = (unresolved == 0 || self.manual.is_some()) && !self.busy();
                    if ui
                        .add_enabled(can_save, egui::Button::new(RichText::new("Save & Mark Resolved").strong()))
                        .on_disabled_hover_text("Decide every conflict first")
                        .clicked()
                    {
                        let (path, text) = (self.file.path.clone(), self.result_text());
                        self.run(&ctx, move |r| conflict::write_resolution(r, &path, &text));
                    }
                    if ui.button("Cancel").clicked() {
                        *cancel = true;
                    }
                    if self.busy() {
                        ui.spinner();
                    }
                });
            });
            if let Some(e) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.add_space(4.0);
        });

        egui::TopBottomPanel::bottom("conflict_result")
            .resizable(true)
            .default_height(260.0)
            .min_height(100.0)
            .show_inside(ui, |ui| self.result_pane(ui));

        egui::CentralPanel::default().show_inside(ui, |ui| self.sides(ui, &rows, &first_row, interactive));
    }

    fn sides(&mut self, ui: &mut egui::Ui, rows: &[Row], first_row: &[usize], interactive: bool) {
        let font = FontId::monospace(14.0);
        let cw = ui.fonts(|f| f.glyph_width(&font, 'M'));
        let row_h = ui.fonts(|f| f.row_height(&font)) + 5.0;
        let fg = ui.visuals().text_color();
        let weak = ui.visuals().weak_text_color();
        let line_col = Color32::from_rgba_unmultiplied(128, 128, 128, 80);
        let gutter = 30.0;
        let half_w = (ui.available_width() / 2.0).max(gutter + (self.max_chars as f32 + 2.0) * cw);
        let width = half_w * 2.0;
        let tint = |c: Color32, selected: bool| {
            Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), if selected { 100 } else { 38 })
        };

        let mut area = egui::ScrollArea::both().id_salt("conflict_sides").auto_shrink(false);
        if let Some(r) = self.scroll_to.take() {
            area = area.vertical_scroll_offset((r as f32 - 1.0).max(0.0) * row_h);
        }

        let (ours_label, theirs_label) = (self.file.ours_label.clone(), self.file.theirs_label.clone());
        let n_conflicts = self.res.conflict_count();
        let mut toggles: Vec<(usize, Side, usize)> = Vec::new();
        let mut sides_set: Vec<(usize, Side, bool)> = Vec::new();
        let mut visible_current = None;

        area.show_rows(ui, row_h, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::click());
                let p = ui.painter_at(rect);
                let cy = rect.center().y;
                let mid = rect.left() + half_w;
                p.vline(mid, rect.y_range(), Stroke::new(1.0_f32, line_col));
                match &rows[i] {
                    Row::Common(ri, k) => {
                        let Region::Common(lines) = &self.file.regions[*ri] else { continue };
                        for x in [rect.left(), mid] {
                            p.text(Pos2::new(x + gutter, cy), egui::Align2::LEFT_CENTER, &lines[*k], font.clone(), fg.gamma_multiply(0.85));
                        }
                    }
                    Row::Header(c) => {
                        let Region::Conflict { ours, theirs, .. } = &self.file.regions[self.conflict_regions[*c]] else { continue };
                        p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(128, 128, 128, 55));
                        if *c == self.current {
                            p.rect_stroke(rect, 0.0, Stroke::new(1.5_f32, OURS), egui::StrokeKind::Inside);
                            visible_current = Some(i);
                        }
                        let pick = &self.res.picks[*c];
                        let all_ours = !ours.is_empty() && pick.ours.iter().all(|x| *x);
                        let all_theirs = !theirs.is_empty() && pick.theirs.iter().all(|x| *x);
                        let ui_font = FontId::proportional(14.0);
                        for (side, on, count, label, colour, x0) in [
                            (Side::Ours, all_ours, ours.len(), &ours_label, OURS, rect.left()),
                            (Side::Theirs, all_theirs, theirs.len(), &theirs_label, THEIRS, mid),
                        ] {
                            let b = Rect::from_center_size(Pos2::new(x0 + 16.0, cy), Vec2::splat(13.0));
                            p.rect_stroke(b, 2.0, Stroke::new(1.5_f32, colour), egui::StrokeKind::Inside);
                            if on {
                                p.rect_filled(b.shrink(2.5), 1.0, colour);
                            }
                            let text = match side {
                                Side::Ours => format!("Conflict {} of {}   take all {} line{} from {}", c + 1, n_conflicts, count, if count == 1 { "" } else { "s" }, label),
                                Side::Theirs => format!("take all {} line{} from {}", count, if count == 1 { "" } else { "s" }, label),
                            };
                            p.text(Pos2::new(x0 + 32.0, cy), egui::Align2::LEFT_CENTER, text, ui_font.clone(), colour);
                            let hit = resp.clicked()
                                && resp.interact_pointer_pos().is_some_and(|pt| pt.x >= x0 && pt.x < x0 + half_w);
                            if hit && interactive {
                                sides_set.push((*c, side, !on));
                            }
                        }
                    }
                    Row::Pair(c, k) => {
                        let Region::Conflict { ours, theirs, .. } = &self.file.regions[self.conflict_regions[*c]] else { continue };
                        let pick = &self.res.picks[*c];
                        for (side, lines, flags, colour, x0) in [
                            (Side::Ours, ours, &pick.ours, OURS, rect.left()),
                            (Side::Theirs, theirs, &pick.theirs, THEIRS, mid),
                        ] {
                            let cell = Rect::from_min_size(Pos2::new(x0, rect.top()), Vec2::new(half_w, row_h));
                            let Some(line) = lines.get(*k) else {
                                p.rect_filled(cell, 0.0, Color32::from_rgba_unmultiplied(128, 128, 128, 14));
                                continue;
                            };
                            let on = flags.get(*k).copied().unwrap_or(false);
                            p.rect_filled(cell, 0.0, tint(colour, on));
                            if on {
                                // A solid stripe makes chosen lines easy to spot.
                                p.rect_filled(Rect::from_min_size(cell.min, Vec2::new(3.0, row_h)), 0.0, colour);
                            }
                            // Check box for this line.
                            let b = Rect::from_center_size(Pos2::new(x0 + 14.0, cy), Vec2::splat(11.0));
                            p.rect_stroke(b, 2.0, Stroke::new(1.0_f32, if on { colour } else { weak }), egui::StrokeKind::Inside);
                            if on {
                                p.rect_filled(b.shrink(2.0), 1.0, colour);
                            }
                            p.text(
                                Pos2::new(x0 + gutter, cy),
                                egui::Align2::LEFT_CENTER,
                                line,
                                font.clone(),
                                if on { ui.visuals().strong_text_color() } else { fg.gamma_multiply(0.85) },
                            );
                            let hit = resp.clicked()
                                && resp.interact_pointer_pos().is_some_and(|pt| pt.x >= x0 && pt.x < x0 + half_w);
                            if hit && interactive {
                                toggles.push((*c, side, *k));
                            }
                        }
                    }
                }
            }
        });
        let _ = (first_row, visible_current);
        for (c, side, on) in sides_set {
            self.res.set_side(c, side, on);
            self.current = c;
        }
        for (c, side, k) in toggles {
            self.res.toggle(c, side, k);
            self.current = c;
        }
    }

    fn result_pane(&mut self, ui: &mut egui::Ui) {
        let font = FontId::monospace(14.0);
        let row_h = ui.fonts(|f| f.row_height(&font)) + 5.0;
        let cw = ui.fonts(|f| f.glyph_width(&font, 'M'));
        let fg = ui.visuals().text_color();
        let weak = ui.visuals().weak_text_color();

        let mut edit = self.manual.is_some();
        let mut reset = false;
        ui.horizontal(|ui| {
            ui.strong("Result");
            ui.weak("(lines appear in the order you picked them)");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.manual.is_some() && ui.button("Reset to selections").clicked() {
                    reset = true;
                }
                if ui
                    .checkbox(&mut edit, "Edit manually")
                    .on_hover_text("Type directly in the result. Line selection above is disabled while editing.")
                    .changed()
                    && !reset
                {
                    self.manual = if edit { Some(self.result_text().replace("\r\n", "\n")) } else { None };
                }
            });
        });
        if reset {
            self.manual = None;
        }
        ui.separator();

        if let Some(text) = &mut self.manual {
            egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .font(font.clone())
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(8)
                        .frame(false),
                );
            });
            return;
        }

        let lines = self.result_lines();
        let width = (lines.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0) as f32 + 6.0) * cw + 60.0;
        egui::ScrollArea::both().id_salt("conflict_result_rows").auto_shrink(false).show_rows(
            ui,
            row_h,
            lines.len(),
            |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for i in range {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(width.max(ui.available_width()), row_h), Sense::hover());
                    let p = ui.painter_at(rect);
                    let cy = rect.center().y;
                    let (line, origin) = &lines[i];
                    let (bar, text_col) = match origin {
                        Origin::Common => (None, fg.gamma_multiply(0.85)),
                        Origin::Ours => (Some(OURS), fg),
                        Origin::Theirs => (Some(THEIRS), fg),
                        Origin::Unresolved(_) => (Some(RED), RED),
                    };
                    if matches!(origin, Origin::Unresolved(_)) {
                        p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0xe5, 0x53, 0x4b, 30));
                    }
                    if let Some(c) = bar {
                        p.rect_filled(Rect::from_min_size(rect.min, Vec2::new(4.0, row_h)), 0.0, c);
                    }
                    p.text(Pos2::new(rect.left() + 46.0, cy), egui::Align2::RIGHT_CENTER, i + 1, font.clone(), weak);
                    p.text(Pos2::new(rect.left() + 54.0, cy), egui::Align2::LEFT_CENTER, line, font.clone(), text_col);
                }
            },
        );
    }
}
