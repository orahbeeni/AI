//! The diff pane: a toolbar plus a unified or side-by-side diff (or the whole
//! file), with per-hunk stage/unstage buttons.

use crate::icons::{self, Icon};
use crate::syntax;
use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use gitgoing_core::diff::{self, ChangeKind, DiffLine, FileDiff, LineKind};

static DEFAULT_SPLIT: AtomicBool = AtomicBool::new(false);
static DEFAULT_IGNORE_WS: AtomicBool = AtomicBool::new(false);

/// Set what newly opened diffs start with (from the user's settings).
pub fn set_defaults(split: bool, ignore_ws: bool) {
    DEFAULT_SPLIT.store(split, Ordering::Relaxed);
    DEFAULT_IGNORE_WS.store(ignore_ws, Ordering::Relaxed);
}

/// A see-through colour that blends in plain sRGB. `Color32::from_rgba_unmultiplied`
/// premultiplies in linear space, which makes low alphas far stronger than they read.
fn tint(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    let m = |c: u8| (c as u16 * a as u16 / 255) as u8;
    Color32::from_rgba_premultiplied(m(r), m(g), m(b), a)
}

impl Default for DiffOpts {
    fn default() -> Self {
        DiffOpts {
            file_view: false,
            split: DEFAULT_SPLIT.load(Ordering::Relaxed),
            ignore_ws: DEFAULT_IGNORE_WS.load(Ordering::Relaxed),
            selected: HashSet::new(),
            current_hunk: 0,
            scroll_to_hunk: None,
            text_sel: None,
        }
    }
}

/// Per-pane display options and navigation state.
pub struct DiffOpts {
    /// Show the whole file instead of a diff.
    pub file_view: bool,
    /// Side-by-side instead of unified.
    pub split: bool,
    /// Ignore whitespace changes (re-reads the diff).
    pub ignore_ws: bool,
    /// Selected changed lines, as (hunk, line within hunk), for line staging.
    pub selected: HashSet<(u32, u32)>,
    current_hunk: usize,
    scroll_to_hunk: Option<usize>,
    /// Text highlighted by dragging, to copy.
    text_sel: Option<TextSel>,
}

/// A drag-selected span of code text. Positions are (row, character column);
/// `side` is the half in side-by-side view (0 in unified view).
struct TextSel {
    title: String,
    side: usize,
    anchor: (usize, usize),
    cursor: (usize, usize),
    dragging: bool,
}

impl TextSel {
    fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.cursor { (self.anchor, self.cursor) } else { (self.cursor, self.anchor) }
    }

    /// The selected character range of a row's text, if any.
    fn range(&self, title: &str, row: usize, side: usize, len: usize) -> Option<(usize, usize)> {
        if self.title != title || self.side != side {
            return None;
        }
        let (a, b) = self.ordered();
        if row < a.0 || row > b.0 {
            return None;
        }
        let start = if row == a.0 { a.1 } else { 0 }.min(len);
        let end = if row == b.0 { b.1 } else { len }.min(len);
        (end > start).then_some((start, end))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HunkBtn {
    Stage,
    Unstage,
}

pub struct Header<'a> {
    /// File path shown above the toolbar.
    pub title: &'a str,
    pub kind: Option<ChangeKind>,
    /// Small status chip, e.g. "Staged".
    pub tag: &'a str,
    /// Label of the edit button, or `None` to hide it.
    pub edit_label: Option<&'a str>,
    /// Per-hunk button, if hunks can be (un)staged here.
    pub hunk_btn: Option<HunkBtn>,
}

#[derive(Default)]
pub struct Out {
    pub close: bool,
    pub edit: bool,
    /// An option that changes the data (file view, whitespace) was toggled.
    pub refetch: bool,
    /// Index of the hunk whose stage/unstage button was clicked.
    pub hunk: Option<usize>,
    /// Index of the hunk whose discard button was clicked.
    pub discard_hunk: Option<usize>,
    /// The selected lines were staged (or unstaged) via the selection bar.
    pub lines: Option<Vec<(u32, u32)>>,
}

pub fn show(
    ui: &mut egui::Ui,
    opts: &mut DiffOpts,
    diff: &Option<Result<FileDiff, String>>,
    loading: bool,
    h: &Header,
) -> Out {
    let mut out = Out::default();
    // Title: change icon, directory dimmed, file name bright.
    ui.horizontal(|ui| {
        if let Some(k) = h.kind {
            icons::change_badge(ui, k);
        }
        let (dir, name) = match h.title.rsplit_once('/') {
            Some((d, n)) => (format!("{d}/"), n),
            None => (String::new(), h.title),
        };
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(RichText::new(dir).size(16.0).color(ui.visuals().weak_text_color()));
        ui.label(RichText::new(name).size(16.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Close diff").clicked() {
                out.close = true;
            }
        });
    });
    ui.add_space(4.0);

    let n_hunks = match diff {
        Some(Ok(FileDiff::Text { lines, .. })) if !opts.file_view => {
            lines.iter().filter(|l| l.kind == LineKind::Hunk).count()
        }
        _ => 0,
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if let Some(label) = h.edit_label {
            if icons::outlined_button(ui, Icon::Pencil, label).clicked() {
                out.edit = true;
            }
        }
        ui.add_space(12.0);
        icons::chip(ui, h.tag);
        match icons::segmented(ui, &["File View", "Diff View"], Some(opts.file_view as usize), true) {
            Some(0) if !opts.file_view => {
                opts.file_view = true;
                out.refetch = true;
            }
            Some(1) if opts.file_view => {
                opts.file_view = false;
                out.refetch = true;
            }
            _ => {}
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::icon_button(ui, Icon::Pilcrow, opts.ignore_ws, "Ignore whitespace changes", !opts.file_view).clicked() {
                opts.ignore_ws = !opts.ignore_ws;
                out.refetch = true;
            }
            if icons::icon_button(ui, Icon::Split, opts.split, "Side-by-side view", true).clicked() {
                opts.split = true;
            }
            if icons::icon_button(ui, Icon::Unified, !opts.split, "Unified view", true).clicked() {
                opts.split = false;
            }
            ui.separator();
            let nav = n_hunks > 0;
            if icons::icon_button(ui, Icon::ArrowDown, false, "Next change", nav).clicked() {
                opts.current_hunk = (opts.current_hunk + 1).min(n_hunks - 1);
                opts.scroll_to_hunk = Some(opts.current_hunk);
            }
            if icons::icon_button(ui, Icon::ArrowUp, false, "Previous change", nav).clicked() {
                opts.current_hunk = opts.current_hunk.saturating_sub(1);
                opts.scroll_to_hunk = Some(opts.current_hunk);
            }
            ui.separator();
            icons::segmented(ui, &["Blame", "History"], None, false);
        });
    });
    ui.add_space(2.0);

    let selectable = h.hunk_btn.is_some() && !opts.file_view && !opts.ignore_ws;
    if selectable && !opts.selected.is_empty() {
        ui.horizontal(|ui| {
            let n = opts.selected.len();
            ui.label(format!("{n} line{} selected", if n == 1 { "" } else { "s" }));
            let label = if h.hunk_btn == Some(HunkBtn::Stage) { "Stage Selected Lines" } else { "Unstage Selected Lines" };
            if ui.button(label).clicked() {
                let mut ids: Vec<_> = opts.selected.iter().copied().collect();
                ids.sort_unstable();
                out.lines = Some(ids);
            }
            if ui.button("Clear").clicked() {
                opts.selected.clear();
            }
        });
        ui.add_space(2.0);
    }

    match diff {
        None if loading => {
            ui.spinner();
        }
        None => {}
        Some(Err(e)) => {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
        Some(Ok(FileDiff::Binary)) => {
            ui.weak("Binary file not shown.");
        }
        Some(Ok(FileDiff::Text { lines, truncated, max_chars })) => {
            if lines.is_empty() {
                ui.weak("No textual changes.");
            } else {
                let btn = if opts.file_view || opts.ignore_ws { None } else { h.hunk_btn };
                let b = body(ui, opts, lines, BodyCfg { title: h.title, truncated: *truncated, max_chars: *max_chars, hunk_btn: btn, selectable });
                out.hunk = b.0;
                out.discard_hunk = b.1;
            }
        }
    }
    out
}

/// Pair removed and added lines side by side. Each row holds the left (old)
/// and right (new) line indices; also returns the row of each hunk header.
fn split_rows(lines: &[DiffLine]) -> (Vec<[Option<usize>; 2]>, Vec<usize>) {
    let mut rows = Vec::with_capacity(lines.len());
    let mut hunk_rows = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind {
            LineKind::Hunk => {
                hunk_rows.push(rows.len());
                rows.push([Some(i), None]);
                i += 1;
            }
            LineKind::Context | LineKind::Meta => {
                rows.push([Some(i), Some(i)]);
                i += 1;
            }
            LineKind::Del | LineKind::Add => {
                let ds = i;
                while i < lines.len() && lines[i].kind == LineKind::Del {
                    i += 1;
                }
                let de = i;
                while i < lines.len() && lines[i].kind == LineKind::Add {
                    i += 1;
                }
                let (nd, na) = (de - ds, i - de);
                for k in 0..nd.max(na) {
                    rows.push([(k < nd).then_some(ds + k), (k < na).then_some(de + k)]);
                }
            }
        }
    }
    (rows, hunk_rows)
}

/// Pair each removed line with the added line at the same position in its
/// change block, so word-level differences can be highlighted.
fn partners(lines: &[DiffLine]) -> Vec<u32> {
    let mut partner = vec![u32::MAX; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Del {
            i += 1;
            continue;
        }
        let ds = i;
        while i < lines.len() && lines[i].kind == LineKind::Del {
            i += 1;
        }
        let de = i;
        while i < lines.len() && lines[i].kind == LineKind::Add {
            i += 1;
        }
        for k in 0..(de - ds).min(i - de) {
            partner[ds + k] = (de + k) as u32;
            partner[de + k] = (ds + k) as u32;
        }
    }
    partner
}

/// The span (in characters) of `a` that differs from `b`, found by trimming the
/// common prefix and suffix. `None` if nothing differs or the whole line does.
fn changed_range(a: &str, b: &str) -> Option<(usize, usize)> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let max_suffix = a.len().min(b.len()) - prefix;
    let suffix = a.iter().rev().zip(b.iter().rev()).take(max_suffix).take_while(|(x, y)| x == y).count();
    let (start, end) = (prefix, a.len() - suffix);
    (end > start && !(start == 0 && end == a.len())).then_some((start, end))
}

/// What the diff body needs to know beyond the lines themselves.
struct BodyCfg<'a> {
    title: &'a str,
    truncated: bool,
    max_chars: usize,
    hunk_btn: Option<HunkBtn>,
    /// Lines can be ticked for line staging.
    selectable: bool,
}

fn body(
    ui: &mut egui::Ui,
    opts: &mut DiffOpts,
    lines: &[DiffLine],
    cfg: BodyCfg,
) -> (Option<usize>, Option<usize>) {
    let BodyCfg { title, truncated, max_chars, hunk_btn, selectable } = cfg;
    let font = FontId::monospace(13.5);
    let cw = ui.fonts(|f| f.glyph_width(&font, 'M'));
    let row_h = ui.fonts(|f| f.row_height(&font)) + 5.0;
    let dark = ui.visuals().dark_mode;
    let fg = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    // Muted tints like GitKraken's: dark green / dark red rows, with a stronger
    // tint on the exact characters that changed.
    let add_bg = tint(0x4c, 0xaf, 0x50, 30);
    let del_bg = tint(0xe5, 0x53, 0x4b, 30);
    let add_hi = tint(0x4c, 0xaf, 0x50, 80);
    let del_hi = tint(0xe5, 0x53, 0x4b, 80);
    let empty_bg = Color32::from_rgba_unmultiplied(128, 128, 128, 14);
    let line_col = Color32::from_rgba_unmultiplied(128, 128, 128, 80);
    let sign_green = Color32::from_rgb(0x4c, 0xaf, 0x50);
    let sign_red = Color32::from_rgb(0xe5, 0x53, 0x4b);

    let num_w = cw * 5.0;
    // Unified gutter: [old no] [new no][+/-] | code
    let old_right = 12.0 + num_w;
    let new_right = old_right + 14.0 + num_w;
    let sign_right = new_right + cw + 8.0;
    let sep_x = sign_right + 4.0;
    let text_x = sep_x + 10.0;

    let (rows, hunk_rows) = if opts.split {
        split_rows(lines)
    } else {
        (Vec::new(), (0..lines.len()).filter(|&i| lines[i].kind == LineKind::Hunk).collect())
    };
    let partner = partners(lines);
    let n_rows = if opts.split { rows.len() } else { lines.len() };
    let total = n_rows + truncated as usize;

    let viewport_w = ui.available_width();
    // Split view: each half has its own gutter (one number) and text.
    let half_gutter = 12.0 + num_w + 12.0;
    let half_w = (viewport_w / 2.0).max(half_gutter + (max_chars as f32 + 2.0) * cw);
    let width = if opts.split {
        half_w * 2.0
    } else {
        viewport_w.max(text_x + (max_chars as f32 + 2.0) * cw)
    };

    let mut area = egui::ScrollArea::both()
        .id_salt(("diff", title, opts.file_view, opts.split))
        .auto_shrink(false);
    if let Some(h) = opts.scroll_to_hunk.take() {
        if let Some(&row) = hunk_rows.get(h) {
            area = area.vertical_scroll_offset(row as f32 * row_h);
        }
    }

    // Draw one line of code at `x`, with its changed characters tinted first.
    let code = |p: &egui::Painter, rect: Rect, x: f32, idx: usize| {
        let l = &lines[idx];
        if let Some(&q) = partner.get(idx).filter(|&&q| q != u32::MAX) {
            if let Some((s, e)) = changed_range(&l.text, &lines[q as usize].text) {
                let hi = if l.kind == LineKind::Add { add_hi } else { del_hi };
                let r = Rect::from_min_max(
                    Pos2::new(x + s as f32 * cw, rect.top()),
                    Pos2::new(x + e as f32 * cw, rect.bottom()),
                );
                p.rect_filled(r, 0.0, hi);
            }
        }
        let color = if l.kind == LineKind::Meta { weak } else { fg };
        let galley = p.layout_job(syntax::highlight(title, &l.text, color, &font, dark));
        p.galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, color);
    };

    // (hunk, index within hunk) of every line, to identify selections.
    let mut line_ids = Vec::with_capacity(lines.len());
    let mut within = 0u32;
    for l in lines {
        if l.kind == LineKind::Hunk {
            within = 0;
        } else {
            within += 1;
        }
        line_ids.push((l.hunk, within.saturating_sub(1)));
    }
    let split = opts.split;
    let code_x = |left: f32, side: usize| if split { left + side as f32 * half_w + half_gutter } else { left + text_x };
    // The diff line shown in a row's half, if it has selectable text.
    let cell_of = |row: usize, side: usize| -> Option<usize> {
        let k = if split { rows.get(row)?[side] } else { (row < lines.len() && side == 0).then_some(row) }?;
        (lines[k].kind != LineKind::Hunk).then_some(k)
    };
    let pos_at = |pp: Pos2, side: usize, origin: (f32, f32)| -> (usize, usize) {
        let row = (((pp.y - origin.1) / row_h).floor().max(0.0) as usize).min(n_rows.saturating_sub(1));
        let col = ((pp.x - code_x(origin.0, side)) / cw).round().max(0.0) as usize;
        (row, col)
    };
    let selected_text = |t: &TextSel| -> String {
        let (a, b) = t.ordered();
        let mut out = Vec::new();
        for row in a.0..=b.0 {
            let Some(k) = cell_of(row, t.side) else { continue };
            let text = &lines[k].text;
            let len = text.chars().count();
            let start = if row == a.0 { a.1 } else { 0 }.min(len);
            let end = if row == b.0 { b.1 } else { len }.min(len);
            if end >= start {
                out.push(text.chars().skip(start).take(end - start).collect::<String>());
            }
        }
        out.join("\n")
    };
    let text_hi = tint(0x4e, 0xa8, 0xde, 90);
    let toggle = |sel: &mut HashSet<(u32, u32)>, idx: usize| {
        if matches!(lines[idx].kind, LineKind::Add | LineKind::Del) && !sel.remove(&line_ids[idx]) {
            sel.insert(line_ids[idx]);
        }
    };
    let selection = tint(0x4e, 0xa8, 0xde, 40);

    let mut clicked = None;
    let mut discard = None;
    let mut origin: Option<(f32, f32)> = None;
    if opts.text_sel.as_ref().is_some_and(|t| t.title != title) {
        opts.text_sel = None;
    }
    area.show_rows(ui, row_h, total, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let visible = ui.clip_rect();
        for i in range {
            let (rect, row_resp) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::click_and_drag());
            let row_origin = (rect.left(), rect.top() - i as f32 * row_h);
            origin = Some(row_origin);
            let p = ui.painter_at(rect);
            let cy = rect.center().y;
            if i >= n_rows {
                p.text(
                    Pos2::new(rect.left() + 12.0, cy),
                    egui::Align2::LEFT_CENTER,
                    format!("… diff truncated at {} lines", diff::MAX_DIFF_LINES),
                    font.clone(),
                    weak,
                );
                continue;
            }

            let cells: [Option<usize>; 2] = if opts.split { rows[i] } else { [Some(i), None] };

            // Hunk header row (same in both modes).
            if let Some(hi) = cells[0].filter(|&k| lines[k].kind == LineKind::Hunk) {
                let l = &lines[hi];
                p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, line_col));
                // The text stops short of the buttons pinned to the visible right edge.
                let labels: &[&str] = match hunk_btn {
                    Some(HunkBtn::Stage) => &["Stage Hunk", "Discard Hunk"],
                    Some(HunkBtn::Unstage) => &["Unstage Hunk"],
                    None => &[],
                };
                let buttons_w: f32 = labels.iter().map(|t| cw * (t.len() as f32 + 2.0) + 8.0).sum();
                let text_end = if labels.is_empty() { rect.right() } else { visible.right() - 18.0 - buttons_w - 4.0 };
                let mut clip = p.clip_rect();
                clip.max.x = clip.max.x.min(text_end);
                p.with_clip_rect(clip).text(Pos2::new(rect.left() + 12.0, cy), egui::Align2::LEFT_CENTER, &*l.text, font.clone(), fg.gamma_multiply(0.8));
                if let Some(kind) = hunk_btn {
                    // Buttons are pinned to the visible right edge so they stay
                    // reachable when scrolled sideways.
                    let mut right = visible.right() - 18.0;
                    let mut button = |ui: &mut egui::Ui, label: &str, col: Color32| -> bool {
                        let w = cw * (label.len() as f32 + 2.0);
                        let btn = Rect::from_min_size(Pos2::new(right - w, rect.top() + 2.0), Vec2::new(w, row_h - 4.0));
                        right -= w + 8.0;
                        let b = egui::Button::new(RichText::new(label).color(col).size(12.0))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(1.0_f32, col));
                        ui.put(btn, b).clicked()
                    };
                    match kind {
                        HunkBtn::Stage => {
                            if button(ui, "Stage Hunk", sign_green) {
                                clicked = Some(l.hunk as usize);
                            }
                            if button(ui, "Discard Hunk", sign_red) {
                                discard = Some(l.hunk as usize);
                            }
                        }
                        HunkBtn::Unstage => {
                            if button(ui, "Unstage Hunk", sign_red) {
                                clicked = Some(l.hunk as usize);
                            }
                        }
                    }
                }
                continue;
            }

            let side_at = |x: f32| (split && x >= rect.left() + half_w) as usize;
            if row_resp.drag_started() {
                if let Some(pp) = ui.input(|inp| inp.pointer.press_origin()) {
                    let side = side_at(pp.x);
                    let at = pos_at(pp, side, row_origin);
                    opts.text_sel = Some(TextSel { title: title.to_owned(), side, anchor: at, cursor: at, dragging: true });
                }
            }
            if row_resp.hovered() {
                if let Some(pp) = row_resp.hover_pos() {
                    if pp.x >= code_x(rect.left(), side_at(pp.x)) - 6.0 {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                    }
                }
            }
            if row_resp.clicked() {
                opts.text_sel = None;
            }
            row_resp.context_menu(|ui| {
                let text = opts.text_sel.as_ref().map(&selected_text).unwrap_or_default();
                if ui.add_enabled(!text.is_empty(), egui::Button::new("Copy")).clicked() {
                    ui.ctx().copy_text(text);
                    ui.close_menu();
                }
            });
            if selectable && row_resp.clicked() {
                let left_side = row_resp
                    .interact_pointer_pos()
                    .is_none_or(|p| !opts.split || p.x < rect.left() + half_w);
                let target = if opts.split { if left_side { cells[0] } else { cells[1] } } else { cells[0] };
                if let Some(k) = target {
                    toggle(&mut opts.selected, k);
                }
            }
            let is_sel = |k: usize| selectable && opts.selected.contains(&line_ids[k]) && matches!(lines[k].kind, LineKind::Add | LineKind::Del);
            let num = |n: u32| if n == 0 { String::new() } else { n.to_string() };
            if opts.split {
                let halves = [
                    (Rect::from_min_max(rect.min, Pos2::new(rect.left() + half_w, rect.bottom())), cells[0], true),
                    (Rect::from_min_max(Pos2::new(rect.left() + half_w, rect.top()), rect.max), cells[1], false),
                ];
                for (r, cell, left) in halves {
                    let hp = p.with_clip_rect(r.intersect(p.clip_rect()));
                    let Some(k) = cell else {
                        hp.rect_filled(r, 0.0, empty_bg);
                        continue;
                    };
                    let l = &lines[k];
                    match l.kind {
                        LineKind::Add => {
                            hp.rect_filled(r, 0.0, add_bg);
                        }
                        LineKind::Del => {
                            hp.rect_filled(r, 0.0, del_bg);
                        }
                        _ => {}
                    }
                    if is_sel(k) {
                        hp.rect_filled(r, 0.0, selection);
                    }
                    if let Some((a, b)) = opts.text_sel.as_ref().and_then(|t| t.range(title, i, if left { 0 } else { 1 }, l.text.chars().count())) {
                        let x = r.left() + half_gutter;
                        hp.rect_filled(Rect::from_min_max(Pos2::new(x + a as f32 * cw, r.top()), Pos2::new(x + b as f32 * cw, r.bottom())), 0.0, text_hi);
                    }
                    let n = if left { l.old_no } else { l.new_no };
                    hp.text(Pos2::new(r.left() + 12.0 + num_w, cy), egui::Align2::RIGHT_CENTER, num(n), font.clone(), fg.gamma_multiply(0.8));
                    hp.vline(r.left() + half_gutter - 6.0, r.y_range(), Stroke::new(1.0_f32, line_col));
                    code(&hp, r, r.left() + half_gutter, k);
                }
                p.vline(rect.left() + half_w, rect.y_range(), Stroke::new(1.0_f32, line_col));
            } else {
                let l = &lines[i];
                match l.kind {
                    LineKind::Add => {
                        p.rect_filled(rect, 0.0, add_bg);
                    }
                    LineKind::Del => {
                        p.rect_filled(rect, 0.0, del_bg);
                    }
                    _ => {}
                }
                if is_sel(i) {
                    p.rect_filled(rect, 0.0, selection);
                }
                if let Some((a, b)) = opts.text_sel.as_ref().and_then(|t| t.range(title, i, 0, l.text.chars().count())) {
                    let x = rect.left() + text_x;
                    p.rect_filled(Rect::from_min_max(Pos2::new(x + a as f32 * cw, rect.top()), Pos2::new(x + b as f32 * cw, rect.bottom())), 0.0, text_hi);
                }
                if selectable && matches!(l.kind, LineKind::Add | LineKind::Del) {
                    // Check box showing whether the line is selected for line staging.
                    let b = Rect::from_center_size(Pos2::new(rect.left() + 8.0, cy), Vec2::splat(9.0));
                    p.rect_stroke(b, 2.0, Stroke::new(1.0_f32, weak), egui::StrokeKind::Inside);
                    if is_sel(i) {
                        p.rect_filled(b.shrink(1.5), 1.0, Color32::from_rgb(0x4e, 0xa8, 0xde));
                    }
                }
                let nums = fg.gamma_multiply(if l.kind == LineKind::Context { 0.8 } else { 1.0 });
                let x0 = rect.left();
                p.text(Pos2::new(x0 + old_right, cy), egui::Align2::RIGHT_CENTER, num(l.old_no), font.clone(), nums);
                p.text(Pos2::new(x0 + new_right, cy), egui::Align2::RIGHT_CENTER, num(l.new_no), font.clone(), nums);
                // +/- drawn as strokes: crisp at any size, no glyph needed.
                let sx = x0 + sign_right - 6.0;
                match l.kind {
                    LineKind::Add => {
                        let st = Stroke::new(1.4_f32, sign_green);
                        p.line_segment([Pos2::new(sx - 3.5, cy), Pos2::new(sx + 3.5, cy)], st);
                        p.line_segment([Pos2::new(sx, cy - 3.5), Pos2::new(sx, cy + 3.5)], st);
                    }
                    LineKind::Del => {
                        p.line_segment(
                            [Pos2::new(sx - 4.0, cy), Pos2::new(sx + 4.0, cy)],
                            Stroke::new(1.4_f32, sign_red),
                        );
                    }
                    _ => {}
                }
                p.vline(x0 + sep_x, rect.y_range(), Stroke::new(1.0_f32, line_col));
                code(&p, rect, x0 + text_x, i);
            }
        }
        // Extend the text selection while dragging, scrolling when the pointer leaves the view.
        if let Some(t) = opts.text_sel.as_mut().filter(|t| t.dragging) {
            let (down, pos) = ui.input(|inp| (inp.pointer.primary_down(), inp.pointer.latest_pos()));
            if down {
                if let (Some(pp), Some(o)) = (pos, origin) {
                    t.cursor = pos_at(pp, t.side, o);
                    if pp.y < visible.top() {
                        ui.scroll_with_delta(Vec2::new(0.0, (visible.top() - pp.y).min(60.0) * 0.5));
                    } else if pp.y > visible.bottom() {
                        ui.scroll_with_delta(Vec2::new(0.0, -(pp.y - visible.bottom()).min(60.0) * 0.5));
                    }
                }
                ui.ctx().request_repaint();
            } else {
                t.dragging = false;
            }
        }
    });
    // Copy the highlighted text.
    if let Some(t) = &opts.text_sel {
        if ui.input(|inp| inp.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
            let text = selected_text(t);
            if !text.is_empty() {
                ui.ctx().copy_text(text);
            }
        }
    }
    (clicked, discard)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(kind: LineKind) -> DiffLine {
        DiffLine { kind, hunk: 0, old_no: 0, new_no: 0, text: "".into() }
    }

    #[test]
    fn word_level_changed_range() {
        assert_eq!(changed_range(r#"version("4.0.0")"#, r#"version("4.1.0")"#), Some((11, 12)));
        assert_eq!(changed_range("same", "same"), None);
        assert_eq!(changed_range("abc", "xyz"), None, "whole line differs");
        assert_eq!(changed_range("ab", "abcd"), None, "pure insertion has no span on the shorter side");
    }

    #[test]
    fn pairs_removed_and_added_lines() {
        use LineKind::*;
        // hunk, context, 2 removed + 3 added, context, 1 added only
        let lines = [l(Hunk), l(Context), l(Del), l(Del), l(Add), l(Add), l(Add), l(Context), l(Add)];
        let (rows, hunks) = split_rows(&lines);
        assert_eq!(hunks, vec![0]);
        assert_eq!(
            rows,
            vec![
                [Some(0), None],
                [Some(1), Some(1)],
                [Some(2), Some(4)],
                [Some(3), Some(5)],
                [None, Some(6)],
                [Some(7), Some(7)],
                [None, Some(8)],
            ]
        );
    }
}
