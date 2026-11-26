//! The trim bar: a track with a draggable start and end handle, plus time formatting and parsing.

use eframe::egui::{self, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Handle {
    Start,
    End,
}

/// Where one video of a combined project ends and the next begins.
#[derive(Clone, Debug, PartialEq)]
pub struct Join {
    /// The time on the timeline where the next video starts.
    pub at: f64,
    /// 1-based position of the video that starts here.
    pub number: usize,
    pub before: String,
    pub after: String,
}

/// The nearest join strictly before (`dir < 0`) or after (`dir > 0`) `t`, with a little room so that standing on a
/// join finds the next one.
pub fn neighbour_join(joins: &[Join], t: f64, dir: i32) -> Option<f64> {
    const ROOM: f64 = 0.05;
    let times = joins.iter().map(|j| j.at);
    if dir < 0 { times.filter(|&a| a < t - ROOM).last() } else { times.filter(|&a| a > t + ROOM).next() }
}

/// How close, in pixels, a dragged handle has to come to a join to snap to it.
const SNAP_PX: f32 = 7.0;
const JOIN_COLOR: Color32 = Color32::from_rgb(255, 176, 46);

/// The shortest part that can be selected.
pub fn min_length(duration: f64) -> f64 {
    0.1_f64.min(duration / 2.0)
}

/// `1:05.25`, or `1:02:05.25` for long videos.
pub fn format_time(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as u64;
    let (h, m, s, c) = (cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{c:02}") } else { format!("{m}:{s:02}.{c:02}") }
}

/// Accepts `90`, `1:30`, `1:30.5` and `0:01:30`.
pub fn parse_time(text: &str) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut total = 0.0;
    for part in &parts {
        let v: f64 = part.trim().parse().ok().filter(|v: &f64| v.is_finite() && *v >= 0.0)?;
        total = total * 60.0 + v;
    }
    Some(total)
}

/// Draws the bar. Returns which handle moved, if one did. `playhead` is the time shown in the preview; `kept` are
/// the parts that stay in the result (the area between the handles is the selection, drawn on top of them). `joins` are
/// the places where combined videos meet: drawn as orange markers, and a dragged handle snaps to them (hold Alt not to).
pub fn range_slider(
    ui: &mut egui::Ui,
    duration: f64,
    start: &mut f64,
    end: &mut f64,
    playhead: f64,
    kept: &[(f64, f64)],
    joins: &[Join],
) -> Option<Handle> {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click_and_drag());
    let track = Rect::from_min_max(pos2(rect.left() + 8.0, rect.center().y - 5.0), pos2(rect.right() - 8.0, rect.center().y + 5.0));
    let x_of = |t: f64| track.left() + (t / duration) as f32 * track.width();
    let t_of = |x: f32| ((x - track.left()) / track.width()).clamp(0.0, 1.0) as f64 * duration;

    let id = Id::new("timeline-handle");
    let mut moved = None;
    if resp.drag_started() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            // Grab whichever handle is closer.
            let handle = if (pos.x - x_of(*start)).abs() <= (pos.x - x_of(*end)).abs() { Handle::Start } else { Handle::End };
            ui.data_mut(|d| d.insert_temp(id, handle));
        }
    }
    if resp.drag_started() || resp.dragged() || resp.clicked() {
        if let (Some(pos), Some(handle)) = (resp.interact_pointer_pos(), ui.data(|d| d.get_temp::<Handle>(id))) {
            let mut t = t_of(pos.x);
            if !ui.input(|i| i.modifiers.alt) {
                let reach = SNAP_PX as f64 / track.width() as f64 * duration;
                if let Some(j) = joins.iter().find(|j| (j.at - t).abs() <= reach) {
                    t = j.at;
                }
            }
            let gap = min_length(duration);
            match handle {
                Handle::Start => *start = t.min(*end - gap).max(0.0),
                Handle::End => *end = t.max(*start + gap).min(duration),
            }
            moved = Some(handle);
        }
    }

    let painter = ui.painter_at(rect);
    let accent = Color32::from_rgb(70, 140, 230);
    let round = CornerRadius::same(3);
    painter.rect_filled(track, round, ui.visuals().extreme_bg_color);
    painter.rect_stroke(track, round, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), StrokeKind::Inside);
    for &(a, b) in kept {
        painter.rect_filled(Rect::from_min_max(pos2(x_of(a), track.top()), pos2(x_of(b), track.bottom())), 0, accent.gamma_multiply(0.55));
    }
    // The selection: a brighter strip under the track, so it shows whether or not it is part of what is kept.
    painter.rect_filled(
        Rect::from_min_max(pos2(x_of(*start), track.bottom() + 2.0), pos2(x_of(*end), track.bottom() + 5.0)),
        1,
        Color32::from_white_alpha(170),
    );

    for j in joins {
        let x = x_of(j.at.clamp(0.0, duration));
        painter.line_segment([pos2(x, rect.top() + 3.0), pos2(x, rect.bottom() - 1.0)], Stroke::new(2.0, JOIN_COLOR));
        painter.add(egui::Shape::convex_polygon(
            vec![pos2(x - 4.0, rect.top()), pos2(x + 4.0, rect.top()), pos2(x, rect.top() + 6.0)],
            JOIN_COLOR,
            Stroke::NONE,
        ));
        painter.text(pos2(x + 4.0, rect.top() - 1.0), egui::Align2::LEFT_TOP, j.number.to_string(), egui::FontId::monospace(10.0), JOIN_COLOR);
    }

    let playhead_x = x_of(playhead.clamp(0.0, duration));
    painter.line_segment([pos2(playhead_x, rect.top() + 2.0), pos2(playhead_x, rect.bottom() - 2.0)], Stroke::new(1.5, Color32::from_gray(230)));

    for t in [*start, *end] {
        let handle = Rect::from_center_size(pos2(x_of(t), rect.center().y), vec2(10.0, rect.height() - 8.0));
        painter.rect_filled(handle, CornerRadius::same(3), accent);
        painter.rect_stroke(handle, CornerRadius::same(3), Stroke::new(1.5, Color32::WHITE), StrokeKind::Inside);
    }
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if let (Some(p), false) = (resp.hover_pos(), resp.dragged()) {
        if let Some(j) = joins.iter().find(|j| (x_of(j.at) - p.x).abs() <= SNAP_PX) {
            let _ = resp.clone().on_hover_text_at_pointer(format!("Join at {}\n{}  →  {}", format_time(j.at), j.before, j.after));
        }
    }
    moved
}

/// A text box showing a time; typing a new one (and leaving the box) updates `value`.
/// Returns true when `value` was changed by the user.
pub fn time_field(ui: &mut egui::Ui, value: &mut f64, text: &mut String, range: (f64, f64)) -> bool {
    let resp = ui.add(egui::TextEdit::singleline(text).desired_width(78.0));
    let mut changed = false;
    if resp.lost_focus() {
        if let Some(t) = parse_time(text) {
            let t = t.clamp(range.0, range.1);
            changed = (t - *value).abs() > 1e-6;
            *value = t;
        }
    }
    if !resp.has_focus() {
        *text = format_time(*value);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times() {
        assert_eq!(format_time(0.0), "0:00.00");
        assert_eq!(format_time(65.25), "1:05.25");
        assert_eq!(format_time(3725.5), "1:02:05.50");
    }

    #[test]
    fn finds_the_join_before_and_after() {
        let join = |at, number| Join { at, number, before: String::new(), after: String::new() };
        let joins = [join(10.0, 2), join(25.0, 3)];
        assert_eq!(neighbour_join(&joins, 0.0, 1), Some(10.0));
        // Standing on a join finds the next one, in both directions.
        assert_eq!(neighbour_join(&joins, 10.0, 1), Some(25.0));
        assert_eq!(neighbour_join(&joins, 25.0, -1), Some(10.0));
        assert_eq!(neighbour_join(&joins, 12.0, -1), Some(10.0));
        assert_eq!(neighbour_join(&joins, 5.0, -1), None);
        assert_eq!(neighbour_join(&joins, 30.0, 1), None);
        assert_eq!(neighbour_join(&[], 1.0, 1), None);
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse_time("90"), Some(90.0));
        assert_eq!(parse_time("1:30"), Some(90.0));
        assert_eq!(parse_time(" 1:02:05.5 "), Some(3725.5));
        assert_eq!(parse_time("1:xx"), None);
        assert_eq!(parse_time("-3"), None);
        assert_eq!(parse_time("1:2:3:4"), None);
        assert_eq!(parse_time(""), None);
    }

    #[test]
    fn format_and_parse_round_trip() {
        for t in [0.0, 1.5, 59.99, 61.0, 3600.0, 7325.25] {
            assert!((parse_time(&format_time(t)).unwrap() - t).abs() < 0.006, "{t}");
        }
    }
}
