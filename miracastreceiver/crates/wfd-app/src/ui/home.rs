//! What the window shows while there is no picture: how to connect, or how far
//! a connection has got.

use egui::{Align, Align2, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};
use wfd_transport::{TransportKind, TransportState};

use super::ReceiverApp;
use super::chrome::transport_summary;
use super::icons::{self, Icon};
use super::theme::*;
use super::widgets::{self, Kind};
use crate::engine::{EngineCmd, Phase};

const COLUMN: f32 = 720.0;

impl ReceiverApp {
    pub(super) fn home(&mut self, ui: &mut Ui) {
        let height = ui.available_height();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let full = ui.available_width();
            let width = (full - 48.0).clamp(200.0, COLUMN);
            // centre vertically, using last frame's content height
            let id = ui.scope_id().with(("home-height", self.snapshot.session.phase == Phase::Idle));
            let last: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(height);
            ui.add_space(((height - last) * 0.45).max(22.0));
            let drawn = ui.horizontal(|ui| {
                ui.add_space(((full - width) * 0.5).max(0.0));
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 12.0;
                    if self.snapshot.session.phase == Phase::Idle {
                        self.ready(ui);
                    } else {
                        self.connecting(ui);
                    }
                });
            });
            let h = drawn.response.rect.height();
            if (h - last).abs() > 0.5 {
                ui.data_mut(|d| d.insert_temp(id, h));
                ui.ctx().request_repaint();
            }
            ui.add_space(22.0);
        });
    }

    fn ready(&mut self, ui: &mut Ui) {
        let s = self.snapshot.clone();
        let wifi_direct_ok = s
            .transports
            .values()
            .any(|(k, st)| *k == TransportKind::WifiDirect && matches!(st, TransportState::Ready { .. } | TransportState::Busy { .. }));
        let discoverable = s
            .transports
            .values()
            .any(|(k, st)| *k != TransportKind::Direct && matches!(st, TransportState::Ready { .. } | TransportState::Busy { .. }));

        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            // short windows need the room for the instructions
            if ui.ctx().content_rect().height() >= 520.0 {
                beacon(ui, discoverable);
                ui.add_space(4.0);
            }
            ui.label(egui::RichText::new("Ready to cast").font(semibold(28.0)).color(TEXT));
            ui.label(egui::RichText::new("On your phone or PC, choose this screen").font(regular(14.0)).color(TEXT_DIM));
            ui.add_space(6.0);
            name_chip(ui, &s.receiver_name);
        });
        ui.add_space(10.0);

        if !wifi_direct_ok && !cfg!(target_os = "macos") {
            widgets::banner(
                ui,
                WARN,
                Icon::Alert,
                "Phones need Wi-Fi Direct, which isn't active on this computer yet. See connection status below.",
            );
        }
        if cfg!(target_os = "macos") {
            widgets::banner(ui, WARN, Icon::Info, "Infrastructure Mode (MS-MICE) is active. Wi-Fi Direct is not supported by macOS.");
        }
        if let Some(e) = &s.last_error {
            widgets::banner(ui, DANGER, Icon::Alert, &format!("Last attempt failed: {e}"));
        }

        let quoted = format!("Choose “{}”", s.receiver_name);
        let phone: [&str; 3] = ["Open Quick Settings", "Tap Smart View or the DeX tile", &quoted];
        let windows: [&str; 3] = ["Press Win + K", "Wait for the list of displays", &quoted];
        if ui.available_width() >= 540.0 {
            ui.columns(2, |c| {
                how_to(&mut c[0], Icon::Phone, "Samsung DeX or Android", &phone);
                how_to(&mut c[1], Icon::Monitor, "Windows", &windows);
            });
        } else {
            how_to(ui, Icon::Phone, "Samsung DeX or Android", &phone);
            how_to(ui, Icon::Monitor, "Windows", &windows);
        }

        let methods: Vec<_> = s.transports.iter().filter(|(_, (k, _))| *k != TransportKind::Direct).collect();
        if !methods.is_empty() {
            widgets::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 10.0;
                widgets::section(ui, "Connection status");
                for (i, (label, (_, state))) in methods.into_iter().enumerate() {
                    if i > 0 {
                        widgets::separator(ui);
                    }
                    method_row(ui, label, state);
                }
            });
        }

        ui.add_space(4.0);
        ui.vertical_centered(|ui| {
            if s.sim_running {
                if widgets::button(Kind::Secondary, "Stop the simulated phone").icon(Icon::Stop).show(ui).clicked() {
                    self.engine.send(EngineCmd::StopSimulation);
                }
            } else if widgets::button(Kind::Ghost, "Test with a simulated phone").icon(Icon::Play).show(ui).clicked() {
                self.engine.send(EngineCmd::StartSimulation);
            }
        });
    }

    fn connecting(&mut self, ui: &mut Ui) {
        let s = self.snapshot.clone();
        let (title, step) = match s.session.phase {
            Phase::Connecting => (format!("Connecting to {}", s.session.peer_name), 0),
            Phase::Negotiating => (format!("{} connected", s.session.peer_name), 1),
            _ => (format!("{} connected", s.session.peer_name), 2),
        };
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            progress_ring(ui);
            ui.add_space(6.0);
            ui.add(egui::Label::new(egui::RichText::new(title).font(semibold(22.0)).color(TEXT)).wrap());
        });
        ui.add_space(6.0);
        let card_width = ui.available_width().min(340.0);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - card_width) * 0.5);
            ui.vertical(|ui| {
                ui.set_width(card_width);
                widgets::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 12.0;
                    let steps = ["Opening the screen-sharing session", "Agreeing on video and sound", "Waiting for the first picture"];
                    for (i, text) in steps.into_iter().enumerate() {
                        progress_step(ui, text, i.cmp(&step));
                    }
                });
            });
        });
        if s.sim_running {
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                if widgets::button(Kind::Secondary, "Stop the simulated phone").icon(Icon::Stop).show(ui).clicked() {
                    self.engine.send(EngineCmd::StopSimulation);
                }
            });
        }
    }
}

/// The cast symbol, with rings pulsing out of it while devices can find us.
fn beacon(ui: &mut Ui, live: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(132.0, 132.0), Sense::hover());
    let c = rect.center();
    let core = 36.0;
    let outer = rect.width() * 0.5;
    // animate only while someone can see it
    let animate = live && ui.input(|i| i.viewport().focused.unwrap_or(true));
    let time = if animate { ui.input(|i| i.time) as f32 } else { 0.8 };
    if live {
        for k in 0..3 {
            let t = (time / 3.2 + k as f32 / 3.0).fract();
            let radius = core + (outer - core) * t;
            ui.painter().circle_stroke(c, radius, Stroke::new(1.2, tint(ACCENT, 0.45 * (1.0 - t))));
        }
    } else {
        ui.painter().circle_stroke(c, core + 14.0, Stroke::new(1.0, BORDER));
    }
    if animate {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
    let color = if live { ACCENT } else { IDLE };
    ui.painter().circle_filled(c, core, tint(color, 0.16));
    ui.painter().circle_stroke(c, core, Stroke::new(1.2, tint(color, 0.5)));
    icons::paint(ui.painter(), Rect::from_center_size(c, vec2(36.0, 36.0)), Icon::Cast, if live { ACCENT_HOVER } else { TEXT_DIM });
}

fn progress_ring(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(96.0, 96.0), Sense::hover());
    let c = rect.center();
    let time = ui.input(|i| i.time) as f32;
    ui.painter().circle_stroke(c, 42.0, Stroke::new(3.0, RAISED));
    let start = time * 4.0;
    let points = (0..=32).map(|i| c + 42.0 * egui::Vec2::angled(start + i as f32 / 32.0 * 1.8)).collect();
    ui.painter().add(egui::Shape::line(points, Stroke::new(3.0, ACCENT)));
    icons::paint(ui.painter(), Rect::from_center_size(c, vec2(34.0, 34.0)), Icon::Phone, TEXT);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

fn progress_step(ui: &mut Ui, text: &str, state: std::cmp::Ordering) {
    use std::cmp::Ordering::*;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        let c = r.center();
        let color = match state {
            Less => {
                ui.painter().circle_filled(c, 9.0, tint(OK, 0.18));
                let tick = vec![c + vec2(-4.0, 0.2), c + vec2(-1.2, 3.0), c + vec2(4.2, -2.8)];
                ui.painter().add(egui::Shape::line(tick, Stroke::new(1.8, OK)));
                TEXT_DIM
            }
            Equal => {
                ui.painter().circle_stroke(c, 8.0, Stroke::new(1.5, ACCENT));
                ui.painter().circle_filled(c, 3.5, ACCENT);
                TEXT
            }
            Greater => {
                ui.painter().circle_stroke(c, 8.0, Stroke::new(1.5, BORDER_STRONG));
                TEXT_FAINT
            }
        };
        ui.label(egui::RichText::new(text).font(medium(13.0)).color(color));
    });
}

/// The receiver's name as it appears in the device's list.
fn name_chip(ui: &mut Ui, name: &str) {
    let g = widgets::line(ui, name, semibold(17.0), TEXT, ui.available_width() - 80.0);
    let (rect, _) = ui.allocate_exact_size(vec2(g.size().x + 62.0, 42.0), Sense::hover());
    ui.painter().rect(rect, 21.0, tint(ACCENT, 0.12), Stroke::new(1.0, tint(ACCENT, 0.45)), egui::StrokeKind::Inside);
    let icon = Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), vec2(18.0, 18.0));
    icons::paint(ui.painter(), icon, Icon::Monitor, ACCENT_HOVER);
    ui.painter().galley(pos2(rect.left() + 42.0, rect.center().y - g.size().y * 0.5), g, TEXT);
}

fn how_to(ui: &mut Ui, icon: Icon, title: &str, steps: &[&str]) {
    widgets::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            icons::show_tile(ui, icon, 32.0, ACCENT);
            ui.add(egui::Label::new(egui::RichText::new(title).font(semibold(13.5)).color(TEXT)).truncate());
        });
        for (i, step) in steps.iter().enumerate() {
            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 9.0, RAISED);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, i + 1, medium(11.0), TEXT_DIM);
                // one line each, so the two cards stay the same height
                let w = ui.available_width();
                let g = widgets::line(ui, step, regular(13.0), TEXT_DIM, w);
                let (r, resp) = ui.allocate_exact_size(vec2(g.size().x, 18.0), Sense::hover());
                let elided = g.elided;
                ui.painter().galley(pos2(r.left(), r.center().y - g.size().y * 0.5), g, TEXT_DIM);
                if elided {
                    resp.on_hover_text(*step);
                }
            });
        }
    });
}

fn method_row(ui: &mut Ui, label: &str, state: &TransportState) {
    let (color, word, detail) = transport_summary(state);
    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 18.0), Sense::hover());
        widgets::dot(ui, r.center(), color);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.set_height(18.0);
                ui.label(egui::RichText::new(label).font(medium(13.0)).color(TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(egui::RichText::new(word).font(medium(12.0)).color(color));
                });
            });
            if !detail.is_empty() {
                // errors explain how to fix things, so never cut them off
                ui.add(egui::Label::new(egui::RichText::new(detail).font(regular(12.0)).color(TEXT_FAINT)).wrap().selectable(true));
            }
        });
    });
}
