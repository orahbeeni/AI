//! Small vector icons drawn with the painter (no image assets).
//!
//! Every icon is designed in a box of roughly ±7 units around its centre and
//! scaled by the caller, so one definition serves the toolbar and the sidebar.

use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Vec2};
use gitgoing_core::diff::ChangeKind;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    None,
    Branch,
    Folder,
    Tag,
    Undo,
    Redo,
    Fetch,
    Pull,
    Push,
    Stash,
    Pop,
    Terminal,
    Search,
    ArrowUp,
    ArrowDown,
    Unified,
    Split,
    Pilcrow,
    Pencil,
    Trash,
    Gear,
}

/// Points along an arc of radius `r` around `c`, from angle `a0` to `a1` (radians, y down).
fn arc(c: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    let n = 14;
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect()
}

/// Filled arrow head with its tip at `tip`, pointing along `dir` (unit vector).
fn head(p: &egui::Painter, tip: Pos2, dir: Vec2, size: f32, color: Color32) {
    let perp = Vec2::new(-dir.y, dir.x);
    let base = tip - dir * size;
    p.add(Shape::convex_polygon(vec![tip, base + perp * size * 0.7, base - perp * size * 0.7], color, Stroke::NONE));
}

/// Draw `icon` centred on `c`; `scale` 1.0 is about 14 px.
pub fn draw(p: &egui::Painter, c: Pos2, icon: Icon, scale: f32, color: Color32) {
    let s = scale;
    let st = Stroke::new(1.3_f32 * s.clamp(1.0, 1.35), color);
    let v = |x: f32, y: f32| c + Vec2::new(x, y) * s;
    let line = |a: Pos2, b: Pos2| p.line_segment([a, b], st);
    let poly = |pts: Vec<Pos2>| {
        p.add(Shape::line(pts, st));
    };
    match icon {
        Icon::None => {}
        Icon::Branch => {
            line(v(-3.0, -4.0), v(-3.0, 4.0));
            p.circle_stroke(v(-3.0, -5.0), 2.0 * s, st);
            p.circle_stroke(v(-3.0, 5.0), 2.0 * s, st);
            p.circle_stroke(v(4.0, -3.0), 2.0 * s, st);
            line(v(4.0, -1.0), v(4.0, 0.0));
            line(v(4.0, 0.0), v(-3.0, 3.0));
        }
        Icon::Folder => {
            let body = Rect::from_center_size(v(0.0, 1.0), Vec2::new(13.0, 9.0) * s);
            p.rect_stroke(body, 1.5, st, egui::StrokeKind::Inside);
            line(body.left_top() + Vec2::new(0.0, -1.5) * s, body.left_top() + Vec2::new(4.5, -1.5) * s);
        }
        Icon::Tag => {
            p.add(Shape::closed_line(
                vec![v(-5.0, -3.5), v(1.0, -3.5), v(6.0, 0.0), v(1.0, 3.5), v(-5.0, 3.5)],
                st,
            ));
            p.circle_filled(v(-2.0, 0.0), 1.2 * s, color);
        }
        Icon::Undo | Icon::Redo => {
            // Counter-clockwise arc ending in an arrow head; Redo is the mirror image.
            let m = if icon == Icon::Redo { -1.0 } else { 1.0 };
            let (a0, a1) = (0.35_f32, -2.55_f32);
            let pts: Vec<Pos2> = arc(Pos2::ZERO, 5.2, a0, a1)
                .into_iter()
                .map(|q| v(q.x * m, q.y + 1.0))
                .collect();
            let end = *pts.last().unwrap();
            poly(pts);
            // Tangent at the end of the arc (angle decreasing).
            let dir = Vec2::new(a1.sin() * m, -a1.cos());
            head(p, end + dir * 2.2 * s, dir, 3.4 * s, color);
        }
        Icon::Fetch => {
            // Circular refresh arrow.
            let (a0, a1) = (0.5_f32, 5.0_f32);
            let pts: Vec<Pos2> = arc(c, 5.4 * s, a0, a1);
            let end = *pts.last().unwrap();
            poly(pts);
            let dir = Vec2::new(-a1.sin(), a1.cos());
            head(p, end + dir * 2.0 * s, dir, 3.4 * s, color);
        }
        Icon::Pull => {
            line(v(0.0, -7.0), v(0.0, 2.0));
            head(p, v(0.0, 5.5), Vec2::new(0.0, 1.0), 4.0 * s, color);
            line(v(-6.5, 7.5), v(6.5, 7.5));
        }
        Icon::Push => {
            line(v(0.0, 4.5), v(0.0, -3.0));
            head(p, v(0.0, -7.0), Vec2::new(0.0, -1.0), 4.0 * s, color);
            line(v(-6.5, 7.5), v(6.5, 7.5));
        }
        Icon::Stash => {
            poly(vec![v(-6.5, 1.0), v(-6.5, 7.0), v(6.5, 7.0), v(6.5, 1.0)]);
            line(v(0.0, -7.0), v(0.0, 1.0));
            head(p, v(0.0, 4.5), Vec2::new(0.0, 1.0), 4.0 * s, color);
        }
        Icon::Pop => {
            poly(vec![v(-6.5, 1.0), v(-6.5, 7.0), v(6.5, 7.0), v(6.5, 1.0)]);
            line(v(0.0, 4.0), v(0.0, -4.0));
            head(p, v(0.0, -7.5), Vec2::new(0.0, -1.0), 4.0 * s, color);
        }
        Icon::Terminal => {
            poly(vec![v(-6.0, -4.5), v(-1.0, 0.0), v(-6.0, 4.5)]);
            line(v(1.0, 5.0), v(7.0, 5.0));
        }
        Icon::ArrowUp => {
            line(v(0.0, 6.0), v(0.0, -3.0));
            head(p, v(0.0, -7.0), Vec2::new(0.0, -1.0), 4.0 * s, color);
        }
        Icon::ArrowDown => {
            line(v(0.0, -6.0), v(0.0, 3.0));
            head(p, v(0.0, 7.0), Vec2::new(0.0, 1.0), 4.0 * s, color);
        }
        Icon::Unified => {
            p.rect_stroke(Rect::from_center_size(c, Vec2::new(14.0, 12.0) * s), 1.5, st, egui::StrokeKind::Inside);
            for y in [-3.0, 0.0, 3.0] {
                line(v(-4.0, y), v(4.0, y));
            }
        }
        Icon::Split => {
            p.rect_stroke(Rect::from_center_size(c, Vec2::new(14.0, 12.0) * s), 1.5, st, egui::StrokeKind::Inside);
            line(v(0.0, -6.0), v(0.0, 6.0));
        }
        Icon::Pilcrow => {
            p.circle_filled(v(-1.5, -3.0), 3.2 * s, color);
            line(v(1.5, -6.5), v(1.5, 6.5));
            line(v(4.5, -6.5), v(4.5, 6.5));
        }
        Icon::Trash => {
            line(v(-6.0, -3.5), v(6.0, -3.5));
            poly(vec![v(-2.5, -3.5), v(-2.5, -6.0), v(2.5, -6.0), v(2.5, -3.5)]);
            poly(vec![v(-4.5, -1.5), v(-3.8, 6.5), v(3.8, 6.5), v(4.5, -1.5)]);
            line(v(-1.2, 0.0), v(-1.2, 4.5));
            line(v(1.2, 0.0), v(1.2, 4.5));
        }
        Icon::Gear => {
            p.circle_stroke(c, 2.6 * s, st);
            p.circle_stroke(c, 5.0 * s, st);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let d = Vec2::new(a.cos(), a.sin());
                p.line_segment([c + d * 5.0 * s, c + d * 7.2 * s], Stroke::new(2.0_f32 * s.clamp(1.0, 1.3), color));
            }
        }
        Icon::Pencil => {
            p.line_segment([v(-2.5, 3.0), v(4.0, -3.5)], Stroke::new(2.4_f32 * s.clamp(1.0, 1.35), color));
            p.add(Shape::convex_polygon(vec![v(-5.5, 6.0), v(-5.5, 2.5), v(-2.0, 6.0)], color, Stroke::NONE));
        }
        Icon::Search => {
            p.circle_stroke(v(-1.5, -1.5), 4.5 * s, st);
            line(v(1.8, 1.8), v(6.5, 6.5));
        }
    }
}

/// A toolbar button: small caption on top, icon below. Returns the click response.
pub fn tool_button(ui: &mut egui::Ui, caption: &str, icon: Icon, enabled: bool) -> Response {
    let size = Vec2::new(54.0, 50.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let visuals = ui.visuals();
    let p = ui.painter_at(rect);
    if enabled && resp.hovered() {
        p.rect_filled(rect.shrink(1.0), 6.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let color = if !enabled {
        visuals.weak_text_color().gamma_multiply(0.55)
    } else if resp.hovered() {
        visuals.strong_text_color()
    } else {
        visuals.text_color()
    };
    let font = egui::TextStyle::Small.resolve(ui.style());
    p.text(
        Pos2::new(rect.center().x, rect.top() + 9.0),
        egui::Align2::CENTER_CENTER,
        caption,
        font,
        visuals.weak_text_color().gamma_multiply(if enabled { 1.0 } else { 0.7 }),
    );
    draw(&p, Pos2::new(rect.center().x, rect.top() + 31.0), icon, 1.45, color);
    resp
}

/// A coloured icon for a file's change kind (replaces the old M/A/D letters).
pub fn change_badge(ui: &mut egui::Ui, kind: ChangeKind) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
    let p = ui.painter_at(rect);
    let c = rect.center();
    let col = crate::detail::kind_color(kind);
    let st = Stroke::new(1.4_f32, col);
    let bold = Stroke::new(1.9_f32, col);
    let v = |x: f32, y: f32| c + Vec2::new(x, y);
    let plus = |r: f32, st: Stroke| {
        p.line_segment([v(-r, 0.0), v(r, 0.0)], st);
        p.line_segment([v(0.0, -r), v(0.0, r)], st);
    };
    let name = match kind {
        ChangeKind::Added => {
            plus(4.5, bold);
            "Added"
        }
        ChangeKind::Untracked => {
            plus(4.5, bold);
            "Untracked (new file)"
        }
        ChangeKind::Deleted => {
            p.line_segment([v(-4.5, 0.0), v(4.5, 0.0)], bold);
            "Deleted"
        }
        ChangeKind::Modified => {
            // Pencil.
            p.line_segment([v(-2.5, 3.0), v(3.5, -3.0)], Stroke::new(2.6_f32, col));
            p.add(Shape::convex_polygon(vec![v(-5.0, 5.5), v(-5.0, 2.0), v(-1.5, 5.5)], col, Stroke::NONE));
            "Modified"
        }
        ChangeKind::Renamed => {
            p.line_segment([v(-5.0, 0.0), v(2.0, 0.0)], bold);
            head(&p, v(6.0, 0.0), Vec2::new(1.0, 0.0), 4.5, col);
            "Renamed"
        }
        ChangeKind::Copied => {
            p.rect_stroke(Rect::from_center_size(v(-1.5, -1.5), Vec2::splat(8.0)), 1.0, st, egui::StrokeKind::Inside);
            p.rect_stroke(Rect::from_center_size(v(1.5, 1.5), Vec2::splat(8.0)), 1.0, st, egui::StrokeKind::Inside);
            "Copied"
        }
        ChangeKind::Conflicted => {
            p.add(Shape::closed_line(vec![v(0.0, -7.0), v(7.5, 6.0), v(-7.5, 6.0)], st));
            p.line_segment([v(0.0, -2.0), v(0.0, 2.0)], st);
            p.circle_filled(v(0.0, 4.0), 0.9, col);
            "Conflicted"
        }
        ChangeKind::Other => {
            p.circle_filled(c, 2.5, col);
            "Changed"
        }
    };
    resp.on_hover_text(name)
}

/// A small square icon button (toggle look when `selected`).
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, selected: bool, tip: &str, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(30.0, 26.0), sense);
    let v = ui.visuals();
    let p = ui.painter_at(rect);
    if selected {
        p.rect_filled(rect, 4.0, v.selection.bg_fill);
    } else if enabled && resp.hovered() {
        p.rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
    }
    let color = if !enabled {
        v.weak_text_color().gamma_multiply(0.5)
    } else if selected || resp.hovered() {
        v.strong_text_color()
    } else {
        v.text_color()
    };
    draw(&p, rect.center(), icon, 1.1, color);
    resp.on_hover_text(tip)
}

/// GK-style accent fill for selected chips and segments.
pub fn accent(v: &egui::Visuals) -> Color32 {
    if v.dark_mode {
        Color32::from_rgb(0x26, 0x37, 0x54)
    } else {
        v.selection.bg_fill
    }
}

/// An outlined button with an icon, e.g. "Edit this file".
pub fn outlined_button(ui: &mut egui::Ui, icon: Icon, text: &str) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, ui.visuals().text_color());
    let size = Vec2::new(30.0 + galley.size().x + 10.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let v = ui.visuals();
    let p = ui.painter_at(rect);
    if resp.hovered() {
        p.rect_filled(rect, 3.0, accent(v));
    }
    p.rect_stroke(rect, 3.0, Stroke::new(1.0_f32, Color32::from_rgb(0x3b, 0x6e, 0xd9)), egui::StrokeKind::Inside);
    draw(&p, Pos2::new(rect.left() + 16.0, rect.center().y), icon, 1.0, v.text_color());
    p.galley(Pos2::new(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0), galley, v.text_color());
    resp
}

/// A static highlighted label, e.g. the "Unstaged" chip.
pub fn chip(ui: &mut egui::Ui, text: &str) {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, ui.visuals().text_color());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(galley.size().x + 24.0, 26.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, accent(ui.visuals()));
    p.galley(
        Pos2::new(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ui.visuals().text_color(),
    );
}

/// A bordered group of buttons, one optionally selected. Returns the clicked index.
pub fn segmented(ui: &mut egui::Ui, labels: &[&str], selected: Option<usize>, enabled: bool) -> Option<usize> {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let color = if enabled { ui.visuals().text_color() } else { ui.visuals().weak_text_color().gamma_multiply(0.7) };
    let galleys: Vec<_> = labels
        .iter()
        .map(|l| ui.painter().layout_no_wrap((*l).to_owned(), font.clone(), color))
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 24.0).collect();
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, 26.0), Sense::hover());
    let p = ui.painter_at(rect);
    let border = Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color);
    let mut clicked = None;
    let mut x = rect.left();
    for (i, (g, w)) in galleys.into_iter().zip(widths).enumerate() {
        let seg = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w, rect.height()));
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let r = ui.interact(seg, ui.id().with(("segment", labels[i])), sense);
        if selected == Some(i) {
            p.rect_filled(seg, 0.0, accent(ui.visuals()));
        } else if enabled && r.hovered() {
            p.rect_filled(seg, 0.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        p.rect_stroke(seg, 0.0, border, egui::StrokeKind::Inside);
        p.galley(Pos2::new(seg.left() + 12.0, seg.center().y - g.size().y / 2.0), g, color);
        if r.clicked() {
            clicked = Some(i);
        }
        x += w;
    }
    clicked
}

/// A compact icon-only button for list rows.
pub fn small_icon_button(ui: &mut egui::Ui, icon: Icon, tip: &str, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.0), sense);
    let v = ui.visuals();
    let p = ui.painter_at(rect);
    if enabled && resp.hovered() {
        p.rect_filled(rect, 3.0, v.widgets.hovered.weak_bg_fill);
    }
    let color = if !enabled {
        v.weak_text_color().gamma_multiply(0.5)
    } else if resp.hovered() {
        v.strong_text_color()
    } else {
        v.weak_text_color()
    };
    draw(&p, rect.center(), icon, 0.95, color);
    resp.on_hover_text(tip)
}

pub enum TabAction {
    None,
    Select,
    Close,
}

/// One tab in the tab bar: rounded top corners, a branch icon, the title, and a
/// close button. The active tab is lifted to the toolbar colour with an accent line.
/// Returns what the user did, and the tab's rectangle.
pub fn tab_button(ui: &mut egui::Ui, title: &str, tooltip: &str, active: bool) -> (TabAction, Rect) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(title.to_owned(), font.clone(), ui.visuals().text_color());
    let width = (galley.size().x + 22.0 + 30.0 + 14.0).clamp(130.0, 260.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    let close_rect = Rect::from_center_size(Pos2::new(rect.right() - 16.0, rect.center().y + 1.0), Vec2::splat(18.0));
    let close = ui.interact(close_rect, resp.id.with("close"), Sense::click());

    let v = ui.visuals();
    let radius = egui::CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 };
    let p = ui.painter_at(rect.expand2(Vec2::new(0.0, 2.0)));
    let hovered = resp.hovered() || close.hovered();
    let fill = if active {
        v.panel_fill
    } else if hovered {
        v.widgets.hovered.weak_bg_fill
    } else {
        v.panel_fill.gamma_multiply(0.55)
    };
    p.rect_filled(rect, radius, fill);
    if !active {
        p.rect_stroke(rect, radius, Stroke::new(1.0_f32, v.widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
    } else {
        // Accent line across the top of the selected tab.
        let top = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 2.5));
        p.rect_filled(top, egui::CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, Color32::from_rgb(0x3b, 0x6e, 0xd9));
    }

    let text_col = if active { v.strong_text_color() } else { v.weak_text_color() };
    draw(&p, Pos2::new(rect.left() + 17.0, rect.center().y + 1.0), Icon::Branch, 0.85, text_col);
    // Title is cut off before the close button rather than wrapped.
    let clip = Rect::from_min_max(
        Pos2::new(rect.left() + 30.0, rect.top()),
        Pos2::new(rect.right() - 30.0, rect.bottom()),
    );
    ui.painter_at(clip).text(
        Pos2::new(rect.left() + 30.0, rect.center().y + 1.0),
        egui::Align2::LEFT_CENTER,
        title,
        font,
        text_col,
    );

    if active || hovered {
        if close.hovered() {
            p.circle_filled(close_rect.center(), 8.5, v.widgets.hovered.bg_fill);
        }
        let c = close_rect.center();
        let st = Stroke::new(1.4_f32, if close.hovered() { v.strong_text_color() } else { v.weak_text_color() });
        p.line_segment([c + Vec2::new(-3.0, -3.0), c + Vec2::new(3.0, 3.0)], st);
        p.line_segment([c + Vec2::new(-3.0, 3.0), c + Vec2::new(3.0, -3.0)], st);
    }
    resp.clone().on_hover_text(tooltip);
    let action = if close.clicked() || resp.middle_clicked() {
        TabAction::Close
    } else if resp.clicked() {
        TabAction::Select
    } else {
        TabAction::None
    };
    (action, rect)
}
