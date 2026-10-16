//! Rows and headers for the lists of changed files in the right-hand panel.
//! They are painted by hand so a row stays quiet until you point at it: its
//! actions (stage, unstage, discard) only appear on hover.

use crate::icons::{self, Icon};
use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Vec2};
use gitgoing_core::diff::ChangeKind;

const ROW_H: f32 = 26.0;

pub struct FileRow {
    pub rect: Rect,
    pub resp: Response,
    /// The pointer is over the row, even if it is over one of its buttons.
    pub hot: bool,
}

/// Draw one file row: change icon and name. Names are cut off before `idle_reserve`
/// pixels at the right edge, or `active_reserve` while the row is hovered or selected
/// (when the caller shows buttons there).
pub fn file_row(
    ui: &mut egui::Ui,
    kind: ChangeKind,
    name: &str,
    tooltip: &str,
    selected: bool,
    idle_reserve: f32,
    active_reserve: f32,
) -> FileRow {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    let hot = ui.rect_contains_pointer(rect);
    let v = ui.visuals();
    let p = ui.painter_at(rect);
    if selected {
        p.rect_filled(rect, 4.0, v.selection.bg_fill.gamma_multiply(0.65));
    } else if hot {
        p.rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
    }
    icons::paint_change(&p, Pos2::new(rect.left() + 15.0, rect.center().y), kind);
    let color = if selected { v.strong_text_color() } else { v.text_color() };
    let reserve = if hot || selected { active_reserve } else { idle_reserve };
    let clip = Rect::from_min_max(Pos2::new(rect.left() + 32.0, rect.top()), Pos2::new(rect.right() - reserve, rect.bottom()));
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter_at(clip).text(Pos2::new(clip.left(), rect.center().y), egui::Align2::LEFT_CENTER, name, font, color);
    FileRow { rect, resp: resp.on_hover_text(tooltip), hot }
}

/// Icon buttons at the right edge of a hovered or selected row, rightmost first.
/// Returns the index of the one clicked.
pub fn hover_buttons(
    ui: &mut egui::Ui,
    row: &FileRow,
    selected: bool,
    salt: &str,
    buttons: &[(Icon, &str, bool)],
) -> Option<usize> {
    if !(row.hot || selected) {
        return None;
    }
    let mut clicked = None;
    for (n, (icon, tip, enabled)) in buttons.iter().enumerate() {
        let c = Pos2::new(row.rect.right() - 16.0 - n as f32 * 26.0, row.rect.center().y);
        let rect = Rect::from_center_size(c, Vec2::splat(22.0));
        let sense = if *enabled { Sense::click() } else { Sense::hover() };
        let r = ui.interact(rect, ui.id().with((salt, n)), sense);
        let v = ui.visuals();
        let p = ui.painter_at(rect.expand(1.0));
        if *enabled && r.hovered() {
            p.rect_filled(rect, 4.0, v.widgets.hovered.bg_fill);
        }
        let col = if !enabled {
            v.weak_text_color().gamma_multiply(0.5)
        } else if r.hovered() {
            v.strong_text_color()
        } else {
            v.weak_text_color()
        };
        icons::draw(&p, c, *icon, 0.95, col);
        r.clone().on_hover_text(*tip);
        if r.clicked() {
            clicked = Some(n);
        }
    }
    clicked
}

/// A small rounded text button at the right edge of a row (always visible).
pub fn pill(ui: &mut egui::Ui, row: &FileRow, salt: &str, text: &str, enabled: bool, color: Color32) -> bool {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let size = Vec2::new(galley.size().x + 14.0, 20.0);
    let rect = Rect::from_center_size(Pos2::new(row.rect.right() - 8.0 - size.x / 2.0, row.rect.center().y), size);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let r = ui.interact(rect, ui.id().with((salt, "pill")), sense);
    let p = ui.painter_at(rect.expand(1.0));
    let fill = if r.hovered() && enabled { color.gamma_multiply(0.35) } else { color.gamma_multiply(0.18) };
    p.rect_filled(rect, 10.0, fill);
    p.rect_stroke(rect, 10.0, Stroke::new(1.0_f32, color.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    p.galley(Pos2::new(rect.left() + 7.0, rect.center().y - galley.size().y / 2.0), galley, color);
    r.clicked() && enabled
}

/// What was clicked in a [`section_header`].
#[derive(Default)]
pub struct HeaderClicks {
    /// The header itself (to expand or collapse the section).
    pub toggle: bool,
    pub text: bool,
    /// The extra action left of the text action.
    pub extra: bool,
    pub icon: bool,
}

/// A section title with a count badge, and optional actions at the right:
/// a text action (e.g. "Stage all") and an icon action (e.g. discard). With
/// `open` set, the header has an arrow and clicking it expands or collapses the section.
pub fn section_header(
    ui: &mut egui::Ui,
    title: &str,
    count: usize,
    open: Option<bool>,
    text_action: Option<(&str, bool)>,
    extra_action: Option<(&str, bool)>,
    icon_action: Option<(Icon, &str, bool)>,
) -> HeaderClicks {
    let sense = if open.is_some() { Sense::click() } else { Sense::hover() };
    let (rect, base) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), sense);
    let v = ui.visuals();
    let (strong, weak) = (v.strong_text_color(), v.weak_text_color());
    let p = ui.painter_at(rect);
    let cy = rect.center().y;
    if open.is_some() && base.hovered() {
        p.rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
    }
    let mut x = rect.left() + 6.0;
    if let Some(is_open) = open {
        let c = Pos2::new(x + 5.0, cy);
        let tri = if is_open {
            vec![c + Vec2::new(-4.0, -2.0), c + Vec2::new(4.0, -2.0), c + Vec2::new(0.0, 3.0)]
        } else {
            vec![c + Vec2::new(-2.0, -4.0), c + Vec2::new(-2.0, 4.0), c + Vec2::new(3.0, 0.0)]
        };
        p.add(egui::Shape::convex_polygon(tri, weak, Stroke::NONE));
        x += 16.0;
    }

    let title_font = egui::FontId::proportional(13.0);
    let g = p.layout_no_wrap(title.to_uppercase(), title_font, strong);
    let w = g.size().x;
    p.galley(Pos2::new(x, cy - g.size().y / 2.0), g, strong);

    // Count badge: high contrast so the number reads at a glance.
    let (badge_fill, badge_text) = if v.dark_mode {
        (Color32::from_rgb(0x3d, 0x4a, 0x63), Color32::WHITE)
    } else {
        (Color32::from_rgb(0xd3, 0xdb, 0xea), Color32::from_rgb(0x1a, 0x22, 0x33))
    };
    let badge_font = egui::FontId::proportional(12.5);
    let bg = p.layout_no_wrap(count.to_string(), badge_font, badge_text);
    let badge = Rect::from_center_size(
        Pos2::new(x + w + 9.0 + (bg.size().x + 12.0) / 2.0, cy),
        Vec2::new((bg.size().x + 12.0).max(22.0), 18.0),
    );
    p.rect_filled(badge, 9.0, badge_fill);
    p.galley(Pos2::new(badge.center().x - bg.size().x / 2.0, cy - bg.size().y / 2.0), bg, badge_text);
    p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, v.widgets.noninteractive.bg_stroke.color));

    let mut text_clicked = false;
    let mut icon_clicked = false;
    let mut right = rect.right() - 4.0;
    if let Some((icon, tip, enabled)) = icon_action {
        let r_rect = Rect::from_center_size(Pos2::new(right - 11.0, cy), Vec2::splat(22.0));
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let r = ui.interact(r_rect, ui.id().with(("section_icon", title)), sense);
        if enabled && r.hovered() {
            p.rect_filled(r_rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
        }
        let col = if !enabled { weak.gamma_multiply(0.5) } else if r.hovered() { strong } else { weak };
        icons::draw(&p, r_rect.center(), icon, 0.95, col);
        r.clone().on_hover_text(tip);
        icon_clicked = r.clicked() && enabled;
        right -= 26.0;
    }
    if let Some((label, enabled)) = text_action {
        let font = egui::TextStyle::Small.resolve(ui.style());
        let accent = Color32::from_rgb(0x6c, 0x9d, 0xf2);
        let col = if enabled { accent } else { weak.gamma_multiply(0.5) };
        let g = p.layout_no_wrap(label.to_owned(), font, col);
        let size = Vec2::new(g.size().x + 14.0, 22.0);
        let a_rect = Rect::from_center_size(Pos2::new(right - size.x / 2.0, cy), size);
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let r = ui.interact(a_rect, ui.id().with(("section_text", title)), sense);
        if enabled && r.hovered() {
            p.rect_filled(a_rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
        }
        p.galley(Pos2::new(a_rect.left() + 7.0, cy - g.size().y / 2.0), g, col);
        text_clicked = r.clicked() && enabled;
        right = a_rect.left() - 2.0;
    }
    let mut extra_clicked = false;
    if let Some((label, enabled)) = extra_action {
        let font = egui::TextStyle::Small.resolve(ui.style());
        let accent = Color32::from_rgb(0x6c, 0x9d, 0xf2);
        let col = if enabled { accent } else { weak.gamma_multiply(0.5) };
        let g = p.layout_no_wrap(label.to_owned(), font, col);
        let size = Vec2::new(g.size().x + 14.0, 22.0);
        let a_rect = Rect::from_center_size(Pos2::new(right - size.x / 2.0, cy), size);
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let r = ui.interact(a_rect, ui.id().with(("section_extra", title)), sense);
        if enabled && r.hovered() {
            p.rect_filled(a_rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
        }
        p.galley(Pos2::new(a_rect.left() + 7.0, cy - g.size().y / 2.0), g, col);
        extra_clicked = r.clicked() && enabled;
    }
    HeaderClicks {
        toggle: base.clicked() && !text_clicked && !extra_clicked && !icon_clicked,
        text: text_clicked,
        extra: extra_clicked,
        icon: icon_clicked,
    }
}
