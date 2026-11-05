//! The drawing canvas, present mode and the flow map.

use crate::app::{App, Drag, Mode};
use crate::draw::{self, Env};
use crate::pen::{Pen, Prim, R};
use crate::render;
use eframe::egui::{self, Color32, CursorIcon, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

const SEL: Color32 = Color32::from_rgb(40, 120, 230);
const GUIDE: Color32 = Color32::from_rgb(230, 60, 140);

fn rect_of(r: R) -> Rect {
    Rect::from_min_size(Pos2::new(r.x, r.y), Vec2::new(r.w, r.h))
}

impl App {
    pub fn origin(&self) -> Pos2 {
        self.canvas_rect.min + self.pan
    }

    pub fn to_model(&self, p: Pos2) -> Pos2 {
        let o = self.origin();
        Pos2::new((p.x - o.x) / self.zoom, (p.y - o.y) / self.zoom)
    }

    pub fn to_screen(&self, r: R) -> Rect {
        let o = self.origin();
        Rect::from_min_size(Pos2::new(o.x + r.x * self.zoom, o.y + r.y * self.zoom), Vec2::new(r.w * self.zoom, r.h * self.zoom))
    }

    pub fn zoom_by(&mut self, k: f32, around: Option<Pos2>) {
        let at = around.unwrap_or_else(|| self.canvas_rect.center());
        let m = self.to_model(at);
        self.zoom = (self.zoom * k).clamp(0.05, 8.0);
        // keep the model point under `at`
        self.pan = at - self.canvas_rect.min - m.to_vec2() * self.zoom;
    }

    fn fit(&mut self) {
        let b = render::screen_bounds(&self.proj, self.cur, self.show_frame);
        let avail = self.canvas_rect.shrink(30.0);
        if avail.width() < 50.0 {
            return;
        }
        self.zoom = (avail.width() / b.w).min(avail.height() / b.h).clamp(0.05, 1.5);
        let center_model = Vec2::new(b.x + b.w / 2.0, b.y + b.h / 2.0);
        self.pan = self.canvas_rect.size() / 2.0 - center_model * self.zoom;
        self.fit_next = false;
    }

    pub fn hit(&self, p: Pos2) -> Option<u64> {
        self.screen().controls.iter().rev().find(|c| !c.hidden && !c.locked && c.contains(p.x, p.y)).map(|c| c.id)
    }

    fn handles(r: Rect) -> [Pos2; 8] {
        let (l, t, rr, b, cx, cy) = (r.min.x, r.min.y, r.max.x, r.max.y, r.center().x, r.center().y);
        [Pos2::new(l, t), Pos2::new(cx, t), Pos2::new(rr, t), Pos2::new(rr, cy), Pos2::new(rr, b), Pos2::new(cx, b), Pos2::new(l, b), Pos2::new(l, cy)]
    }

    fn sel_bbox(&self) -> Option<R> {
        let cs = self.selected();
        if cs.is_empty() {
            return None;
        }
        let x0 = cs.iter().map(|c| c.x).fold(f32::MAX, f32::min);
        let y0 = cs.iter().map(|c| c.y).fold(f32::MAX, f32::min);
        let x1 = cs.iter().map(|c| c.x + c.w).fold(f32::MIN, f32::max);
        let y1 = cs.iter().map(|c| c.y + c.h).fold(f32::MIN, f32::max);
        Some(R::new(x0, y0, x1 - x0, y1 - y0))
    }

    pub fn canvas(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.canvas_rect = rect;
        if self.fit_next {
            self.fit();
        }
        let ctx = ui.ctx().clone();
        let painter = ui.painter_at(rect);
        let bg = if self.dark_ui { Color32::from_gray(38) } else { Color32::from_rgb(236, 235, 232) };
        painter.rect_filled(rect, 0.0, bg);

        // ---- zoom and pan
        let (scroll, zdelta, space, hover) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.key_down(egui::Key::Space), i.pointer.hover_pos()));
        if resp.hovered() || resp.contains_pointer() {
            if zdelta != 1.0 {
                self.zoom_by(zdelta, hover);
            } else if scroll != Vec2::ZERO {
                self.pan += scroll;
            }
        }
        let panning = resp.dragged_by(PointerButton::Middle) || (space && resp.dragged_by(PointerButton::Primary));
        if panning {
            self.pan += resp.drag_delta();
            ctx.set_cursor_icon(CursorIcon::Grabbing);
        }

        // ---- draw the screen
        let origin = self.origin();
        let m = render::egui_measure(&ctx, self.sketch);
        let mut pen = Pen::new(self.sketch, false, &m);
        {
            let env = Env { project: &self.proj, depth: 0 };
            draw::screen(&mut pen, &env, self.cur, self.show_frame);
        }
        render::paint(&painter, &pen.out, origin, self.zoom, self.sketch);
        let s = self.screen().clone();
        let art = self.to_screen(R::new(0.0, 0.0, s.w, s.h));
        if self.grid && self.grid_size * self.zoom >= 5.0 {
            let col = Color32::from_rgba_unmultiplied(60, 120, 220, 28);
            let mut x = 0.0;
            while x <= s.w {
                let sx = origin.x + x * self.zoom;
                painter.line_segment([Pos2::new(sx, art.min.y), Pos2::new(sx, art.max.y)], Stroke::new(1.0, col));
                x += self.grid_size;
            }
            let mut y = 0.0;
            while y <= s.h {
                let sy = origin.y + y * self.zoom;
                painter.line_segment([Pos2::new(art.min.x, sy), Pos2::new(art.max.x, sy)], Stroke::new(1.0, col));
                y += self.grid_size;
            }
        }
        // screen name above the artboard
        let title_y = render::screen_bounds(&self.proj, self.cur, self.show_frame).y + 18.0;
        painter.text(
            Pos2::new(art.min.x, origin.y + title_y * self.zoom - 6.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{}{}", s.name, if s.is_component { "  (component)" } else { "" }),
            egui::FontId::proportional(13.0),
            if self.dark_ui { Color32::from_gray(170) } else { Color32::from_gray(110) },
        );

        // link markers
        for c in s.controls.iter().filter(|c| c.link.is_some() && !c.hidden) {
            let r = self.to_screen(R::new(c.x, c.y, c.w, c.h));
            let p = Pos2::new(r.max.x - 2.0, r.min.y + 2.0);
            painter.circle_filled(p, 8.0, SEL);
            painter.text(p, egui::Align2::CENTER_CENTER, "🔗", egui::FontId::proportional(9.0), Color32::WHITE);
        }

        // ---- interaction
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let shift = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        let alt = ui.input(|i| i.modifiers.alt);
        let single = if self.sel.len() == 1 { self.selected().first().map(|c| (c.id, R::new(c.x, c.y, c.w, c.h), c.locked)) } else { None };
        let handle_at = |app: &App, p: Pos2| -> Option<usize> {
            let (_, r, locked) = single?;
            if locked {
                return None;
            }
            let sr = app.to_screen(r);
            Self::handles(sr).iter().position(|h| h.distance(p) < 7.0)
        };

        if let (Some(p), Drag::None) = (hover, &self.drag) {
            if rect.contains(p) && !panning {
                if let Some(h) = handle_at(self, p) {
                    ctx.set_cursor_icon(match h {
                        0 | 4 => CursorIcon::ResizeNwSe,
                        2 | 6 => CursorIcon::ResizeNeSw,
                        1 | 5 => CursorIcon::ResizeVertical,
                        _ => CursorIcon::ResizeHorizontal,
                    });
                } else if let Some(id) = self.hit(self.to_model(p)) {
                    let c = self.screen().controls.iter().find(|c| c.id == id).unwrap();
                    painter.rect_stroke(self.to_screen(R::new(c.x, c.y, c.w, c.h)), 2.0, Stroke::new(1.0, SEL.gamma_multiply(0.6)), StrokeKind::Outside);
                }
            }
        }

        if resp.drag_started_by(PointerButton::Primary) && !space {
            let start_screen = ui.input(|i| i.pointer.press_origin()).unwrap_or(pointer.unwrap_or_default());
            let start = self.to_model(start_screen);
            self.editing = None;
            if let (Some(h), Some((id, r, _))) = (handle_at(self, start_screen), single) {
                self.drag = Drag::Resize { id, handle: h, orig: r, start };
            } else if let Some(id) = self.hit(start) {
                if !self.sel.contains(&id) {
                    let ids = self.with_group(id);
                    if shift {
                        self.sel.extend(ids);
                    } else {
                        self.sel = ids;
                    }
                }
                let orig: Vec<(u64, f32, f32)> = self.selected().iter().filter(|c| !c.locked).map(|c| (c.id, c.x, c.y)).collect();
                let bbox = self.sel_bbox().unwrap_or(R::new(start.x, start.y, 0.0, 0.0));
                self.drag = Drag::Move { start, orig, bbox };
            } else {
                self.drag = Drag::Marquee { start, add: shift };
            }
        }

        let cur_model = pointer.map(|p| self.to_model(p));
        self.guides.clear();
        if resp.dragged_by(PointerButton::Primary) && !space {
            if let Some(p) = cur_model {
                match &self.drag {
                    Drag::Move { start, orig, bbox } => {
                        let (orig, bbox) = (orig.clone(), *bbox);
                        let mut d = p - *start;
                        if self.snap && !alt {
                            d = self.snap_move(bbox, d);
                        }
                        let cs = &mut self.proj.screens[self.cur].controls;
                        for (id, x, y) in orig {
                            if let Some(c) = cs.iter_mut().find(|c| c.id == id) {
                                c.x = (x + d.x).round();
                                c.y = (y + d.y).round();
                            }
                        }
                    }
                    Drag::Resize { id, handle, orig, start } => {
                        let (id, h, o) = (*id, *handle, *orig);
                        let d = p - *start;
                        let (mut x0, mut y0, mut x1, mut y1) = (o.x, o.y, o.r(), o.b());
                        if matches!(h, 0 | 6 | 7) {
                            x0 += d.x;
                        }
                        if matches!(h, 2 | 3 | 4) {
                            x1 += d.x;
                        }
                        if matches!(h, 0 | 1 | 2) {
                            y0 += d.y;
                        }
                        if matches!(h, 4 | 5 | 6) {
                            y1 += d.y;
                        }
                        if self.snap && !alt {
                            let g = self.grid_size;
                            let sn = |v: f32| (v / g).round() * g;
                            if matches!(h, 0 | 6 | 7) {
                                x0 = sn(x0);
                            }
                            if matches!(h, 2 | 3 | 4) {
                                x1 = sn(x1);
                            }
                            if matches!(h, 0 | 1 | 2) {
                                y0 = sn(y0);
                            }
                            if matches!(h, 4 | 5 | 6) {
                                y1 = sn(y1);
                            }
                        }
                        if x1 - x0 < 4.0 {
                            if matches!(h, 0 | 6 | 7) {
                                x0 = x1 - 4.0;
                            } else {
                                x1 = x0 + 4.0;
                            }
                        }
                        if y1 - y0 < 4.0 {
                            if matches!(h, 0 | 1 | 2) {
                                y0 = y1 - 4.0;
                            } else {
                                y1 = y0 + 4.0;
                            }
                        }
                        let keep_ratio = !shift && matches!(h, 0 | 2 | 4 | 6) && o.h > 0.0 && self.screen().controls.iter().any(|c| c.id == id && c.kind == "picture" && c.fit == crate::images::Fit::Stretch);
                        if keep_ratio {
                            // pictures keep their proportions when dragged by a corner (hold Shift to stretch)
                            let s = ((x1 - x0) / o.w).max((y1 - y0) / o.h).max(4.0 / o.w.min(o.h));
                            let (nw, nh) = (o.w * s, o.h * s);
                            if matches!(h, 0 | 6) {
                                x0 = x1 - nw;
                            } else {
                                x1 = x0 + nw;
                            }
                            if matches!(h, 0 | 2) {
                                y0 = y1 - nh;
                            } else {
                                y1 = y0 + nh;
                            }
                        }
                        if let Some(c) = self.proj.screens[self.cur].controls.iter_mut().find(|c| c.id == id) {
                            c.x = x0.round();
                            c.y = y0.round();
                            c.w = (x1 - x0).round();
                            c.h = (y1 - y0).round();
                        }
                    }
                    Drag::Marquee { start, .. } => {
                        let r = Rect::from_two_pos(*start, p);
                        let sr = Rect::from_min_max(self.to_screen(R::new(r.min.x, r.min.y, 0.0, 0.0)).min, self.to_screen(R::new(r.max.x, r.max.y, 0.0, 0.0)).min);
                        painter.rect_filled(sr, 0.0, SEL.gamma_multiply(0.08));
                        painter.rect_stroke(sr, 0.0, Stroke::new(1.0, SEL), StrokeKind::Inside);
                    }
                    Drag::None => {}
                }
            }
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            if let (Drag::Marquee { start, add }, Some(p)) = (&self.drag, cur_model) {
                let r = Rect::from_two_pos(*start, p);
                let hits: Vec<u64> = self
                    .screen()
                    .controls
                    .iter()
                    .filter(|c| !c.hidden && !c.locked && r.intersects(rect_of(R::new(c.x, c.y, c.w, c.h))))
                    .map(|c| c.id)
                    .collect();
                if *add {
                    for h in hits {
                        if !self.sel.contains(&h) {
                            self.sel.push(h);
                        }
                    }
                } else {
                    self.sel = hits;
                }
            }
            self.drag = Drag::None;
        }
        if resp.clicked() {
            if let Some(p) = cur_model {
                match self.hit(p) {
                    Some(id) => {
                        let ids = self.with_group(id);
                        if shift {
                            if self.sel.contains(&id) {
                                self.sel.retain(|x| !ids.contains(x));
                            } else {
                                self.sel.extend(ids);
                            }
                        } else {
                            self.sel = ids;
                        }
                    }
                    None => {
                        if !shift {
                            self.sel.clear();
                        }
                    }
                }
                self.editing = None;
            }
        }
        if resp.double_clicked() {
            if let Some(id) = cur_model.and_then(|p| self.hit(p)) {
                if self.screen().controls.iter().any(|c| c.id == id && c.kind == "picture") {
                    self.sel = vec![id];
                    self.choose_picture_for(id);
                    return;
                }
                let comp = self.screen().controls.iter().find(|c| c.id == id).and_then(|c| c.component.clone());
                if let Some(target) = comp.and_then(|k| self.proj.screen_idx(&k)) {
                    self.open_screen(target);
                    self.status = "Editing the component — every copy updates".into();
                } else {
                    self.sel = vec![id];
                    self.editing = Some(id);
                }
            }
        }
        resp.context_menu(|ui| {
            if self.sel.is_empty() {
                if ui.button("Paste").clicked() {
                    self.paste(None);
                    ui.close();
                }
                if ui.button("Insert picture…").clicked() {
                    self.insert_pictures_dialog();
                    ui.close();
                }
                if ui.button("Select all").clicked() {
                    self.sel = self.screen().controls.iter().filter(|c| !c.locked).map(|c| c.id).collect();
                    ui.close();
                }
                return;
            }
            let pic = self.selected().first().filter(|c| c.kind == "picture").map(|c| c.id);
            if let Some(id) = pic {
                if ui.button("Replace picture…").clicked() {
                    self.choose_picture_for(id);
                    ui.close();
                }
                if ui.button("Make background transparent…").clicked() {
                    self.open_bg_tool(id);
                    ui.close();
                }
                if ui.button("Save picture as…").clicked() {
                    self.save_picture_as(id);
                    ui.close();
                }
            } else if ui.button("Edit text").clicked() {
                self.editing = self.sel.first().copied();
                ui.close();
            }
            if ui.button("Duplicate").clicked() {
                self.duplicate();
                ui.close();
            }
            if ui.button("Copy").clicked() {
                let ctx = ui.ctx().clone();
                self.copy(&ctx);
                ui.close();
            }
            ui.separator();
            for (label, how) in [("Bring to front", 2), ("Bring forward", 1), ("Send backward", -1), ("Send to back", -2)] {
                if ui.button(label).clicked() {
                    self.arrange(how);
                    ui.close();
                }
            }
            ui.separator();
            if self.sel.len() > 1 && ui.button("Group").clicked() {
                self.group();
                ui.close();
            }
            if ui.button("Ungroup").clicked() {
                self.ungroup();
                ui.close();
            }
            if ui.button("Lock / unlock").clicked() {
                self.toggle_lock();
                ui.close();
            }
            ui.separator();
            if ui.button("Delete").clicked() {
                self.delete_selection();
                ui.close();
            }
        });

        // ---- drop from the parts library
        if let Some(kind) = self.lib_drag.clone() {
            let released = ui.input(|i| i.pointer.any_released());
            if let Some(p) = hover {
                let lp = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("lib_ghost"));
                let gp = ctx.layer_painter(lp);
                let k = crate::catalog::kind(&kind);
                let size = k.map(|k| Vec2::new(k.w, k.h)).unwrap_or(Vec2::splat(60.0)) * self.zoom;
                let gr = Rect::from_center_size(p, size);
                gp.rect_filled(gr, 4.0, SEL.gamma_multiply(0.12));
                gp.rect_stroke(gr, 4.0, Stroke::new(1.5, SEL), StrokeKind::Inside);
                gp.text(gr.center(), egui::Align2::CENTER_CENTER, k.map(|k| k.name).unwrap_or(""), egui::FontId::proportional(12.0), SEL);
                ctx.set_cursor_icon(CursorIcon::Grabbing);
                if released {
                    if rect.contains(p) {
                        let m = self.to_model(p);
                        self.add_control(&kind, Some(m));
                    }
                    self.lib_drag = None;
                }
            }
            if released {
                self.lib_drag = None;
            }
        }

        // ---- selection overlay
        for c in self.selected() {
            let r = self.to_screen(R::new(c.x, c.y, c.w, c.h));
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, if c.locked { Color32::GRAY } else { SEL }), StrokeKind::Outside);
        }
        if let Some((_, r, locked)) = single {
            let sr = self.to_screen(r);
            if !locked {
                for h in Self::handles(sr) {
                    painter.rect(Rect::from_center_size(h, Vec2::splat(8.0)), 1.5, Color32::WHITE, Stroke::new(1.5, SEL), StrokeKind::Middle);
                }
            }
            if sr.width() > 50.0 {
                painter.text(
                    Pos2::new(sr.center().x, sr.max.y + 12.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{:.0} × {:.0}", r.w, r.h),
                    egui::FontId::proportional(10.5),
                    SEL,
                );
            }
        } else if self.sel.len() > 1 {
            if let Some(b) = self.sel_bbox() {
                painter.rect_stroke(self.to_screen(b), 0.0, Stroke::new(1.0, SEL.gamma_multiply(0.6)), StrokeKind::Outside);
            }
        }
        let o = self.origin();
        for (vertical, v) in self.guides.clone() {
            if vertical {
                let x = o.x + v * self.zoom;
                painter.line_segment([Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)], Stroke::new(1.0, GUIDE));
            } else {
                let y = o.y + v * self.zoom;
                painter.line_segment([Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)], Stroke::new(1.0, GUIDE));
            }
        }

        // ---- inline text editing
        if let Some(id) = self.editing {
            if let Some(c) = self.screen().controls.iter().find(|c| c.id == id).cloned() {
                let r = self.to_screen(R::new(c.x, c.y, c.w, c.h));
                let w = r.width().max(220.0);
                let multiline = c.text.contains('\n') || c.h > 60.0;
                let mut text = c.text.clone();
                let area = egui::Area::new(egui::Id::new("inline_edit")).order(egui::Order::Foreground).fixed_pos(r.min).show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(w);
                        let te = if multiline {
                            egui::TextEdit::multiline(&mut text).desired_rows(((c.h / 22.0) as usize).clamp(2, 14)).desired_width(w)
                        } else {
                            egui::TextEdit::singleline(&mut text).desired_width(w)
                        };
                        let r = ui.add(te.id(egui::Id::new(("edit", id))));
                        if !r.has_focus() && !r.lost_focus() {
                            r.request_focus();
                        }
                        ui.weak(if multiline { "Esc or click outside when done" } else { "Enter when done" });
                        r
                    })
                    .inner
                });
                let te = area.inner;
                if text != c.text {
                    if let Some(cc) = self.screen_mut().controls.iter_mut().find(|x| x.id == id) {
                        cc.text = text;
                    }
                }
                if te.lost_focus() {
                    self.editing = None;
                }
            } else {
                self.editing = None;
            }
        }

        if self.screen().controls.is_empty() {
            painter.text(
                art.center(),
                egui::Align2::CENTER_CENTER,
                "Click or drag parts from the strip above,\nor press ⌘K and type what you need.",
                egui::FontId::proportional(15.0),
                Color32::from_gray(150),
            );
        }
    }

    /// Snaps a move so the selection lines up with other parts, the screen, or the grid.
    fn snap_move(&mut self, b: R, d: Vec2) -> Vec2 {
        let tol = 6.0 / self.zoom;
        let s = self.screen();
        let mut xs = vec![0.0, s.w / 2.0, s.w];
        let mut ys = vec![0.0, s.h / 2.0, s.h];
        for c in s.controls.iter().filter(|c| !self.sel.contains(&c.id) && !c.hidden) {
            xs.extend([c.x, c.x + c.w / 2.0, c.x + c.w]);
            ys.extend([c.y, c.y + c.h / 2.0, c.y + c.h]);
        }
        let mut out = d;
        let mut guides = vec![];
        let best = |edges: [f32; 3], cands: &[f32]| -> Option<(f32, f32)> {
            let mut best: Option<(f32, f32)> = None;
            for e in edges {
                for c in cands {
                    let diff = c - e;
                    if diff.abs() < tol && best.map(|(bd, _)| diff.abs() < bd.abs()).unwrap_or(true) {
                        best = Some((diff, *c));
                    }
                }
            }
            best
        };
        let nx = b.x + d.x;
        let ny = b.y + d.y;
        let g = self.grid_size;
        match best([nx, nx + b.w / 2.0, nx + b.w], &xs) {
            Some((diff, line)) => {
                out.x += diff;
                guides.push((true, line));
            }
            None => out.x = (nx / g).round() * g - b.x,
        }
        match best([ny, ny + b.h / 2.0, ny + b.h], &ys) {
            Some((diff, line)) => {
                out.y += diff;
                guides.push((false, line));
            }
            None => out.y = (ny / g).round() * g - b.y,
        }
        self.guides = guides;
        out
    }

    // ------------------------------------------------------------ present mode

    pub fn present(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(28));
        if self.proj.screens[self.cur].is_component {
            if let Some(i) = self.proj.screens.iter().position(|s| !s.is_component) {
                self.cur = i;
            }
        }
        let ctx = ui.ctx().clone();
        let b = render::screen_bounds(&self.proj, self.cur, true);
        let top = 44.0;
        let avail = Rect::from_min_max(rect.min + Vec2::new(20.0, top), rect.max - Vec2::new(20.0, 20.0));
        let z = (avail.width() / b.w).min(avail.height() / b.h).min(1.5);
        let origin = Pos2::new(avail.center().x - (b.x + b.w / 2.0) * z, avail.center().y - (b.y + b.h / 2.0) * z);
        let m = render::egui_measure(&ctx, self.sketch);
        let mut pen = Pen::new(self.sketch, false, &m);
        {
            let env = Env { project: &self.proj, depth: 0 };
            draw::screen(&mut pen, &env, self.cur, true);
        }
        render::paint(&painter, &pen.out, origin, z, self.sketch);
        let hover = ui.input(|i| i.pointer.hover_pos());
        let mut go = None;
        let links: Vec<(R, String)> = pen.out.iter().filter_map(|p| if let Prim::Link { r, target } = p { Some((*r, target.clone())) } else { None }).collect();
        let show_all = ui.input(|i| i.modifiers.shift);
        for (r, target) in links.iter().rev() {
            let sr = Rect::from_min_size(Pos2::new(origin.x + r.x * z, origin.y + r.y * z), Vec2::new(r.w * z, r.h * z));
            let over = hover.map(|h| sr.contains(h)).unwrap_or(false);
            if over || show_all {
                painter.rect_filled(sr, 4.0, SEL.gamma_multiply(0.18));
                painter.rect_stroke(sr, 4.0, Stroke::new(1.5, SEL), StrokeKind::Outside);
            }
            if over {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
                if resp.clicked() && go.is_none() {
                    go = self.proj.screen_idx(target);
                }
            }
        }
        if resp.clicked() && go.is_none() && hover.is_some() {
            self.status = "Nothing to click there — hold Shift to see the links".into();
        }
        if let Some(i) = go {
            self.history.push(self.cur);
            self.cur = i;
        }
        // top bar
        let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 36.0));
        painter.rect_filled(bar, 0.0, Color32::from_gray(20));
        let mut ui2 = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(Vec2::new(10.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        ui2.style_mut().visuals = egui::Visuals::dark();
        if ui2.add_enabled(!self.history.is_empty(), egui::Button::new("Back")).clicked() {
            if let Some(p) = self.history.pop() {
                self.cur = p;
            }
        }
        ui2.label(egui::RichText::new(&self.proj.screens[self.cur].name).strong());
        ui2.weak("Click linked parts to move between screens · Shift shows the links · Esc to stop");
        ui2.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Stop presenting").clicked() {
                self.mode = Mode::Edit;
            }
        });
        let notes = self.proj.screens[self.cur].notes.clone();
        if !notes.is_empty() {
            egui::Area::new(egui::Id::new("present_notes")).fixed_pos(rect.right_bottom() - Vec2::new(300.0, 120.0)).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(Color32::from_rgb(255, 246, 180)).show(ui, |ui| {
                    ui.set_max_width(270.0);
                    ui.label(egui::RichText::new(notes).color(Color32::from_gray(40)));
                });
            });
        }
    }

    // ------------------------------------------------------------ flow map

    pub fn flow(&mut self, ui: &mut egui::Ui) {
        let (rect, _resp) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        let painter = ui.painter_at(rect);
        let bg = if self.dark_ui { Color32::from_gray(38) } else { Color32::from_rgb(236, 235, 232) };
        painter.rect_filled(rect, 0.0, bg);
        let ctx = ui.ctx().clone();
        let idx: Vec<usize> = (0..self.proj.screens.len()).filter(|i| !self.proj.screens[*i].is_component).collect();
        let n = idx.len().max(1);
        let cols = ((n as f32).sqrt().ceil() as usize).max(1).min(6);
        let rows = n.div_ceil(cols);
        let cell = Vec2::new((rect.width() - 40.0) / cols as f32, (rect.height() - 60.0) / rows as f32);
        let mut boxes = std::collections::HashMap::new();
        let m = render::egui_measure(&ctx, self.sketch);
        let text_col = if self.dark_ui { Color32::from_gray(220) } else { Color32::from_gray(50) };
        painter.text(rect.min + Vec2::new(20.0, 18.0), egui::Align2::LEFT_CENTER, "Flow map — every screen and the links between them. Click a screen to edit it.", egui::FontId::proportional(14.0), text_col);
        for (k, &i) in idx.iter().enumerate() {
            let (cx, cy) = (k % cols, k / cols);
            let cr = Rect::from_min_size(rect.min + Vec2::new(20.0 + cx as f32 * cell.x, 40.0 + cy as f32 * cell.y), cell).shrink(28.0);
            let s = &self.proj.screens[i];
            let z = (cr.width() / s.w).min((cr.height() - 22.0) / s.h);
            let sz = Vec2::new(s.w * z, s.h * z);
            let tr = Rect::from_min_size(Pos2::new(cr.center().x - sz.x / 2.0, cr.min.y), sz);
            painter.rect_filled(tr.expand(3.0), 6.0, Color32::from_black_alpha(25));
            let mut pen = Pen::new(self.sketch, false, &m);
            draw::thumbnail(&mut pen, &self.proj, i, R::new(tr.min.x, tr.min.y, tr.width(), tr.height()));
            render::paint(&painter, &pen.out, Pos2::ZERO, 1.0, self.sketch);
            painter.text(Pos2::new(tr.center().x, tr.max.y + 14.0), egui::Align2::CENTER_CENTER, &s.name, egui::FontId::proportional(13.0), text_col);
            let sid = s.id.clone();
            let resp = ui.interact(tr, egui::Id::new(("flow", i)), Sense::click());
            if resp.hovered() {
                painter.rect_stroke(tr.expand(2.0), 4.0, Stroke::new(2.0, SEL), StrokeKind::Outside);
            }
            if resp.clicked() {
                self.open_screen(i);
                self.mode = Mode::Edit;
            }
            boxes.insert(sid, tr);
        }
        // arrows for links
        let mut drawn = std::collections::HashSet::new();
        for s in &self.proj.screens {
            let Some(from) = boxes.get(&s.id) else { continue };
            let mut targets: Vec<String> = s.controls.iter().filter_map(|c| c.link.clone()).collect();
            if let Some(bg) = &s.background {
                targets.extend(self.proj.screen(bg).map(|b| b.controls.iter().filter_map(|c| c.link.clone()).collect::<Vec<_>>()).unwrap_or_default());
            }
            for t in targets {
                let Some(tid) = self.proj.screen(&t).map(|x| x.id.clone()) else { continue };
                if tid == s.id || !drawn.insert((s.id.clone(), tid.clone())) {
                    continue;
                }
                let Some(to) = boxes.get(&tid) else { continue };
                let (a, b) = edge_points(*from, *to);
                let col = SEL.gamma_multiply(0.85);
                let mid = Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0 - 24.0);
                let pts: Vec<Pos2> = (0..=20)
                    .map(|k| {
                        let t = k as f32 / 20.0;
                        let u = 1.0 - t;
                        Pos2::new(u * u * a.x + 2.0 * u * t * mid.x + t * t * b.x, u * u * a.y + 2.0 * u * t * mid.y + t * t * b.y)
                    })
                    .collect();
                painter.add(egui::Shape::line(pts.clone(), Stroke::new(2.0, col)));
                let dir = (b - pts[18]).normalized();
                let perp = Vec2::new(-dir.y, dir.x);
                painter.add(egui::Shape::convex_polygon(vec![b, b - dir * 11.0 + perp * 5.5, b - dir * 11.0 - perp * 5.5], col, Stroke::NONE));
            }
        }
    }
}

fn edge_points(a: Rect, b: Rect) -> (Pos2, Pos2) {
    let d = b.center() - a.center();
    let pick = |r: Rect, d: Vec2| -> Pos2 {
        if d.x.abs() * r.height() > d.y.abs() * r.width() {
            Pos2::new(if d.x > 0.0 { r.max.x + 4.0 } else { r.min.x - 4.0 }, r.center().y)
        } else {
            Pos2::new(r.center().x, if d.y > 0.0 { r.max.y + 4.0 } else { r.min.y - 4.0 })
        }
    };
    (pick(a, d), pick(b, -d))
}
