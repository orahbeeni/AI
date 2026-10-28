//! The screen arrangement: this computer's monitors in the middle, each client beside the edge
//! it sits on. Clients are dragged to another edge to move them.

use egui::{Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::geom::{Screen, Side};

pub struct Machine {
    pub name: String,
    pub screen: Screen,
    pub connected: bool,
    /// Has the cursor right now.
    pub active: bool,
}

pub struct Placed {
    pub machine: Machine,
    pub side: Side,
}

/// A client being dragged: its name and how far it has moved.
#[derive(Default)]
pub struct DragState {
    name: Option<String>,
    offset: Vec2,
}

const GAP: f32 = 14.0;
const PAD: f32 = 16.0;

/// Draws the arrangement; returns a client moved to a new edge.
pub fn canvas(ui: &mut Ui, server: &Machine, clients: &[Placed], drag: &mut DragState, height: f32) -> Option<(String, Side)> {
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let visuals = ui.visuals().clone();
    let painter = ui.painter_at(area);
    painter.rect_filled(area, CornerRadius::same(10), visuals.extreme_bg_color);

    // One scale for everything, so relative monitor sizes stay true.
    let size = |s: &Screen| {
        let b = s.bbox();
        vec2(b.w.max(1) as f32, b.h.max(1) as f32)
    };
    let extent = |side: Side| -> Vec2 {
        clients.iter().filter(|c| c.side == side).fold(Vec2::ZERO, |acc, c| {
            let s = size(&c.machine.screen);
            match side {
                Side::Left | Side::Right => vec2(acc.x.max(s.x), acc.y + s.y),
                Side::Top | Side::Bottom => vec2(acc.x + s.x, acc.y.max(s.y)),
            }
        })
    };
    let sv = size(&server.screen);
    let (l, r, t, b) = (extent(Side::Left), extent(Side::Right), extent(Side::Top), extent(Side::Bottom));
    let gaps_x = GAP * ((l.x > 0.0) as u8 + (r.x > 0.0) as u8) as f32;
    let gaps_y = GAP * ((t.y > 0.0) as u8 + (b.y > 0.0) as u8) as f32;
    let fit_w = (area.width() - 2.0 * PAD - gaps_x) / (l.x + sv.x.max(t.x).max(b.x) + r.x);
    let fit_h = (area.height() - 2.0 * PAD - 18.0 - gaps_y) / (t.y + sv.y.max(l.y).max(r.y) + b.y);
    let scale = fit_w.min(fit_h).min(0.11);

    // Centre the whole arrangement.
    let total = vec2((l.x + sv.x + r.x) * scale + gaps_x, (t.y + sv.y + b.y) * scale + gaps_y);
    let origin = area.center() - total / 2.0;
    let server_min = pos2(origin.x + l.x * scale + if l.x > 0.0 { GAP } else { 0.0 }, origin.y + t.y * scale + if t.y > 0.0 { GAP } else { 0.0 });
    let server_rect = Rect::from_min_size(server_min, sv * scale);

    // Where each client goes: stacked along its edge, centred on the server.
    let mut placed: Vec<(usize, Rect)> = Vec::new();
    for side in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
        let on_side: Vec<usize> = (0..clients.len()).filter(|&i| clients[i].side == side).collect();
        let along: f32 = on_side.iter().map(|&i| size(&clients[i].machine.screen)).map(|s| if matches!(side, Side::Left | Side::Right) { s.y } else { s.x }).sum::<f32>() * scale
            + GAP * on_side.len().saturating_sub(1) as f32;
        let mut cursor = match side {
            Side::Left | Side::Right => server_rect.center().y - along / 2.0,
            Side::Top | Side::Bottom => server_rect.center().x - along / 2.0,
        };
        for i in on_side {
            let s = size(&clients[i].machine.screen) * scale;
            let min = match side {
                Side::Left => pos2(server_rect.left() - GAP - s.x, cursor),
                Side::Right => pos2(server_rect.right() + GAP, cursor),
                Side::Top => pos2(cursor, server_rect.top() - GAP - s.y),
                Side::Bottom => pos2(cursor, server_rect.bottom() + GAP),
            };
            cursor += if matches!(side, Side::Left | Side::Right) { s.y } else { s.x } + GAP;
            placed.push((i, Rect::from_min_size(min, s)));
        }
    }

    draw_machine(&painter, ui, server_rect, server, scale, true);
    painter.text(server_rect.center_bottom() + vec2(0.0, 4.0), Align2::CENTER_TOP, "this computer", FontId::proportional(11.0), visuals.weak_text_color());

    let mut moved = None;
    for (i, rect) in placed {
        let c = &clients[i];
        let id = ui.id().with(("client", &c.machine.name));
        let resp = ui.interact(rect, id, Sense::drag()).on_hover_cursor(CursorIcon::Grab);
        let mut rect = rect;
        if resp.drag_started() {
            *drag = DragState { name: Some(c.machine.name.clone()), offset: Vec2::ZERO };
        }
        if drag.name.as_deref() == Some(c.machine.name.as_str()) {
            drag.offset += resp.drag_delta();
            rect = rect.translate(drag.offset);
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            let target = side_of(server_rect, rect.center());
            highlight_edge(&painter, server_rect, target, visuals.selection.bg_fill);
            if resp.drag_stopped() {
                *drag = DragState::default();
                if target != c.side {
                    moved = Some((c.machine.name.clone(), target));
                }
            }
        }
        draw_machine(&painter, ui, rect, &c.machine, scale, false);
    }
    if clients.is_empty() {
        painter.text(
            pos2(area.center().x, area.bottom() - 12.0),
            Align2::CENTER_BOTTOM,
            "Paired computers appear here",
            FontId::proportional(12.0),
            visuals.weak_text_color(),
        );
    }
    moved
}

/// Which edge of `server` a point is beyond, judged relative to the server's proportions.
fn side_of(server: Rect, p: Pos2) -> Side {
    let d = p - server.center();
    let (nx, ny) = (d.x / server.width().max(1.0), d.y / server.height().max(1.0));
    if nx.abs() >= ny.abs() {
        if nx < 0.0 { Side::Left } else { Side::Right }
    } else if ny < 0.0 {
        Side::Top
    } else {
        Side::Bottom
    }
}

fn highlight_edge(painter: &egui::Painter, r: Rect, side: Side, color: Color32) {
    let (a, b) = match side {
        Side::Left => (r.left_top(), r.left_bottom()),
        Side::Right => (r.right_top(), r.right_bottom()),
        Side::Top => (r.left_top(), r.right_top()),
        Side::Bottom => (r.left_bottom(), r.right_bottom()),
    };
    painter.line_segment([a, b], Stroke::new(4.0, color));
}

fn draw_machine(painter: &egui::Painter, ui: &Ui, rect: Rect, m: &Machine, scale: f32, is_server: bool) {
    let v = ui.visuals();
    let accent = if v.dark_mode { Color32::from_rgb(90, 160, 255) } else { Color32::from_rgb(30, 110, 230) };
    let (fill, stroke) = if is_server {
        (v.widgets.inactive.bg_fill, Stroke::new(1.5, accent))
    } else if m.active {
        (accent.gamma_multiply(0.22), Stroke::new(2.5, accent))
    } else if m.connected {
        (v.widgets.inactive.bg_fill, Stroke::new(1.0, v.widgets.inactive.fg_stroke.color))
    } else {
        (v.faint_bg_color, Stroke::new(1.0, v.weak_text_color().gamma_multiply(0.6)))
    };
    let origin = m.screen.bbox();
    for mon in &m.screen.monitors {
        let mr = Rect::from_min_size(
            rect.min + vec2((mon.x - origin.x) as f32, (mon.y - origin.y) as f32) * scale,
            vec2(mon.w as f32, mon.h as f32) * scale,
        )
        .shrink(1.0);
        painter.rect_filled(mr, CornerRadius::same(4), fill);
        painter.rect_stroke(mr, CornerRadius::same(4), stroke, StrokeKind::Inside);
    }
    let text_color = if m.connected || is_server { v.strong_text_color() } else { v.weak_text_color() };
    let galley = painter.layout(m.name.clone(), FontId::proportional(12.5), text_color, rect.width().max(60.0) - 6.0);
    let pos = rect.center() - galley.size() / 2.0;
    if m.active {
        // Where the cursor is now.
        painter.circle_filled(pos + vec2(galley.size().x + 9.0, galley.size().y / 2.0), 4.0, accent);
    }
    painter.galley(pos, galley, text_color);
}
