//! The settings on the right: format, picture size, how many videos, how big each may be, and Export.

use super::theme::*;
use crate::app::{open_folder, App, PartState, SIZE_PRESETS};
use cutline::media::{clock, human_bytes};
use cutline::plan::{format_available, Fit, FormatChoice, Plan, Quality, FORMATS};
use eframe::egui::{self, vec2, Color32, Margin, RichText, Stroke, Ui};

impl App {
    pub fn output_panel(&mut self, ui: &mut Ui, plan: Option<&Plan>) {
        let exporting = self.exporting();
        egui::Panel::bottom("export footer")
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(0, 10)).fill(PANEL))
            .show_separator_line(false)
            .resizable(false)
            .show(ui, |ui| self.export_footer(ui, plan));
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.style_mut().spacing.item_spacing.y = 10.0;
            ui.add_enabled_ui(!exporting, |ui| {
                self.format_card(ui);
                self.size_card(ui, plan);
            });
            self.parts_card(ui, plan, exporting);
            ui.add_enabled_ui(!exporting, |ui| self.save_card(ui, plan));
        });
    }

    fn format_card(&mut self, ui: &mut Ui) {
        card(ui, |ui| {
            section_label(ui, "Save as");
            let label = match &self.format {
                FormatChoice::Listed(index) => FORMATS[*index].label.to_string(),
                FormatChoice::Other(_) => "Other format...".to_string(),
            };
            egui::ComboBox::from_id_salt("format").selected_text(label).width(ui.available_width()).show_ui(ui, |ui| {
                let encoders = self.tools.as_ref().map(|tools| tools.encoders.clone()).unwrap_or_default();
                for (index, format) in FORMATS.iter().enumerate() {
                    let usable = format_available(format, &encoders);
                    let name = if usable { format.label.to_string() } else { format!("{} (not in your FFmpeg)", format.label) };
                    let selected = self.format == FormatChoice::Listed(index);
                    if ui.add_enabled(usable, egui::Button::selectable(selected, name)).clicked() {
                        self.format = FormatChoice::Listed(index);
                        ui.close();
                    }
                }
                let other = matches!(self.format, FormatChoice::Other(_));
                if ui.add(egui::Button::selectable(other, "Other format...")).clicked() {
                    self.format = FormatChoice::Other(self.other_format.clone());
                    ui.close();
                }
            });
            if matches!(self.format, FormatChoice::Other(_)) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("File type").color(MUTED));
                    let edit = egui::TextEdit::singleline(&mut self.other_format).hint_text("e.g. ogv").desired_width(ui.available_width());
                    if ui.add(edit).changed() {
                        self.format = FormatChoice::Other(self.other_format.clone());
                    }
                });
            }
        });
    }

    fn size_card(&mut self, ui: &mut Ui, plan: Option<&Plan>) {
        card(ui, |ui| {
            section_label(ui, "Final video size");
            let shown = match plan {
                Some(plan) if self.size_preset == 0 && !self.clips.is_empty() => format!("Same as my videos  ({} x {})", plan.width, plan.height),
                _ => SIZE_PRESETS[self.size_preset].0.to_string(),
            };
            egui::ComboBox::from_id_salt("frame").selected_text(shown).width(ui.available_width()).show_ui(ui, |ui| {
                for (index, (name, _)) in SIZE_PRESETS.iter().enumerate() {
                    if ui.add(egui::Button::selectable(self.size_preset == index, *name)).clicked() {
                        self.size_preset = index;
                        ui.close();
                    }
                }
            });
            if self.size_preset + 1 == SIZE_PRESETS.len() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.custom_width).desired_width(64.0).hint_text("Width"));
                    ui.label(RichText::new("x").color(MUTED));
                    ui.add(egui::TextEdit::singleline(&mut self.custom_height).desired_width(64.0).hint_text("Height"));
                    ui.label(RichText::new("pixels").color(MUTED));
                });
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 4.0;
                let width = (ui.available_width() - 8.0) / 3.0;
                for (fit, name, tip) in [
                    (Fit::Bars, "Fit", "Show the whole picture, with black bars if the shape differs"),
                    (Fit::Crop, "Fill", "Fill the frame, cropping the edges"),
                    (Fit::Stretch, "Stretch", "Fill the frame by stretching the picture"),
                ] {
                    let selected = self.fit == fit;
                    let text = RichText::new(name).size(12.5).color(if selected { Color32::WHITE } else { MUTED });
                    let button = egui::Button::new(text)
                        .fill(if selected { ACCENT.linear_multiply(0.75) } else { RAISED })
                        .stroke(Stroke::new(1.0, if selected { ACCENT } else { LINE }))
                        .min_size(vec2(width, 28.0));
                    if ui.add(button).on_hover_text(tip).clicked() {
                        self.fit = fit;
                    }
                }
            });
        });
    }

    fn parts_card(&mut self, ui: &mut Ui, plan: Option<&Plan>, exporting: bool) {
        card(ui, |ui| {
            ui.add_enabled_ui(!exporting && self.part_states.is_empty(), |ui| {
                ui.horizontal(|ui| {
                    section_label(ui, "Number of videos");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.style_mut().spacing.item_spacing.x = 4.0;
                        let mut count = self.part_limits.len();
                        if ui.add_sized(vec2(28.0, 26.0), egui::Button::new("+")).clicked() && count < 99 {
                            count += 1;
                        }
                        ui.add_sized(vec2(40.0, 26.0), egui::DragValue::new(&mut count).range(1..=99).speed(0.1));
                        if ui.add_sized(vec2(28.0, 26.0), egui::Button::new("-")).clicked() && count > 1 {
                            count -= 1;
                        }
                        self.set_part_count(count);
                    });
                });
                let each = plan.and_then(|plan| plan.parts.first()).map_or(0.0, |part| part.duration);
                if self.part_limits.len() > 1 && each > 0.0 {
                    ui.label(RichText::new(format!("Cut into {} equal parts of {} each, same quality.", self.part_limits.len(), clock(each))).size(12.0).color(MUTED));
                }
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    section_label(ui, "Max file size");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let flipped = toggle(ui, &mut self.same_limit);
                        ui.label(RichText::new("Same for all").size(12.0).color(MUTED));
                        if flipped && self.same_limit {
                            let first = self.part_limits[0].clone();
                            self.part_limits.iter_mut().for_each(|limit| *limit = first.clone());
                        }
                    });
                });
            });

            for index in 0..self.part_limits.len() {
                self.part_row(ui, index, plan);
            }
            if !exporting && self.part_states.is_empty() {
                ui.label(RichText::new("Clear a size for no limit, at the best quality.").size(11.5).color(FAINT));
                if self.limits_differ() {
                    ui.label(RichText::new("Different sizes will make the quality differ between parts.").size(11.5).color(WARN));
                }
            }
        });
    }

    fn part_row(&mut self, ui: &mut Ui, index: usize, plan: Option<&Plan>) {
        let part = plan.and_then(|plan| plan.parts.get(index));
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, part_color(index));
            let title = if self.part_limits.len() == 1 { "Video".to_string() } else { format!("Part {}", index + 1) };
            ui.label(RichText::new(title).size(13.0));
            if let Some(part) = part.filter(|part| part.duration > 0.0) {
                ui.label(RichText::new(clock(part.duration)).size(12.0).color(FAINT));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match self.part_states.get(index).cloned() {
                    Some(PartState::Waiting) => {
                        ui.label(RichText::new("Waiting").size(12.0).color(FAINT));
                    }
                    Some(PartState::Working { label, fraction }) => {
                        ui.label(RichText::new(label).size(12.0).color(MUTED));
                        ui.add(egui::ProgressBar::new(fraction).desired_width(ui.available_width().max(40.0)).desired_height(8.0));
                    }
                    Some(PartState::Done { bytes, over_limit, .. }) => {
                        let color = if over_limit { WARN } else { GOOD };
                        let text = if over_limit { format!("{} (over)", human_bytes(bytes as f64)) } else { human_bytes(bytes as f64) };
                        ui.label(RichText::new(text).size(12.5).color(color).strong());
                    }
                    Some(PartState::Failed(message)) => {
                        ui.label(RichText::new("Failed").size(12.5).color(BAD)).on_hover_text(message);
                    }
                    None => {
                        if let Some(part) = part {
                            if !self.clips.is_empty() {
                                quality_chip(ui, part.quality, part.video_kbps);
                            }
                        }
                        ui.label(RichText::new("MB").size(12.0).color(MUTED));
                        let valid = self.parse_limit(index).is_ok();
                        let mut edit = egui::TextEdit::singleline(&mut self.part_limits[index])
                            .desired_width(52.0)
                            .align(egui::Align2::RIGHT_CENTER)
                            .hint_text("none");
                        if !valid {
                            edit = edit.text_color(BAD);
                        }
                        let mut output = edit.show(ui);
                        // A click selects the whole number, so typing replaces it.
                        if output.response.gained_focus() {
                            let everything = egui::text::CCursorRange::select_all_str(&self.part_limits[index]);
                            output.state.cursor.set_char_range(Some(everything));
                            output.state.store(ui.ctx(), output.response.id);
                        }
                        if output.response.changed() && self.same_limit {
                            let value = self.part_limits[index].clone();
                            self.part_limits.iter_mut().for_each(|limit| *limit = value.clone());
                        }
                    }
                }
            });
        });
    }

    fn save_card(&mut self, ui: &mut Ui, plan: Option<&Plan>) {
        card(ui, |ui| {
            section_label(ui, "Save to");
            ui.horizontal(|ui| {
                ui.label(RichText::new("Name").color(MUTED));
                let edit = egui::TextEdit::singleline(&mut self.base_name).desired_width(ui.available_width() - 48.0);
                if ui.add(edit).changed() {
                    self.name_edited = true;
                }
                let ext = plan.map_or("mp4".to_string(), |plan| plan.ext.clone());
                ui.label(RichText::new(format!(".{ext}")).color(MUTED));
            });
            ui.horizontal(|ui| {
                let shown = match &self.out_dir {
                    Some(dir) => dir.display().to_string(),
                    None => "Same folder as my videos".to_string(),
                };
                let width = ui.available_width() - 78.0;
                ui.allocate_ui_with_layout(vec2(width, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(shown).size(12.0).color(MUTED)).truncate());
                });
                if soft_button(ui, "Change").clicked() {
                    self.pick_out_dir();
                }
            });
        });
    }

    fn export_footer(&mut self, ui: &mut Ui, plan: Option<&Plan>) {
        let width = ui.available_width();
        let count = self.part_limits.len();
        if self.exporting() {
            let fraction = self.overall_fraction();
            ui.add(egui::ProgressBar::new(fraction).desired_width(width).desired_height(10.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{:.0}%", fraction * 100.0)).strong());
                if let Some(eta) = self.export_eta() {
                    ui.label(RichText::new(format!("about {} left", clock(eta))).size(12.5).color(MUTED));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if soft_button(ui, "Cancel").clicked() {
                        self.cancel_export();
                    }
                });
            });
            return;
        }
        if let Some(result) = self.export_result.clone() {
            ui.label(RichText::new(result.title.as_str()).color(if result.ok { GOOD } else { WARN }).strong());
            ui.horizontal(|ui| {
                if result.folder.is_some() && soft_button(ui, "Open folder").clicked() {
                    if let Some(folder) = &result.folder {
                        open_folder(folder);
                    }
                }
                if soft_button(ui, "New export").clicked() {
                    self.clear_results();
                }
            });
            return;
        }
        let problem = plan.and_then(|plan| plan.problem.clone()).or_else(|| self.input_problem());
        let label = if count == 1 { "Export video".to_string() } else { format!("Export {count} videos") };
        let ready = self.tools.is_some() && plan.is_some() && problem.is_none();
        let button = primary_button(ui, &label, vec2(width, 46.0), ready);
        if let (true, Some(plan)) = (button.clicked(), plan) {
            self.start_export(plan.clone());
        }
        if let Some(problem) = problem {
            ui.label(RichText::new(problem).size(12.0).color(if self.clips.is_empty() { FAINT } else { WARN }));
        } else if let Some(plan) = plan {
            let budget: Option<u64> = plan.parts.iter().map(|part| part.limit_bytes).sum();
            let hint = match budget {
                Some(bytes) => format!("{} x {} · up to {} in total", plan.width, plan.height, human_bytes(bytes as f64)),
                None => format!("{} x {}", plan.width, plan.height),
            };
            ui.label(RichText::new(hint).size(12.0).color(FAINT));
        }
    }
}

fn quality_chip(ui: &mut Ui, quality: Quality, video_kbps: Option<f64>) {
    let color = match quality {
        Quality::Best | Quality::Great => GOOD,
        Quality::Good => Color32::from_rgb(150, 210, 120),
        Quality::Fair => WARN,
        Quality::Low | Quality::TooSmall => BAD,
    };
    let tip = match video_kbps {
        Some(kbps) if kbps > 0.0 => format!("Estimated quality. The video gets about {:.1} Mbps.", kbps / 1000.0),
        _ => "Estimated quality.".to_string(),
    };
    let text = RichText::new(quality.label()).size(11.5).color(color);
    ui.add_sized(vec2(46.0, 20.0), egui::Label::new(text)).on_hover_text(tip);
}
