//! Branch compare: a separate window that asks for comma separated partial file
//! names, then lists the files that differ between the checked-out branch and
//! the chosen one and match any of them.

use eframe::egui::{self, Color32, RichText};
use gitgoing_core::compare::{self, Changed};
use gitgoing_core::diff::ChangeKind;
use std::path::PathBuf;

pub struct CompareWin {
    repo: PathBuf,
    /// Full ref names, and the short names shown to the user.
    base_ref: String,
    other_ref: String,
    base_name: String,
    other_name: String,
    input: String,
    focus: bool,
    results: Option<Results>,
    error: Option<String>,
}

struct Results {
    terms: Vec<String>,
    /// Files that match the terms.
    files: Vec<Changed>,
    /// Files that differ at all.
    total: usize,
}

impl CompareWin {
    pub fn new(repo: PathBuf, base_ref: String, base_name: String, other_ref: String, other_name: String) -> Self {
        CompareWin {
            repo,
            base_ref,
            other_ref,
            base_name,
            other_name,
            input: String::new(),
            focus: true,
            results: None,
            error: None,
        }
    }

    /// Show the window; returns false once the user closed it.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let id = egui::ViewportId::from_hash_of(("compare", &self.base_ref, &self.other_ref));
        let title = format!("Compare {} with {}", self.base_name, self.other_name);
        let builder = egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([720.0, 560.0])
            .with_min_inner_size([420.0, 280.0]);
        let mut open = true;
        ctx.show_viewport_immediate(id, builder, |ctx, class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                open = false;
            }
            if class == egui::ViewportClass::Embedded {
                egui::Window::new(&title).default_size([640.0, 480.0]).show(ctx, |ui| open &= self.content(ui));
            } else {
                egui::CentralPanel::default().show(ctx, |ui| open &= self.content(ui));
            }
        });
        open
    }

    /// Run the comparison with the current input.
    fn run(&mut self) {
        self.error = None;
        match compare::changed_files(&self.repo, &self.base_ref, &self.other_ref) {
            Ok(all) => {
                let terms = compare::parse_terms(&self.input);
                let total = all.len();
                let files = all.into_iter().filter(|f| compare::matches(&f.path, &terms)).collect();
                self.results = Some(Results { terms, files, total });
            }
            Err(e) => self.error = Some(e.message().to_owned()),
        }
    }

    /// Draw the window; returns false when the user pressed Close.
    fn content(&mut self, ui: &mut egui::Ui) -> bool {
        let mut keep = true;
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&self.base_name).strong());
            ui.label("(checked out)  vs  ");
            ui.label(RichText::new(&self.other_name).strong());
        });
        ui.add_space(8.0);
        if self.results.is_none() {
            keep = self.input_stage(ui);
        } else {
            keep = self.result_stage(ui) && keep;
        }
        if let Some(e) = &self.error {
            ui.add_space(6.0);
            ui.colored_label(Color32::from_rgb(0xe5, 0x53, 0x4b), e);
        }
        keep
    }

    fn input_stage(&mut self, ui: &mut egui::Ui) -> bool {
        let mut keep = true;
        ui.label("Partial file names, separated by commas:");
        ui.add_space(4.0);
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.input)
                .hint_text("e.g. user, order, config")
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut self.focus) {
            edit.request_focus();
        }
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(4.0);
        ui.weak("Only files that differ between the two branches are listed. Leave empty to list all of them.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Compare").clicked() || enter {
                self.run();
            }
            if ui.button("Cancel").clicked() {
                keep = false;
            }
        });
        keep
    }

    fn result_stage(&mut self, ui: &mut egui::Ui) -> bool {
        let mut keep = true;
        let Some(res) = &self.results else { return keep };
        let what = if res.terms.is_empty() { "all files".to_owned() } else { format!("\"{}\"", res.terms.join("\", \"")) };
        ui.label(format!("{} of {} differing file(s) match {what}", res.files.len(), res.total));
        ui.add_space(6.0);
        let mut back = false;
        ui.horizontal(|ui| {
            back = ui.button("← New search").clicked();
            if ui.add_enabled(!res.files.is_empty(), egui::Button::new("Copy list")).clicked() {
                let text = res.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>().join("\n");
                ui.ctx().copy_text(text);
            }
            if ui.button("Close").clicked() {
                keep = false;
            }
        });
        ui.separator();
        if res.files.is_empty() {
            ui.add_space(8.0);
            ui.weak("No differing file has a matching name.");
        } else {
            egui::ScrollArea::both().auto_shrink(false).show_rows(ui, ui.text_style_height(&egui::TextStyle::Monospace) + 4.0, res.files.len(), |ui, rows| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                for f in &res.files[rows] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(f.kind.letter().to_string()).monospace().color(kind_color(f.kind)));
                        ui.label(RichText::new(&f.path).monospace());
                    });
                }
            });
        }
        if back {
            self.results = None;
            self.focus = true;
        }
        keep
    }
}

fn kind_color(k: ChangeKind) -> Color32 {
    match k {
        ChangeKind::Added => Color32::from_rgb(0x4e, 0xc9, 0x6b),
        ChangeKind::Deleted => Color32::from_rgb(0xe5, 0x53, 0x4b),
        _ => Color32::from_rgb(0xe5, 0xc0, 0x7b),
    }
}
