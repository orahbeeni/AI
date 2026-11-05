//! Adding your own pictures: file dialog, drag and drop, the clipboard, saving them out and the
//! "make background transparent" tool.

use crate::app::{App, Mode};
use crate::images::{self, BgParams, Img, ImgRef};
use crate::model::Control;
use eframe::egui::{self, Color32, Pos2, RichText, Sense, Vec2};
use std::collections::BTreeMap;
use std::path::Path;

/// What goes on the system clipboard when several parts are copied: the parts and their pictures,
/// so they can be pasted into another project or another window.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ClipPayload {
    pub sketchframe: u32,
    pub controls: Vec<Control>,
    #[serde(default, with = "images::store")]
    pub images: BTreeMap<String, ImgRef>,
}

pub struct BgTool {
    control: u64,
    img: ImgRef,
    size: (usize, usize),
    small: Vec<u8>,
    pub params: BgParams,
    shown: Option<BgParams>,
    tex: Option<egui::TextureHandle>,
}

fn clipboard_image() -> Option<(u32, u32, Vec<u8>)> {
    let img = arboard::Clipboard::new().ok()?.get_image().ok()?;
    Some((img.width as u32, img.height as u32, img.bytes.into_owned()))
}

fn pick_files(multiple: bool) -> Vec<std::path::PathBuf> {
    let d = rfd::FileDialog::new().add_filter("Pictures", images::FILE_EXTENSIONS).set_title("Choose a picture");
    if multiple {
        d.pick_files().unwrap_or_default()
    } else {
        d.pick_file().into_iter().collect()
    }
}

fn read_picture(path: &Path) -> Result<Img, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "picture".into());
    Img::from_bytes(&name, bytes)
}

impl App {
    fn screen_center(&self) -> Pos2 {
        self.to_model(self.canvas_rect.center())
    }

    /// Puts a new picture on the current screen, centred on `at` (or the middle of the view).
    pub fn place_picture(&mut self, img: Img, at: Option<Pos2>) {
        let c = at.unwrap_or_else(|| self.screen_center());
        let name = img.name.clone();
        let id = self.proj.add_picture(self.cur, img, Some((c.x, c.y)), None);
        self.sel = vec![id];
        self.status = format!("Added picture {name}");
    }

    /// File → Insert picture…
    pub fn insert_pictures_dialog(&mut self) {
        let files = pick_files(true);
        self.insert_picture_files(&files, None);
    }

    pub fn insert_picture_files(&mut self, files: &[std::path::PathBuf], at: Option<Pos2>) {
        let mut errors = vec![];
        let mut added = vec![];
        for (n, f) in files.iter().enumerate() {
            match read_picture(f) {
                Ok(img) => {
                    let c = at.unwrap_or_else(|| self.screen_center()) + Vec2::splat(24.0 * n as f32);
                    added.push(self.proj.add_picture(self.cur, img, Some((c.x, c.y)), None));
                }
                Err(e) => errors.push(e),
            }
        }
        if !added.is_empty() {
            self.status = format!("Added {} picture{}", added.len(), if added.len() == 1 { "" } else { "s" });
            self.sel = added;
        }
        if let Some(e) = errors.first() {
            self.status = e.clone();
        }
    }

    /// Chooses the picture of a "My picture" part (also: double-click it).
    pub fn choose_picture_for(&mut self, id: u64) {
        let Some(f) = pick_files(false).into_iter().next() else { return };
        match read_picture(&f) {
            Ok(img) => self.set_picture(id, img),
            Err(e) => self.status = e,
        }
    }

    /// Gives a part a picture. A part that had none takes the picture's own proportions.
    pub fn set_picture(&mut self, id: u64, img: Img) {
        let (iw, ih) = (img.w, img.h);
        let (sw, sh) = (self.screen().w, self.screen().h);
        let name = img.name.clone();
        let image = self.proj.add_image(img);
        if let Some(c) = self.proj.screens[self.cur].controls.iter_mut().find(|c| c.id == id) {
            c.name = Some(name);
            if c.image.is_none() {
                let s = (sw * 0.9 / iw).min(sh * 0.9 / ih).min(1.0);
                let (cx, cy) = (c.x + c.w / 2.0, c.y + c.h / 2.0);
                c.w = (iw * s).round().max(8.0);
                c.h = (ih * s).round().max(8.0);
                c.x = (cx - c.w / 2.0).round();
                c.y = (cy - c.h / 2.0).round();
            }
            c.image = Some(image);
            self.status = "Picture set".into();
        }
    }

    fn picture_of(&self, c: &Control) -> Option<ImgRef> {
        c.image.as_deref().and_then(|i| self.proj.images.get(i)).cloned()
    }

    // ------------------------------------------------------------ clipboard

    /// The selected picture as pixels for the system clipboard.
    fn clipboard_pixels(&self) -> Option<egui::ColorImage> {
        let c = self.selected().into_iter().next()?;
        let img = self.picture_of(c)?;
        img.color_image((img.w * 2.0).clamp(256.0, 2048.0) as u32)
    }

    /// Copy: parts go on the clipboard as JSON (pictures inside). A single picture goes as an image
    /// so other programs can paste it; Sketchframe still pastes it as the same part.
    pub fn copy(&mut self, ctx: &egui::Context) {
        self.clip = self.selected().into_iter().cloned().collect();
        if self.clip.is_empty() {
            return;
        }
        self.clip_images = self.clip.iter().filter_map(|c| c.image.as_ref()).filter_map(|id| self.proj.images.get(id).map(|i| (id.clone(), i.clone()))).collect();
        self.clip_image = None;
        if self.clip.len() == 1 && self.clip[0].kind == "picture" {
            if let Some(ci) = self.clipboard_pixels() {
                let bytes: Vec<u8> = ci.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
                self.clip_image = Some(images::fnv(&bytes));
                ctx.copy_image(ci);
                self.status = "Copied picture".into();
                return;
            }
        }
        let payload = ClipPayload { sketchframe: 1, controls: self.clip.clone(), images: self.clip_images.clone() };
        ctx.copy_text(serde_json::to_string(&payload).unwrap_or_default());
        self.status = format!("Copied {}", self.clip.len());
    }

    /// A picture on the system clipboard becomes a "My picture" part. Returns false when there is none.
    pub fn try_paste_image(&mut self) -> bool {
        let Some((w, h, rgba)) = clipboard_image() else { return false };
        self.image_pasted = true;
        let hash = images::fnv(&rgba);
        if self.clip_image == Some(hash) && !self.clip.is_empty() {
            let items = self.clip.clone();
            self.paste_items(items);
            return true;
        }
        match Img::from_rgba("Pasted picture", w, h, rgba) {
            Ok(img) => self.place_picture(img, None),
            Err(e) => self.status = format!("Can't paste that picture: {e}"),
        }
        true
    }

    /// Ctrl+V sends no event when only a picture is on the clipboard, so the key release is used.
    pub fn watch_paste_key(&mut self, ctx: &egui::Context, typing: bool) {
        let now = ctx.input(|i| i.time);
        if ctx.input(|i| i.modifiers.command) {
            self.last_cmd = now;
        }
        let released = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::V, pressed: false, .. })));
        if released && !typing && self.mode == Mode::Edit && now - self.last_cmd < 0.5 {
            if self.image_pasted {
                self.image_pasted = false;
            } else {
                self.try_paste_image();
            }
        }
    }

    // ------------------------------------------------------------ drag and drop from the desktop

    pub fn handle_drops(&mut self, ctx: &egui::Context) {
        if self.mode != Mode::Edit {
            return;
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop_hint")));
            let r = self.canvas_rect;
            p.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(64, 118, 204, 40));
            p.rect_stroke(r.shrink(6.0), 8.0, egui::Stroke::new(2.0, Color32::from_rgb(64, 118, 204)), egui::StrokeKind::Inside);
            p.text(r.center(), egui::Align2::CENTER_CENTER, "Drop pictures here", egui::FontId::proportional(22.0), Color32::from_rgb(64, 118, 204));
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        let pos = ctx.input(|i| i.pointer.latest_pos());
        let at = pos.filter(|p| self.canvas_rect.contains(*p)).map(|p| self.to_model(p));
        let over = at.and_then(|p| self.hit(p)).filter(|id| self.screen().controls.iter().any(|c| c.id == *id && c.kind == "picture"));
        let mut n = 0;
        for f in dropped {
            let loaded = read_picture(f.path());
            match loaded {
                // dropping onto a picture swaps its picture, anywhere else adds a new one
                Ok(img) if n == 0 && over.is_some() => self.set_picture(over.unwrap_or_default(), img),
                Ok(img) => {
                    let c = at.unwrap_or_else(|| self.screen_center()) + Vec2::splat(24.0 * n as f32);
                    let id = self.proj.add_picture(self.cur, img, Some((c.x, c.y)), None);
                    self.sel = vec![id];
                    self.status = "Added picture".into();
                }
                Err(e) => self.status = e,
            }
            n += 1;
        }
    }

    // ------------------------------------------------------------ saving a picture out

    pub fn save_picture_as(&mut self, id: u64) {
        let Some(img) = self.screen().controls.iter().find(|c| c.id == id).and_then(|c| self.picture_of(c)) else { return };
        let stem = Path::new(&img.name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "picture".into());
        let ext = img.extension();
        let Some(f) = rfd::FileDialog::new().set_file_name(format!("{stem}.{ext}")).add_filter(ext, &[ext]).save_file() else { return };
        self.status = match std::fs::write(&f, &img.bytes) {
            Ok(()) => format!("Saved {}", f.display()),
            Err(e) => format!("Can't save: {e}"),
        };
    }

    // ------------------------------------------------------------ make the background transparent

    pub fn open_bg_tool(&mut self, id: u64) {
        let Some(img) = self.screen().controls.iter().find(|c| c.id == id).and_then(|c| self.picture_of(c)) else { return };
        let Some((w, h, rgba)) = img.rgba() else {
            self.status = "SVG pictures are already transparent where nothing is drawn".into();
            return;
        };
        let (sw, sh, small) = images::thumbnail(w, h, &rgba, 420);
        let color = images::guess_background(sw, sh, &small);
        self.bg_tool = Some(BgTool { control: id, img, size: (sw, sh), small, params: BgParams { color, tolerance: 12.0, from_edges: true }, shown: None, tex: None });
    }

    pub fn bg_tool_window(&mut self, ctx: &egui::Context) {
        let Some(mut tool) = self.bg_tool.take() else { return };
        if tool.shown != Some(tool.params) {
            let mut px = tool.small.clone();
            images::remove_background(tool.size.0, tool.size.1, &mut px, &tool.params);
            let ci = egui::ColorImage::from_rgba_unmultiplied([tool.size.0, tool.size.1], &px);
            tool.tex = Some(ctx.load_texture("bg_preview", ci, egui::TextureOptions::LINEAR));
            tool.shown = Some(tool.params);
        }
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Make the background transparent").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label("Click the picture on the background colour to pick it, then raise the tolerance until the background is gone.");
            let (sw, sh) = (tool.size.0 as f32, tool.size.1 as f32);
            let scale = (420.0 / sw).min(320.0 / sh).max(0.1);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(sw * scale, sh * scale), Sense::click());
            let painter = ui.painter_at(rect);
            let cell = 10.0;
            let (mut y, mut row) = (rect.min.y, 0);
            while y < rect.max.y {
                let (mut x, mut col) = (rect.min.x, 0);
                while x < rect.max.x {
                    let c = if (row + col) % 2 == 0 { Color32::from_gray(235) } else { Color32::from_gray(200) };
                    painter.rect_filled(egui::Rect::from_min_size(Pos2::new(x, y), Vec2::splat(cell)).intersect(rect), 0.0, c);
                    x += cell;
                    col += 1;
                }
                y += cell;
                row += 1;
            }
            if let Some(t) = &tool.tex {
                painter.image(t.id(), rect, egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let (x, y) = (((p.x - rect.min.x) / scale) as usize, ((p.y - rect.min.y) / scale) as usize);
                    let i = (y.min(tool.size.1 - 1) * tool.size.0 + x.min(tool.size.0 - 1)) * 4;
                    tool.params.color = [tool.small[i], tool.small[i + 1], tool.small[i + 2]];
                }
            }
            let mut col = Color32::from_rgb(tool.params.color[0], tool.params.color[1], tool.params.color[2]);
            ui.horizontal(|ui| {
                ui.label("Background colour");
                if ui.color_edit_button_srgba(&mut col).changed() {
                    tool.params.color = [col.r(), col.g(), col.b()];
                }
            });
            ui.add(egui::Slider::new(&mut tool.params.tolerance, 0.0..=60.0).text("Tolerance"));
            ui.checkbox(&mut tool.params.from_edges, "Only remove what touches the edge").on_hover_text("Keeps the same colour where it is inside the subject, such as white eyes");
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Apply").strong()).clicked() {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if apply {
            if let Some((w, h, mut rgba)) = tool.img.rgba() {
                images::remove_background(w as usize, h as usize, &mut rgba, &tool.params);
                let stem = Path::new(&tool.img.name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                match Img::from_rgba(&format!("{stem} (transparent).png"), w, h, rgba) {
                    Ok(new) => {
                        let image = self.proj.add_image(new);
                        if let Some(c) = self.proj.screens[self.cur].controls.iter_mut().find(|c| c.id == tool.control) {
                            c.image = Some(image);
                        }
                        self.status = "Background made transparent".into();
                    }
                    Err(e) => self.status = e,
                }
            }
        } else if open && !cancel {
            self.bg_tool = Some(tool);
        }
    }
}

#[cfg(test)]
mod tests {
    /// Needs a desktop session with a clipboard: run with `cargo test clipboard -- --ignored`.
    #[test]
    #[ignore]
    fn clipboard_keeps_transparency() {
        let mut px = vec![0u8; 4 * 4 * 4];
        px[0..4].copy_from_slice(&[200, 30, 30, 255]);
        px[4..8].copy_from_slice(&[30, 30, 200, 128]);
        let mut cb = arboard::Clipboard::new().expect("clipboard");
        cb.set_image(arboard::ImageData { width: 4, height: 4, bytes: px.clone().into() }).expect("set");
        let (w, h, got) = super::clipboard_image().expect("get");
        assert_eq!((w, h), (4, 4));
        assert_eq!(got[3], 255);
        assert_eq!(got[7], 128, "alpha survives the clipboard");
        assert_eq!(got[11], 0, "transparent pixels stay transparent");
    }
}
