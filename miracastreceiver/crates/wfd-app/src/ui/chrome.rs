//! The bars around the picture: the header and the session controls.

use egui::{Align, CornerRadius, Layout, Sense, vec2};
use wfd_transport::{TransportKind, TransportState};

use super::icons::{self, Icon};
use super::theme::*;
use super::widgets::{self, Kind};
use super::{Drawer, ReceiverApp};
use crate::engine::{EngineCmd, Phase};

pub const HEADER_HEIGHT: f32 = 58.0;
pub const CONTROLS_HEIGHT: f32 = 62.0;

/// Colour, one-word state and full explanation of a connection method.
pub fn transport_summary(state: &TransportState) -> (egui::Color32, &'static str, String) {
    match state {
        TransportState::Ready { detail } => (OK, "Ready", detail.clone()),
        TransportState::Busy { detail } => (WARN, "Busy", detail.clone()),
        TransportState::Starting => (WARN, "Starting", "Starting up".into()),
        TransportState::Unavailable { reason } => (IDLE, "Unavailable", reason.clone()),
        TransportState::Error { message } => (DANGER, "Error", message.clone()),
    }
}

/// A panel frame with a hairline on the side that faces the content.
pub fn bar_frame() -> egui::Frame {
    egui::Frame::new().fill(SURFACE).inner_margin(egui::Margin::symmetric(14, 0))
}

impl ReceiverApp {
    pub(super) fn header(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_width();
        ui.allocate_ui_with_layout(vec2(full, HEADER_HEIGHT), Layout::left_to_right(Align::Center), |ui| {
            ui.set_height(HEADER_HEIGHT);
            ui.spacing_mut().item_spacing.x = 11.0;
            icons::show_tile(ui, Icon::Cast, 36.0, ACCENT);
            let name = self.snapshot.receiver_name.clone();
            widgets::title_block(ui, &name, "Miracast Receiver", (full - 280.0).clamp(90.0, 280.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if widgets::icon_button(ui, Icon::Sliders, "Settings", self.drawer == Some(Drawer::Settings)).clicked() {
                    self.toggle_drawer(Drawer::Settings);
                }
                if widgets::icon_button(ui, Icon::Expand, "Full screen (F11, or double-click the picture)", false).clicked() {
                    self.toggle_fullscreen(ui.ctx());
                }
                if widgets::icon_button(ui, Icon::Stats, "Stats overlay (F3)", self.settings.show_overlay).clicked() {
                    self.settings.show_overlay = !self.settings.show_overlay;
                    self.settings.save();
                }
                if widgets::icon_button(ui, Icon::List, "Activity", self.drawer == Some(Drawer::Activity)).clicked() {
                    self.toggle_drawer(Drawer::Activity);
                }
                ui.add_space(8.0);
                ui.spacing_mut().item_spacing.x = 8.0;
                for (label, (kind, state)) in self.snapshot.transports.iter().rev() {
                    // drop the pills rather than overlap the title in narrow windows
                    if *kind == TransportKind::Direct || ui.available_width() < 150.0 {
                        continue;
                    }
                    let (color, word, detail) = transport_summary(state);
                    widgets::status_pill(ui, color, label).on_hover_text(format!("{word}: {detail}"));
                }
            });
        });
    }

    pub(super) fn session_controls(&mut self, ui: &mut egui::Ui) {
        let s = self.snapshot.session.clone();
        let full = ui.available_width();
        ui.allocate_ui_with_layout(vec2(full, CONTROLS_HEIGHT), Layout::right_to_left(Align::Center), |ui| {
            ui.set_height(CONTROLS_HEIGHT);
            ui.spacing_mut().item_spacing.x = 6.0;
            let narrow = full < 640.0;

            let button = widgets::button(Kind::Danger, "Disconnect").icon(Icon::Stop);
            let button = if narrow { button.icon_only() } else { button };
            if button.show(ui).clicked() {
                self.engine.send(EngineCmd::Disconnect);
            }
            ui.add_space(8.0);
            if self.snapshot.share_url.is_some()
                && widgets::icon_button(ui, Icon::Folder, "Share files with the device", self.drawer == Some(Drawer::Files)).clicked()
            {
                self.toggle_drawer(Drawer::Files);
            }
            if s.phase == Phase::Playing
                && widgets::icon_button(ui, Icon::Refresh, "Refresh the picture (asks the device for a key frame)", false).clicked()
            {
                self.engine.send(EngineCmd::RequestIdr);
            }
            ui.add_space(8.0);
            let mut volume = self.settings.volume as f32;
            let slider_width = if full < 440.0 { 60.0 } else { 104.0 };
            if widgets::slider(ui, &mut volume, 0.0..=1.5, slider_width, "Volume").changed() {
                self.settings.volume = f64::from(volume);
                self.engine.send(EngineCmd::SetVolume(f64::from(volume)));
                self.settings.save();
            }
            let (icon, tip) = if self.settings.muted { (Icon::VolumeOff, "Unmute") } else { (Icon::Volume, "Mute") };
            if widgets::icon_button(ui, icon, tip, self.settings.muted).clicked() {
                self.settings.muted = !self.settings.muted;
                self.engine.send(EngineCmd::SetMuted(self.settings.muted));
                self.settings.save();
            }

            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                widgets::dot(ui, r.center(), if s.phase == Phase::Playing { OK } else { WARN });
                let title = match s.phase {
                    Phase::Playing => s.peer_name.clone(),
                    Phase::Paused => format!("{} (paused)", s.peer_name),
                    Phase::Negotiating => format!("Negotiating with {}…", s.peer_name),
                    Phase::Connecting => format!("Connecting to {}…", s.peer_name),
                    Phase::Idle => "Waiting for a device".into(),
                };
                let mut detail = Vec::new();
                if let Some(k) = s.kind {
                    detail.push(k.label().to_string());
                }
                if let (Phase::Playing, Some(t)) = (s.phase, s.since) {
                    detail.push(clock(t.elapsed().as_secs()));
                }
                if let Some(v) = &s.video {
                    detail.push(v.clone());
                }
                // room for the input hint beside the title, if there is any
                let hint = s.uibc.as_ref().map(|u| {
                    let text = match (u.relative_mouse, self.captured) {
                        (true, true) => "Mouse locked · Ctrl+Alt releases",
                        (true, false) => "Click the picture to control",
                        (false, _) => "Mouse and keyboard active",
                    };
                    (text, u.description.clone())
                });
                let available = ui.available_width();
                let hint_width = 250.0;
                let show_hint = hint.is_some() && available > 210.0 + hint_width;
                let text_width = if show_hint { available - hint_width - 16.0 } else { available - 8.0 };
                widgets::title_block(ui, &title, &detail.join("  ·  "), text_width.max(40.0));
                if let (true, Some((text, description))) = (show_hint, hint) {
                    ui.add_space(6.0);
                    let g = widgets::line(ui, text, medium(12.0), TEXT_DIM, hint_width - 20.0);
                    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 20.0, 26.0), Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(13), RAISED);
                    ui.painter().galley(rect.center() - g.size() * 0.5, g, TEXT_DIM);
                    resp.on_hover_text(description);
                }
            });
        });
    }
}

/// `m:ss`, or `h:mm:ss` for long sessions.
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_session_time() {
        assert_eq!(clock(5), "0:05");
        assert_eq!(clock(754), "12:34");
        assert_eq!(clock(3723), "1:02:03");
    }
}
