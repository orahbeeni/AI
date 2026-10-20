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
use crate::links::{self, Target};
use crate::pane::{LinkHover, Pane, TermSize};

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

    // The link under the pointer.
    if let Some(h) = &pane.link_hover {
        let col = theme.accent.color();
        for &(line, c0, c1) in &h.segments {
            let row = line + offset;
            if row >= 0 && (row as usize) < term.screen_lines() {
                let r = cell_rect(row as usize, c0, c1 - c0);
                painter.hline(r.x_range(), (r.max.y - 1.5).round() + 0.5, Stroke::new(1.5, col));
            }
        }
    }

    // A bar beside each command's output: green when it succeeded, red when it failed.
    if s.command_marks && s.padding >= 6.0 {
        let marks = pane.shared.marks.lock().unwrap();
        if marks.seen {
            let rows = term.screen_lines() as i32;
            for b in marks.blocks.iter() {
                let Some(out) = b.output else { continue };
                // From the prompt through the output; a command that printed nothing still gets its own line.
                let first = marks.line(b.prompt);
                let last = match b.last_output_line() {
                    Some(k) => marks.line(k),
                    None if b.running() => cursor.point.line.0,
                    None => marks.line(out) - 1,
                };
                let (r0, r1) = ((first + offset).max(0), (last + offset).min(rows - 1));
                if r1 < r0 {
                    continue;
                }
                let colour = if b.running() {
                    theme.accent.color()
                } else if b.exit.unwrap_or(0) == 0 {
                    Color32::from_rgb(80, 190, 110)
                } else {
                    Color32::from_rgb(230, 80, 80)
                };
                let top = cell_rect(r0 as usize, 0, 1).min.y + 1.0;
                let bottom = cell_rect(r1 as usize, 0, 1).max.y - 1.0;
                let x = origin.x - 6.0;
                painter.rect_filled(Rect::from_min_max(pos2(x, top), pos2(x + 2.5, bottom)), 1.0, colour);
            }
        }
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
    /// A link was clicked.
    pub open: Option<Target>,
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

    let Some(p) = pointer else {
        pane.link_hover = None;
        return out;
    };
    let offset = pane.term.lock().grid().display_offset();
    let (point, side, col, row) = point_at(p, text_rect, m, pane.size, offset);

    // Links: with the link key held (Ctrl, or ⌘ on macOS) URLs and file names under the pointer light up.
    let link_key = if cfg!(target_os = "macos") { mods.mac_cmd } else { mods.ctrl };
    pane.link_hover = if s.clickable_links && link_key && hovered && !mouse_mode && !pane.selecting { link_at(pane, point) } else { None };
    if pane.link_hover.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        if pressed[0] {
            out.open = pane.link_hover.as_ref().map(|h| h.target.clone());
        }
    }

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
    if hovered && pressed[0] && out.open.is_none() {
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

/// The URL or file name at `point`, if any.
fn link_at(pane: &mut Pane, point: Point) -> Option<LinkHover> {
    let (segments, target) = {
        let t = pane.term.lock();
        let grid = t.grid();
        let cols = t.columns();
        if point.line < t.topmost_line() || point.line > t.bottommost_line() {
            return None;
        }

        // An explicit hyperlink (OSC 8) wins over guessing from the text.
        if let Some(link) = grid[point].hyperlink() {
            let uri = link.uri().to_string();
            let same = |c: usize| grid[Point::new(point.line, Column(c))].hyperlink().is_some_and(|l| l.uri() == uri);
            let (mut a, mut b) = (point.column.0, point.column.0);
            while a > 0 && same(a - 1) {
                a -= 1;
            }
            while b + 1 < cols && same(b + 1) {
                b += 1;
            }
            return Some(LinkHover { segments: vec![(point.line.0, a, b + 1)], target: Target::Url(uri) });
        }

        // The logical line around the pointer: rows that wrap into each other read as one.
        let wraps = |line: i32| grid[Point::new(Line(line), Column(cols - 1))].flags.contains(Flags::WRAPLINE);
        let (mut first, mut last) = (point.line.0, point.line.0);
        while first > t.topmost_line().0 && point.line.0 - first < 20 && wraps(first - 1) {
            first -= 1;
        }
        while last < t.bottommost_line().0 && last - point.line.0 < 20 && wraps(last) {
            last += 1;
        }
        let mut text = String::new();
        let mut cells: Vec<(i32, usize, usize)> = Vec::new();
        let mut at = None;
        for line in first..=last {
            for c in 0..cols {
                let cell = &grid[Point::new(Line(line), Column(c))];
                if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
                if line == point.line.0 && (c..c + width).contains(&point.column.0) {
                    at = Some(cells.len());
                }
                text.push(if cell.c == '\0' || cell.c == '\t' { ' ' } else { cell.c });
                cells.push((line, c, width));
            }
        }
        let hit = links::link_at(&text, at?)?;
        let mut segments: Vec<(i32, usize, usize)> = Vec::new();
        for &(line, c, w) in &cells[hit.start..hit.end] {
            match segments.last_mut() {
                Some((l, _, end)) if *l == line && *end == c => *end = c + w,
                _ => segments.push((line, c, c + w)),
            }
        }
        (segments, hit.target)
    };

    // A file name only counts when it leads somewhere; looking is cached because the hover repeats every frame.
    if let Target::File { path, .. } = &target {
        let key = format!("{}\0{path}", pane.cwd.display());
        let fresh = pane.link_cache.get(&key).filter(|(when, _)| when.elapsed() < Duration::from_secs(20)).map(|(_, f)| f.clone());
        let found = fresh.unwrap_or_else(|| {
            let f = links::resolve(path, &pane.cwd);
            if pane.link_cache.len() > 300 {
                pane.link_cache.clear();
            }
            pane.link_cache.insert(key, (Instant::now(), f.clone()));
            f
        });
        found?;
    }
    Some(LinkHover { segments, target })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use alacritty_terminal::grid::Dimensions;

    use super::*;
    use crate::config::Settings;

    /// The cell at which `needle` starts on the screen, once the shell has printed it.
    fn find_cell(pane: &Pane, needle: &str) -> Point {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            {
                let t = pane.term.lock();
                for line in t.topmost_line().0..=t.bottommost_line().0 {
                    let text = t.bounds_to_string(Point::new(Line(line), Column(0)), Point::new(Line(line), t.last_column()));
                    // The shell echoes the command too; the output is the line that does not start at the prompt.
                    if let Some(col) = text.find(needle).filter(|_| !text.contains("printf")) {
                        return Point::new(Line(line), Column(col));
                    }
                }
            }
            assert!(Instant::now() < deadline, "never saw {needle:?}:\n{}", pane.scrollback_text(30));
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    #[cfg(unix)]
    fn links_under_the_pointer() {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!("term-linkview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "x").unwrap();

        let ctx = egui::Context::default();
        let mut s = Settings::default();
        s.shell = "/bin/sh".into();
        let mut pane = Pane::spawn(1, &s, Some(dir.clone()), TermSize { cols: 100, rows: 24 }, (8.0, 16.0), &ctx, None).unwrap();
        pane.run_command(r"printf 'see %s:42 and %s, gone %s now\n' src/lib.rs https://example.com/a/b. nowhere/x.rs");
        pane.run_command(r"printf '\033]8;;https://hidden.test/page\033\\click me\033]8;;\033\\ ok\n'");

        // A file that exists, with its line number.
        let at = find_cell(&pane, "src/lib.rs:42");
        let hover = link_at(&mut pane, Point::new(at.line, Column(at.column.0 + 3))).expect("file link");
        assert_eq!(hover.target, Target::File { path: "src/lib.rs".into(), line: Some(42), col: None });
        assert_eq!(hover.segments, vec![(at.line.0, at.column.0, at.column.0 + "src/lib.rs:42".len())]);

        // A URL without the sentence's full stop.
        let at = find_cell(&pane, "https://example.com");
        let hover = link_at(&mut pane, Point::new(at.line, Column(at.column.0 + 5))).expect("url link");
        assert_eq!(hover.target, Target::Url("https://example.com/a/b".into()));
        assert_eq!(hover.segments[0].2 - hover.segments[0].1, "https://example.com/a/b".len());

        // A path that leads nowhere is not a link, and neither is plain text.
        let at = find_cell(&pane, "nowhere/x.rs");
        assert!(link_at(&mut pane, at).is_none());
        let at = find_cell(&pane, "and");
        assert!(link_at(&mut pane, at).is_none());

        // An explicit hyperlink uses its own address, not the visible text.
        let at = find_cell(&pane, "click me");
        let hover = link_at(&mut pane, Point::new(at.line, Column(at.column.0 + 2))).expect("hyperlink");
        assert_eq!(hover.target, Target::Url("https://hidden.test/page".into()));
        assert_eq!(hover.segments, vec![(at.line.0, at.column.0, at.column.0 + "click me".len())]);
        let _ = pane.term.lock().columns();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
