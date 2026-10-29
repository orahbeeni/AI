//! The conflict editor, laid out like GitKraken's: the two conflicting versions
//! (A and B) side by side on top, where whole hunks or single lines are picked
//! with a click, and the output file underneath. The output is an ordinary
//! editor at all times; picking a hunk rewrites just that conflict's lines in
//! it, and the conflicts stay highlighted while you type around them.

use crate::detail::spawn;
use crate::diffview::tint;
use crate::syntax;
use eframe::egui::{self, text::LayoutJob, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, TextFormat, Vec2};
use gitgoing_core::conflict::{self, ConflictFile, Kind, Region, Resolution, Side};
use gitgoing_core::merge;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

pub enum Outcome {
    Open,
    Cancelled,
    /// The file was resolved and staged.
    Resolved,
}

/// One row of the side-by-side view: a line both versions share, or line `k` of conflict `c`
/// (on the shorter side the row is blank).
enum Row {
    Common(usize, usize),
    Pair(usize, usize),
}

pub struct ConflictEditor {
    repo: PathBuf,
    file: ConflictFile,
    res: Resolution,
    rows: Vec<Row>,
    /// Line numbers of each row in version A and B (blank rows have none).
    nums: Vec<[Option<usize>; 2]>,
    /// First row of each conflict, and its index in `file.regions`.
    first_row: Vec<usize>,
    conflict_regions: Vec<usize>,
    /// Longest line (in characters) of version A and of version B.
    side_chars: [usize; 2],
    /// Each version scrolls sideways on its own, following the other to the same column.
    split_scroll: bool,
    split_loaded: bool,
    /// Sideways scroll of A and B in that mode.
    hoff: [f32; 2],
    voff: f32,
    /// The output file being edited (always with `\n` line ends).
    out: String,
    /// Where each conflict's lines are in `out`: (first line, line count).
    ranges: Vec<(usize, usize)>,
    current: usize,
    scroll_sides: Option<usize>,
    scroll_out: Option<usize>,
    save_rx: Option<Receiver<Result<(), String>>>,
    error: Option<String>,
    /// Shown in a window of its own instead of the main window's centre pane.
    popped: bool,
}

const A_COL: (u8, u8, u8) = (0x2f, 0xa8, 0xc4);
const B_COL: (u8, u8, u8) = (0xe5, 0xb8, 0x3b);
const OUT_COL: (u8, u8, u8) = (0xb0, 0x4c, 0xd0);

/// Thickness of the scroll bars in the A / B panes.
const BAR: f32 = 12.0;

/// Move both panes to the column pane `driver` is at (as far as the other one reaches).
fn follow_cols(hoff: &mut [f32; 2], driver: usize, max_off: [f32; 2]) {
    let x = hoff[driver].clamp(0.0, max_off[driver]);
    *hoff = [x.min(max_off[0]), x.min(max_off[1])];
}

/// A scroll bar that is always drawn (a plain track when there is nothing to scroll).
/// `view` is how much of `content` shows at once; returns whether `off` changed.
#[allow(clippy::too_many_arguments)]
fn scrollbar(ui: &mut egui::Ui, id: egui::Id, track: Rect, vertical: bool, off: &mut f32, max: f32, view: f32, content: f32) -> bool {
    ui.painter().rect_filled(track, 0.0, ui.visuals().extreme_bg_color);
    if max <= 0.0 || content <= 0.0 {
        return false;
    }
    let (len, start) = if vertical { (track.height(), track.top()) } else { (track.width(), track.left()) };
    let thumb = (len * view / content).clamp(28.0_f32.min(len), len);
    let travel = (len - thumb).max(1.0);
    let resp = ui.interact(track, id, Sense::click_and_drag());
    let before = *off;
    if resp.dragged() {
        let d = if vertical { resp.drag_delta().y } else { resp.drag_delta().x };
        *off = (*off + d / travel * max).clamp(0.0, max);
    } else if let Some(pt) = resp.clicked().then(|| resp.interact_pointer_pos()).flatten() {
        let at = if vertical { pt.y } else { pt.x } - start - thumb / 2.0;
        *off = (at / travel * max).clamp(0.0, max);
    }
    let pos = start + *off / max * travel;
    let rect = if vertical {
        Rect::from_min_size(Pos2::new(track.left(), pos), Vec2::new(track.width(), thumb))
    } else {
        Rect::from_min_size(Pos2::new(pos, track.top()), Vec2::new(thumb, track.height()))
    };
    let col = if resp.dragged() || resp.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    ui.painter().rect_filled(rect.shrink(2.0), 3.0, col);
    *off != before
}

fn rgb(c: (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

impl ConflictEditor {
    pub fn open(repo: &Path, path: &str) -> Result<Self, String> {
        let file = conflict::load(repo, path)?;
        let res = Resolution::new(file.regions.clone());
        let mut rows = Vec::new();
        let mut nums = Vec::new();
        let mut first_row = Vec::new();
        let mut conflict_regions = Vec::new();
        let (mut a, mut b) = (0usize, 0usize);
        let mut out_lines: Vec<String> = Vec::new();
        let mut ranges = Vec::new();
        for (ri, r) in file.regions.iter().enumerate() {
            match r {
                Region::Common(lines) => {
                    for k in 0..lines.len() {
                        a += 1;
                        b += 1;
                        rows.push(Row::Common(ri, k));
                        nums.push([Some(a), Some(b)]);
                    }
                    out_lines.extend(lines.iter().cloned());
                }
                Region::Conflict { ours, base, theirs } => {
                    let c = conflict_regions.len();
                    conflict_regions.push(ri);
                    first_row.push(rows.len());
                    for k in 0..ours.len().max(theirs.len()).max(1) {
                        rows.push(Row::Pair(c, k));
                        let na = (k < ours.len()).then(|| {
                            a += 1;
                            a
                        });
                        let nb = (k < theirs.len()).then(|| {
                            b += 1;
                            b
                        });
                        nums.push([na, nb]);
                    }
                    // Until a side is picked, the output shows the common ancestor's lines.
                    ranges.push((out_lines.len(), base.len()));
                    out_lines.extend(base.iter().cloned());
                }
            }
        }
        let mut side_chars = [0usize; 2];
        for r in &file.regions {
            let (a, b) = match r {
                Region::Common(l) => (l, l),
                Region::Conflict { ours, theirs, .. } => (ours, theirs),
            };
            side_chars[0] = side_chars[0].max(a.iter().map(|l| l.chars().count()).max().unwrap_or(0));
            side_chars[1] = side_chars[1].max(b.iter().map(|l| l.chars().count()).max().unwrap_or(0));
        }
        let out = conflict::to_text(&out_lines, false, file.final_newline);
        let scroll = first_row.first().copied();
        Ok(ConflictEditor {
            repo: repo.to_path_buf(),
            file,
            res,
            rows,
            nums,
            first_row,
            conflict_regions,
            side_chars,
            split_scroll: false,
            split_loaded: false,
            hoff: [0.0; 2],
            voff: 0.0,
            out,
            scroll_out: ranges.first().map(|r| r.0),
            ranges,
            current: 0,
            scroll_sides: scroll,
            save_rx: None,
            error: None,
            popped: false,
        })
    }

    /// Draw the editor in `ui` (it fills the space it is given).
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Outcome {
        if let Some(rx) = &self.save_rx {
            if let Ok(r) = rx.try_recv() {
                self.save_rx = None;
                match r {
                    Ok(()) => return Outcome::Resolved,
                    Err(e) => self.error = Some(e),
                }
            }
        }
        let mut cancel = false;
        match self.file.kind {
            Kind::Text => self.text_editor(ui, &mut cancel),
            _ => self.whole_file_choice(ui, &mut cancel),
        }
        if cancel { Outcome::Cancelled } else { Outcome::Open }
    }

    pub fn popped(&self) -> bool {
        self.popped
    }

    /// Draw the editor in its own window. Closing that window puts it back in the main one.
    pub fn window(&mut self, ctx: &egui::Context) -> Outcome {
        let mut outcome = Outcome::Open;
        let id = egui::ViewportId::from_hash_of(("conflict", &self.file.path));
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("Resolve conflicts: {}", self.file.path))
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0]);
        ctx.show_viewport_immediate(id, builder, |ctx, class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                self.popped = false;
            }
            if class == egui::ViewportClass::Embedded {
                // Backends without real extra windows show it as a window inside the main one.
                egui::Window::new(format!("Resolve conflicts: {}", self.file.path))
                    .default_size([1100.0, 700.0])
                    .show(ctx, |ui| outcome = self.ui(ui));
            } else {
                egui::CentralPanel::default().show(ctx, |ui| outcome = self.ui(ui));
            }
        });
        outcome
    }

    fn busy(&self) -> bool {
        self.save_rx.is_some()
    }

    /// Run `f` on a background thread; the editor closes when it succeeds.
    fn run(&mut self, ctx: &egui::Context, f: impl FnOnce(&Path) -> Result<(), String> + Send + 'static) {
        let repo = self.repo.clone();
        self.error = None;
        self.save_rx = Some(spawn(ctx, move || f(&repo)));
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

    // ---- picking ---------------------------------------------------------

    fn side_lines(&self, c: usize, side: Side) -> &[String] {
        match &self.file.regions[self.conflict_regions[c]] {
            Region::Conflict { ours, theirs, .. } => match side {
                Side::Ours => ours,
                Side::Theirs => theirs,
            },
            Region::Common(_) => &[],
        }
    }

    /// Whether hunk `c` of `side` counts as taken. A side with no lines (it deleted them)
    /// is taken when the conflict is decided with nothing from the other side.
    fn hunk_on(&self, c: usize, side: Side) -> bool {
        let p = &self.res.picks[c];
        let (mine, other) = match side {
            Side::Ours => (&p.ours, &p.theirs),
            Side::Theirs => (&p.theirs, &p.ours),
        };
        if mine.is_empty() {
            p.touched && !other.iter().any(|x| *x)
        } else {
            mine.iter().all(|x| *x)
        }
    }

    fn set_hunk(&mut self, c: usize, side: Side, on: bool) {
        if self.side_lines(c, side).is_empty() {
            if on {
                self.res.set_conflict(c, false, false);
            } else {
                self.res.picks[c].touched = false;
            }
        } else {
            self.res.set_side(c, side, on);
        }
        self.apply(c);
    }

    fn toggle_line(&mut self, c: usize, side: Side, k: usize) {
        self.res.toggle(c, side, k);
        self.apply(c);
    }

    /// Write conflict `c`'s picked lines (or the ancestor's, if nothing is decided) into the output.
    fn apply(&mut self, c: usize) {
        let new_lines: Vec<String> = {
            let Region::Conflict { ours, base, theirs } = &self.file.regions[self.conflict_regions[c]] else { return };
            let p = &self.res.picks[c];
            if p.touched {
                p.order
                    .iter()
                    .filter_map(|(s, k)| match s {
                        Side::Ours => ours.get(*k),
                        Side::Theirs => theirs.get(*k),
                    })
                    .cloned()
                    .collect()
            } else {
                base.clone()
            }
        };
        let mut lines: Vec<String> = self.out.split('\n').map(str::to_owned).collect();
        let (start, len) = self.ranges[c];
        let end = (start + len).min(lines.len());
        let start = start.min(end);
        let delta = new_lines.len() as isize - (end - start) as isize;
        lines.splice(start..end, new_lines.iter().cloned());
        self.ranges[c] = (start, new_lines.len());
        for (i, r) in self.ranges.iter_mut().enumerate() {
            if i != c && r.0 >= end && r.0 >= start {
                r.0 = (r.0 as isize + delta).max(0) as usize;
            }
        }
        self.out = lines.join("\n");
        self.current = c;
    }

    fn goto(&mut self, c: usize) {
        if c < self.first_row.len() {
            self.current = c;
            self.scroll_sides = Some(self.first_row[c]);
            self.scroll_out = Some(self.ranges[c].0);
        }
    }

    // ---- layout ----------------------------------------------------------

    fn text_editor(&mut self, ui: &mut egui::Ui, cancel: &mut bool) {
        let ctx = ui.ctx().clone();
        let undecided = self.res.picks.iter().filter(|p| !p.touched).count();
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.file.path).size(15.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Leave the file conflicted for now").clicked() {
                    *cancel = true;
                }
                let (label, tip) = if self.popped {
                    ("Back to main window", "Show the editor in the main window again")
                } else {
                    ("Open in window", "Move the editor to a window of its own, for more room")
                };
                if ui.button(label).on_hover_text(tip).clicked() {
                    self.popped = !self.popped;
                }
                let flag = egui::Id::new("conflict_split_scroll");
                if !self.split_loaded {
                    self.split_loaded = true;
                    self.split_scroll = ctx.data_mut(|d| d.get_persisted(flag)).unwrap_or(false);
                }
                if ui
                    .checkbox(&mut self.split_scroll, "Scroll A and B separately")
                    .on_hover_text("Give A and B a sideways scroll bar each; scrolling one moves the other to the same column")
                    .changed()
                {
                    let on = self.split_scroll;
                    ctx.data_mut(|d| d.insert_persisted(flag, on));
                }
                let save = egui::Button::new(RichText::new("Save & mark resolved").strong().color(Color32::WHITE))
                    .fill(Color32::from_rgb(0x2e, 0x7d, 0x32));
                if ui
                    .add_enabled(!self.busy(), save)
                    .on_hover_text("Write the output to the file and stage it")
                    .clicked()
                {
                    let text = if self.file.crlf { self.out.replace('\n', "\r\n") } else { self.out.clone() };
                    let path = self.file.path.clone();
                    self.run(&ctx, move |r| conflict::write_resolution(r, &path, &text));
                }
                if self.busy() {
                    ui.spinner();
                }
                if undecided > 0 {
                    ui.colored_label(
                        rgb(B_COL),
                        format!("{undecided} conflict{} not picked yet (the output shows the original lines there)", if undecided == 1 { "" } else { "s" }),
                    );
                }
            });
        });
        if let Some(e) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
        ui.add_space(2.0);

        let h = ui.available_height();
        egui::TopBottomPanel::bottom(egui::Id::new(("conflict_output", &self.file.path)))
            .resizable(true)
            .default_height(h * 0.45)
            .min_height(120.0)
            .show_inside(ui, |ui| self.output(ui));
        egui::CentralPanel::default().show_inside(ui, |ui| self.sides(ui));
    }

    fn sides(&mut self, ui: &mut egui::Ui) {
        let font = FontId::monospace(13.5);
        let cw = ui.fonts(|f| f.glyph_width(&font, 'M'));
        let row_h = ui.fonts(|f| f.row_height(&font)) + 4.0;
        let dark = ui.visuals().dark_mode;
        let fg = ui.visuals().text_color();
        let weak = ui.visuals().weak_text_color();
        let line_col = Color32::from_rgba_unmultiplied(128, 128, 128, 80);
        let gutter = 22.0 + cw * 5.0 + 10.0;
        let view_w = ui.available_width();
        let split = self.split_scroll;
        // One pane's width: when A and B scroll together they share a wide canvas, otherwise each fits its half.
        let pane_w = if split {
            (view_w / 2.0).max(gutter + 60.0)
        } else {
            (view_w / 2.0).max(gutter + (self.side_chars[0].max(self.side_chars[1]) as f32 + 2.0) * cw)
        };
        let head_w = if split { pane_w } else { view_w / 2.0 };
        let n = self.first_row.len();

        // Headers: a check box that takes that version for every conflict, and whose version it is.
        let mut take_all: Option<(Side, bool)> = None;
        let (hrect, hresp) = ui.allocate_exact_size(Vec2::new(view_w, 28.0), Sense::click());
        let hp = ui.painter_at(hrect);
        hp.rect_filled(hrect, 0.0, ui.visuals().faint_bg_color);
        for (s, side, letter, label, col) in [
            (0usize, Side::Ours, "A", &self.file.ours_label, A_COL),
            (1, Side::Theirs, "B", &self.file.theirs_label, B_COL),
        ] {
            let x0 = hrect.left() + s as f32 * head_w;
            let all = n > 0 && (0..n).all(|c| self.hunk_on(c, side));
            let cb = Rect::from_center_size(Pos2::new(x0 + 14.0, hrect.center().y), Vec2::splat(13.0));
            hp.rect_stroke(cb, 2.0, Stroke::new(1.2_f32, weak), egui::StrokeKind::Inside);
            if all {
                hp.rect_filled(cb.shrink(2.5), 1.0, rgb(col));
            }
            let badge = Rect::from_center_size(Pos2::new(x0 + 38.0, hrect.center().y), Vec2::splat(18.0));
            hp.rect_stroke(badge, 3.0, Stroke::new(1.5_f32, rgb(col)), egui::StrokeKind::Inside);
            hp.text(badge.center(), egui::Align2::CENTER_CENTER, letter, FontId::proportional(13.0), rgb(col));
            hp.text(
                Pos2::new(x0 + 54.0, hrect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                FontId::proportional(13.5),
                ui.visuals().strong_text_color(),
            );
            if hresp.clicked() && hresp.interact_pointer_pos().is_some_and(|p| p.x >= x0 && p.x < x0 + head_w) {
                take_all = Some((side, !all));
            }
        }
        hp.vline(hrect.left() + head_w, hrect.y_range(), Stroke::new(1.0_f32, line_col));
        hresp.on_hover_text("Click a side to take it for every conflict");
        if let Some((side, on)) = take_all {
            for c in 0..n {
                self.set_hunk(c, side, on);
            }
        }

        let mut hoff = self.hoff;
        let mut voff = self.voff;
        let jump = self.scroll_sides.take().map(|r| (r as f32 - 3.0).max(0.0) * row_h);
        let mut hunk_click: Option<(usize, Side)> = None;
        let mut line_click: Option<(usize, Side, usize)> = None;
        let path = self.file.path.clone();
        let sides_id = self.file.path.clone();
        {
            // One row: both versions' cells, and what a click at `click` picked.
            let mut paint_row = |ui: &egui::Ui, p: &egui::Painter, i: usize, rect: Rect, click: Option<Pos2>, hovered: bool, lane: f32, hoff: [f32; 2]| {
                let cy = rect.center().y;
                for (s, side, col) in [(0usize, Side::Ours, A_COL), (1, Side::Theirs, B_COL)] {
                    let x0 = rect.left() + s as f32 * pane_w;
                    let cell = Rect::from_min_size(Pos2::new(x0, rect.top()), Vec2::new(pane_w, row_h));
                    let hit = Rect::from_min_max(cell.min, Pos2::new(cell.max.x - lane, cell.max.y));
                    let cp = p.with_clip_rect(cell.intersect(p.clip_rect()));
                    let text_clip = Rect::from_min_max(Pos2::new(x0 + gutter - 2.0, cell.top()), Pos2::new(cell.max.x - lane, cell.max.y));
                    let tp = p.with_clip_rect(text_clip.intersect(p.clip_rect()));
                    let shift = if split { hoff[s] } else { 0.0 };
                    let text: Option<&str> = match self.rows[i] {
                        Row::Common(ri, k) => match &self.file.regions[ri] {
                            Region::Common(l) => Some(&l[k]),
                            Region::Conflict { .. } => None,
                        },
                        Row::Pair(c, k) => {
                            let lines = self.side_lines(c, side);
                            let on = lines.get(k).is_some() && self.res.picks[c].flags(side).get(k).copied().unwrap_or(false);
                            let current = c == self.current;
                            if k < lines.len() {
                                cp.rect_filled(cell, 0.0, tint(col.0, col.1, col.2, if on { 95 } else { 34 }));
                                if on || current {
                                    cp.rect_filled(Rect::from_min_size(cell.min, Vec2::new(3.0, row_h)), 0.0, rgb(col));
                                }
                            } else {
                                cp.rect_filled(cell, 0.0, Color32::from_rgba_unmultiplied(128, 128, 128, 14));
                            }
                            // The hunk's check box sits in the gutter of its first row.
                            if k == 0 {
                                let on = self.hunk_on(c, side);
                                let cb = Rect::from_center_size(Pos2::new(x0 + 12.0, cy), Vec2::splat(12.0));
                                cp.rect_stroke(cb, 2.0, Stroke::new(1.2_f32, if on { rgb(col) } else { weak }), egui::StrokeKind::Inside);
                                if on {
                                    cp.rect_filled(cb.shrink(2.5), 1.0, rgb(col));
                                }
                            }
                            if let Some(pt) = click.filter(|pt| hit.contains(*pt)) {
                                if k == 0 && pt.x < x0 + 24.0 {
                                    hunk_click = Some((c, side));
                                } else if k < lines.len() {
                                    line_click = Some((c, side, k));
                                }
                            }
                            lines.get(k).map(String::as_str)
                        }
                    };
                    if let Some(n) = self.nums[i][s] {
                        cp.text(Pos2::new(x0 + gutter - 10.0, cy), egui::Align2::RIGHT_CENTER, n, font.clone(), weak);
                    }
                    if let Some(t) = text {
                        let job = syntax::highlight(&path, t, fg, &font, dark);
                        let galley = ui.fonts(|f| f.layout_job(job));
                        tp.galley(Pos2::new(x0 + gutter - shift, cy - galley.size().y / 2.0), galley, fg);
                    }
                }
                if matches!(self.rows[i], Row::Pair(..)) && hovered {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            };

            if split {
                // Each version has its own scroll bars, but they move together: the same row and the
                // same column (as far as the shorter version reaches).
                let full = ui.available_rect_before_wrap();
                let ar = ui.allocate_rect(full, Sense::click());
                let content_h = self.rows.len() as f32 * row_h;
                let content_w = [0, 1].map(|s| (self.side_chars[s] + 2) as f32 * cw);
                let (mut v_lane, mut h_lane) = (0.0_f32, 0.0_f32);
                for _ in 0..2 {
                    h_lane = if (0..2).any(|s| content_w[s] > pane_w - v_lane - gutter) { BAR } else { 0.0 };
                    v_lane = if content_h > full.height() - h_lane { BAR } else { 0.0 };
                }
                let view_w = pane_w - v_lane;
                let view_h = (full.height() - h_lane).max(1.0);
                let max_off = [0, 1].map(|s| (content_w[s] - (view_w - gutter)).max(0.0));
                let max_v = (content_h - view_h).max(0.0);
                if let Some(j) = jump {
                    voff = j;
                }
                let viewport = Rect::from_min_size(full.min, Vec2::new(full.width(), view_h));
                let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|pt| viewport.contains(*pt));
                if let Some(pt) = pointer {
                    let d = ui.input(|i| i.smooth_scroll_delta);
                    if d != Vec2::ZERO {
                        let s = usize::from(pt.x >= full.left() + pane_w);
                        voff -= d.y;
                        hoff[s] -= d.x;
                        follow_cols(&mut hoff, s, max_off);
                    }
                }
                voff = voff.clamp(0.0, max_v);
                hoff = [hoff[0].min(max_off[0]), hoff[1].min(max_off[1])];

                let p = ui.painter_at(viewport);
                let click = ar.clicked().then(|| ar.interact_pointer_pos()).flatten();
                let first = (voff / row_h).floor() as usize;
                let last = (((voff + view_h) / row_h).ceil() as usize).min(self.rows.len());
                for i in first..last {
                    let rect = Rect::from_min_size(
                        Pos2::new(full.left(), full.top() + i as f32 * row_h - voff),
                        Vec2::new(pane_w * 2.0, row_h),
                    );
                    let hovered = pointer.is_some_and(|pt| rect.contains(pt));
                    paint_row(ui, &p, i, rect, click, hovered, v_lane, hoff);
                }
                ui.painter_at(full).vline(full.left() + pane_w, full.y_range(), Stroke::new(1.0_f32, line_col));

                let mut driver = None;
                for s in 0..2 {
                    let px = full.left() + s as f32 * pane_w;
                    if v_lane > 0.0 {
                        let track = Rect::from_min_size(Pos2::new(px + pane_w - BAR, full.top()), Vec2::new(BAR, view_h));
                        scrollbar(ui, egui::Id::new(("conflict_vbar", &sides_id, s)), track, true, &mut voff, max_v, view_h, content_h);
                    }
                    if h_lane > 0.0 {
                        let track = Rect::from_min_size(Pos2::new(px, full.top() + view_h), Vec2::new(view_w, BAR));
                        let before = hoff[s];
                        scrollbar(ui, egui::Id::new(("conflict_hbar", &sides_id, s)), track, false, &mut hoff[s], max_off[s], view_w - gutter, content_w[s]);
                        if hoff[s] != before {
                            driver = Some(s);
                        }
                    }
                    if v_lane > 0.0 && h_lane > 0.0 {
                        let corner = Rect::from_min_size(Pos2::new(px + pane_w - BAR, full.top() + view_h), Vec2::splat(BAR));
                        ui.painter().rect_filled(corner, 0.0, ui.visuals().extreme_bg_color);
                    }
                }
                if let Some(s) = driver {
                    follow_cols(&mut hoff, s, max_off);
                }
            } else {
                let mut area = egui::ScrollArea::both().id_salt(("conflict_sides", &self.file.path)).auto_shrink(false);
                if let Some(j) = jump {
                    area = area.vertical_scroll_offset(j);
                }
                // show_rows measures a row with the spacing in force when it starts, so rows must be
                // drawn with that same (zero) spacing or the highlights drift against the scroll position.
                ui.spacing_mut().item_spacing.y = 0.0;
                area.show_rows(ui, row_h, self.rows.len(), |ui, range| {
                    let content_left = ui.max_rect().left();
                    for i in range {
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(pane_w * 2.0, row_h), Sense::click());
                        let p = ui.painter_at(rect);
                        let click = resp.clicked().then(|| resp.interact_pointer_pos()).flatten();
                        paint_row(ui, &p, i, rect, click, resp.hovered(), 0.0, [0.0; 2]);
                    }
                    // The divider runs the full height, also below a short file.
                    let clip = ui.clip_rect();
                    ui.painter().vline(content_left + pane_w, clip.y_range(), Stroke::new(1.0_f32, line_col));
                });
            }
        }
        self.hoff = hoff;
        self.voff = voff;
        if let Some((c, side)) = hunk_click {
            let on = self.hunk_on(c, side);
            self.set_hunk(c, side, !on);
        }
        if let Some((c, side, k)) = line_click {
            self.toggle_line(c, side, k);
        }
    }

    fn output(&mut self, ui: &mut egui::Ui) {
        let font = FontId::monospace(13.5);
        let row_h = ui.fonts(|f| f.row_height(&font));
        let cw = ui.fonts(|f| f.glyph_width(&font, 'M'));
        let dark = ui.visuals().dark_mode;
        let fg = ui.visuals().text_color();
        let weak = ui.visuals().weak_text_color();
        let n = self.ranges.len();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Output").size(15.0).strong());
            ui.add_space(16.0);
            if n > 0 {
                ui.colored_label(rgb(OUT_COL), format!("conflict {} of {}", self.current.min(n - 1) + 1, n));
                if crate::icons::icon_button(ui, crate::icons::Icon::ArrowUp, false, "Previous conflict", true).clicked() {
                    self.goto(self.current.saturating_sub(1));
                }
                if crate::icons::icon_button(ui, crate::icons::Icon::ArrowDown, false, "Next conflict", true).clicked() {
                    self.goto((self.current + 1).min(n - 1));
                }
            } else {
                ui.weak("no conflicting lines");
            }
            ui.weak("  ·  edit freely; picking a hunk above rewrites only that conflict");
        });
        ui.separator();

        let mut area = egui::ScrollArea::both().id_salt(("conflict_out", &self.file.path)).auto_shrink(false);
        if let Some(line) = self.scroll_out.take() {
            area = area.vertical_scroll_offset((line as f32 - 3.0).max(0.0) * row_h);
        }
        let path = self.file.path.clone();
        let before = self.out.clone();
        let ranges = self.ranges.clone();
        let current = self.current;
        let mut changed = false;
        area.show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let n_lines = self.out.split('\n').count();
                let gutter_w = cw * 6.0 + 12.0;
                let (grect, _) = ui.allocate_exact_size(Vec2::new(gutter_w, n_lines as f32 * row_h), Sense::hover());
                let p = ui.painter().clone();
                // The bands go under the text but are measured from the text's own rows, which are
                // only known once it is laid out, so their place is reserved now and filled in below.
                let under = p.add(egui::Shape::Noop);

                let font2 = font.clone();
                let mut layouter = move |ui: &egui::Ui, text: &str, _wrap: f32| -> Arc<egui::Galley> {
                    let mut job = LayoutJob::default();
                    // Very large outputs skip colouring: it is redone on every keystroke.
                    if text.len() > 400_000 {
                        job.append(text, 0.0, TextFormat::simple(font2.clone(), fg));
                    } else {
                        for (i, line) in text.split('\n').enumerate() {
                            if i > 0 {
                                job.append("\n", 0.0, TextFormat::simple(font2.clone(), fg));
                            }
                            let lj = syntax::highlight(&path, line, fg, &font2, dark);
                            let off = job.text.len();
                            job.text.push_str(&lj.text);
                            for mut s in lj.sections {
                                s.byte_range = s.byte_range.start + off..s.byte_range.end + off;
                                job.sections.push(s);
                            }
                        }
                    }
                    job.wrap.max_width = f32::INFINITY;
                    ui.fonts(|f| f.layout_job(job))
                };
                let r = egui::TextEdit::multiline(&mut self.out)
                    .id_salt(("conflict_out_text", &self.file.path))
                    .font(font.clone())
                    .code_editor()
                    .frame(false)
                    .margin(egui::Margin::ZERO)
                    .desired_width(f32::INFINITY)
                    .desired_rows(n_lines)
                    .layouter(&mut layouter)
                    .show(ui);
                changed = r.response.changed();

                // Vertical extent of a line, as the editor really laid it out.
                let top = r.galley_pos.y;
                let span = |line: usize| match r.galley.rows.get(line) {
                    Some(row) => (top + row.rect.min.y, top + row.rect.max.y),
                    None => (top + line as f32 * row_h, top + (line + 1) as f32 * row_h),
                };
                let mut shapes = Vec::new();
                for (c, &(start, len)) in ranges.iter().enumerate() {
                    let alpha = if c == current { 80 } else { 45 };
                    for line in start..start + len {
                        let (y0, y1) = span(line);
                        let band = Rect::from_min_max(Pos2::new(grect.left(), y0), Pos2::new(grect.left() + 100_000.0, y1));
                        shapes.push(egui::Shape::rect_filled(band, 0.0, tint(OUT_COL.0, OUT_COL.1, OUT_COL.2, alpha)));
                        shapes.push(egui::Shape::rect_filled(
                            Rect::from_min_size(band.min, Vec2::new(3.0, y1 - y0)),
                            0.0,
                            rgb(OUT_COL),
                        ));
                    }
                    if len == 0 {
                        // An empty resolution: a thin marker where its lines would go.
                        let y = span(start).0;
                        shapes.push(egui::Shape::hline(grect.left()..=grect.left() + 100_000.0, y, Stroke::new(2.0_f32, rgb(OUT_COL))));
                    }
                }
                p.set(under, egui::Shape::Vec(shapes));

                let clip = ui.clip_rect();
                for line in 0..n_lines.min(r.galley.rows.len().max(1)) {
                    let (y0, y1) = span(line);
                    if y1 < clip.top() || y0 > clip.bottom() {
                        continue;
                    }
                    p.text(Pos2::new(grect.right() - 10.0, (y0 + y1) / 2.0), egui::Align2::RIGHT_CENTER, line + 1, font.clone(), weak);
                }
            });
        });
        if changed {
            self.follow_edit(&before);
        }
    }

    /// Keep the conflict ranges on the same lines after the output was edited by hand.
    fn follow_edit(&mut self, before: &str) {
        let old: Vec<&str> = before.split('\n').collect();
        let new: Vec<&str> = self.out.split('\n').collect();
        let previous = self.ranges.clone();
        conflict::remap_ranges(&old, &new, &mut self.ranges);
        // Typing inside a conflict counts as deciding it.
        for (c, (&(os, ol), &(ns, nl))) in previous.iter().zip(self.ranges.iter()).enumerate() {
            if old.get(os..os + ol) != new.get(ns..ns + nl) {
                let p = &mut self.res.picks[c];
                if !p.touched {
                    p.touched = true;
                    p.ours.iter_mut().for_each(|x| *x = false);
                    p.theirs.iter_mut().for_each(|x| *x = false);
                    p.order.clear();
                }
                self.current = c;
            }
        }
    }
}

/// The pick flags of one side.
trait Flags {
    fn flags(&self, side: Side) -> &[bool];
}

impl Flags for conflict::Pick {
    fn flags(&self, side: Side) -> &[bool] {
        match side {
            Side::Ours => &self.ours,
            Side::Theirs => &self.theirs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitgoing_core::{branches, status};

    /// A repository stopped in a merge with two conflicts in `f`; returns the editor for it.
    fn editor(dir: &Path) -> ConflictEditor {
        let git = |args: &[&str]| gitgoing_core::gitcmd::run(Some(dir), args).unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "user.email", "t@example.com"]);
        let w = |c: &str, m: &str| {
            std::fs::write(dir.join("f"), c).unwrap();
            status::stage(dir, &["f".into()]).unwrap();
            status::commit(dir, m).unwrap();
        };
        w("a\nx\nb\nc\nd\ny\ne\n", "base");
        let main = git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim().to_owned();
        branches::create_and_checkout(dir, "feature").unwrap();
        w("a\nB1\nb\nc\nd\nB2\ne\n", "feature");
        branches::checkout(dir, &main).unwrap();
        w("a\nA1\nA1b\nb\nc\nd\nA2\ne\n", "main");
        gitgoing_core::merge::merge(dir, "feature").unwrap();
        ConflictEditor::open(dir, "f").unwrap()
    }

    #[test]
    fn output_starts_from_the_ancestor_and_picks_rewrite_only_their_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let mut ed = editor(dir.path());
        assert_eq!(ed.out, "a\nx\nb\nc\nd\ny\ne\n");
        assert_eq!(ed.ranges, vec![(1, 1), (5, 1)]);

        ed.set_hunk(0, Side::Ours, true); // A's two lines push the second conflict down
        assert_eq!(ed.out, "a\nA1\nA1b\nb\nc\nd\ny\ne\n");
        assert_eq!(ed.ranges, vec![(1, 2), (6, 1)]);

        ed.set_hunk(1, Side::Theirs, true);
        ed.toggle_line(0, Side::Theirs, 0); // B's line goes after A's
        assert_eq!(ed.out, "a\nA1\nA1b\nB1\nb\nc\nd\nB2\ne\n");
        assert!(ed.hunk_on(0, Side::Ours) && ed.hunk_on(0, Side::Theirs));

        ed.set_hunk(0, Side::Ours, false);
        assert_eq!(ed.out, "a\nB1\nb\nc\nd\nB2\ne\n");
        assert_eq!(ed.ranges, vec![(1, 1), (5, 1)]);
    }

    #[test]
    fn hand_edits_keep_conflicts_in_place_and_survive_later_picks() {
        let dir = tempfile::tempdir().unwrap();
        let mut ed = editor(dir.path());
        // Type a new first line, and change a common line.
        let before = ed.out.clone();
        ed.out = "top\na\nx\nb\nC!\nd\ny\ne\n".to_owned();
        ed.follow_edit(&before);
        assert_eq!(ed.ranges, vec![(2, 1), (6, 1)]);
        assert_eq!(ed.res.unresolved_count(), 2, "edits outside conflicts decide nothing");

        // Picking a hunk keeps the hand edits elsewhere.
        ed.set_hunk(1, Side::Theirs, true);
        assert_eq!(ed.out, "top\na\nx\nb\nC!\nd\nB2\ne\n");

        // Typing inside a conflict decides it.
        let before = ed.out.clone();
        ed.out = "top\na\nmine\nb\nC!\nd\nB2\ne\n".to_owned();
        ed.follow_edit(&before);
        assert_eq!(ed.res.unresolved_count(), 0);
        assert_eq!(ed.ranges[0], (2, 1));
    }
}
