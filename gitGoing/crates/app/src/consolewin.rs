//! The command console: a separate, terminal-looking window that lists the git
//! commands the app runs, as they happen.

use eframe::egui::{self, text::LayoutJob, Color32, FontId, Sense, TextFormat, Vec2};
use gitgoing_core::gitlog::{self, Kind, Line};

const KEEP: usize = 5000;

pub struct Console {
    open: bool,
    lines: Vec<Line>,
    last_id: u64,
    filter: String,
    show_reads: bool,
    follow: bool,
}

impl Default for Console {
    fn default() -> Self {
        Console { open: false, lines: Vec::new(), last_id: 0, filter: String::new(), show_reads: true, follow: true }
    }
}

const BG: Color32 = Color32::from_rgb(0x0c, 0x0c, 0x0c);
const TIME: Color32 = Color32::from_rgb(0x6b, 0x6b, 0x6b);
const PROMPT: Color32 = Color32::from_rgb(0x4e, 0xc9, 0x6b);
const CMD: Color32 = Color32::from_rgb(0xf2, 0xf2, 0xf2);
const EQUIV: Color32 = Color32::from_rgb(0x9c, 0xdc, 0xfe);
const READ: Color32 = Color32::from_rgb(0x8f, 0x9a, 0xa5);
const OUT: Color32 = Color32::from_rgb(0xc8, 0xc8, 0xc8);
const ERR: Color32 = Color32::from_rgb(0xf4, 0x71, 0x67);
const DONE: Color32 = Color32::from_rgb(0x6a, 0x99, 0x55);
const NOTE: Color32 = Color32::from_rgb(0x7a, 0x86, 0x92);

impl Console {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        gitlog::set_live(self.open);
        if self.open {
            self.lines.clear();
            self.last_id = 0;
            self.pull();
        }
    }

    /// Take in whatever the log has gained since the last frame.
    fn pull(&mut self) {
        for l in gitlog::since(self.last_id) {
            self.last_id = l.id;
            self.lines.push(l);
        }
        if self.lines.len() > KEEP {
            self.lines.drain(..self.lines.len() - KEEP);
        }
    }

    /// Indices of the lines that pass the filters.
    fn visible(&self) -> Vec<usize> {
        let f = self.filter.trim().to_lowercase();
        (0..self.lines.len())
            .filter(|&i| {
                let l = &self.lines[i];
                (self.show_reads || l.kind != Kind::Read)
                    && (f.is_empty() || l.text.to_lowercase().contains(&f) || l.repo.to_lowercase().contains(&f))
            })
            .collect()
    }

    fn plain(l: &Line) -> String {
        let t = format!("[{:>9.3}] ", l.at);
        match l.kind {
            Kind::Cmd | Kind::Equiv | Kind::Read => format!("{t}{}{}", if l.repo.is_empty() { String::new() } else { format!("{} $ ", l.repo) }, l.text),
            _ => format!("{t}  {}", l.text),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        self.pull();
        let id = egui::ViewportId::from_hash_of("gitgoing-console");
        let builder = egui::ViewportBuilder::default()
            .with_title("gitgoing: git command console")
            .with_inner_size([940.0, 520.0])
            .with_min_inner_size([480.0, 240.0]);
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |ctx, class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                close = true;
            }
            if class == egui::ViewportClass::Embedded {
                egui::Window::new("Git command console").default_size([860.0, 460.0]).show(ctx, |ui| self.content(ui));
            } else {
                egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ctx, |ui| self.content(ui));
            }
        });
        if close {
            self.toggle();
        }
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        let font = FontId::monospace(13.0);
        let row_h = ui.fonts(|f| f.row_height(&font)) + 3.0;
        let before = self.visible();

        egui::TopBottomPanel::top("console_bar")
            .frame(egui::Frame::new().fill(Color32::from_rgb(0x1a, 0x1a, 0x1a)).inner_margin(6.0))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Clear").clicked() {
                        gitlog::clear();
                        self.lines.clear();
                    }
                    if ui.button("Copy all").on_hover_text("Copy the visible lines to the clipboard").clicked() {
                        let text = before.iter().map(|&i| Self::plain(&self.lines[i])).collect::<Vec<_>>().join("\n");
                        ui.ctx().copy_text(text);
                    }
                    ui.checkbox(&mut self.show_reads, "Show read-only commands");
                    ui.checkbox(&mut self.follow, "Follow output");
                    ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter").desired_width(160.0));
                    ui.weak(format!("{} lines", before.len()));
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    ui.colored_label(CMD, "command  = run through the system git");
                    ui.colored_label(EQUIV, "command  [libgit2] = done in-process; this is the equivalent");
                    ui.colored_label(READ, "command  [read] = only reads");
                });
            });

        let shown = self.visible();
        let n = shown.len();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(6.0)).show_inside(ui, |ui| {
            if n == 0 {
                ui.colored_label(NOTE, "No commands yet. Use the app and they will appear here.");
                return;
            }
            egui::ScrollArea::both()
                .id_salt("console_lines")
                .auto_shrink(false)
                .stick_to_bottom(self.follow)
                .show_rows(ui, row_h, n, |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for i in range {
                        let l = &self.lines[shown[i]];
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::hover());
                        let galley = ui.painter().layout_job(Self::job(l, &font));
                        let pos = egui::Pos2::new(rect.left(), rect.center().y - galley.size().y / 2.0);
                        ui.painter_at(rect).galley(pos, galley, CMD);
                    }
                });
        });
    }

    /// One log line as coloured text, like a terminal would print it.
    fn job(l: &Line, font: &FontId) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;
        let mut add = |text: &str, color: Color32| {
            job.append(text, 0.0, TextFormat::simple(font.clone(), color));
        };
        add(&format!("[{:>9.3}] ", l.at), TIME);
        match l.kind {
            Kind::Cmd | Kind::Equiv | Kind::Read => {
                if !l.repo.is_empty() {
                    add(&l.repo, PROMPT);
                    add(" $ ", CMD);
                }
                let (color, tag) = match l.kind {
                    Kind::Cmd => (CMD, ""),
                    Kind::Equiv => (EQUIV, "  [libgit2]"),
                    _ => (READ, "  [read]"),
                };
                // A trailing "   # note" is shown as a comment.
                match l.text.split_once("   # ") {
                    Some((cmd, note)) => {
                        add(cmd, color);
                        add(&format!("   # {note}"), NOTE);
                    }
                    None => add(&l.text, color),
                }
                add(tag, NOTE);
            }
            Kind::Out => add(&format!("  {}", l.text), OUT),
            Kind::Err => add(&format!("  {}", l.text), ERR),
            Kind::Done => add(&format!("  -> {}", l.text), DONE),
        }
        job
    }
}
