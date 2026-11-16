//! Colours, spacing and small shared widgets.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, Rect, RichText, Sense, Stroke, StrokeKind, TextStyle, Theme, Ui, Vec2};

pub const BG: Color32 = Color32::from_rgb(15, 17, 21);
pub const PANEL: Color32 = Color32::from_rgb(22, 25, 31);
pub const CARD: Color32 = Color32::from_rgb(29, 33, 41);
pub const RAISED: Color32 = Color32::from_rgb(38, 43, 53);
pub const LINE: Color32 = Color32::from_rgb(46, 52, 64);
pub const TEXT: Color32 = Color32::from_rgb(233, 236, 241);
pub const MUTED: Color32 = Color32::from_rgb(142, 150, 165);
pub const FAINT: Color32 = Color32::from_rgb(92, 100, 115);
pub const ACCENT: Color32 = Color32::from_rgb(110, 123, 255);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(132, 144, 255);
pub const GOOD: Color32 = Color32::from_rgb(62, 214, 176);
pub const WARN: Color32 = Color32::from_rgb(245, 184, 75);
pub const BAD: Color32 = Color32::from_rgb(240, 96, 112);

const PART_COLORS: [Color32; 6] = [
    Color32::from_rgb(110, 123, 255),
    Color32::from_rgb(62, 214, 176),
    Color32::from_rgb(245, 184, 75),
    Color32::from_rgb(240, 111, 168),
    Color32::from_rgb(79, 184, 245),
    Color32::from_rgb(165, 214, 78),
];

const CLIP_COLORS: [Color32; 6] = [
    Color32::from_rgb(52, 62, 110),
    Color32::from_rgb(40, 88, 78),
    Color32::from_rgb(100, 78, 44),
    Color32::from_rgb(98, 54, 80),
    Color32::from_rgb(42, 84, 108),
    Color32::from_rgb(72, 90, 50),
];

pub fn part_color(index: usize) -> Color32 {
    PART_COLORS[index % PART_COLORS.len()]
}

pub fn clip_color(id: u64) -> Color32 {
    CLIP_COLORS[id as usize % CLIP_COLORS.len()]
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();
        let spacing = &mut style.spacing;
        spacing.item_spacing = Vec2::new(8.0, 8.0);
        spacing.button_padding = Vec2::new(10.0, 6.0);
        spacing.interact_size = Vec2::new(36.0, 28.0);
        spacing.combo_height = 320.0;

        let visuals = &mut style.visuals;
        visuals.dark_mode = true;
        visuals.panel_fill = PANEL;
        visuals.window_fill = CARD;
        visuals.extreme_bg_color = BG;
        visuals.faint_bg_color = CARD;
        visuals.override_text_color = Some(TEXT);
        visuals.selection.bg_fill = ACCENT.linear_multiply(0.45);
        visuals.selection.stroke = Stroke::new(1.0, ACCENT_HOVER);
        visuals.hyperlink_color = ACCENT_HOVER;
        visuals.window_stroke = Stroke::new(1.0, LINE);
        let radius = CornerRadius::same(7);
        for (widget, fill, border) in [
            (&mut visuals.widgets.noninteractive, CARD, LINE),
            (&mut visuals.widgets.inactive, RAISED, LINE),
            (&mut visuals.widgets.hovered, Color32::from_rgb(50, 56, 69), Color32::from_rgb(70, 78, 94)),
            (&mut visuals.widgets.active, Color32::from_rgb(58, 65, 80), ACCENT),
            (&mut visuals.widgets.open, Color32::from_rgb(50, 56, 69), ACCENT),
        ] {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.bg_stroke = Stroke::new(1.0, border);
            widget.corner_radius = radius;
            widget.fg_stroke = Stroke::new(1.2, TEXT);
            widget.expansion = 0.0;
        }
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    });
}

/// A small heading over a group of controls.
pub fn section_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.5).strong().color(MUTED));
}

/// A rounded group of controls.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// An on/off switch. Returns true when flipped.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> bool {
    let size = Vec2::new(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let fill = Color32::from_rgb(
        egui::lerp(60.0..=110.0, t) as u8,
        egui::lerp(66.0..=123.0, t) as u8,
        egui::lerp(80.0..=255.0, t) as u8,
    );
    let radius = rect.height() / 2.0;
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    painter.circle_filled(egui::pos2(x, rect.center().y), radius - 3.0, Color32::WHITE);
    response.changed()
}

/// A big filled button.
pub fn primary_button(ui: &mut Ui, text: &str, size: Vec2, enabled: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let fill = match (enabled, response.hovered()) {
        (false, _) => RAISED,
        (true, true) => ACCENT_HOVER,
        (true, false) => ACCENT,
    };
    let color = if enabled { Color32::WHITE } else { FAINT };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(9), fill);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, FontId::new(15.0, FontFamily::Proportional), color);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A flat text button that lights up when hovered.
pub fn soft_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).size(12.5)).fill(RAISED).stroke(Stroke::new(1.0, LINE)))
}

/// A square button with a drawn icon.
pub fn icon_button(ui: &mut Ui, size: f32, enabled: bool, draw: impl FnOnce(&egui::Painter, Rect, Color32)) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), if enabled { Sense::click() } else { Sense::hover() });
    let color = if !enabled {
        FAINT
    } else if response.hovered() {
        TEXT
    } else {
        Color32::from_rgb(188, 194, 206)
    };
    let painter = ui.painter();
    if enabled && response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), RAISED);
    }
    draw(painter, rect, color);
    response
}

pub fn triangle(painter: &egui::Painter, center: egui::Pos2, half: f32, direction: f32, color: Color32) {
    let points = vec![
        center + Vec2::new(-half * direction, -half),
        center + Vec2::new(half * direction, 0.0),
        center + Vec2::new(-half * direction, half),
    ];
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

pub fn draw_play(painter: &egui::Painter, rect: Rect, color: Color32) {
    triangle(painter, rect.center() + Vec2::new(1.5, 0.0), rect.width() * 0.24, 1.0, color);
}

pub fn draw_pause(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    let (half_width, half_height) = (rect.width() * 0.07, rect.height() * 0.22);
    for side in [-1.0, 1.0] {
        let bar = Rect::from_center_size(center + Vec2::new(side * rect.width() * 0.13, 0.0), Vec2::new(half_width * 2.0, half_height * 2.0));
        painter.rect_filled(bar, CornerRadius::same(1), color);
    }
}

pub fn draw_skip(painter: &egui::Painter, rect: Rect, color: Color32, direction: f32) {
    let center = rect.center();
    let half = rect.width() * 0.19;
    triangle(painter, center + Vec2::new(-1.5 * direction, 0.0), half, direction, color);
    let x = center.x + (half + 0.5) * direction;
    painter.line_segment(
        [egui::pos2(x, center.y - half), egui::pos2(x, center.y + half)],
        Stroke::new(2.0, color),
    );
}

pub fn draw_close(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    let half = rect.width() * 0.2;
    let stroke = Stroke::new(1.6, color);
    painter.line_segment([center + Vec2::splat(-half), center + Vec2::splat(half)], stroke);
    painter.line_segment([center + Vec2::new(-half, half), center + Vec2::new(half, -half)], stroke);
}

pub fn draw_chevron(painter: &egui::Painter, rect: Rect, color: Color32, up: bool) {
    let center = rect.center();
    let half = rect.width() * 0.2;
    let lift = if up { -half * 0.5 } else { half * 0.5 };
    let tip = center + Vec2::new(0.0, -lift * 2.0);
    let stroke = Stroke::new(1.8, color);
    painter.line_segment([center + Vec2::new(-half, lift), tip], stroke);
    painter.line_segment([tip, center + Vec2::new(half, lift)], stroke);
}

/// A dashed outline.
pub fn dashed_rect(painter: &egui::Painter, rect: Rect, stroke: Stroke) {
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    for pair in corners.windows(2) {
        painter.add(egui::Shape::dashed_line(&[pair[0], pair[1]], stroke, 10.0, 7.0));
    }
}

/// The outline around a selected thing.
pub fn outline(painter: &egui::Painter, rect: Rect, radius: u8, stroke: Stroke) {
    painter.rect_stroke(rect, CornerRadius::same(radius), stroke, StrokeKind::Inside);
}
