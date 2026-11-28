//! The look of the app: colours, type and egui's widget style.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke, TextStyle, ThemePreference, Vec2, vec2};

pub const BG: Color32 = Color32::from_rgb(13, 15, 19);
pub const SURFACE: Color32 = Color32::from_rgb(19, 22, 28);
pub const CARD: Color32 = Color32::from_rgb(23, 27, 34);
pub const RAISED: Color32 = Color32::from_rgb(32, 37, 47);
pub const HOVER: Color32 = Color32::from_rgb(42, 49, 61);
pub const FIELD: Color32 = Color32::from_rgb(14, 16, 21);
pub const BORDER: Color32 = Color32::from_rgb(37, 43, 54);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(58, 67, 83);

pub const TEXT: Color32 = Color32::from_rgb(233, 236, 242);
pub const TEXT_DIM: Color32 = Color32::from_rgb(160, 169, 184);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(112, 122, 139);

pub const ACCENT: Color32 = Color32::from_rgb(76, 141, 255);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(105, 160, 255);
/// Filled buttons: dark enough for white text.
pub const ACCENT_FILL: Color32 = Color32::from_rgb(47, 111, 230);
pub const OK: Color32 = Color32::from_rgb(63, 207, 142);
pub const WARN: Color32 = Color32::from_rgb(242, 184, 75);
pub const DANGER: Color32 = Color32::from_rgb(242, 109, 109);
pub const IDLE: Color32 = Color32::from_rgb(112, 122, 139);

/// `color` at the given opacity, for tinted backgrounds.
pub fn tint(color: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (alpha * 255.0) as u8)
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(ThemePreference::Dark);
    ctx.all_styles_mut(style);
}

/// Inter for all UI text; egui's own fonts stay as fallbacks for symbols and
/// scripts Inter lacks (device names can be anything).
fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let fallbacks = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let faces: [(&str, FontFamily, &'static [u8]); 3] = [
        ("inter", FontFamily::Proportional, include_bytes!("../../../../assets/fonts/Inter-Regular.otf")),
        ("inter-medium", FontFamily::Name("medium".into()), include_bytes!("../../../../assets/fonts/Inter-Medium.otf")),
        ("inter-semibold", FontFamily::Name("semibold".into()), include_bytes!("../../../../assets/fonts/Inter-SemiBold.otf")),
    ];
    for (name, family, bytes) in faces {
        f.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
        let mut chain = vec![name.to_string()];
        chain.extend(fallbacks.iter().cloned());
        f.families.insert(family, chain);
    }
    f
}

fn style(s: &mut egui::Style) {
    s.text_styles = [
        (TextStyle::Small, regular(12.0)),
        (TextStyle::Body, regular(13.5)),
        (TextStyle::Button, medium(13.0)),
        (TextStyle::Heading, semibold(19.0)),
        (TextStyle::Monospace, mono(12.5)),
    ]
    .into();
    s.animation_time = 0.14;
    s.interaction.selectable_labels = false;

    let sp = &mut s.spacing;
    sp.item_spacing = vec2(8.0, 8.0);
    sp.button_padding = vec2(12.0, 6.0);
    sp.interact_size = vec2(40.0, 30.0);
    sp.window_margin = egui::Margin::same(16);
    sp.menu_margin = egui::Margin::same(6);
    sp.combo_height = 260.0;
    sp.icon_width = 18.0;
    sp.scroll = egui::style::ScrollStyle::thin();

    let v = &mut s.visuals;
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = Shadow { offset: [0, 12], blur: 36, spread: 0, color: Color32::from_black_alpha(140) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(120) };
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = SURFACE;
    v.weak_text_color = Some(TEXT_FAINT);
    v.hyperlink_color = ACCENT_HOVER;
    v.warn_fg_color = WARN;
    v.error_fg_color = DANGER;
    v.selection.bg_fill = tint(ACCENT, 0.32);
    v.selection.stroke = Stroke::new(1.0, TEXT);

    let w = &mut v.widgets;
    let radius = CornerRadius::same(8);
    for (state, fill, border, text) in [
        (&mut w.noninteractive, CARD, BORDER, TEXT_DIM),
        (&mut w.inactive, RAISED, BORDER, TEXT),
        (&mut w.hovered, HOVER, BORDER_STRONG, TEXT),
        (&mut w.active, HOVER, ACCENT, TEXT),
        (&mut w.open, HOVER, BORDER_STRONG, TEXT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, border);
        state.fg_stroke = Stroke::new(1.2, text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    // a visible resting frame for text fields and combo boxes
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    s.spacing.text_edit_width = f32::INFINITY;
    s.spacing.default_area_size = Vec2::new(420.0, 400.0);
}
