//! Things drawn over everything else: the connection prompt, the drop target,
//! the stats card and the full-screen hint.

use egui::{Align2, Color32, Key, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use wfd_transport::{PairingMethod, PairingReply};

use super::ReceiverApp;
use super::icons::{self, Icon};
use super::theme::*;
use super::widgets::{self, Kind};
use crate::engine::EngineCmd;

impl ReceiverApp {
    pub(super) fn pairing_modal(&mut self, ctx: &egui::Context) {
        let Some(p) = self.snapshot.prompts.first().cloned() else { return };
        let mut reply = None;
        let frame = widgets::card().inner_margin(egui::Margin::same(24)).stroke(Stroke::new(1.0, BORDER_STRONG)).shadow(egui::Shadow {
            offset: [0, 16],
            blur: 48,
            spread: 0,
            color: Color32::from_black_alpha(160),
        });
        egui::Modal::new(egui::Id::NULL.with(("pair", p.id))).frame(frame).backdrop_color(Color32::from_black_alpha(170)).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 96.0).clamp(240.0, 360.0));
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.vertical_centered(|ui| {
                if ctx.content_rect().height() >= 420.0 {
                    icons::show_tile(ui, Icon::Phone, 56.0, ACCENT);
                    ui.add_space(4.0);
                }
                ui.add(egui::Label::new(egui::RichText::new(&p.device_name).font(semibold(19.0)).color(TEXT)).wrap());
                ui.label(egui::RichText::new(format!("wants to show its screen  ·  {}", p.kind)).color(TEXT_DIM));
                ui.add_space(4.0);
                match &p.method {
                    PairingMethod::PushButton => {}
                    PairingMethod::DisplayPin(pin) => {
                        widgets::hint(ui, "Enter this PIN on the device");
                        pin_digits(ui, pin);
                    }
                    PairingMethod::EnterPin => {
                        widgets::hint(ui, "Type the PIN shown on the device");
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.pin_entry)
                                .hint_text("PIN")
                                .font(mono(22.0))
                                .align(egui::Align2::CENTER_CENTER)
                                .margin(egui::Margin::symmetric(12, 10))
                                .desired_width(220.0),
                        );
                        r.request_focus();
                    }
                }
            });
            if p.method == PairingMethod::PushButton {
                widgets::separator(ui);
                widgets::switch_row(ui, "Always allow without asking", "", &mut self.always_allow);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let w = (ui.available_width() - 10.0) * 0.5;
                let can_allow = p.method != PairingMethod::EnterPin || self.pin_entry.trim().len() >= 4;
                if widgets::button(Kind::Secondary, "Deny").width(w).height(40.0).show(ui).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape))
                {
                    reply = Some(PairingReply::Reject);
                }
                if widgets::button(Kind::Primary, "Allow").width(w).height(40.0).enabled(can_allow).show(ui).clicked()
                    || (can_allow && reply.is_none() && ui.input(|i| i.key_pressed(Key::Enter)))
                {
                    reply = Some(match p.method {
                        PairingMethod::EnterPin => PairingReply::AcceptWithPin(self.pin_entry.trim().to_string()),
                        _ => PairingReply::Accept,
                    });
                }
            });
        });
        if let Some(r) = reply {
            if self.always_allow && r == PairingReply::Accept {
                self.settings.auto_accept = true;
                self.settings.save();
                self.engine.send(EngineCmd::UpdateSettings(self.settings.clone()));
            }
            self.pin_entry.clear();
            self.always_allow = false;
            self.engine.send(EngineCmd::PairingReply { id: p.id, reply: r });
            // hide it right away; the engine confirms on its next update
            self.snapshot.prompts.retain(|x| x.id != p.id);
        }
    }

    pub(super) fn drop_overlay(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let (title, color) = match &self.snapshot.share_url {
            Some(_) => (format!("Drop to share with {}", self.snapshot.session.peer_name), ACCENT),
            None => ("Connect a device to share files".into(), IDLE),
        };
        let area = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::NULL.with("drop")));
        painter.rect_filled(area, 0.0, tint(BG, 0.96));
        let r = area.shrink(20.0);
        let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        painter.extend(egui::Shape::dashed_line(&corners, Stroke::new(1.5, color), 10.0, 7.0));
        icons::tile(&painter, area.center() - vec2(0.0, 34.0), 68.0, Icon::Upload, color);
        painter.text(area.center() + vec2(0.0, 34.0), Align2::CENTER_CENTER, title, semibold(20.0), TEXT);
    }

    pub(super) fn stats_overlay(&self, ui: &Ui, rect: Rect) {
        let m = self.snapshot.media.clone().unwrap_or_default();
        let s = &self.snapshot.session;
        let rows = [
            ("Decoder", format!("{} · {}", m.decoder, if self.renderer.is_gpu() { "GPU YUV shader" } else { "CPU convert" })),
            ("Video", format!("{}x{} · decode {:.1} fps · display {:.1} fps", m.width, m.height, m.fps, self.render_fps)),
            ("Network", format!("{:.1} Mbit/s · lost packets {}", m.bitrate_kbps / 1000.0, m.lost_packets)),
            (
                "Latency",
                format!(
                    "decode-to-display {:.0} ms · buffer {} ms · {}",
                    self.frame_age_ms,
                    self.settings.jitter_ms,
                    if self.settings.av_sync { "A/V sync" } else { "low-latency" }
                ),
            ),
            ("Stream", format!("{} · {}", s.video.as_deref().unwrap_or("-"), s.audio.as_deref().unwrap_or("no audio"))),
            ("Input", s.uibc.as_ref().map_or("off".to_string(), |u| u.description.clone())),
        ];
        let painter = ui.painter();
        let label_w = 76.0;
        let max_value = (rect.width() - label_w - 52.0).max(80.0);
        let values: Vec<_> = rows.iter().map(|(_, v)| widgets::line(ui, v, mono(12.0), TEXT, max_value)).collect();
        let width = values.iter().map(|g| g.size().x).fold(0.0, f32::max) + label_w + 28.0;
        let row_h = 20.0;
        let card = Rect::from_min_size(rect.min + vec2(12.0, 12.0), vec2(width, row_h * rows.len() as f32 + 20.0));
        painter.rect(card, 12.0, tint(BG, 0.84), Stroke::new(1.0, tint(Color32::WHITE, 0.10)), StrokeKind::Inside);
        for (i, ((label, _), value)) in rows.iter().zip(values).enumerate() {
            let y = card.top() + 10.0 + row_h * (i as f32 + 0.5);
            painter.text(pos2(card.left() + 14.0, y), Align2::LEFT_CENTER, label, medium(12.0), TEXT_FAINT);
            painter.galley(pos2(card.left() + 14.0 + label_w, y - value.size().y * 0.5), value, TEXT);
        }
    }

    /// A reminder of how to get out of full screen, fading after a few seconds.
    pub(super) fn fullscreen_hint(&self, ui: &Ui, rect: Rect) {
        let Some(t) = self.fullscreen_hint else { return };
        let left = 3.5 - t.elapsed().as_secs_f32();
        if !self.fullscreen || left <= 0.0 {
            return;
        }
        let alpha = (left / 0.5).min(1.0);
        let g = ui.painter().layout_no_wrap("Press F11 to leave full screen".into(), medium(14.0), TEXT);
        let pill = Rect::from_center_size(rect.center_top() + vec2(0.0, 44.0), vec2(g.size().x + 36.0, 38.0));
        ui.painter().rect_filled(pill, 19.0, tint(BG, 0.82 * alpha));
        ui.painter().galley_with_override_text_color(pill.center() - g.size() * 0.5, g, TEXT.gamma_multiply(alpha));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
}

/// An eight-digit PIN reads better as two groups of boxed digits.
fn pin_digits(ui: &mut Ui, pin: &str) {
    let n = pin.chars().count().max(1);
    let split = if n == 8 { 4 } else { n };
    let gap = 5.0;
    let group_gap = if split < n { 10.0 } else { 0.0 };
    let w = ((ui.available_width() - gap * (n - 1) as f32 - group_gap) / n as f32).min(34.0);
    let total = w * n as f32 + gap * (n - 1) as f32 + group_gap;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 46.0), Sense::hover());
    let mut x = rect.left();
    for (i, c) in pin.chars().enumerate() {
        if i == split {
            x += group_gap;
        }
        let b = Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height()));
        ui.painter().rect(b, 8.0, FIELD, Stroke::new(1.0, BORDER_STRONG), StrokeKind::Inside);
        ui.painter().text(b.center(), Align2::CENTER_CENTER, c, semibold(22.0), TEXT);
        x += w + gap;
    }
}
