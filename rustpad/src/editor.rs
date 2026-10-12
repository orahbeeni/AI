//! The text editing widget: gutter, syntax-colored text, selections, carets,
//! folding, word wrap, scrollbars, mouse + keyboard input, auto-complete popup.

use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Event, EventFilter, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
};

use crate::cmd::Cmd;
use crate::doc::{Doc, Metrics, Move, Sel};
use crate::lang::Style;
use crate::lexer::Span;
use crate::settings::Settings;
use crate::theme::Theme;
use crate::util;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    Chars,
    Words(usize, usize),
    Lines(usize),
    GutterLines(usize),
    Rect,
    VBar(f32),
    HBar(f32),
}

pub struct AutoComplete {
    pub items: Vec<String>,
    pub sel: usize,
    /// Char position where the typed prefix starts.
    pub start: usize,
}

pub struct EditorState {
    pub id: egui::Id,
    pub focus_request: bool,
    pub char_w: f32,
    pub line_h: f32,
    font_size: f32,
    last: Option<(u64, u64, usize)>,
    drag: Option<Drag>,
    clicks: (f64, Pos2, u8),
    pub ac: Option<AutoComplete>,
    bracket_cache: Option<(u64, usize, Option<(usize, usize)>)>,
    blink_epoch: f64,
    last_edit: f64,
    /// Visible row range of the last paint: (first row, page rows) — used by the minimap.
    pub view_rows: (u64, u64),
    pub text_rect: Rect,
    pub scroll_to_caret: bool,
    pub has_focus: bool,
    pub scroll: (f32, f32),
    scroll_doc: u64,
}

impl EditorState {
    pub fn new(id: egui::Id) -> Self {
        EditorState {
            id,
            focus_request: true,
            char_w: 8.0,
            line_h: 16.0,
            font_size: 0.0,
            last: None,
            drag: None,
            clicks: (0.0, Pos2::ZERO, 0),
            ac: None,
            bracket_cache: None,
            blink_epoch: 0.0,
            last_edit: 0.0,
            view_rows: (0, 0),
            text_rect: Rect::NOTHING,
            scroll_to_caret: false,
            has_focus: false,
            scroll: (0.0, 0.0),
            scroll_doc: 0,
        }
    }
}

/// One painted/hit-testable visual row.
struct Row {
    line: usize,
    /// Char range within the line.
    rs: usize,
    re: usize,
    last_in_line: bool,
    first_in_line: bool,
    /// Display column of each char boundary in this row (len = re-rs+1); empty = identity.
    cols: Vec<u32>,
}

const LONG_LINE: usize = 20_000;

fn build_cols(text: &str, rs: usize, re: usize, tab: usize) -> Vec<u32> {
    let mut cols = Vec::with_capacity(re - rs + 1);
    let mut col = 0usize;
    for ch in text.chars().skip(rs).take(re - rs) {
        cols.push(col as u32);
        col += util::char_cells(ch, col, tab);
    }
    cols.push(col as u32);
    cols
}

/// Split a line into visual rows.
fn rows_for_line(doc: &Doc, line: usize, text: &str, wrap_cols: usize) -> Vec<Row> {
    let n = text.chars().count();
    let tab = doc.opts.tab_width;
    let starts = if wrap_cols == 0 || n <= 1 { vec![0] } else { util::wrap_rows(text, wrap_cols, tab) };
    let long = n > LONG_LINE;
    let mut out = Vec::with_capacity(starts.len());
    for (i, &rs) in starts.iter().enumerate() {
        let re = starts.get(i + 1).copied().unwrap_or(n);
        out.push(Row {
            line,
            rs,
            re,
            last_in_line: i + 1 == starts.len(),
            first_in_line: i == 0,
            cols: if long { Vec::new() } else { build_cols(text, rs, re, tab) },
        });
    }
    out
}

struct Layout {
    rect: Rect,
    text_rect: Rect,
    vbar: Rect,
    hbar: Rect,
    num_x: f32,
    num_w: f32,
    bm_x: f32,
    bm_w: f32,
    fold_x: f32,
    fold_w: f32,
    text_x: f32,
    wrap_cols: usize,
    page_rows: u64,
}

pub struct Params<'a> {
    pub theme: &'a Theme,
    pub set: &'a Settings,
    pub time: f64,
}

pub fn font_for(set: &Settings) -> FontId {
    FontId::monospace((set.font_size + set.zoom).clamp(6.0, 72.0))
}

fn vthumb(doc: &Doc, lay: &Layout, max_scroll_row: f32) -> Rect {
    let track = lay.vbar;
    let total = (max_scroll_row + lay.page_rows as f32).max(1.0);
    let h = (track.height() * lay.page_rows as f32 / total).clamp(24.0, track.height().max(24.0));
    let frac = if max_scroll_row > 0.0 { doc.scroll_row / max_scroll_row } else { 0.0 };
    let y = track.top() + frac * (track.height() - h).max(0.0);
    Rect::from_min_size(Pos2::new(track.left() + 2.0, y), Vec2::new(track.width() - 4.0, h))
}

fn hthumb(doc: &Doc, lay: &Layout, max_scroll_x: f32) -> Rect {
    let track = lay.hbar;
    let total = max_scroll_x + lay.text_rect.width();
    let w = (track.width() * lay.text_rect.width() / total.max(1.0)).clamp(24.0, track.width().max(24.0));
    let frac = if max_scroll_x > 0.0 { doc.scroll_x / max_scroll_x } else { 0.0 };
    let x = track.left() + frac * (track.width() - w).max(0.0);
    Rect::from_min_size(Pos2::new(x, track.top() + 2.0), Vec2::new(w, track.height() - 4.0))
}

/// Show the editor in the remaining space of `ui`. Returns commands produced by input.
pub fn show(ui: &mut Ui, doc: &mut Doc, ed: &mut EditorState, p: &Params) -> Vec<Cmd> {
    let mut cmds: Vec<Cmd> = Vec::new();
    let set = p.set;
    let rect = ui.available_rect_before_wrap();
    let resp = ui.interact(rect, ed.id, Sense::click_and_drag());
    ui.allocate_rect(rect, Sense::hover());
    let painter = ui.painter_at(rect);
    let font = font_for(set);

    if (ed.font_size - font.size).abs() > f32::EPSILON || ed.char_w < 1.0 {
        ed.font_size = font.size;
        let (w, h) = ui.ctx().fonts_mut(|f| {
            let w: f32 = "MMMMMMMMMMMMMMMMMMMM".chars().map(|c| f.glyph_width(&font, c)).sum::<f32>() / 20.0;
            (w, f.row_height(&font))
        });
        ed.char_w = w.max(1.0);
        ed.line_h = (h + 1.0).ceil();
    }
    let cw = ed.char_w;
    let lh = ed.line_h;

    // per-pane scroll position (two panes may show the same document)
    if ed.scroll_doc != doc.id {
        ed.scroll = (doc.scroll_row, doc.scroll_x);
        ed.scroll_doc = doc.id;
        ed.last = None;
    }
    doc.scroll_row = ed.scroll.0;
    doc.scroll_x = ed.scroll.1;
    if let Some(r) = doc.scroll_request.take() {
        doc.scroll_row = r;
    }

    // ---- layout
    let total_lines = doc.len_lines();
    let digits = total_lines.to_string().len().max(3);
    let num_w = if set.line_numbers { digits as f32 * cw + 14.0 } else { 0.0 };
    let bm_w = if set.bookmark_margin { 16.0 } else { 0.0 };
    let fold_w = if set.fold_margin { 16.0 } else { 0.0 };
    let bar = 13.0;
    let wrap_on = set.word_wrap && doc.len_chars() < 6_000_000;
    let text_x = rect.left() + num_w + bm_w + fold_w + 4.0;
    let vbar = Rect::from_min_max(Pos2::new(rect.right() - bar, rect.top()), Pos2::new(rect.right(), rect.bottom() - if wrap_on { 0.0 } else { bar }));
    let hbar = if wrap_on {
        Rect::NOTHING
    } else {
        Rect::from_min_max(Pos2::new(rect.left() + num_w + bm_w + fold_w, rect.bottom() - bar), Pos2::new(rect.right() - bar, rect.bottom()))
    };
    let text_rect = Rect::from_min_max(
        Pos2::new(text_x, rect.top()),
        Pos2::new(rect.right() - bar, rect.bottom() - if wrap_on { 0.0 } else { bar }),
    );
    let wrap_cols = if wrap_on { (((text_rect.width() - 6.0) / cw).floor() as usize).max(8) } else { 0 };
    let page_rows = ((text_rect.height() / lh).floor() as u64).max(1);
    let lay = Layout {
        rect,
        text_rect,
        vbar,
        hbar,
        num_x: rect.left(),
        num_w,
        bm_x: rect.left() + num_w,
        bm_w,
        fold_x: rect.left() + num_w + bm_w,
        fold_w,
        text_x,
        wrap_cols,
        page_rows,
    };
    ed.text_rect = text_rect;

    // ---- fold + row map
    if set.fold_margin || !doc.collapsed.is_empty() {
        doc.folds();
        if doc.folds_pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(40));
        }
    }
    {
        let mut rm = std::mem::take(&mut doc.rowmap);
        rm.ensure(doc, wrap_cols);
        doc.rowmap = rm;
    }
    let total_rows = doc.rowmap.total_rows();

    // ---- focus
    let id = ed.id;
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(id, EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false });
    });
    if ed.focus_request {
        resp.request_focus();
        ed.focus_request = false;
    }
    if resp.clicked() || resp.drag_started() {
        resp.request_focus();
    }
    let focused = resp.has_focus() || ui.memory(|m| m.has_focus(id));
    ed.has_focus = focused;

    // ---- scrolling input
    let max_scroll_row = if set.scroll_beyond_last_line {
        total_rows.saturating_sub(1) as f32
    } else {
        total_rows.saturating_sub(page_rows) as f32
    };
    if resp.hovered() || (ed.drag.is_some()) {
        let (scroll, zoom, mods) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers));
        if zoom != 1.0 && zoom.is_finite() {
            cmds.push(if zoom > 1.0 { Cmd::ZoomIn } else { Cmd::ZoomOut });
        } else if mods.shift && scroll.y != 0.0 {
            doc.scroll_x -= scroll.y;
        } else {
            doc.scroll_row -= scroll.y / lh;
            doc.scroll_x -= scroll.x;
        }
    }
    // ---- caret reveal on change
    let head = doc.caret();
    let cur = (doc.id, doc.version, head);
    let reveal = match ed.last {
        Some((id0, v0, h0)) => id0 == doc.id && (v0 != cur.1 || h0 != cur.2),
        None => false,
    };
    if ed.last.is_none_or(|l| l.1 != cur.1) {
        ed.last_edit = p.time;
    }
    ed.last = Some(cur);
    if reveal || ed.scroll_to_caret {
        ed.scroll_to_caret = false;
        reveal_caret(doc, ed, &lay, wrap_cols);
    }
    let max_cols_px = (doc.max_cols as f32 + 4.0) * cw;
    let max_scroll_x = if wrap_on { 0.0 } else { (max_cols_px - text_rect.width() + cw * 4.0).max(0.0) };
    doc.scroll_row = doc.scroll_row.clamp(0.0, max_scroll_row.max(0.0));
    doc.scroll_x = doc.scroll_x.clamp(0.0, max_scroll_x);
    if wrap_on {
        doc.scroll_x = 0.0;
    }

    // ---- mouse
    handle_mouse(ui, &resp, doc, ed, &lay, set, p.time, max_scroll_row, max_scroll_x, total_rows);
    if let Some(pos) = resp.hover_pos() {
        if text_rect.contains(pos) {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
    }
    // right-click: put the caret under the pointer unless it is inside the selection
    if resp.hovered() && ui.input(|i| i.pointer.secondary_pressed()) {
        if let Some(pt) = ui.input(|i| i.pointer.interact_pos()) {
            if text_rect.contains(pt) {
                let pos = pos_from_point(doc, ed, &lay, pt);
                if !doc.sels.iter().any(|s| !s.is_empty() && s.start() <= pos && pos <= s.end()) {
                    doc.set_caret(pos);
                }
                ui.memory_mut(|m| m.request_focus(id));
            }
        }
    }
    resp.context_menu(|ui| {
        use crate::ui::menus::item;
        item(ui, &mut cmds, "Cut", Cmd::Cut);
        item(ui, &mut cmds, "Copy", Cmd::Copy);
        item(ui, &mut cmds, "Paste", Cmd::Paste);
        item(ui, &mut cmds, "Delete", Cmd::DeleteSel);
        item(ui, &mut cmds, "Select All", Cmd::SelectAll);
        ui.separator();
        item(ui, &mut cmds, "Toggle Comment", Cmd::ToggleComment);
        item(ui, &mut cmds, "UPPERCASE", Cmd::Case(crate::ops::CaseMode::Upper));
        item(ui, &mut cmds, "lowercase", Cmd::Case(crate::ops::CaseMode::Lower));
        item(ui, &mut cmds, "Toggle Bookmark", Cmd::ToggleBookmark);
        ui.separator();
        item(ui, &mut cmds, "Find...", Cmd::Find);
        item(ui, &mut cmds, "Replace...", Cmd::Replace);
        item(ui, &mut cmds, "Go to Matching Brace", Cmd::GotoBrace);
    });

    // ---- keyboard
    if focused {
        handle_keys(ui, doc, ed, &mut cmds, p.time);
    }

    // ---- paint
    paint(ui, &painter, doc, ed, &lay, p, &font);

    ed.scroll = (doc.scroll_row, doc.scroll_x);

    if focused {
        // tell the OS where the caret is so IME candidate windows appear next to it
        let (row, col) = locate_pos(doc, doc.caret(), wrap_cols);
        let x = text_x + col as f32 * cw - doc.scroll_x;
        let y = rect.top() + (row as f32 - doc.scroll_row) * lh;
        ui.ctx().output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Normal,
                rect: text_rect,
                cursor_rect: Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, lh)),
                should_interrupt_composition: false,
            });
        });
    }

    // blink scheduling
    if focused && set.caret_blink {
        let t = (p.time - ed.blink_epoch).rem_euclid(1.0);
        let next = if t < 0.5 { 0.5 - t } else { 1.0 - t };
        ui.ctx().request_repaint_after(Duration::from_secs_f64(next.max(0.02)));
    }
    cmds
}

fn metrics(ed: &EditorState, wrap_cols: usize) -> Metrics {
    Metrics { wrap_cols, page_rows: ed.view_rows.1.max(1) as usize }
}

pub fn view_metrics(doc: &Doc, ed: &EditorState, set: &Settings) -> Metrics {
    let wrap_cols = if set.word_wrap && doc.len_chars() < 6_000_000 {
        (((ed.text_rect.width() - 6.0) / ed.char_w).floor() as usize).max(8)
    } else {
        0
    };
    metrics(ed, wrap_cols)
}

/// Locate the visual row/col of an absolute position.
fn locate_pos(doc: &mut Doc, pos: usize, wrap_cols: usize) -> (u64, usize) {
    let line = doc.line_of(pos);
    let ls = doc.line_start(line);
    let c = pos - ls;
    let text = doc.line_text(line);
    let first = doc.rowmap.first_row_of_line(line);
    if wrap_cols == 0 {
        return (first, util::display_col(&text, c, doc.opts.tab_width));
    }
    let starts = util::wrap_rows(&text, wrap_cols, doc.opts.tab_width);
    let ri = starts.iter().rposition(|&s| s <= c).unwrap_or(0);
    let row_text: String = text.chars().skip(starts[ri]).collect();
    (first + ri as u64, util::display_col(&row_text, c - starts[ri], doc.opts.tab_width))
}

fn reveal_caret(doc: &mut Doc, ed: &mut EditorState, lay: &Layout, wrap_cols: usize) {
    let head = doc.caret();
    let line = doc.line_of(head);
    doc.reveal_line(line);
    let mut rm = std::mem::take(&mut doc.rowmap);
    rm.ensure(doc, wrap_cols);
    doc.rowmap = rm;
    let (row, col) = locate_pos(doc, head, wrap_cols);
    let top = doc.scroll_row;
    let page = lay.page_rows as f32;
    let r = row as f32;
    if r < top {
        doc.scroll_row = r;
    } else if r > top + page - 1.0 {
        doc.scroll_row = r - page + 1.0;
    }
    if wrap_cols == 0 {
        let x = col as f32 * ed.char_w;
        let w = lay.text_rect.width();
        let margin = ed.char_w * 3.0;
        if x < doc.scroll_x + margin {
            doc.scroll_x = (x - margin * 4.0).max(0.0);
        } else if x > doc.scroll_x + w - margin - ed.char_w {
            doc.scroll_x = x - w + margin * 4.0 + ed.char_w;
        }
    }
    ed.blink_epoch = 0.0;
}

/// Visual row → rows of the line. Returns the Row for a given absolute visual row.
fn row_at(doc: &mut Doc, vrow: u64, wrap_cols: usize) -> (Row, String) {
    let (line, ri) = doc.rowmap.locate(vrow);
    let text = doc.line_text(line);
    let mut rows = rows_for_line(doc, line, &text, wrap_cols);
    let idx = (ri as usize).min(rows.len() - 1);
    (rows.swap_remove(idx), text)
}

fn pos_from_point(doc: &mut Doc, ed: &EditorState, lay: &Layout, pt: Pos2) -> usize {
    let vrow_f = doc.scroll_row + (pt.y - lay.rect.top()) / ed.line_h;
    let vrow = vrow_f.floor().max(0.0) as u64;
    if vrow >= doc.rowmap.total_rows() {
        return doc.len_chars();
    }
    let (row, text) = row_at(doc, vrow, lay.wrap_cols);
    let x = (pt.x - lay.text_x + doc.scroll_x) / ed.char_w;
    let idx = idx_at_col(&row, x);
    let ls = doc.line_start(row.line);
    let _ = text;
    let max = if row.last_in_line { row.re } else { row.re.saturating_sub(1).max(row.rs) };
    ls + idx.clamp(row.rs, max)
}

fn idx_at_col(row: &Row, x: f32) -> usize {
    let n = row.re - row.rs;
    if row.cols.is_empty() {
        return (row.rs as f32 + x.round().max(0.0)) as usize;
    }
    let x = x.max(0.0);
    let mut lo = 0;
    for i in 0..n {
        let a = row.cols[i] as f32;
        let b = row.cols[i + 1] as f32;
        if x < b {
            lo = if x - a > (b - a) / 2.0 { i + 1 } else { i };
            return row.rs + lo;
        }
        lo = i + 1;
    }
    row.rs + lo
}

#[allow(clippy::too_many_arguments)]
fn handle_mouse(
    ui: &mut Ui,
    resp: &egui::Response,
    doc: &mut Doc,
    ed: &mut EditorState,
    lay: &Layout,
    set: &Settings,
    time: f64,
    max_scroll_row: f32,
    max_scroll_x: f32,
    total_rows: u64,
) {
    let (pressed, down, released, pos, mods) =
        ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.interact_pos(), i.modifiers));
    let lh = ed.line_h;

    let v_thumb = |doc: &Doc| vthumb(doc, lay, max_scroll_row);
    let h_thumb = |doc: &Doc| hthumb(doc, lay, max_scroll_x);

    if pressed && resp.hovered() {
        if let Some(pt) = pos {
            ed.blink_epoch = time;
            if lay.vbar.contains(pt) {
                let th = v_thumb(doc);
                if th.contains(pt) {
                    ed.drag = Some(Drag::VBar(pt.y - th.top()));
                } else {
                    // page jump
                    let dir = if pt.y < th.top() { -1.0 } else { 1.0 };
                    doc.scroll_row += dir * lay.page_rows as f32;
                }
                return;
            }
            if lay.hbar != Rect::NOTHING && lay.hbar.contains(pt) {
                let th = h_thumb(doc);
                if th.contains(pt) {
                    ed.drag = Some(Drag::HBar(pt.x - th.left()));
                } else {
                    let dir = if pt.x < th.left() { -1.0 } else { 1.0 };
                    doc.scroll_x += dir * lay.text_rect.width() * 0.9;
                }
                return;
            }
            // click counting
            let n = if time - ed.clicks.0 < 0.45 && (pt - ed.clicks.1).length() < 5.0 { (ed.clicks.2 % 3) + 1 } else { 1 };
            ed.clicks = (time, pt, n);
            ed.ac = None;

            if pt.x < lay.text_rect.left() {
                // gutter
                let vrow = (doc.scroll_row + (pt.y - lay.rect.top()) / lh).floor().max(0.0) as u64;
                if vrow >= total_rows {
                    return;
                }
                let (line, _) = doc.rowmap.locate(vrow);
                if set.bookmark_margin && pt.x >= lay.bm_x && pt.x < lay.bm_x + lay.bm_w {
                    doc.toggle_bookmark(line);
                } else if set.fold_margin && pt.x >= lay.fold_x && pt.x < lay.fold_x + lay.fold_w {
                    doc.toggle_fold(line);
                } else {
                    // select line(s)
                    let s = doc.line_start(line);
                    let e = if line + 1 < doc.len_lines() { doc.line_start(line + 1) } else { doc.len_chars() };
                    if mods.shift {
                        let a = doc.primary_sel().anchor;
                        doc.set_selection(a, e);
                    } else {
                        doc.set_selection(s, e);
                    }
                    ed.drag = Some(Drag::GutterLines(line));
                }
                return;
            }

            let pos_abs = pos_from_point(doc, ed, lay, pt);
            if mods.alt && !mods.ctrl {
                let (l, _) = doc.linecol(pos_abs);
                let col = ((pt.x - lay.text_x + doc.scroll_x) / ed.char_w).round().max(0.0) as usize;
                doc.rect_anchor = Some((l, col));
                doc.set_caret(pos_abs);
                ed.drag = Some(Drag::Rect);
            } else if n == 3 {
                let l = doc.line_of(pos_abs);
                let s = doc.line_start(l);
                let e = if l + 1 < doc.len_lines() { doc.line_start(l + 1) } else { doc.len_chars() };
                doc.set_selection(s, e);
                ed.drag = Some(Drag::Lines(l));
            } else if n == 2 {
                let (s, e) = doc.word_range_at(pos_abs);
                if mods.ctrl {
                    doc.add_selection(Sel::new(s, e));
                } else {
                    doc.set_selection(s, e);
                }
                ed.drag = Some(Drag::Words(s, e));
            } else if mods.shift {
                let a = doc.primary_sel().anchor;
                doc.set_selection(a, pos_abs);
                ed.drag = Some(Drag::Chars);
            } else if mods.ctrl {
                doc.add_selection(Sel::caret(pos_abs));
                ed.drag = Some(Drag::Chars);
            } else {
                doc.set_caret(pos_abs);
                ed.drag = Some(Drag::Chars);
            }
        }
    }

    if down {
        if let (Some(drag), Some(pt)) = (ed.drag, pos) {
            match drag {
                Drag::VBar(off) => {
                    let track = lay.vbar;
                    let th = v_thumb(doc);
                    let span = (track.height() - th.height()).max(1.0);
                    let frac = ((pt.y - off - track.top()) / span).clamp(0.0, 1.0);
                    doc.scroll_row = frac * max_scroll_row;
                }
                Drag::HBar(off) => {
                    let track = lay.hbar;
                    let th = h_thumb(doc);
                    let span = (track.width() - th.width()).max(1.0);
                    let frac = ((pt.x - off - track.left()) / span).clamp(0.0, 1.0);
                    doc.scroll_x = frac * max_scroll_x;
                }
                _ => {
                    // auto-scroll while dragging outside the text area
                    let tr = lay.text_rect;
                    if pt.y < tr.top() {
                        doc.scroll_row -= ((tr.top() - pt.y) / lh).min(3.0) * 0.5 + 0.2;
                        ui.ctx().request_repaint();
                    } else if pt.y > tr.bottom() {
                        doc.scroll_row += ((pt.y - tr.bottom()) / lh).min(3.0) * 0.5 + 0.2;
                        ui.ctx().request_repaint();
                    }
                    if pt.x < tr.left() && drag != Drag::GutterLines(0) {
                        doc.scroll_x -= (tr.left() - pt.x).min(60.0) * 0.3;
                        ui.ctx().request_repaint();
                    } else if pt.x > tr.right() {
                        doc.scroll_x += (pt.x - tr.right()).min(60.0) * 0.3;
                        ui.ctx().request_repaint();
                    }
                    doc.scroll_row = doc.scroll_row.clamp(0.0, max_scroll_row.max(0.0));
                    doc.scroll_x = doc.scroll_x.clamp(0.0, max_scroll_x);
                    let clamped = Pos2::new(pt.x.max(lay.text_x), pt.y.clamp(tr.top(), tr.bottom() - 1.0));
                    let pos_abs = pos_from_point(doc, ed, lay, clamped);
                    match drag {
                        Drag::Chars => {
                            let n = doc.sels.len();
                            let primary = doc.primary.min(n - 1);
                            let a = doc.sels[primary].anchor;
                            doc.sels[primary] = Sel::new(a, pos_abs);
                            doc.normalize();
                        }
                        Drag::Words(ws, we) => {
                            let (s, e) = doc.word_range_at(pos_abs);
                            if pos_abs < ws {
                                doc.set_selection(we, s);
                            } else {
                                doc.set_selection(ws, e.max(we));
                            }
                        }
                        Drag::Lines(l0) | Drag::GutterLines(l0) => {
                            let l = doc.line_of(pos_abs);
                            let end_of = |d: &Doc, l: usize| if l + 1 < d.len_lines() { d.line_start(l + 1) } else { d.len_chars() };
                            if l >= l0 {
                                doc.set_selection(doc.line_start(l0), end_of(doc, l));
                            } else {
                                doc.set_selection(end_of(doc, l0), doc.line_start(l));
                            }
                        }
                        Drag::Rect => {
                            if let Some((al, acol)) = doc.rect_anchor {
                                let (cl, _) = doc.linecol(pos_abs);
                                let col = ((pt.x - lay.text_x + doc.scroll_x) / ed.char_w).round().max(0.0) as usize;
                                set_rect_selection(doc, al, acol, cl, col);
                            }
                        }
                        _ => {}
                    }
                    ed.blink_epoch = time;
                }
            }
        }
    }
    if released {
        ed.drag = None;
    }
}

pub fn set_rect_selection(doc: &mut Doc, al: usize, acol: usize, cl: usize, ccol: usize) {
    let (l0, l1) = (al.min(cl), al.max(cl));
    let (c0, c1) = (acol.min(ccol), acol.max(ccol));
    let tab = doc.opts.tab_width;
    let mut sels = Vec::new();
    for l in l0..=l1 {
        let text = doc.line_text(l);
        let a = util::index_at_display_col(&text, c0, tab);
        let b = util::index_at_display_col(&text, c1, tab);
        let ls = doc.line_start(l);
        let n = text.chars().count();
        // anchor side follows the drag direction
        let (x, y) = (ls + a.min(n), ls + b.min(n));
        if ccol >= acol {
            sels.push(Sel::new(x, y));
        } else {
            sels.push(Sel::new(y, x));
        }
    }
    doc.sels = sels;
    doc.primary = if cl >= al { doc.sels.len() - 1 } else { 0 };
    let keep = doc.rect_anchor;
    doc.normalize();
    doc.rect_anchor = keep;
}

fn handle_keys(ui: &mut Ui, doc: &mut Doc, ed: &mut EditorState, cmds: &mut Vec<Cmd>, time: f64) {
    let events = ui.input(|i| i.events.clone());
    let mods_now = ui.input(|i| i.modifiers);
    for ev in events {
        match ev {
            Event::Text(s) => {
                if (mods_now.command || mods_now.ctrl) && !mods_now.alt {
                    continue;
                }
                let s: String = s.chars().filter(|c| !c.is_control() || *c == '\t').collect();
                if s.is_empty() {
                    continue;
                }
                ed.blink_epoch = time;
                cmds.push(Cmd::Type(s));
            }
            Event::Ime(egui::ImeEvent::Commit(s)) => {
                if !s.is_empty() {
                    cmds.push(Cmd::Type(s));
                }
            }
            Event::Copy => cmds.push(Cmd::Copy),
            Event::Cut => cmds.push(Cmd::Cut),
            Event::Paste(s) => cmds.push(Cmd::PasteText(s)),
            Event::Key { key, pressed: true, modifiers: m, .. } => {
                ed.blink_epoch = time;
                // auto-complete popup captures navigation keys
                if let Some(ac) = ed.ac.as_mut() {
                    match key {
                        Key::ArrowDown if m.is_none() => {
                            ac.sel = (ac.sel + 1) % ac.items.len().max(1);
                            continue;
                        }
                        Key::ArrowUp if m.is_none() => {
                            ac.sel = (ac.sel + ac.items.len().max(1) - 1) % ac.items.len().max(1);
                            continue;
                        }
                        Key::Enter | Key::Tab if m.is_none() => {
                            if let Some(item) = ac.items.get(ac.sel).cloned() {
                                accept_completion(doc, ac.start, &item);
                            }
                            ed.ac = None;
                            continue;
                        }
                        Key::Escape => {
                            ed.ac = None;
                            continue;
                        }
                        _ => {}
                    }
                }
                let ctrl = m.command || m.ctrl;
                let shift = m.shift;
                let mv = |mv: Move| Cmd::Move(mv, shift);
                if m.alt && shift && !ctrl {
                    let rect = match key {
                        Key::ArrowLeft => Some(Move::Left),
                        Key::ArrowRight => Some(Move::Right),
                        Key::ArrowUp => Some(Move::Up),
                        Key::ArrowDown => Some(Move::Down),
                        Key::Home => Some(Move::Home),
                        Key::End => Some(Move::End),
                        _ => None,
                    };
                    if let Some(r) = rect {
                        cmds.push(Cmd::RectMove(r));
                        continue;
                    }
                }
                let cmd = match key {
                    Key::Enter => Some(Cmd::Newline),
                    Key::Backspace => Some(if ctrl { Cmd::DeleteWordLeft } else { Cmd::Backspace }),
                    Key::Delete => Some(if shift { Cmd::Cut } else if ctrl { Cmd::DeleteWordRight } else { Cmd::DeleteFwd }),
                    Key::Tab => Some(Cmd::Tab),
                    Key::ArrowLeft if ctrl => Some(mv(Move::WordLeft)),
                    Key::ArrowRight if ctrl => Some(mv(Move::WordRight)),
                    Key::ArrowLeft => Some(mv(Move::Left)),
                    Key::ArrowRight => Some(mv(Move::Right)),
                    Key::ArrowUp if ctrl && !m.alt => Some(mv(Move::ParaUp)),
                    Key::ArrowDown if ctrl && !m.alt => Some(mv(Move::ParaDown)),
                    Key::ArrowUp => Some(mv(Move::Up)),
                    Key::ArrowDown => Some(mv(Move::Down)),
                    Key::Home if ctrl => Some(mv(Move::DocStart)),
                    Key::End if ctrl => Some(mv(Move::DocEnd)),
                    Key::Home => Some(mv(Move::Home)),
                    Key::End => Some(mv(Move::End)),
                    Key::PageUp if !ctrl => Some(mv(Move::PageUp)),
                    Key::PageDown if !ctrl => Some(mv(Move::PageDown)),
                    Key::Insert if !ctrl && !shift => Some(Cmd::ToggleOverwrite),
                    Key::Escape => {
                        if doc.sels.len() > 1 {
                            doc.collapse_to_primary();
                        } else if doc.has_selection() {
                            let h = doc.caret();
                            doc.set_caret(h);
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(c) = cmd {
                    cmds.push(c);
                }
            }
            _ => {}
        }
    }
}

pub fn accept_completion(doc: &mut Doc, start: usize, item: &str) {
    let caret = doc.caret();
    doc.set_selection(start, caret);
    doc.type_text(item);
}

/// Words in the document starting with the identifier prefix before the caret.
pub fn compute_completion(doc: &Doc) -> Option<AutoComplete> {
    let caret = doc.caret();
    let mut start = caret;
    while start > 0 && util::is_word_char(doc.rope.char(start - 1)) {
        start -= 1;
    }
    let prefix = doc.slice(start, caret);
    if prefix.is_empty() {
        return None;
    }
    let text = doc.text_string();
    let mut words: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let lower = prefix.to_lowercase();
    let mut cur_start: Option<usize> = None;
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        if util::is_word_char(c) {
            cur_start.get_or_insert(i);
        } else if let Some(s) = cur_start.take() {
            let w = &text[s..i];
            if w.len() > prefix.len() && w.to_lowercase().starts_with(&lower) {
                words.insert(w);
            }
            if words.len() > 400 {
                break;
            }
        }
    }
    // drop the word being typed at the caret itself
    let items: Vec<String> = words.into_iter().filter(|w| *w != prefix).map(String::from).take(60).collect();
    if items.is_empty() { None } else { Some(AutoComplete { items, sel: 0, start }) }
}

// ------------------------------------------------------------------ paint

fn paint(ui: &mut Ui, painter: &egui::Painter, doc: &mut Doc, ed: &mut EditorState, lay: &Layout, p: &Params, font: &FontId) {
    let theme = p.theme;
    let set = p.set;
    let cw = ed.char_w;
    let lh = ed.line_h;
    let rect = lay.rect;

    painter.rect_filled(rect, 0.0, theme.bg.c());
    let gutter_rect = Rect::from_min_max(rect.min, Pos2::new(lay.text_rect.left() - 4.0, rect.bottom()));
    painter.rect_filled(gutter_rect, 0.0, theme.gutter_bg.c());

    let top_row = doc.scroll_row.floor().max(0.0) as u64;
    let frac = doc.scroll_row - top_row as f32;
    let total_rows = doc.rowmap.total_rows();
    let n_visible = (lay.text_rect.height() / lh).ceil() as u64 + 1;
    ed.view_rows = (top_row, lay.page_rows);

    let text_clip = Rect::from_min_max(Pos2::new(lay.text_rect.left() - 2.0, rect.top()), lay.text_rect.max);
    let text_painter = painter.with_clip_rect(text_clip.intersect(rect));
    let sels: Vec<Sel> = doc.sels.clone();
    let primary_line = doc.line_of(doc.caret());
    let focused = ed.has_focus;
    let caret_on = focused && (!set.caret_blink || (p.time - ed.blink_epoch).rem_euclid(1.0) < 0.5);

    // bracket matching (cached)
    let bracket = if set.match_brackets && sels.len() == 1 {
        let key = (doc.version, doc.caret());
        match ed.bracket_cache {
            Some((v, c, r)) if (v, c) == key => r,
            _ => {
                let r = doc.find_matching_bracket(key.1);
                ed.bracket_cache = Some((key.0, key.1, r));
                r
            }
        }
    } else {
        None
    };

    // smart highlight needle
    let needle: Option<String> = if set.smart_highlight && sels.len() == 1 && !sels[0].is_empty() {
        let s = doc.slice(sels[0].start(), sels[0].end());
        if s.chars().count() <= 100 && !s.contains(['\n', '\r']) && !s.trim().is_empty() {
            Some(s)
        } else {
            None
        }
    } else {
        None
    };
    let needle_is_word = needle.as_ref().is_some_and(|n| n.chars().all(util::is_word_char));

    // visible marks
    let vis_start_pos = {
        let (l, _) = doc.rowmap.locate(top_row);
        doc.line_start(l)
    };
    let vis_end_pos = {
        let (l, _) = doc.rowmap.locate((top_row + n_visible).min(total_rows.saturating_sub(1)));
        doc.line_end(l)
    };
    let marks: Vec<crate::doc::Mark> = doc.marks.iter().filter(|m| m.end >= vis_start_pos && m.start <= vis_end_pos).copied().collect();

    let tab = doc.opts.tab_width;
    let vis_cols = (lay.text_rect.width() / cw).ceil() as usize + 4;
    let c0 = (doc.scroll_x / cw).floor() as usize;
    let c0 = c0.saturating_sub(2);
    let mut spans: Vec<Span> = Vec::new();

    let mut vrow = top_row;
    let mut y = rect.top() - frac * lh;
    let mut last_line_drawn = usize::MAX;
    let mut cached_line = usize::MAX;
    let mut cached_rows: Vec<Row> = Vec::new();
    let mut cached_text = String::new();

    while vrow < total_rows && y < rect.bottom() {
        let (line, ri) = doc.rowmap.locate(vrow);
        if line != cached_line {
            cached_text = doc.line_spans(line, &mut spans);
            let long = cached_text.chars().count() > LONG_LINE;
            if long {
                spans.clear();
            }
            cached_rows = rows_for_line(doc, line, &cached_text, lay.wrap_cols);
            cached_line = line;
            let n = cached_text.chars().count();
            if n > doc.max_cols {
                doc.max_cols = n;
            }
        }
        let row = &cached_rows[(ri as usize).min(cached_rows.len() - 1)];
        let ls = doc.line_start(line);
        let row_a = ls + row.rs;
        let row_b = ls + row.re;
        let row_rect = Rect::from_min_max(Pos2::new(lay.text_x - 2.0, y), Pos2::new(lay.text_rect.right(), y + lh));
        let x_of = |abs: usize| -> f32 {
            let idx = abs.saturating_sub(row_a).min(row.re - row.rs);
            let col = if row.cols.is_empty() { idx } else { row.cols[idx.min(row.cols.len() - 1)] as usize };
            lay.text_x + col as f32 * cw - doc.scroll_x
        };

        // current line highlight
        if set.highlight_current_line && line == primary_line && sels.len() == 1 && sels[0].is_empty() {
            text_painter.rect_filled(row_rect, 0.0, theme.current_line.c());
        }

        // find marks
        for m in &marks {
            if m.start < row_b.max(row_a + 1) && m.end > row_a && m.start < row_b + usize::from(row.last_in_line) {
                let s = m.start.max(row_a);
                let e = m.end.min(row_b);
                if e > s {
                    let col = theme.marks[(m.style as usize).min(4)];
                    text_painter.rect_filled(Rect::from_min_max(Pos2::new(x_of(s), y), Pos2::new(x_of(e), y + lh)), 1.0, col.a(150));
                }
            }
        }

        // smart highlight occurrences
        if let Some(n) = &needle {
            if row.re > row.rs {
                let row_text: String = cached_text.chars().skip(row.rs).take(row.re - row.rs).collect();
                for (bi, _) in row_text.match_indices(n.as_str()) {
                    let before = row_text[..bi].chars().next_back();
                    let after = row_text[bi + n.len()..].chars().next();
                    if needle_is_word && (before.is_some_and(util::is_word_char) || after.is_some_and(util::is_word_char)) {
                        continue;
                    }
                    let cs = row.rs + row_text[..bi].chars().count();
                    let ce = cs + n.chars().count();
                    let (a, b) = (ls + cs, ls + ce);
                    if sels.iter().any(|s| s.start() == a && s.end() == b) {
                        continue;
                    }
                    text_painter.rect_filled(Rect::from_min_max(Pos2::new(x_of(a), y), Pos2::new(x_of(b), y + lh)), 1.0, theme.smart_highlight.c());
                }
            }
        }

        // selections
        let first_sel = sels.partition_point(|s| s.end() < row_a);
        for s in &sels[first_sel..] {
            if s.start() > row_b {
                break;
            }
            if s.is_empty() {
                continue;
            }
            let a = s.start().max(row_a);
            let b = s.end().min(row_b);
            let covers_eol = row.last_in_line && s.end() > row_b && s.start() <= row_b;
            if b < a || (b == a && !covers_eol) {
                continue;
            }
            if !row.last_in_line && s.start() >= row_b {
                continue;
            }
            let mut x1 = x_of(b);
            if covers_eol {
                x1 += cw * 0.6;
            }
            let x0 = x_of(a);
            let col = if focused { theme.selection.c() } else { theme.selection.a(140) };
            text_painter.rect_filled(Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x1.max(x0 + 1.0), y + lh)), 0.0, col);
        }

        // bracket highlights
        if let Some((a, b)) = bracket {
            for q in [a, b] {
                if q >= row_a && (q < row_b || (row.last_in_line && q == row_b)) {
                    text_painter.rect_filled(Rect::from_min_max(Pos2::new(x_of(q), y), Pos2::new(x_of(q + 1), y + lh)), 1.0, theme.bracket_match.c());
                }
            }
        }

        // indent guides
        if set.indent_guides && row.first_in_line && tab > 0 {
            let lead = util::leading_ws(&cached_text);
            let lead_w = util::display_width(lead, tab);
            let mut c = tab;
            while c < lead_w {
                let x = lay.text_x + c as f32 * cw - doc.scroll_x;
                if x > lay.text_x - 1.0 {
                    text_painter.line_segment([Pos2::new(x - 1.0, y), Pos2::new(x - 1.0, y + lh)], Stroke::new(1.0, theme.guide.c()));
                }
                c += tab;
            }
        }

        // text
        paint_row_text(&text_painter, &cached_text, &spans, row, y, lay, doc.scroll_x, tab, cw, font, theme, set, c0, vis_cols);

        // end-of-line markers
        if set.show_eol && row.last_in_line && line + 1 < doc.len_lines() {
            let label = match line_eol(doc, line) {
                "\r\n" => "CRLF",
                "\r" => "CR",
                _ => "LF",
            };
            let x = x_of(row_b) + 3.0;
            let f = FontId::monospace((font.size * 0.62).max(7.0));
            let r = text_painter.text(Pos2::new(x, y + lh * 0.2), Align2::LEFT_TOP, label, f, theme.whitespace.c());
            text_painter.rect_stroke(r.expand(1.0), 2.0, Stroke::new(1.0, theme.whitespace.a(120)), StrokeKind::Inside);
        }

        // folded marker
        if row.last_in_line && doc.collapsed.contains(&line) {
            if let Some(&(_, end)) = doc.fold_starts.iter().find(|f| f.0 == line) {
                let x = x_of(row_b) + 8.0;
                let label = format!(" ⋯ {} lines ", end - line);
                let f = FontId::monospace((font.size * 0.8).max(8.0));
                let r = text_painter.text(Pos2::new(x, y + lh * 0.1), Align2::LEFT_TOP, label, f, theme.gutter_fg.c());
                text_painter.rect_stroke(r, 3.0, Stroke::new(1.0, theme.gutter_fg.a(160)), StrokeKind::Inside);
            }
        }

        // carets
        if caret_on {
            for (i, s) in sels.iter().enumerate() {
                let h = s.head;
                let here = h >= row_a && (h < row_b || (row.last_in_line && h == row_b));
                if !here {
                    continue;
                }
                let x = x_of(h);
                let primary = i == doc.primary.min(sels.len() - 1);
                let col = if primary { theme.caret.c() } else { theme.caret.a(200) };
                if doc.overwrite {
                    text_painter.rect_stroke(Rect::from_min_size(Pos2::new(x, y), Vec2::new(cw, lh)), 0.0, Stroke::new(1.5, col), StrokeKind::Inside);
                } else {
                    text_painter.rect_filled(Rect::from_min_size(Pos2::new(x - 0.5, y), Vec2::new(2.0, lh)), 0.0, col);
                }
            }
        }

        // gutter
        if row.first_in_line && line != last_line_drawn {
            last_line_drawn = line;
            let gp = painter.with_clip_rect(Rect::from_min_max(rect.min, Pos2::new(lay.text_rect.left() - 3.0, rect.bottom())));
            if lay.num_w > 0.0 {
                let c = if line == primary_line { theme.gutter_active_fg.c() } else { theme.gutter_fg.c() };
                gp.text(Pos2::new(lay.num_x + lay.num_w - 7.0, y), Align2::RIGHT_TOP, (line + 1).to_string(), font.clone(), c);
            }
            if lay.bm_w > 0.0 && doc.bookmarks.contains(&line) {
                gp.circle_filled(Pos2::new(lay.bm_x + lay.bm_w / 2.0, y + lh / 2.0), (lh * 0.3).min(5.5), theme.bookmark.c());
            }
            if lay.fold_w > 0.0 {
                if let Ok(i) = doc.fold_starts.binary_search_by_key(&line, |f| f.0) {
                    let _ = i;
                    let collapsed = doc.collapsed.contains(&line);
                    let c = Pos2::new(lay.fold_x + lay.fold_w / 2.0, y + lh / 2.0);
                    let r = Rect::from_center_size(c, Vec2::splat(9.0));
                    let col = theme.fold_marker.c();
                    gp.rect_stroke(r, 1.0, Stroke::new(1.0, col), StrokeKind::Inside);
                    gp.line_segment([Pos2::new(c.x - 2.5, c.y), Pos2::new(c.x + 2.5, c.y)], Stroke::new(1.0, col));
                    if collapsed {
                        gp.line_segment([Pos2::new(c.x, c.y - 2.5), Pos2::new(c.x, c.y + 2.5)], Stroke::new(1.0, col));
                    }
                }
            }
        }

        vrow += 1;
        y += lh;
    }

    // edge column
    if set.show_edge && lay.wrap_cols == 0 {
        let x = lay.text_x + set.edge_column as f32 * cw - doc.scroll_x;
        if x > lay.text_rect.left() && x < lay.text_rect.right() {
            text_painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.0, theme.edge.c()));
        }
    }

    // gutter separator
    painter.line_segment(
        [Pos2::new(lay.text_rect.left() - 4.0, rect.top()), Pos2::new(lay.text_rect.left() - 4.0, rect.bottom())],
        Stroke::new(1.0, theme.guide.c()),
    );

    // scrollbars
    let max_scroll_row = if set.scroll_beyond_last_line { total_rows.saturating_sub(1) as f32 } else { total_rows.saturating_sub(lay.page_rows) as f32 };
    let max_scroll_x = if lay.wrap_cols != 0 { 0.0 } else { ((doc.max_cols as f32 + 4.0) * cw - lay.text_rect.width() + cw * 4.0).max(0.0) };
    paint_scrollbars(painter, doc, lay, theme, max_scroll_row, max_scroll_x, ed.drag);

    // autocomplete popup
    if let Some(ac) = &ed.ac {
        let (_, col) = locate_pos(doc, ac.start, lay.wrap_cols);
        let (vr, _) = locate_pos(doc, ac.start, lay.wrap_cols);
        let pos = Pos2::new(lay.text_x + col as f32 * cw - doc.scroll_x, rect.top() + (vr as f32 - doc.scroll_row + 1.0) * lh);
        let items = ac.items.clone();
        let sel = ac.sel;
        egui::Area::new(ui.id().with("ac_popup")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(140.0);
                for (i, it) in items.iter().take(12).enumerate() {
                    let text = egui::RichText::new(it).monospace();
                    let r = ui.selectable_label(i == sel, text);
                    if i == sel {
                        r.scroll_to_me(None);
                    }
                }
            });
        });
    }
}

fn line_eol(doc: &Doc, line: usize) -> &'static str {
    let e = doc.line_end(line);
    match (doc.char_at(e), doc.char_at(e + 1)) {
        (Some('\r'), Some('\n')) => "\r\n",
        (Some('\r'), _) => "\r",
        _ => "\n",
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_row_text(
    painter: &egui::Painter,
    text: &str,
    spans: &[Span],
    row: &Row,
    y: f32,
    lay: &Layout,
    scroll_x: f32,
    tab: usize,
    cw: f32,
    font: &FontId,
    theme: &Theme,
    set: &Settings,
    c0: usize,
    vis_cols: usize,
) {
    if row.re == row.rs {
        return;
    }
    let long = row.cols.is_empty();
    // byte offsets of the row
    let mut iter = text.char_indices().skip(row.rs);
    let bs = iter.next().map(|(b, _)| b).unwrap_or(text.len());
    let be = text.char_indices().nth(row.re).map(|(b, _)| b).unwrap_or(text.len());
    let mut span_i = spans.partition_point(|s| s.end <= bs);

    let mut col = 0usize;
    let mut run = String::new();
    let mut run_style = Style::Default;
    let mut run_col = 0usize;
    let origin = lay.text_x - scroll_x;
    let y_text = y;

    let flush = |run: &mut String, style: Style, start_col: usize| {
        if run.is_empty() {
            return;
        }
        if !run.chars().all(|c| c == ' ') {
            let color = theme.style_color(style);
            painter.text(Pos2::new(origin + start_col as f32 * cw, y_text), Align2::LEFT_TOP, run.as_str(), font.clone(), color);
        }
        run.clear();
    };

    let c_end = c0 + vis_cols;
    for (bi, ch) in text[bs..be].char_indices() {
        let bi = bi + bs;
        if long && col > c_end + row.rs {
            break;
        }
        while span_i < spans.len() && spans[span_i].end <= bi {
            span_i += 1;
        }
        let st = if span_i < spans.len() && spans[span_i].start <= bi { spans[span_i].style } else { Style::Default };
        let w = if long { 1 } else { util::char_cells(ch, col, tab) };
        let visible = col + w >= c0 && col <= c_end;
        if !visible {
            if !run.is_empty() {
                flush(&mut run, run_style, run_col);
            }
            col += w;
            continue;
        }
        if st != run_style || run.is_empty() {
            flush(&mut run, run_style, run_col);
            run_style = st;
            run_col = col;
        }
        if ch == '\t' {
            if set.show_whitespace {
                flush(&mut run, run_style, run_col);
                let x0 = origin + col as f32 * cw;
                let ym = y_text + painter_row_mid(font);
                let c = theme.whitespace.c();
                painter.line_segment([Pos2::new(x0 + 2.0, ym), Pos2::new(x0 + w as f32 * cw - 3.0, ym)], Stroke::new(1.0, c));
                painter.line_segment([Pos2::new(x0 + w as f32 * cw - 6.0, ym - 3.0), Pos2::new(x0 + w as f32 * cw - 3.0, ym)], Stroke::new(1.0, c));
                painter.line_segment([Pos2::new(x0 + w as f32 * cw - 6.0, ym + 3.0), Pos2::new(x0 + w as f32 * cw - 3.0, ym)], Stroke::new(1.0, c));
                run_col = col + w;
            } else {
                for _ in 0..w {
                    run.push(' ');
                }
            }
        } else if ch == ' ' {
            if set.show_whitespace {
                let x0 = origin + col as f32 * cw;
                painter.circle_filled(Pos2::new(x0 + cw / 2.0, y_text + painter_row_mid(font)), 1.1, theme.whitespace.c());
            }
            run.push(' ');
        } else if ch.is_control() {
            run.push('\u{00B7}');
        } else {
            run.push(ch);
        }
        col += w;
    }
    flush(&mut run, run_style, run_col);
}

fn painter_row_mid(font: &FontId) -> f32 {
    (font.size * 1.2) / 2.0
}

fn paint_scrollbars(painter: &egui::Painter, doc: &Doc, lay: &Layout, theme: &Theme, max_scroll_row: f32, max_scroll_x: f32, drag: Option<Drag>) {
    let track_col = if theme.dark { Color32::from_white_alpha(8) } else { Color32::from_black_alpha(10) };
    let thumb_col = if theme.dark { Color32::from_white_alpha(60) } else { Color32::from_black_alpha(70) };
    let thumb_hot = if theme.dark { Color32::from_white_alpha(100) } else { Color32::from_black_alpha(110) };
    painter.rect_filled(lay.vbar, 0.0, track_col);
    let thumb = vthumb(doc, lay, max_scroll_row);
    painter.rect_filled(thumb, 4.0, if matches!(drag, Some(Drag::VBar(_))) { thumb_hot } else { thumb_col });
    if lay.hbar != Rect::NOTHING {
        painter.rect_filled(lay.hbar, 0.0, track_col);
        if max_scroll_x > 0.0 {
            let t = hthumb(doc, lay, max_scroll_x);
            painter.rect_filled(t, 4.0, if matches!(drag, Some(Drag::HBar(_))) { thumb_hot } else { thumb_col });
        }
    }
}
