//! Branch compare: a separate window that asks for comma separated partial file
//! names, then lists the files that differ between the checked-out branch and
//! the chosen one and match any of them.

use crate::startup;
use crate::filetree::{self, ViewMode};
use eframe::egui::{self, Color32, RichText};
use gitgoing_core::compare::{self, Changed};
use gitgoing_core::diff::ChangeKind;
use std::path::PathBuf;

pub struct CompareWin {
    /// Fixed at creation so changing a branch doesn't reopen the window.
    id: egui::ViewportId,
    repo: PathBuf,
    /// Full ref names, and the short names shown to the user.
    base_ref: String,
    other_ref: String,
    base_name: String,
    other_name: String,
    input: String,
    focus: bool,
    results: Option<Results>,
    mode: ViewMode,
    /// All branches as `(full ref, short name)` for the pickers.
    refs: Vec<(String, String)>,
    /// Saved partial-name lists `(name, terms)` and the name being typed for a new one.
    saved: Vec<(String, String)>,
    save_name: String,
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
        let refs = compare::branch_refs(&repo).unwrap_or_default();
        let id = egui::ViewportId::from_hash_of(("compare", &base_ref, &other_ref));
        CompareWin {
            id,
            repo,
            base_ref,
            other_ref,
            base_name,
            other_name,
            input: String::new(),
            focus: true,
            results: None,
            mode: ViewMode::default(),
            refs,
            saved: startup::load_term_lists(),
            save_name: String::new(),
            error: None,
        }
    }

    /// Show the window; returns false once the user closed it.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let id = self.id;
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
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            changed |= branch_picker(ui, "compare_base", &self.refs, &mut self.base_ref, &mut self.base_name);
            ui.label("vs");
            changed |= branch_picker(ui, "compare_other", &self.refs, &mut self.other_ref, &mut self.other_name);
        });
        if changed && self.results.is_some() {
            self.run();
        }
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
        ui.add_space(6.0);
        self.saved_lists(ui);
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

    /// Load, save and delete named lists of partial file names.
    fn saved_lists(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut delete = None;
            egui::ComboBox::from_id_salt("compare_saved").selected_text("Saved lists").width(160.0).show_ui(ui, |ui| {
                if self.saved.is_empty() {
                    ui.weak("None saved yet.");
                }
                for (i, (name, terms)) in self.saved.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(false, name).on_hover_text(terms).clicked() {
                            self.input = terms.clone();
                            self.save_name = name.clone();
                            ui.close_menu();
                        }
                        if ui.small_button("✕").on_hover_text("Delete this list").clicked() {
                            delete = Some(i);
                        }
                    });
                }
            });
            ui.add(egui::TextEdit::singleline(&mut self.save_name).hint_text("list name").desired_width(140.0));
            let name = self.save_name.trim().to_owned();
            let can_save = !name.is_empty() && !compare::parse_terms(&self.input).is_empty();
            if ui.add_enabled(can_save, egui::Button::new("Save list")).on_hover_text("Save the names above under this name").clicked() {
                let terms = self.input.trim().to_owned();
                match self.saved.iter_mut().find(|(n, _)| *n == name) {
                    Some(e) => e.1 = terms,
                    None => self.saved.push((name, terms)),
                }
                startup::save_term_lists(&self.saved);
            }
            if let Some(i) = delete {
                self.saved.remove(i);
                startup::save_term_lists(&self.saved);
            }
        });
    }

    fn result_stage(&mut self, ui: &mut egui::Ui) -> bool {
        let mut keep = true;
        let mode = &mut self.mode;
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
        filetree::mode_toggle(ui, mode);
        ui.add_space(4.0);
        if res.files.is_empty() {
            ui.add_space(8.0);
            ui.weak("No differing file has a matching name.");
        } else {
            let mode = *mode;
            let row = |ui: &mut egui::Ui, f: &Changed, shown: &str| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(f.kind.letter().to_string()).monospace().color(kind_color(f.kind)));
                    ui.label(RichText::new(shown).monospace()).on_hover_text(&f.path);
                });
            };
            if mode == ViewMode::Tree {
                egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    let items: Vec<(&str, usize)> = res.files.iter().enumerate().map(|(i, f)| (f.path.as_str(), i)).collect();
                    filetree::show(ui, "compare_files", &items, &mut |ui, i, name| row(ui, &res.files[i], name));
                });
            } else {
                egui::ScrollArea::both().auto_shrink(false).show_rows(ui, ui.text_style_height(&egui::TextStyle::Monospace) + 4.0, res.files.len(), |ui, rows| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    for f in &res.files[rows] {
                        let shown = if mode == ViewMode::Path { f.path.as_str() } else { filetree::file_name(&f.path) };
                        row(ui, f, shown);
                    }
                });
            }
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

/// A dropdown of branches with a text box that narrows the list. Returns true
/// when a different branch was chosen.
fn branch_picker(
    ui: &mut egui::Ui,
    salt: &str,
    refs: &[(String, String)],
    full: &mut String,
    short: &mut String,
) -> bool {
    let mut picked = false;
    // Each picker keeps its own filter text.
    let fid = egui::Id::new((salt, "filter"));
    let mut filter: String = ui.data(|d| d.get_temp(fid)).unwrap_or_default();
    let combo = egui::ComboBox::from_id_salt(salt).selected_text(RichText::new(short.as_str()).strong()).width(200.0);
    let resp = combo.show_ui(ui, |ui| {
        let edit = ui.add(egui::TextEdit::singleline(&mut filter).hint_text("filter…").desired_width(f32::INFINITY));
        if !edit.has_focus() && filter.is_empty() {
            edit.request_focus();
        }
        let needle = filter.to_lowercase();
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            let mut any = false;
            for (f, s) in refs.iter().filter(|(_, s)| s.to_lowercase().contains(&needle)) {
                any = true;
                if ui.selectable_label(f == full, s).clicked() && f != full {
                    *full = f.clone();
                    *short = s.clone();
                    picked = true;
                }
            }
            if !any {
                ui.weak("No matching branch.");
            }
        });
    });
    // Start every opening with an empty filter.
    if resp.inner.is_none() {
        filter.clear();
    }
    ui.data_mut(|d| d.insert_temp(fid, filter));
    picked
}
