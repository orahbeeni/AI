//! The right-hand panel: properties of the screen or of the selected parts.

use crate::app::App;
use crate::catalog;
use crate::model::{parse_hex, to_hex, Align, State};
use eframe::egui::{self, Color32, RichText, Vec2};

const SWATCHES: [&str; 10] = ["#ffffff", "#eaeaea", "#2b2b2b", "#4076cc", "#d44a3a", "#40a060", "#e69628", "#8e5bd0", "#fff18c", "#cfe0fb"];

fn swatch_row(ui: &mut egui::Ui, value: &mut Option<String>, id: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        let none = ui.add(egui::Button::new("–").min_size(Vec2::splat(20.0)).selected(value.is_none())).on_hover_text("Default");
        if none.clicked() {
            *value = None;
        }
        for hex in SWATCHES {
            let c = parse_hex(hex).unwrap();
            let col = Color32::from_rgb(c[0], c[1], c[2]);
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::click());
            ui.painter().rect_filled(r, 4.0, col);
            let on = value.as_deref().map(|v| v.eq_ignore_ascii_case(hex)).unwrap_or(false);
            ui.painter().rect_stroke(r, 4.0, egui::Stroke::new(if on { 2.0 } else { 1.0 }, if on { Color32::from_rgb(40, 120, 230) } else { Color32::GRAY }), egui::StrokeKind::Inside);
            if resp.on_hover_text(hex).clicked() {
                *value = Some(hex.to_string());
            }
        }
        let mut c = value.as_deref().and_then(parse_hex).unwrap_or([128, 128, 128, 255]);
        let mut col = Color32::from_rgb(c[0], c[1], c[2]);
        if ui.push_id(id, |ui| ui.color_edit_button_srgba(&mut col)).inner.changed() {
            c = [col.r(), col.g(), col.b(), 255];
            *value = Some(to_hex(c));
        }
    });
}

impl App {
    pub fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 6.0;
        match self.sel.len() {
            0 => self.screen_props(ui),
            1 => self.control_props(ui),
            _ => self.multi_props(ui),
        }
    }

    fn screen_props(&mut self, ui: &mut egui::Ui) {
        ui.heading("Screen");
        let others: Vec<(String, String)> = self.proj.screens.iter().filter(|s| s.id != self.screen().id).map(|s| (s.id.clone(), s.name.clone())).collect();
        let s = &mut self.proj.screens[self.cur];
        egui::Grid::new("screen_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut s.name);
            ui.end_row();
            ui.label("Device");
            let cur_name = catalog::device(&s.device).map(|d| d.name).unwrap_or("Custom");
            egui::ComboBox::from_id_salt("device").selected_text(cur_name).width(180.0).show_ui(ui, |ui| {
                for d in catalog::DEVICES {
                    if ui.selectable_label(s.device == d.id, d.name).clicked() {
                        s.device = d.id.to_string();
                        s.w = d.w;
                        s.h = d.h;
                        self.fit_next = true;
                    }
                }
            });
            ui.end_row();
            ui.label("Size");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.w).range(20.0..=8000.0).prefix("w "));
                ui.add(egui::DragValue::new(&mut s.h).range(20.0..=8000.0).prefix("h "));
                if ui.small_button("Rotate").on_hover_text("Turn sideways").clicked() {
                    std::mem::swap(&mut s.w, &mut s.h);
                    self.fit_next = true;
                }
            });
            ui.end_row();
            ui.label("Look");
            ui.checkbox(&mut s.dark, "Dark mode");
            ui.end_row();
            ui.label("Template");
            let bg_name = s.background.as_ref().and_then(|b| others.iter().find(|o| &o.0 == b)).map(|o| o.1.clone()).unwrap_or_else(|| "None".into());
            egui::ComboBox::from_id_salt("bg").selected_text(bg_name).width(180.0).show_ui(ui, |ui| {
                if ui.selectable_label(s.background.is_none(), "None").clicked() {
                    s.background = None;
                }
                for (id, name) in &others {
                    if ui.selectable_label(s.background.as_deref() == Some(id), name).clicked() {
                        s.background = Some(id.clone());
                    }
                }
            })
            .response
            .on_hover_text("Draw another screen underneath this one (headers, menus you reuse)");
            ui.end_row();
            ui.label("Component");
            ui.checkbox(&mut s.is_component, "Reusable in other screens");
            ui.end_row();
        });
        ui.label(RichText::new("Notes").strong());
        ui.add(egui::TextEdit::multiline(&mut s.notes).hint_text("Comments, questions, what happens here…").desired_rows(4).desired_width(f32::INFINITY));
        ui.separator();
        ui.label(RichText::new("Project").strong());
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.proj.name);
        });
        ui.add_space(6.0);
        ui.weak("Tip: click a part to change it. Drag on empty space to select several.");
    }

    fn control_props(&mut self, ui: &mut egui::Ui) {
        let id = self.sel[0];
        let screens: Vec<(String, String, bool)> = self.proj.screens.iter().map(|s| (s.id.clone(), s.name.clone(), s.is_component)).collect();
        let here = self.screen().id.clone();
        let Some(idx) = self.screen().controls.iter().position(|c| c.id == id) else {
            self.sel.clear();
            return;
        };
        let mut open_component = None;
        let mut action: Option<&str> = None;
        let pic = self.proj.screens[self.cur].controls[idx].image.as_deref().and_then(|i| self.proj.images.get(i)).cloned();
        {
            let c = &mut self.proj.screens[self.cur].controls[idx];
            let kind = catalog::kind(&c.kind);
            ui.horizontal(|ui| {
                ui.heading(kind.map(|k| k.name).unwrap_or(&c.kind));
                if c.locked {
                    ui.label("🔒");
                }
            });
            egui::ComboBox::from_id_salt("kind").selected_text("Change to…").width(200.0).show_ui(ui, |ui| {
                for cat in catalog::CATS {
                    ui.label(RichText::new(*cat).weak());
                    for k in catalog::KINDS.iter().filter(|k| k.cat == *cat) {
                        if ui.selectable_label(c.kind == k.id, format!("   {}", k.name)).clicked() {
                            c.kind = k.id.to_string();
                        }
                    }
                }
            });
            let help = kind.map(|k| k.help).unwrap_or("");
            if c.kind == "picture" {
                ui.label(RichText::new("Picture").strong());
                match &pic {
                    Some(img) => {
                        ui.label(format!("{} · {}×{} px · {}", img.name, img.w, img.h, img.mime.trim_start_matches("image/").trim_end_matches("+xml").to_uppercase()));
                    }
                    None => {
                        ui.weak("No picture yet. Choose a file, or paste or drop one.");
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button(if pic.is_some() { "Replace…" } else { "Choose…" }).clicked() {
                        action = Some("pic_choose");
                    }
                    if let Some(img) = &pic {
                        if ui.button("Original size").on_hover_text("Size the part to the picture's own pixels").clicked() {
                            c.w = img.w;
                            c.h = img.h;
                        }
                        if ui.button("Match proportions").on_hover_text("Keep the width, change the height to fit the picture").clicked() {
                            c.h = (c.w * img.h / img.w).round().max(1.0);
                        }
                    }
                });
                if let Some(img) = &pic {
                    ui.horizontal_wrapped(|ui| {
                        if !img.is_svg() && ui.button("Make background transparent…").clicked() {
                            action = Some("pic_bg");
                        }
                        if ui.button("Copy picture").clicked() {
                            action = Some("pic_copy");
                        }
                        if ui.button("Save as…").clicked() {
                            action = Some("pic_save");
                        }
                    });
                }
                egui::ComboBox::from_id_salt("fit").selected_text(c.fit.label()).width(220.0).show_ui(ui, |ui| {
                    for f in crate::images::Fit::ALL {
                        ui.selectable_value(&mut c.fit, f, f.label());
                    }
                });
                ui.weak("Transparent PNG, GIF, WebP and SVG pictures stay transparent. Fill below colours the area behind the picture. Dragging a corner keeps the proportions; hold Shift to stretch.");
            } else {
                ui.label(RichText::new("Text").strong());
                let multiline = c.text.contains('\n') || help.contains("line") || c.kind == "paragraph";
                if multiline {
                    ui.add(egui::TextEdit::multiline(&mut c.text).desired_rows(4).desired_width(f32::INFINITY));
                } else {
                    ui.add(egui::TextEdit::singleline(&mut c.text).desired_width(f32::INFINITY));
                }
                if !help.is_empty() {
                    ui.weak(help);
                }
            }
            if c.kind == "icon" || c.kind == "icon_button" || c.kind == "fab" {
                egui::ComboBox::from_id_salt("icon").selected_text(c.text.clone()).width(200.0).height(300.0).show_ui(ui, |ui| {
                    for name in catalog::ICONS {
                        if ui.selectable_label(c.text == *name, *name).clicked() {
                            c.text = name.to_string();
                        }
                    }
                });
            }
            ui.separator();
            egui::Grid::new("ctl_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Position");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut c.x).prefix("x "));
                    ui.add(egui::DragValue::new(&mut c.y).prefix("y "));
                });
                ui.end_row();
                ui.label("Size");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut c.w).range(1.0..=8000.0).prefix("w "));
                    ui.add(egui::DragValue::new(&mut c.h).range(1.0..=8000.0).prefix("h "));
                });
                ui.end_row();
                ui.label("State");
                egui::ComboBox::from_id_salt("state").selected_text(c.state.label()).show_ui(ui, |ui| {
                    for s in State::ALL {
                        ui.selectable_value(&mut c.state, s, s.label());
                    }
                });
                ui.end_row();
                let uses_value = kind.map(|k| k.value != 0.0 || k.help.contains("Value")).unwrap_or(false);
                if uses_value {
                    ui.label("Value");
                    let max = match c.kind.as_str() {
                        "rating" => 5.0,
                        "date_picker" => 31.0,
                        "carousel" | "page_dots" => 7.0,
                        "arrow" => 1.0,
                        _ => 100.0,
                    };
                    ui.add(egui::Slider::new(&mut c.value, 0.0..=max).step_by(if max <= 31.0 { 1.0 } else { 0.0 }));
                    ui.end_row();
                }
                ui.label("Link to");
                let link_name = c.link.as_ref().and_then(|l| screens.iter().find(|s| &s.0 == l)).map(|s| s.1.clone()).unwrap_or_else(|| "Nothing".into());
                egui::ComboBox::from_id_salt("link").selected_text(link_name).width(170.0).show_ui(ui, |ui| {
                    if ui.selectable_label(c.link.is_none(), "Nothing").clicked() {
                        c.link = None;
                    }
                    for (sid, name, comp) in &screens {
                        if *comp || *sid == here {
                            continue;
                        }
                        if ui.selectable_label(c.link.as_deref() == Some(sid), name).clicked() {
                            c.link = Some(sid.clone());
                        }
                    }
                })
                .response
                .on_hover_text("In present mode and in the HTML prototype, clicking this opens that screen");
                ui.end_row();
                if c.kind == "component" {
                    ui.label("Component");
                    let cname = c.component.as_ref().and_then(|l| screens.iter().find(|s| &s.0 == l)).map(|s| s.1.clone()).unwrap_or_else(|| "Pick one".into());
                    egui::ComboBox::from_id_salt("component").selected_text(cname).width(170.0).show_ui(ui, |ui| {
                        for (sid, name, comp) in &screens {
                            if *comp && *sid != here && ui.selectable_label(c.component.as_deref() == Some(sid), name).clicked() {
                                c.component = Some(sid.clone());
                            }
                        }
                    });
                    ui.end_row();
                }
                ui.label("Layer name");
                let mut nm = c.name.clone().unwrap_or_default();
                if ui.add(egui::TextEdit::singleline(&mut nm).hint_text("optional").desired_width(170.0)).changed() {
                    c.name = if nm.is_empty() { None } else { Some(nm) };
                }
                ui.end_row();
            });
            if c.kind == "component" {
                if let Some(t) = &c.component {
                    if ui.button("✏ Edit this component").clicked() {
                        open_component = Some(t.clone());
                    }
                }
            }
            ui.separator();
            ui.label(RichText::new("Style").strong());
            ui.weak("Fill");
            swatch_row(ui, &mut c.fill, "fill_pick");
            ui.weak("Text and lines");
            swatch_row(ui, &mut c.color, "color_pick");
            ui.horizontal(|ui| {
                let mut fs = c.font_size.unwrap_or(0.0);
                if ui.add(egui::DragValue::new(&mut fs).range(0.0..=200.0).prefix("size ").custom_formatter(|v, _| if v == 0.0 { "auto".into() } else { format!("{v:.0}") })).changed() {
                    c.font_size = if fs <= 0.0 { None } else { Some(fs) };
                }
                ui.toggle_value(&mut c.bold, RichText::new("B").strong());
                ui.toggle_value(&mut c.italic, RichText::new("I").italics());
                ui.toggle_value(&mut c.underline, RichText::new("U").underline());
            });
            ui.horizontal(|ui| {
                ui.weak("Align");
                for (a, l) in [(Align::Auto, "Auto"), (Align::Left, "Left"), (Align::Center, "Center"), (Align::Right, "Right")] {
                    ui.selectable_value(&mut c.align, a, l);
                }
            });
            ui.horizontal(|ui| {
                ui.weak("Opacity");
                let mut o = c.opacity.unwrap_or(1.0);
                if ui.add(egui::Slider::new(&mut o, 0.05..=1.0).show_value(false)).changed() {
                    c.opacity = if o >= 0.999 { None } else { Some(o) };
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut c.locked, "Locked");
                ui.checkbox(&mut c.hidden, "Hidden");
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                for (label, a) in [("To front", "front"), ("Forward", "fwd"), ("Backward", "back"), ("To back", "bottom")] {
                    if ui.button(label).clicked() {
                        action = Some(a);
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Duplicate").clicked() {
                    action = Some("dup");
                }
                if ui.button("Delete").clicked() {
                    action = Some("del");
                }
                if ui.button("Center on screen").clicked() {
                    action = Some("center");
                }
            });
        }
        match action {
            Some("front") => self.arrange(2),
            Some("fwd") => self.arrange(1),
            Some("back") => self.arrange(-1),
            Some("bottom") => self.arrange(-2),
            Some("dup") => self.duplicate(),
            Some("pic_choose") => self.choose_picture_for(id),
            Some("pic_bg") => self.open_bg_tool(id),
            Some("pic_save") => self.save_picture_as(id),
            Some("pic_copy") => {
                let ctx = ui.ctx().clone();
                self.copy(&ctx);
            }
            Some("del") => self.delete_selection(),
            Some("center") => {
                let (w, h) = (self.screen().w, self.screen().h);
                let c = &mut self.proj.screens[self.cur].controls[idx];
                c.x = ((w - c.w) / 2.0).round();
                c.y = ((h - c.h) / 2.0).round();
            }
            _ => {}
        }
        if let Some(t) = open_component.and_then(|k| self.proj.screen_idx(&k)) {
            self.open_screen(t);
        }
    }

    fn multi_props(&mut self, ui: &mut egui::Ui) {
        ui.heading(format!("{} parts", self.sel.len()));
        ui.label(RichText::new("Line up").strong());
        let mut how: Option<&str> = None;
        ui.horizontal_wrapped(|ui| {
            for (l, h, tip) in [
                ("Left", "left", "Left edges"),
                ("Centre", "hcenter", "Centres"),
                ("Right", "right", "Right edges"),
                ("Top", "top", "Top edges"),
                ("Middle", "vcenter", "Middles"),
                ("Bottom", "bottom", "Bottom edges"),
            ] {
                if ui.button(l).on_hover_text(tip).clicked() {
                    how = Some(h);
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Space evenly across").clicked() {
                how = Some("dist_h");
            }
            if ui.button("Space evenly down").clicked() {
                how = Some("dist_v");
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Same width").clicked() {
                how = Some("same_w");
            }
            if ui.button("Same height").clicked() {
                how = Some("same_h");
            }
        });
        if let Some(h) = how {
            self.align(h);
        }
        ui.separator();
        ui.label(RichText::new("Fill for all").strong());
        let mut fill: Option<String> = self.selected().first().and_then(|c| c.fill.clone());
        let before = fill.clone();
        swatch_row(ui, &mut fill, "multi_fill");
        if fill != before {
            let sel = self.sel.clone();
            for c in self.screen_mut().controls.iter_mut().filter(|c| sel.contains(&c.id)) {
                c.fill = fill.clone();
            }
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Group").clicked() {
                self.group();
            }
            if ui.button("Ungroup").clicked() {
                self.ungroup();
            }
            if ui.button("Lock").clicked() {
                self.toggle_lock();
            }
            if ui.button("Duplicate").clicked() {
                let ctx = ui.ctx().clone();
                self.copy(&ctx);
                self.paste(None);
            }
            if ui.button("Delete").clicked() {
                self.delete_selection();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Make a component from these").on_hover_text("Moves them into a new component and leaves a copy that updates everywhere").clicked() {
                self.make_component();
            }
        });
    }

    fn align(&mut self, how: &str) {
        let sel = self.sel.clone();
        let cs: Vec<(u64, f32, f32, f32, f32)> = self.selected().iter().filter(|c| !c.locked).map(|c| (c.id, c.x, c.y, c.w, c.h)).collect();
        if cs.len() < 2 {
            return;
        }
        let x0 = cs.iter().map(|c| c.1).fold(f32::MAX, f32::min);
        let y0 = cs.iter().map(|c| c.2).fold(f32::MAX, f32::min);
        let x1 = cs.iter().map(|c| c.1 + c.3).fold(f32::MIN, f32::max);
        let y1 = cs.iter().map(|c| c.2 + c.4).fold(f32::MIN, f32::max);
        let first = cs[0];
        let mut order = cs.clone();
        let screen = &mut self.proj.screens[self.cur];
        fn get(screen: &mut crate::model::Screen, id: u64) -> Option<&mut crate::model::Control> { screen.controls.iter_mut().find(|c| c.id == id) }
        match how {
            "dist_h" | "dist_v" => {
                let horiz = how == "dist_h";
                order.sort_by(|a, b| if horiz { a.1.total_cmp(&b.1) } else { a.2.total_cmp(&b.2) });
                let total: f32 = order.iter().map(|c| if horiz { c.3 } else { c.4 }).sum();
                let span = if horiz { x1 - x0 } else { y1 - y0 };
                let gap = (span - total) / (order.len() - 1) as f32;
                let mut pos = if horiz { x0 } else { y0 };
                for c in &order {
                    if let Some(cc) = get(screen, c.0) {
                        if horiz {
                            cc.x = pos.round();
                            pos += cc.w + gap;
                        } else {
                            cc.y = pos.round();
                            pos += cc.h + gap;
                        }
                    }
                }
            }
            _ => {
                for c in screen.controls.iter_mut().filter(|c| sel.contains(&c.id) && !c.locked) {
                    match how {
                        "left" => c.x = x0,
                        "right" => c.x = x1 - c.w,
                        "hcenter" => c.x = ((x0 + x1) / 2.0 - c.w / 2.0).round(),
                        "top" => c.y = y0,
                        "bottom" => c.y = y1 - c.h,
                        "vcenter" => c.y = ((y0 + y1) / 2.0 - c.h / 2.0).round(),
                        "same_w" => c.w = first.3,
                        "same_h" => c.h = first.4,
                        _ => {}
                    }
                }
            }
        }
    }

    /// Turns the selected parts into a component and puts an instance in their place.
    fn make_component(&mut self) {
        let parts: Vec<crate::model::Control> = self.selected().into_iter().cloned().collect();
        if parts.is_empty() {
            return;
        }
        let x0 = parts.iter().map(|c| c.x).fold(f32::MAX, f32::min);
        let y0 = parts.iter().map(|c| c.y).fold(f32::MAX, f32::min);
        let x1 = parts.iter().map(|c| c.x + c.w).fold(f32::MIN, f32::max);
        let y1 = parts.iter().map(|c| c.y + c.h).fold(f32::MIN, f32::max);
        let n = self.proj.screens.iter().filter(|s| s.is_component).count() + 1;
        let cid = self.proj.add_screen(&format!("Component {n}"), "none");
        {
            let s = self.proj.screens.last_mut().unwrap();
            s.is_component = true;
            s.w = (x1 - x0).max(10.0);
            s.h = (y1 - y0).max(10.0);
            s.controls = parts
                .into_iter()
                .map(|mut c| {
                    c.x -= x0;
                    c.y -= y0;
                    c.group = None;
                    c
                })
                .collect();
        }
        let sel = self.sel.clone();
        self.screen_mut().controls.retain(|c| !sel.contains(&c.id));
        let mut inst = self.proj.make_control("component", x0, y0);
        inst.w = x1 - x0;
        inst.h = y1 - y0;
        inst.component = Some(cid);
        let iid = inst.id;
        self.screen_mut().controls.push(inst);
        self.sel = vec![iid];
        self.status = "Component created — double-click it to edit, every copy updates".into();
    }
}
