//! The picture in the middle and its play controls.

use super::theme::*;
use crate::app::App;
use cutline::media::{clock, fine_clock};
use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Rect, RichText, Stroke, Ui};

impl App {
    pub fn preview_panel(&mut self, ui: &mut Ui) {
        if self.tools.is_none() {
            self.missing_ffmpeg(ui);
            return;
        }
        if self.clips.is_empty() {
            self.empty_state(ui);
            return;
        }

        let transport_height = 44.0;
        let area = ui.available_rect_before_wrap();
        let picture_area = Rect::from_min_max(area.min, pos2(area.right(), area.bottom() - transport_height));
        let painter = ui.painter().clone();
        painter.rect_filled(picture_area, CornerRadius::same(10), BG);

        let (width, height) = self.output_dims();
        let aspect = width as f32 / height as f32;
        let mut fit_width = picture_area.width() - 16.0;
        let mut fit_height = fit_width / aspect;
        if fit_height > picture_area.height() - 16.0 {
            fit_height = picture_area.height() - 16.0;
            fit_width = fit_height * aspect;
        }
        let fitted = Rect::from_center_size(picture_area.center(), vec2(fit_width, fit_height));
        painter.rect_filled(fitted, CornerRadius::same(3), Color32::BLACK);
        if let Some(texture) = &self.preview_texture {
            egui::Image::new(texture).paint_at(ui, fitted);
        } else {
            painter.text(fitted.center(), Align2::CENTER_CENTER, "Loading picture...", FontId::new(14.0, FontFamily::Proportional), FAINT);
        }

        let transport = Rect::from_min_max(pos2(area.left(), picture_area.bottom() + 8.0), area.max);
        let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(transport).layout(egui::Layout::left_to_right(egui::Align::Center)));
        bar.style_mut().spacing.item_spacing.x = 4.0;
        if icon_button(&mut bar, 32.0, true, |p, r, c| draw_skip(p, r, c, -1.0)).on_hover_text("Previous video").clicked() {
            self.skip_clip(false);
        }
        let playing = self.playing;
        let play = icon_button(&mut bar, 36.0, true, |p, r, c| if playing { draw_pause(p, r, c) } else { draw_play(p, r, c) })
            .on_hover_text("Play or pause (Space). The preview has no sound.");
        if play.clicked() {
            self.toggle_play();
        }
        if icon_button(&mut bar, 32.0, true, |p, r, c| draw_skip(p, r, c, 1.0)).on_hover_text("Next video").clicked() {
            self.skip_clip(true);
        }
        bar.add_space(10.0);
        bar.label(RichText::new(fine_clock(self.playhead)).monospace().size(14.0));
        bar.label(RichText::new(format!("/ {}", clock(self.total_duration()))).monospace().size(13.0).color(FAINT));
        let exporting = self.exporting();
        bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.style_mut().spacing.item_spacing.x = 6.0;
            let selection = self.selection();
            let remove_label = match selection {
                Some((start, end)) => format!("Remove {}", clock(end - start)),
                None => "Remove".to_string(),
            };
            let remove = egui::Button::new(RichText::new(remove_label).size(12.5).color(if selection.is_some() { Color32::WHITE } else { FAINT }))
                .fill(if selection.is_some() { BAD.gamma_multiply(0.8) } else { RAISED })
                .stroke(Stroke::new(1.0, if selection.is_some() { BAD } else { LINE }));
            let tip = "Cut the marked stretch out of the video (X)";
            if ui.add_enabled(selection.is_some() && !exporting, remove).on_hover_text(tip).on_disabled_hover_text("Mark a start and an end first").clicked() {
                self.remove_selection();
            }
            let end_set = self.mark_out.is_some();
            let mark_end = egui::Button::new(RichText::new("Mark end").size(12.5)).fill(RAISED).stroke(Stroke::new(1.0, if end_set { BAD } else { LINE }));
            if ui.add_enabled(!exporting, mark_end).on_hover_text("The stretch to remove ends at the playhead (O)").clicked() {
                self.mark_end();
            }
            let start_set = self.mark_in.is_some();
            let mark_start = egui::Button::new(RichText::new("Mark start").size(12.5)).fill(RAISED).stroke(Stroke::new(1.0, if start_set { BAD } else { LINE }));
            if ui.add_enabled(!exporting, mark_start).on_hover_text("The stretch to remove starts at the playhead (I)").clicked() {
                self.mark_start();
            }
            if self.can_undo_cut() && !exporting && soft_button(ui, "Undo").on_hover_text("Undo the last cut (Ctrl+Z)").clicked() {
                self.undo_cut();
            }
        });
    }

    fn empty_state(&mut self, ui: &mut Ui) {
        let area = ui.available_rect_before_wrap();
        let painter = ui.painter().clone();
        let zone = area.shrink(14.0);
        painter.rect_filled(zone, CornerRadius::same(16), Color32::from_rgb(19, 21, 27));
        dashed_rect(&painter, zone.shrink(1.0), Stroke::new(1.5, LINE));
        let center = zone.center() - vec2(0.0, 28.0);
        let strip = Rect::from_center_size(center - vec2(0.0, 34.0), vec2(120.0, 76.0));
        painter.rect_filled(strip, CornerRadius::same(10), RAISED);
        for row in [strip.top() + 6.0, strip.bottom() - 15.0] {
            for hole in 0..6 {
                let x = strip.left() + 11.0 + hole as f32 * 18.5;
                painter.rect_filled(Rect::from_min_size(pos2(x, row), vec2(10.0, 9.0)), CornerRadius::same(2), BG);
            }
        }
        triangle(&painter, strip.center() + vec2(2.0, 0.0), 13.0, 1.0, ACCENT_HOVER);
        painter.text(center + vec2(0.0, 28.0), Align2::CENTER_CENTER, "Drop your videos here", FontId::new(23.0, FontFamily::Proportional), TEXT);
        painter.text(
            center + vec2(0.0, 58.0),
            Align2::CENTER_CENTER,
            "They join back to back in alphabetical order. You can rearrange them after.",
            FontId::new(14.0, FontFamily::Proportional),
            MUTED,
        );

        let button = Rect::from_center_size(center + vec2(0.0, 106.0), vec2(190.0, 44.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
        if primary_button(&mut child, "Choose videos", button.size(), true).clicked() {
            self.add_with_dialog();
        }
    }

    fn missing_ffmpeg(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(70.0);
            ui.label(RichText::new("Cutline needs FFmpeg").size(23.0).strong());
            ui.add_space(4.0);
            ui.label(RichText::new("FFmpeg is the free program that does the actual editing. It isn't on this computer yet.").color(MUTED));
            ui.add_space(14.0);
            let hint = if cfg!(target_os = "macos") {
                "On a Mac, with Homebrew:  brew install ffmpeg"
            } else if cfg!(windows) {
                "On Windows, in a terminal:  winget install ffmpeg"
            } else {
                "On Linux, use your package manager to install: ffmpeg"
            };
            ui.label(RichText::new(hint).monospace().color(TEXT));
            ui.add_space(18.0);
            ui.hyperlink_to("Download FFmpeg", "https://ffmpeg.org/download.html");
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                let total = 296.0;
                ui.add_space((ui.available_width() - total).max(0.0) / 2.0);
                if soft_button(ui, "Check again").clicked() {
                    self.find_tools();
                }
                if soft_button(ui, "It's installed somewhere else...").clicked() {
                    self.pick_ffmpeg();
                }
            });
        });
    }
}
