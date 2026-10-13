//! Transient notifications in the bottom-left corner ("Pulled Successfully").
//!
//! They stack above the status bar, fade out on their own and can be closed
//! with the ×. Hovering one keeps it open.

use eframe::egui::{self, Align2, Color32, FontId, Margin, Rect, Sense, Stroke, Vec2};

/// Seconds a toast stays fully visible (restarted while the pointer is on it).
const LIFETIME: f64 = 6.0;
/// Seconds it takes to fade out at the end.
const FADE: f64 = 0.5;
const WIDTH: f32 = 280.0;
/// Oldest toasts are dropped beyond this many.
const MAX: usize = 5;
const INFO: Color32 = Color32::from_rgb(0x3a, 0xa8, 0xcc);
const GAP: f32 = 6.0;
const EDGE: f32 = 12.0;

struct Toast {
    title: String,
    detail: Option<String>,
    expires: f64,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    pub fn push(&mut self, ctx: &egui::Context, title: impl Into<String>, detail: Option<String>) {
        let now = ctx.input(|i| i.time);
        self.items.push(Toast { title: title.into(), detail, expires: now + LIFETIME });
        if self.items.len() > MAX {
            self.items.remove(0);
        }
        ctx.request_repaint();
    }

    /// Draw the stack with its bottom edge at `bottom` (the top of the status bar).
    pub fn show(&mut self, ctx: &egui::Context, bottom: f32) {
        if self.items.is_empty() {
            return;
        }
        let now = ctx.input(|i| i.time);
        self.items.retain(|t| t.expires > now);
        if self.items.is_empty() {
            return;
        }
        let from_bottom = ctx.screen_rect().bottom() - bottom + EDGE;
        let mut closed = None;
        let mut hovered = false;
        egui::Area::new(egui::Id::new("toasts"))
            .order(egui::Order::Foreground)
            .anchor(Align2::LEFT_BOTTOM, Vec2::new(EDGE, -from_bottom))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = GAP;
                for (i, t) in self.items.iter().enumerate() {
                    let alpha = (((t.expires - now) / FADE) as f32).clamp(0.0, 1.0);
                    let r = card(ui, t, i, alpha);
                    hovered |= r.hovered;
                    if r.closed {
                        closed = Some(i);
                    }
                }
            });
        if let Some(i) = closed {
            self.items.remove(i);
        }
        if hovered {
            for t in &mut self.items {
                t.expires = t.expires.max(now + LIFETIME);
            }
        }
        // Keep animating the fade, and make sure we wake up to expire them.
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

struct CardOut {
    closed: bool,
    hovered: bool,
}

fn card(ui: &mut egui::Ui, t: &Toast, index: usize, alpha: f32) -> CardOut {
    let id = ui.id().with(("toast", index));
    let rect = ui
        .scope(|ui| {
            ui.set_opacity(alpha);
            let frame = egui::Frame::new()
                .fill(INFO)
                .corner_radius(4.0)
                .inner_margin(Margin { left: 14, right: 30, top: 12, bottom: 12 })
                .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(90) });
            frame
                .show(ui, |ui| {
                    ui.set_width(WIDTH - 44.0);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        info_icon(ui);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(egui::RichText::new(&t.title).strong().size(15.0).color(Color32::WHITE));
                            if let Some(d) = &t.detail {
                                ui.label(egui::RichText::new(d).color(Color32::WHITE));
                            }
                        });
                    });
                })
                .response
                .rect
        })
        .inner;

    // The ×, at the top right.
    let x_rect = Rect::from_center_size(rect.right_top() + Vec2::new(-15.0, 15.0), Vec2::splat(18.0));
    let x = ui.interact(x_rect, id.with("close"), Sense::click());
    let color = if x.hovered() { Color32::WHITE } else { Color32::from_white_alpha(200) };
    let stroke = Stroke::new(1.6_f32, color.gamma_multiply(alpha));
    let c = x_rect.center();
    let d = 4.0;
    let p = ui.painter();
    p.line_segment([c + Vec2::new(-d, -d), c + Vec2::new(d, d)], stroke);
    p.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
    CardOut { closed: x.clicked(), hovered: ui.rect_contains_pointer(rect) }
}

/// A white disc with an "i" cut out of it.
fn info_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
    let p = ui.painter();
    p.circle_filled(rect.center(), 11.0, Color32::WHITE);
    p.text(
        rect.center() + Vec2::new(0.0, 0.5),
        Align2::CENTER_CENTER,
        "i",
        FontId::proportional(15.0),
        INFO,
    );
}
