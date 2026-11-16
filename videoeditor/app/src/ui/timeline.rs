//! The strip along the bottom: every clip end to end, and where the result videos will be cut.

use super::theme::*;
use crate::app::App;
use cutline::media::{clock, THUMB_HEIGHT, THUMB_WIDTH};
use cutline::plan::Plan;
use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Rect, Sense, Stroke, Ui};

const STEPS: [f64; 14] = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0];

impl App {
    pub fn timeline_panel(&mut self, ui: &mut Ui, plan: Option<&Plan>) {
        let area = ui.available_rect_before_wrap();
        let ruler = Rect::from_min_size(area.min, vec2(area.width(), 22.0));
        let bands = Rect::from_min_size(pos2(area.left(), ruler.bottom() + 2.0), vec2(area.width(), 20.0));
        let track = Rect::from_min_max(pos2(area.left(), bands.bottom() + 5.0), area.max);
        ui.allocate_rect(area, Sense::hover());
        let painter = ui.painter_at(area.expand(1.0));
        let total = self.total_duration();

        if self.clips.is_empty() {
            painter.rect_filled(track, CornerRadius::same(8), BG);
            dashed_rect(&painter, track.shrink(1.0), Stroke::new(1.0, LINE));
            painter.text(track.center(), Align2::CENTER_CENTER, "Your timeline will appear here", FontId::new(13.0, FontFamily::Proportional), FAINT);
            return;
        }

        let x_at = |time: f64| area.left() + (time / total) as f32 * area.width();

        // Ruler: click or drag to move the playhead.
        let ruler_hit = ui.interact(ruler.union(bands), ui.scope_id().with("ruler"), Sense::click_and_drag());
        if ruler_hit.is_pointer_button_down_on() {
            if let Some(pointer) = ruler_hit.interact_pointer_pos() {
                self.seek(((pointer.x - area.left()) / area.width()) as f64 * total);
            }
        }
        if ruler_hit.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let step = STEPS.iter().copied().find(|step| (step / total) as f32 * area.width() >= 72.0).unwrap_or(14400.0);
        let mut tick = 0.0;
        while tick < total {
            let x = x_at(tick);
            painter.line_segment([pos2(x, ruler.bottom() - 7.0), pos2(x, ruler.bottom())], Stroke::new(1.0, FAINT));
            let minor = x_at(tick + step / 2.0);
            painter.line_segment([pos2(minor, ruler.bottom() - 4.0), pos2(minor, ruler.bottom())], Stroke::new(1.0, LINE));
            if x + 42.0 < area.right() {
                painter.text(pos2(x + 4.0, ruler.top() + 3.0), Align2::LEFT_TOP, clock(tick), FontId::new(10.5, FontFamily::Monospace), FAINT);
            }
            tick += step;
        }

        // The result videos.
        if let Some(plan) = plan {
            for part in &plan.parts {
                let rect = Rect::from_min_max(
                    pos2(x_at(part.start) + 1.0, bands.top()),
                    pos2(x_at(part.start + part.duration) - 1.0, bands.bottom()),
                );
                let color = part_color(part.index);
                painter.rect_filled(rect, CornerRadius::same(4), color.linear_multiply(0.28));
                let label = if plan.parts.len() == 1 {
                    format!("Final video · {}", clock(part.duration))
                } else {
                    format!("Part {} · {}", part.index + 1, clock(part.duration))
                };
                painter.with_clip_rect(rect.intersect(painter.clip_rect())).text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    FontId::new(11.5, FontFamily::Proportional),
                    color,
                );
            }
        }

        // The clips.
        painter.rect_filled(track, CornerRadius::same(8), BG);
        let mut blocks: Vec<Rect> = Vec::with_capacity(self.clips.len());
        let mut clip_start = 0.0;
        for clip in &self.clips {
            let left = x_at(clip_start);
            clip_start += clip.kept_duration();
            let right = x_at(clip_start);
            blocks.push(Rect::from_min_max(pos2(left, track.top()), pos2(right, track.bottom())));
        }
        let mut clicked = None;
        for (index, block) in blocks.iter().enumerate() {
            let response = ui.interact(*block, ui.scope_id().with(("clip block", self.clips[index].id)), Sense::click_and_drag());
            if response.clicked() {
                clicked = Some((index, response.interact_pointer_pos()));
            }
            if response.drag_started() && !self.exporting() {
                self.timeline_drag = Some(index);
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            self.draw_block(ui, &painter, index, *block);
        }

        if let Some(plan) = plan {
            for part in plan.parts.iter().skip(1) {
                let x = x_at(part.start);
                painter.line_segment([pos2(x, bands.bottom()), pos2(x, track.bottom())], Stroke::new(2.0, part_color(part.index)));
            }
        }

        if let Some(from) = self.timeline_drag {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let pointer = ui.ctx().pointer_latest_pos().unwrap_or_default();
            let target = blocks.iter().position(|block| pointer.x < block.center().x).unwrap_or(blocks.len());
            if target != from && target != from + 1 {
                let x = if target < blocks.len() { blocks[target].left() } else { blocks[blocks.len() - 1].right() };
                painter.line_segment([pos2(x, track.top() - 3.0), pos2(x, track.bottom() + 1.0)], Stroke::new(3.0, Color32::WHITE));
            }
            let ghost = Rect::from_center_size(pos2(pointer.x, track.center().y), vec2(blocks[from].width().clamp(70.0, 180.0), track.height() - 8.0));
            painter.rect_filled(ghost, CornerRadius::same(6), ACCENT.linear_multiply(0.45));
            outline(&painter, ghost, 6, Stroke::new(1.5, ACCENT_HOVER));
            if ui.input(|input| input.pointer.any_released()) {
                self.timeline_drag = None;
                if target != from && target != from + 1 {
                    self.move_clip(from, target);
                }
            }
        }

        // The stretch marked for removal.
        let mark_color = Color32::from_rgb(240, 96, 112);
        let full_height = Rect::from_min_max(pos2(area.left(), bands.top()), pos2(area.right(), track.bottom()));
        if let Some((start, end)) = self.selection() {
            let rect = Rect::from_min_max(pos2(x_at(start), full_height.top()), pos2(x_at(end), full_height.bottom()));
            painter.rect_filled(rect, CornerRadius::same(3), mark_color.gamma_multiply(0.30));
            outline(&painter, rect, 3, Stroke::new(1.5, mark_color));
            painter.with_clip_rect(rect.intersect(painter.clip_rect())).text(
                pos2(rect.center().x, bands.center().y),
                Align2::CENTER_CENTER,
                format!("Remove {}", clock(end - start)),
                FontId::new(11.5, FontFamily::Proportional),
                Color32::WHITE,
            );
        } else {
            for (mark, label) in [(self.mark_in, "start"), (self.mark_out, "end")] {
                let Some(at) = mark else { continue };
                let x = x_at(at);
                painter.line_segment([pos2(x, full_height.top()), pos2(x, full_height.bottom())], Stroke::new(2.0, mark_color));
                let align = if label == "start" { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER };
                let offset = if label == "start" { 4.0 } else { -4.0 };
                painter.text(pos2(x + offset, bands.center().y), align, label, FontId::new(11.0, FontFamily::Proportional), mark_color);
            }
        }

        // Where stretches were cut out: click one to put it back.
        let mut restore = None;
        for marker in self.cut_markers() {
            let x = x_at(marker.at);
            let knob = Rect::from_center_size(pos2(x, track.top() + 9.0), vec2(16.0, 16.0));
            let response = ui
                .interact(knob, ui.scope_id().with(("cut", marker.clip_id, marker.range.0.to_bits())), Sense::click())
                .on_hover_text(format!("{} removed here. Click to put it back.", clock(marker.range.1 - marker.range.0)));
            painter.line_segment([pos2(x, track.top()), pos2(x, track.bottom())], Stroke::new(2.0, mark_color));
            let fill = if response.hovered() { Color32::WHITE } else { mark_color };
            painter.circle_filled(knob.center(), 7.0, fill);
            let cross = if response.hovered() { mark_color } else { Color32::WHITE };
            draw_close(&painter, knob, cross);
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                restore = Some((marker.clip_id, marker.range));
            }
        }
        if let Some((clip_id, range)) = restore {
            self.restore_cut(clip_id, range);
            return;
        }

        if let Some((index, pointer)) = clicked {
            self.selected = Some(self.clips[index].id);
            if let Some(pointer) = pointer {
                self.seek(((pointer.x - area.left()) / area.width()) as f64 * total);
            }
        }

        // The playhead, on top of everything.
        let x = x_at(self.playhead.min(total));
        painter.line_segment([pos2(x, ruler.top() + 8.0), pos2(x, track.bottom())], Stroke::new(1.6, Color32::WHITE));
        let cap = vec![
            pos2(x - 6.0, ruler.top()),
            pos2(x + 6.0, ruler.top()),
            pos2(x + 6.0, ruler.top() + 6.0),
            pos2(x, ruler.top() + 13.0),
            pos2(x - 6.0, ruler.top() + 6.0),
        ];
        painter.add(egui::Shape::convex_polygon(cap, Color32::WHITE, Stroke::NONE));
    }

    fn draw_block(&self, ui: &Ui, painter: &egui::Painter, index: usize, block: Rect) {
        let clip = &self.clips[index];
        let inner = block.shrink2(vec2(1.0, 0.0));
        let dimmed = self.timeline_drag == Some(index);
        painter.rect_filled(inner, CornerRadius::same(6), clip_color(clip.id));
        let tile_width = inner.height() * THUMB_WIDTH as f32 / THUMB_HEIGHT as f32;
        let tiles = ((inner.width() / tile_width).ceil() as usize).max(1);
        let textures: Vec<_> = clip.thumbs.iter().flatten().collect();
        if !textures.is_empty() {
            let strip = painter.with_clip_rect(inner.intersect(painter.clip_rect()));
            for tile in 0..tiles {
                let left = inner.left() + tile as f32 * tile_width;
                let slot = (tile * clip.thumbs.len()) / tiles;
                let Some(texture) = clip.thumbs.get(slot).and_then(|t| t.as_ref()).or(textures.first().copied()) else { continue };
                let rect = Rect::from_min_size(pos2(left, inner.top()), vec2(tile_width, inner.height()));
                let tint = if dimmed { Color32::from_gray(70) } else { Color32::from_gray(205) };
                strip.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
            }
        }
        let selected = self.selected == Some(clip.id);
        let stroke = if selected { Stroke::new(2.0, ACCENT_HOVER) } else { Stroke::new(1.0, Color32::from_white_alpha(38)) };
        outline(painter, inner, 6, stroke);

        let label_band = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 22.0), inner.max);
        let text_painter = painter.with_clip_rect(inner.shrink(1.0).intersect(painter.clip_rect()));
        text_painter.rect_filled(label_band, CornerRadius::ZERO, Color32::from_black_alpha(150));
        let font = FontId::new(11.5, FontFamily::Proportional);
        text_painter.text(pos2(inner.left() + 7.0, label_band.center().y), Align2::LEFT_CENTER, format!("{}  {}", index + 1, clip.info.name), font.clone(), TEXT);
        let duration = clock(clip.kept_duration());
        let duration_width = ui.fonts_mut(|fonts| fonts.layout_no_wrap(duration.clone(), font.clone(), TEXT).size().x);
        if inner.width() > duration_width + 100.0 {
            text_painter.text(pos2(inner.right() - 7.0, label_band.center().y), Align2::RIGHT_CENTER, duration, font, MUTED);
        }
    }
}
