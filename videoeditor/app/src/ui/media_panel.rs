//! The list of videos on the left.

use super::theme::*;
use crate::app::App;
use cutline::media::{clock, human_bytes};
use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Rect, RichText, Sense, Stroke, Ui};

const ROW_HEIGHT: f32 = 64.0;

enum RowAction {
    Select(usize),
    Move(usize, usize),
    Remove(usize),
}

impl App {
    pub fn media_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Videos").size(15.0).strong());
            if !self.clips.is_empty() {
                ui.label(RichText::new(self.clips.len().to_string()).size(13.0).color(FAINT));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = !self.exporting();
                if ui.add_enabled(enabled, egui::Button::new(RichText::new("+ Add").size(12.5))).on_hover_text("Choose videos to add").clicked() {
                    self.add_with_dialog();
                }
                if self.manual_order
                    && ui
                        .add_enabled(enabled, egui::Button::new(RichText::new("A-Z").size(12.5)))
                        .on_hover_text("Put back in alphabetical order")
                        .clicked()
                {
                    self.sort_alphabetically();
                }
            });
        });

        if self.clips.is_empty() && self.probing == 0 {
            ui.add_space(4.0);
            ui.label(RichText::new("Drop videos anywhere in the window. They line up back to back, in alphabetical order.").size(12.5).color(MUTED));
            return;
        }

        let mut action = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.style_mut().spacing.item_spacing.y = 4.0;
            let width = ui.available_width();
            let mut rows: Vec<Rect> = Vec::with_capacity(self.clips.len());
            for index in 0..self.clips.len() {
                let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click_and_drag());
                rows.push(rect);
                if response.clicked() {
                    action = Some(RowAction::Select(index));
                }
                if response.drag_started() && !self.exporting() {
                    self.list_drag = Some(index);
                }
                self.draw_row(ui, index, rect, response.hovered(), &mut action);
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }

            if self.probing > 0 {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    let label = if self.probing == 1 { "Reading a video...".to_string() } else { format!("Reading {} videos...", self.probing) };
                    ui.label(RichText::new(label).size(12.5).color(MUTED));
                });
            }

            if let Some(from) = self.list_drag {
                let pointer = ui.ctx().pointer_latest_pos().unwrap_or_default();
                let target = rows.iter().position(|row| pointer.y < row.center().y).unwrap_or(rows.len());
                if target != from && target != from + 1 && !rows.is_empty() {
                    let y = if target < rows.len() { rows[target].top() - 2.0 } else { rows[rows.len() - 1].bottom() + 2.0 };
                    ui.painter().line_segment([pos2(rows[0].left(), y), pos2(rows[0].right(), y)], Stroke::new(2.5, ACCENT_HOVER));
                }
                if ui.input(|input| input.pointer.any_released()) {
                    self.list_drag = None;
                    if target != from && target != from + 1 {
                        action = Some(RowAction::Move(from, target));
                    }
                }
            }
        });

        match action {
            Some(RowAction::Select(index)) => self.select_clip(index),
            Some(RowAction::Move(from, target)) => self.move_clip(from, target),
            Some(RowAction::Remove(index)) => self.remove_clip(index),
            None => {}
        }
    }

    fn draw_row(&self, ui: &mut Ui, index: usize, rect: Rect, hovered: bool, action: &mut Option<RowAction>) {
        let clip = &self.clips[index];
        let selected = self.selected == Some(clip.id);
        let dragged = self.list_drag == Some(index);
        let fill = if selected {
            Color32::from_rgb(40, 46, 66)
        } else if hovered {
            RAISED
        } else {
            CARD
        };
        let painter = ui.painter().clone();
        painter.rect_filled(rect, CornerRadius::same(9), fill);
        if selected {
            outline(&painter, rect, 9, Stroke::new(1.2, ACCENT));
        }

        let thumb = Rect::from_min_size(rect.min + vec2(9.0, 9.0), vec2(82.0, 46.0));
        match clip.thumbs.get(clip.thumbs.len() / 2).and_then(|t| t.as_ref()).or(clip.thumbs.iter().flatten().next()) {
            Some(texture) => {
                let tint = if dragged { Color32::from_gray(90) } else { Color32::WHITE };
                egui::Image::new(texture).corner_radius(CornerRadius::same(5)).tint(tint).paint_at(ui, thumb);
            }
            None => {
                painter.rect_filled(thumb, CornerRadius::same(5), clip_color(clip.id));
            }
        }
        let badge = Rect::from_min_size(thumb.min + vec2(3.0, 3.0), vec2(18.0, 16.0));
        painter.rect_filled(badge, CornerRadius::same(4), Color32::from_black_alpha(170));
        painter.text(badge.center(), Align2::CENTER_CENTER, (index + 1).to_string(), FontId::new(10.5, FontFamily::Proportional), TEXT);

        let text_left = thumb.right() + 10.0;
        let text_right = rect.right() - if hovered { 78.0 } else { 10.0 };
        let name_clip = Rect::from_min_max(pos2(text_left, rect.top()), pos2(text_right, rect.bottom()));
        let name_painter = painter.with_clip_rect(name_clip.intersect(painter.clip_rect()));
        let color = if dragged { FAINT } else { TEXT };
        name_painter.text(pos2(text_left, rect.top() + 21.0), Align2::LEFT_CENTER, &clip.info.name, FontId::new(13.5, FontFamily::Proportional), color);
        let details = if clip.removed.is_empty() {
            format!("{} x {} · {} · {}", clip.info.width, clip.info.height, clock(clip.info.duration), human_bytes(clip.info.bytes as f64))
        } else {
            let cuts = clip.removed.len();
            let noun = if cuts == 1 { "cut" } else { "cuts" };
            format!("{} x {} · {} of {} · {cuts} {noun}", clip.info.width, clip.info.height, clock(clip.kept_duration()), clock(clip.info.duration))
        };
        name_painter.text(pos2(text_left, rect.top() + 42.0), Align2::LEFT_CENTER, details, FontId::new(11.5, FontFamily::Proportional), MUTED);

        if hovered && !self.exporting() {
            let mut buttons = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_size(pos2(rect.right() - 76.0, rect.center().y - 12.0), vec2(72.0, 24.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            buttons.style_mut().spacing.item_spacing.x = 0.0;
            let up = icon_button(&mut buttons, 24.0, index > 0, |p, r, c| draw_chevron(p, r, c, true)).on_hover_text("Earlier");
            if up.clicked() {
                *action = Some(RowAction::Move(index, index - 1));
            }
            let down = icon_button(&mut buttons, 24.0, index + 1 < self.clips.len(), |p, r, c| draw_chevron(p, r, c, false)).on_hover_text("Later");
            if down.clicked() {
                *action = Some(RowAction::Move(index, index + 2));
            }
            if icon_button(&mut buttons, 24.0, true, draw_close).on_hover_text("Remove").clicked() {
                *action = Some(RowAction::Remove(index));
            }
        }
    }
}
