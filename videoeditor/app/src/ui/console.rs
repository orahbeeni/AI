//! The "View console" panel: every FFmpeg command and its output as it runs.

use super::theme::*;
use crate::app::App;
use cutline::log::LogKind;
use eframe::egui::{self, RichText, Ui};

impl App {
    pub fn console_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Console").strong());
            ui.label(RichText::new("the FFmpeg commands, as they run").size(12.0).color(FAINT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if soft_button(ui, "Clear").clicked() {
                    self.console.clear();
                }
                if soft_button(ui, "Copy all").clicked() {
                    let text: Vec<&str> = self.console.iter().map(|line| line.text.as_str()).collect();
                    ui.ctx().copy_text(text.join("\n"));
                    self.toast("Copied the console");
                }
            });
        });
        egui::Frame::new().fill(BG).corner_radius(8.0).inner_margin(10.0).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
                ui.style_mut().spacing.item_spacing.y = 3.0;
                if self.console.is_empty() {
                    ui.label(RichText::new("Commands appear here as they run.").monospace().color(FAINT));
                }
                for line in &self.console {
                    let text = RichText::new(line.text.as_str()).monospace();
                    let styled = match line.kind {
                        LogKind::Command => RichText::new(format!("$ {}", line.text)).monospace().color(ACCENT_HOVER).strong(),
                        LogKind::Info => text.color(TEXT),
                        LogKind::Output => text.color(MUTED),
                        LogKind::Warning => text.color(WARN),
                        LogKind::Error => text.color(BAD),
                        LogKind::Success | LogKind::Live => text.color(GOOD),
                    };
                    ui.add(egui::Label::new(styled).wrap().selectable(true));
                }
            });
        });
    }
}
