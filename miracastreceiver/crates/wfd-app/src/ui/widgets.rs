//! The app's own controls, so every screen is built from the same pieces.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Color32, CornerRadius, CursorIcon, FontId, Frame, Galley, Key, Margin, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui,
    Vec2, WidgetInfo, pos2, vec2,
};

use super::icons::{self, Icon};
use super::theme::*;

/// One line of text, cut off with an ellipsis at `max_width`.
pub fn line(ui: &Ui, text: &str, font: FontId, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat { font_id: font, color, ..Default::default() });
    job.wrap.max_width = max_width.max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.painter().layout_job(job)
}

/// A title with a quieter line under it, taking only the width it needs.
pub fn title_block(ui: &mut Ui, title: &str, sub: &str, max_width: f32) -> Response {
    let a = line(ui, title, semibold(13.5), TEXT, max_width);
    let b = line(ui, sub, regular(12.0), TEXT_FAINT, max_width);
    let gap = 1.0;
    let size = vec2(a.size().x.max(b.size().x), a.size().y + gap + b.size().y);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let second = rect.min + vec2(0.0, a.size().y + gap);
    ui.painter().galley(rect.min, a, TEXT);
    ui.painter().galley(second, b, TEXT_FAINT);
    resp
}

/// Fill used by a clickable surface in its current state.
fn state_fill(ui: &Ui, resp: &Response, rest: Color32, hover: Color32) -> Color32 {
    let t = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered() || resp.has_focus());
    rest.lerp_to_gamma(hover, t)
}

fn focus_ring(ui: &Ui, resp: &Response, rect: Rect, radius: u8) {
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(radius + 2), Stroke::new(1.5, ACCENT), StrokeKind::Outside);
    }
}

/// A square button showing only an icon; `label` is its tooltip.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, label));
    let rest = if selected { tint(ACCENT, 0.18) } else { Color32::TRANSPARENT };
    let hover = if selected { tint(ACCENT, 0.28) } else { RAISED };
    let fill = state_fill(ui, &resp, rest, hover);
    ui.painter().rect_filled(rect, 9.0, fill);
    let color = if selected {
        ACCENT_HOVER
    } else if resp.hovered() {
        TEXT
    } else {
        TEXT_DIM
    };
    icons::paint(ui.painter(), rect.shrink(8.0), icon, color);
    focus_ring(ui, &resp, rect, 9);
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(label)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

pub struct Button<'a> {
    kind: Kind,
    text: &'a str,
    icon: Option<Icon>,
    width: f32,
    height: f32,
    enabled: bool,
    icon_only: bool,
}

pub fn button(kind: Kind, text: &str) -> Button<'_> {
    Button { kind, text, icon: None, width: 0.0, height: 34.0, enabled: true, icon_only: false }
}

impl Button<'_> {
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Draws just the icon; the text becomes the tooltip.
    pub fn icon_only(mut self) -> Self {
        self.icon_only = true;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let shown = if self.icon_only { "" } else { self.text };
        let galley = ui.painter().layout_no_wrap(shown.to_owned(), medium(13.0), TEXT);
        let icon_w = match (self.icon, self.icon_only) {
            (None, _) => 0.0,
            (Some(_), false) => 16.0 + 7.0,
            (Some(_), true) => 16.0,
        };
        let content = icon_w + galley.size().x;
        let padding = if self.icon_only { 18.0 } else { 28.0 };
        let size = vec2((content + padding).max(self.width), self.height);
        let sense = if self.enabled { Sense::click() } else { Sense::hover() };
        let (rect, resp) = ui.allocate_exact_size(size, sense);
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, self.enabled, self.text));
        let (rest, hover, border, text) = match self.kind {
            Kind::Primary => (ACCENT_FILL, ACCENT, Color32::TRANSPARENT, Color32::WHITE),
            Kind::Secondary => (RAISED, HOVER, BORDER_STRONG, TEXT),
            Kind::Ghost => (Color32::TRANSPARENT, RAISED, Color32::TRANSPARENT, TEXT_DIM),
            Kind::Danger => (tint(DANGER, 0.13), tint(DANGER, 0.26), tint(DANGER, 0.35), DANGER),
        };
        let fade = |c: Color32| if self.enabled { c } else { c.gamma_multiply(0.45) };
        let fill = if self.enabled { state_fill(ui, &resp, rest, hover) } else { fade(rest) };
        ui.painter().rect(rect, 9.0, fill, Stroke::new(1.0, fade(border)), StrokeKind::Inside);
        let mut x = rect.center().x - content * 0.5;
        if let Some(icon) = self.icon {
            let r = Rect::from_min_size(pos2(x, rect.center().y - 8.0), vec2(16.0, 16.0));
            icons::paint(ui.painter(), r, icon, fade(text));
            x += icon_w;
        }
        let at = pos2(x, rect.center().y - galley.size().y * 0.5);
        ui.painter().galley_with_override_text_color(at, galley, fade(text));
        focus_ring(ui, &resp, rect, 9);
        let resp = if self.enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp };
        if self.icon_only { resp.on_hover_text(self.text) } else { resp }
    }
}

fn paint_switch(ui: &Ui, rect: Rect, on: bool, resp: &Response) {
    let t = ui.ctx().animate_bool_responsive(resp.id.with("switch"), on);
    let off = if resp.hovered() { BORDER_STRONG } else { HOVER };
    ui.painter().rect_filled(rect, rect.height() * 0.5, off.lerp_to_gamma(ACCENT_FILL, t));
    let r = rect.height() * 0.5 - 3.0;
    let x = egui::lerp((rect.left() + r + 3.0)..=(rect.right() - r - 3.0), t);
    ui.painter().circle_filled(pos2(x, rect.center().y), r, Color32::WHITE);
}

/// A full-width setting with a switch; clicking anywhere on it flips it.
pub fn switch_row(ui: &mut Ui, title: &str, detail: &str, on: &mut bool) -> Response {
    let width = ui.available_width();
    let switch = vec2(36.0, 20.0);
    let text_w = width - switch.x - 14.0;
    let title_g = ui.painter().layout(title.to_owned(), medium(13.0), TEXT, text_w);
    let detail_g = (!detail.is_empty()).then(|| ui.painter().layout(detail.to_owned(), regular(12.0), TEXT_FAINT, text_w));
    let text_h = title_g.size().y + detail_g.as_ref().map_or(0.0, |g| g.size().y + 2.0);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, text_h.max(switch.y) + 6.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, *on, title));
    let top = rect.top() + (rect.height() - text_h) * 0.5;
    let second = pos2(rect.left(), top + title_g.size().y + 2.0);
    ui.painter().galley(pos2(rect.left(), top), title_g, TEXT);
    if let Some(g) = detail_g {
        ui.painter().galley(second, g, TEXT_FAINT);
    }
    let sw = Rect::from_min_size(pos2(rect.right() - switch.x, rect.center().y - switch.y * 0.5), switch);
    paint_switch(ui, sw, *on, &resp);
    focus_ring(ui, &resp, sw, 10);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// A row of mutually exclusive choices filling the available width.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    ui.painter().rect(rect, 9.0, FIELD, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    let inner = rect.shrink(3.0);
    let w = inner.width() / options.len() as f32;
    let group = ui.next_auto_id();
    for (i, (v, label)) in options.iter().enumerate() {
        let r = Rect::from_min_size(inner.min + vec2(w * i as f32, 0.0), vec2(w, inner.height()));
        let resp = ui.interact(r, group.with(i), Sense::click());
        if resp.clicked() {
            *value = *v;
        }
        let selected = *value == *v;
        resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, selected, *label));
        if selected {
            ui.painter().rect(r, 7.0, HOVER, Stroke::new(1.0, BORDER_STRONG), StrokeKind::Inside);
        }
        let color = if selected || resp.hovered() { TEXT } else { TEXT_DIM };
        let g = line(ui, label, medium(12.5), color, w - 8.0);
        ui.painter().galley(r.center() - g.size() * 0.5, g, color);
        focus_ring(ui, &resp, r, 7);
        resp.on_hover_cursor(CursorIcon::PointingHand);
    }
    ui.skip_ahead_auto_ids(1);
}

/// A thin slider. `value` is clamped to the range.
pub fn slider(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32, label: &str) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click_and_drag());
    let (min, max) = (*range.start(), *range.end());
    let rail = Rect::from_center_size(rect.center(), vec2(rect.width() - 12.0, 4.0));
    let before = *value;
    if let Some(p) = resp.interact_pointer_pos() {
        let t = ((p.x - rail.left()) / rail.width()).clamp(0.0, 1.0);
        *value = min + t * (max - min);
    }
    if resp.has_focus() {
        let step = (max - min) / 20.0;
        ui.input(|i| {
            if i.key_pressed(Key::ArrowLeft) {
                *value -= step;
            }
            if i.key_pressed(Key::ArrowRight) {
                *value += step;
            }
        });
    }
    *value = value.clamp(min, max);
    if *value != before {
        resp.mark_changed();
    }
    resp.widget_info(|| WidgetInfo::slider(true, f64::from(*value), label));
    let t = (*value - min) / (max - min);
    let knob = pos2(rail.left() + t * rail.width(), rail.center().y);
    ui.painter().rect_filled(rail, 2.0, HOVER);
    ui.painter().rect_filled(Rect::from_min_max(rail.min, pos2(knob.x, rail.max.y)), 2.0, ACCENT);
    let grow = ui.ctx().animate_bool_responsive(resp.id, resp.hovered() || resp.dragged() || resp.has_focus());
    ui.painter().circle_filled(knob, 5.5 + grow * 1.5, Color32::WHITE);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// A small rounded label with a coloured status dot.
pub fn status_pill(ui: &mut Ui, color: Color32, text: &str) -> Response {
    let g = line(ui, text, medium(12.0), TEXT_DIM, 180.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 32.0, 28.0), Sense::hover());
    ui.painter().rect(rect, 14.0, CARD, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    dot(ui, pos2(rect.left() + 14.0, rect.center().y), color);
    ui.painter().galley(pos2(rect.left() + 23.0, rect.center().y - g.size().y * 0.5), g, TEXT_DIM);
    resp
}

/// A status dot with a soft halo.
pub fn dot(ui: &Ui, center: egui::Pos2, color: Color32) {
    ui.painter().circle_filled(center, 6.5, tint(color, 0.18));
    ui.painter().circle_filled(center, 3.5, color);
}

pub fn card() -> Frame {
    Frame::new().fill(CARD).stroke(Stroke::new(1.0, BORDER)).corner_radius(14.0).inner_margin(Margin::same(18))
}

/// A tinted box for warnings, errors and notes.
pub fn banner(ui: &mut Ui, color: Color32, icon: Icon, text: &str) {
    Frame::new()
        .fill(tint(color, 0.10))
        .stroke(Stroke::new(1.0, tint(color, 0.32)))
        .corner_radius(11.0)
        .inner_margin(Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::left_to_right(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                icons::show(ui, icon, 17.0, color);
                ui.add(egui::Label::new(egui::RichText::new(text).color(TEXT)).wrap());
            });
        });
}

/// A small capitalised heading above a group of controls.
pub fn section(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text.to_uppercase()).font(semibold(11.0)).color(TEXT_FAINT).extra_letter_spacing(0.8)));
}

/// The caption above a control.
pub fn caption(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(medium(13.0)).color(TEXT)));
}

/// Quiet explanatory text.
pub fn hint(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(regular(12.0)).color(TEXT_FAINT)).wrap());
}

pub fn separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, BORDER);
}
