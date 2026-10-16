//! Drawing a terminal's grid and handling the mouse over it.

use std::time::{Duration, Instant};

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};
use eframe::egui::{
    self, Color32, CursorIcon, FontFamily, FontId, Painter, PointerButton, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2,
};

use crate::config::{CursorKind, Rgb, Settings, Theme};
use crate::input::{MouseAction, mouse_report};
use crate::pane::{Pane, TermSize};

pub const FONT_FAMILY: &str = "term";

#[derive(Clone)]
pub struct Metrics {
    pub font: FontId,
    pub cw: f32,
    pub ch: f32,
    /// Font's natural row height, for centring text in taller cells.
    pub row: f32,
}

impl Metrics {
    pub fn new(ctx: &egui::Context, s: &Settings) -> Metrics {
        let font = FontId::new(s.font_size, FontFamily::Name(FONT_FAMILY.into()));
        let (cw, row) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        Metrics { font, cw: cw.max(1.0), ch: (row * s.line_height).round().max(1.0), row }
    }

    pub fn grid_size(&self, text: Rect) -> TermSize {
        TermSize { cols: ((text.width() / self.cw).floor() as usize).max(2), rows: ((text.height() / self.ch).floor() as usize).max(1) }
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// Resolves the emulator's colour references against the theme and any program overrides.
struct Palette<'a> {
    theme: &'a Theme,
    colors: &'a Colors,
    bold_is_bright: bool,
}

impl Palette<'_> {
    fn index(&self, i: usize) -> Rgb {
        if let Some(c) = self.colors[i] {
            return Rgb(c.r, c.g, c.b);
        }
        match i {
            0..=255 => self.theme.indexed(i),
            256 | 267 => self.theme.foreground,
            257 => self.theme.background,
            258 => self.theme.cursor,
            259..=266 => mix(self.theme.palette[i - 259], self.theme.background, 0.33),
            268 => mix(self.theme.foreground, self.theme.background, 0.33),
            _ => self.theme.foreground,
        }
    }

    fn fg(&self, c: AnsiColor, flags: Flags) -> Rgb {
        let bright = self.bold_is_bright && flags.contains(Flags::BOLD);
        match c {
            AnsiColor::Spec(rgb) => Rgb(rgb.r, rgb.g, rgb.b),
            AnsiColor::Indexed(i) if bright && i < 8 => self.index(i as usize + 8),
            AnsiColor::Indexed(i) => self.index(i as usize),
            AnsiColor::Named(n) => {
                let i = n as usize;
                if bright && i < 8 {
                    self.index(i + 8)
                } else if flags.contains(Flags::DIM) && i < 8 {
                    self.index(NamedColor::DimBlack as usize + i)
                } else {
                    self.index(i)
                }
            }
        }
    }

    fn bg(&self, c: AnsiColor) -> Rgb {
        match c {
            AnsiColor::Spec(rgb) => Rgb(rgb.r, rgb.g, rgb.b),
            AnsiColor::Indexed(i) => self.index(i as usize),
            AnsiColor::Named(n) => self.index(n as usize),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Rgb,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

struct Run {
    row: usize,
    col: usize,
    text: String,
    style: Style,
}

/// Paint one terminal into `text_rect`.
pub fn draw(painter: &Painter, text_rect: Rect, pane: &Pane, s: &Settings, m: &Metrics, focused: bool, blink_on: bool) {
    let term = pane.term.lock();
    let content = term.renderable_content();
    let theme = &s.theme;
    let pal = Palette { theme, colors: content.colors, bold_is_bright: s.bold_is_bright };
    let default_bg = pal.index(NamedColor::Background as usize);
    let offset = content.display_offset as i32;
    let selection = content.selection;
    let cursor = content.cursor;
    let cursor_visible = cursor.shape != CursorShape::Hidden && (blink_on || !focused);
    let block_cursor = cursor_visible && focused && cursor.shape == CursorShape::Block;
    let origin = text_rect.min;
    let cols = term.columns();

    let cell_rect = |row: usize, col: usize, width: usize| {
        Rect::from_min_size(pos2(origin.x + col as f32 * m.cw, origin.y + row as f32 * m.ch), vec2(m.cw * width as f32, m.ch))
    };

    let mut bg_run: Option<(usize, usize, usize, Rgb)> = None; // row, start col, end col (excl), colour
    let mut runs: Vec<Run> = Vec::new();
    let mut singles: Vec<(usize, usize, String, Style, usize)> = Vec::new();
    let mut cur: Option<Run> = None;

    let flush_bg = |run: &mut Option<(usize, usize, usize, Rgb)>| {
        if let Some((row, a, b, c)) = run.take() {
            let r = cell_rect(row, a, b - a);
            painter.rect_filled(r, 0.0, c.color());
        }
    };

    for indexed in content.display_iter {
        let row = (indexed.point.line.0 + offset) as usize;
        let col = indexed.point.column.0;
        let cell = indexed.cell;
        let flags = cell.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let width = if flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };

        let mut fg = pal.fg(cell.fg, flags);
        let mut bg = pal.bg(cell.bg);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) && !matches!(cell.fg, AnsiColor::Named(n) if (n as usize) < 8) {
            fg = mix(fg, bg, 0.33);
        }
        if selection.is_some_and(|sel| sel.contains(indexed.point)) {
            bg = theme.selection_bg;
            fg = theme.selection_fg;
        }
        if block_cursor && indexed.point == cursor.point {
            bg = theme.cursor;
            fg = theme.cursor_text;
        }

        // Backgrounds, merged along the row.
        let paint_bg = bg != default_bg;
        match &mut bg_run {
            Some((r, _, end, c)) if paint_bg && *r == row && *end == col && *c == bg => *end = col + width,
            _ => {
                flush_bg(&mut bg_run);
                if paint_bg {
                    bg_run = Some((row, col, col + width, bg));
                }
            }
        }

        // Text, merged into runs of ASCII with one style; anything else is placed cell by cell
        // so fallback fonts with other advances cannot push the rest of the row out of line.
        let ch = cell.c;
        let visible = ch != ' ' && ch != '\t' && !flags.contains(Flags::HIDDEN);
        let style = Style {
            fg,
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
            underline: flags.intersects(Flags::ALL_UNDERLINES),
            strike: flags.contains(Flags::STRIKEOUT),
        };
        let decorated = style.underline || style.strike;
        if !visible && !decorated {
            // Spaces join a run only when the run continues right after them.
            if let Some(r) = &mut cur
                && r.row == row
                && r.col + r.text.chars().count() == col
            {
                r.text.push(' ');
                continue;
            }
            continue;
        }
        let zw = cell.zerowidth();
        if ch.is_ascii() && width == 1 && zw.is_none() {
            let c = if visible { ch } else { ' ' };
            match &mut cur {
                Some(r) if r.row == row && r.style == style && r.col + r.text.chars().count() == col => r.text.push(c),
                _ => {
                    if let Some(r) = cur.take() {
                        runs.push(r);
                    }
                    cur = Some(Run { row, col, text: c.to_string(), style });
                }
            }
        } else {
            let mut text = ch.to_string();
            if let Some(zw) = zw {
                text.extend(zw.iter());
            }
            singles.push((row, col, text, style, width));
        }
    }
    flush_bg(&mut bg_run);
    if let Some(r) = cur.take() {
        runs.push(r);
    }

    let text_y = (m.ch - m.row) / 2.0;
    let paint_text = |row: usize, col: usize, text: &str, st: Style, width: usize, centre: bool| {
        let mut trimmed = text.trim_end();
        if trimmed.is_empty() {
            trimmed = "";
        }
        let r = cell_rect(row, col, if centre { width } else { text.chars().count().max(1) });
        if !trimmed.is_empty() {
            let mut job = egui::text::LayoutJob::simple_singleline(trimmed.to_owned(), m.font.clone(), st.fg.color());
            if st.italic {
                job.sections[0].format.italics = true;
            }
            let galley = painter.layout_job(job);
            let x = if centre { r.center().x - galley.size().x / 2.0 } else { r.min.x };
            let p = pos2(x, r.min.y + text_y);
            if st.bold && s.faux_bold {
                painter.galley(p + vec2(0.6, 0.0), galley.clone(), st.fg.color());
            }
            painter.galley(p, galley, st.fg.color());
        }
        if st.underline {
            let y = (r.max.y - 1.5).round() + 0.5;
            painter.hline(r.x_range(), y, Stroke::new(1.0, st.fg.color()));
        }
        if st.strike {
            painter.hline(r.x_range(), r.center().y.round() + 0.5, Stroke::new(1.0, st.fg.color()));
        }
    };
    for r in &runs {
        paint_text(r.row, r.col, &r.text, r.style, 1, false);
    }
    for (row, col, text, st, width) in &singles {
        paint_text(*row, *col, text, *st, *width, true);
    }

    // Cursor, unless it is a filled block (already drawn as the cell's colours).
    let cursor_row = cursor.point.line.0 + offset;
    if cursor_visible && !block_cursor && cursor_row >= 0 && (cursor_row as usize) < term.screen_lines() {
        let wide = term.grid()[cursor.point].flags.contains(Flags::WIDE_CHAR);
        let r = cell_rect(cursor_row as usize, cursor.point.column.0.min(cols - 1), if wide { 2 } else { 1 });
        let c = theme.cursor.color();
        let shape = if focused {
            cursor.shape
        } else {
            match s.cursor_shape {
                CursorKind::Block => CursorShape::HollowBlock,
                _ => cursor.shape,
            }
        };
        match shape {
            CursorShape::Beam => {
                painter.rect_filled(Rect::from_min_size(r.min, vec2(2.0, r.height())), 0.0, c);
            }
            CursorShape::Underline => {
                painter.rect_filled(Rect::from_min_max(pos2(r.min.x, r.max.y - 2.0), r.max), 0.0, c);
            }
            _ => {
                painter.rect_stroke(r.shrink(0.5), 0.0, Stroke::new(1.0, c), egui::StrokeKind::Inside);
            }
        }
    }
}

/// Grid position under a screen position; the side says which half of the cell was hit.
pub fn point_at(p: Pos2, text_rect: Rect, m: &Metrics, size: TermSize, display_offset: usize) -> (Point, Side, usize, usize) {
    let x = (p.x - text_rect.min.x) / m.cw;
    let y = (p.y - text_rect.min.y) / m.ch;
    let col = (x.floor().max(0.0) as usize).min(size.cols - 1);
    let row = (y.floor().max(0.0) as usize).min(size.rows - 1);
    let side = if x - x.floor() > 0.5 && x >= 0.0 { Side::Right } else { Side::Left };
    let side = if x >= size.cols as f32 { Side::Right } else { side };
    (Point::new(Line(row as i32 - display_offset as i32), Column(col)), side, col, row)
}

#[derive(Default)]
pub struct MouseOut {
    pub focus: bool,
    pub copy: Option<String>,
    /// Paste from the clipboard (false) or the primary selection (true).
    pub paste: Option<bool>,
    pub menu_allowed: bool,
}

/// Selection, wheel scrolling and mouse reporting for one terminal.
pub fn mouse(ui: &Ui, resp: &Response, pane: &mut Pane, text_rect: Rect, m: &Metrics, s: &Settings) -> MouseOut {
    let mut out = MouseOut::default();
    let (mods, pointer, wheel, pressed, released, primary_down) = ui.input(|i| {
        let pressed = [PointerButton::Primary, PointerButton::Middle, PointerButton::Secondary].map(|b| i.pointer.button_pressed(b));
        let released = [PointerButton::Primary, PointerButton::Middle, PointerButton::Secondary].map(|b| i.pointer.button_released(b));
        // Wheel movement in lines: notches scroll three lines, trackpads by distance.
        let wheel: f32 = i
            .raw
            .events
            .iter()
            .map(|e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => match unit {
                    egui::MouseWheelUnit::Line => delta.y * 3.0,
                    egui::MouseWheelUnit::Point => delta.y / m.ch,
                    egui::MouseWheelUnit::Page => delta.y * pane.size.rows as f32,
                },
                _ => 0.0,
            })
            .sum();
        (i.modifiers, i.pointer.latest_pos(), wheel, pressed, released, i.pointer.primary_down())
    });
    let hovered = resp.hovered();
    let mode = *pane.term.lock().mode();
    let mouse_mode = mode.intersects(TermMode::MOUSE_MODE) && !mods.shift;
    out.menu_allowed = !mouse_mode && s.right_click_menu;

    if hovered || pane.selecting {
        ui.ctx().set_cursor_icon(if mouse_mode { CursorIcon::Default } else { CursorIcon::Text });
    }

    let Some(p) = pointer else { return out };
    let offset = pane.term.lock().grid().display_offset();
    let (point, side, col, row) = point_at(p, text_rect, m, pane.size, offset);

    // Wheel.
    if hovered && wheel != 0.0 {
        pane.scroll_accum += wheel;
        let lines = pane.scroll_accum.trunc() as i32;
        pane.scroll_accum -= lines as f32;
        if lines != 0 {
            if mouse_mode {
                let button = if lines > 0 { 64 } else { 65 };
                for _ in 0..lines.unsigned_abs().min(8) {
                    if let Some(b) = mouse_report(button, MouseAction::Press, col, row, mods, mode) {
                        pane.write(b);
                    }
                }
            } else if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
                let seq: &[u8] = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
                let bytes: Vec<u8> = seq.repeat(lines.unsigned_abs() as usize);
                pane.write(bytes);
            } else {
                let mut t = pane.term.lock();
                t.scroll_display(Scroll::Delta(lines));
                let at_bottom = t.grid().display_offset() == 0;
                drop(t);
                if at_bottom && !pane.selecting {
                    // Back at the bottom: follow output again.
                    pane.set_lock(false);
                }
            }
        }
    }

    // Presses.
    if hovered && pressed.iter().any(|b| *b) {
        out.focus = true;
    }
    if mouse_mode {
        let cell = (col, row);
        for (i, btn) in [0u8, 1, 2].into_iter().enumerate() {
            if hovered && pressed[i] {
                if let Some(b) = mouse_report(btn, MouseAction::Press, col, row, mods, mode) {
                    pane.write(b);
                }
                pane.mouse_reported_button = Some(btn);
                pane.last_mouse_cell = Some(cell);
            }
            if released[i] && pane.mouse_reported_button == Some(btn) {
                if let Some(b) = mouse_report(btn, MouseAction::Release, col, row, mods, mode) {
                    pane.write(b);
                }
                pane.mouse_reported_button = None;
            }
        }
        let motion = mode.contains(TermMode::MOUSE_MOTION) || (mode.contains(TermMode::MOUSE_DRAG) && pane.mouse_reported_button.is_some());
        if motion && (hovered || pane.mouse_reported_button.is_some()) && pane.last_mouse_cell != Some(cell) {
            pane.last_mouse_cell = Some(cell);
            let btn = pane.mouse_reported_button.unwrap_or(3);
            if let Some(b) = mouse_report(btn, MouseAction::Motion, col, row, mods, mode) {
                pane.write(b);
            }
        }
        return out;
    }

    // Selection.
    if hovered && pressed[0] {
        let now = Instant::now();
        let repeat = pane.last_click.is_some_and(|(t, at)| now.duration_since(t) < Duration::from_millis(400) && at.distance(p) < 5.0);
        pane.click_count = if repeat { pane.click_count % 3 + 1 } else { 1 };
        pane.last_click = Some((now, p));

        let mut t = pane.term.lock();
        let extend = mods.shift && pane.click_count == 1 && t.selection.is_some();
        if extend {
            if let Some(sel) = &mut t.selection {
                sel.update(point, side);
            }
        } else {
            let ty = match pane.click_count {
                2 => SelectionType::Semantic,
                3 => SelectionType::Lines,
                _ if mods.alt => SelectionType::Block,
                _ => SelectionType::Simple,
            };
            t.selection = Some(Selection::new(ty, point, side));
        }
        drop(t);
        pane.selecting = true;
        if s.lock_on_select {
            pane.set_hold(true);
        }
    }

    if pane.selecting && primary_down {
        // Auto-scroll when dragging past the top or bottom edge.
        let mut t = pane.term.lock();
        if p.y < text_rect.min.y {
            let n = (((text_rect.min.y - p.y) / m.ch).ceil() as i32).clamp(1, 10);
            t.scroll_display(Scroll::Delta(n));
            ui.ctx().request_repaint();
        } else if p.y > text_rect.max.y {
            let n = (((p.y - text_rect.max.y) / m.ch).ceil() as i32).clamp(1, 10);
            t.scroll_display(Scroll::Delta(-n));
            ui.ctx().request_repaint();
        }
        let offset = t.grid().display_offset();
        let (point, side, _, _) = point_at(p, text_rect, m, pane.size, offset);
        if let Some(sel) = &mut t.selection {
            sel.update(point, side);
        }
    }

    if pane.selecting && released[0] {
        pane.selecting = false;
        let mut t = pane.term.lock();
        let empty = t.selection.as_ref().is_none_or(|s| s.is_empty());
        if empty {
            t.selection = None;
            drop(t);
            pane.set_hold(false);
        } else {
            out.copy = t.selection_to_string().filter(|s| !s.is_empty());
        }
    }

    if hovered && pressed[1] && s.middle_click_paste {
        out.paste = Some(cfg!(target_os = "linux"));
    }
    if hovered && pressed[2] && !s.right_click_menu {
        out.paste = Some(false);
    }
    out
}

/// The "view held" badge in a terminal's corner; returns true when clicked (jump to the bottom).
pub fn hold_badge(ui: &mut Ui, text_rect: Rect, pane: &Pane, s: &Settings) -> bool {
    let offset = pane.term.lock().grid().display_offset();
    let locked = pane.shared.locked.load(std::sync::atomic::Ordering::Relaxed);
    if offset == 0 && !locked {
        return false;
    }
    let held = pane.shared.held_lines.load(std::sync::atomic::Ordering::Relaxed);
    let mut label = if locked { "⏸ Scroll locked".to_string() } else { "⏸ Scrolled back".to_string() };
    if held > 0 {
        label.push_str(&format!(" · {held} new lines"));
    }
    label.push_str("  ↓");
    let font = FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(label, font, Color32::WHITE);
    let size = galley.size() + vec2(16.0, 8.0);
    let rect = Rect::from_min_size(text_rect.max - size - vec2(8.0, 6.0), size);
    let resp = ui.interact(rect, ui.id().with(("badge", pane.id)), Sense::click());
    let accent = s.theme.accent.color();
    let fill = if resp.hovered() { accent } else { accent.gamma_multiply(0.85) };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().galley(rect.min + vec2(8.0, 4.0), galley, Color32::WHITE);
    resp.clone().on_hover_text("Output keeps arriving below. Click to jump to the bottom and follow it.");
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.clicked()
}
