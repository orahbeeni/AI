//! The "Clone repository" window.

use crate::detail::spawn;
use eframe::egui;
use gitgoing_core::clone;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

pub enum Outcome {
    Keep,
    Close,
    /// Cloned successfully; open this folder.
    Open(PathBuf),
}

pub struct CloneDialog {
    url: String,
    parent: String,
    name: String,
    name_edited: bool,
    error: Option<String>,
    rx: Option<Receiver<Result<PathBuf, String>>>,
}

fn home() -> String {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).display().to_string())
        .unwrap_or_default()
}

impl CloneDialog {
    pub fn new() -> Self {
        CloneDialog {
            url: String::new(),
            parent: home(),
            name: String::new(),
            name_edited: false,
            error: None,
            rx: None,
        }
    }

    fn dest(&self) -> PathBuf {
        PathBuf::from(self.parent.trim()).join(self.name.trim())
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        if let Some(rx) = &self.rx {
            if let Ok(r) = rx.try_recv() {
                self.rx = None;
                match r {
                    Ok(p) => return Outcome::Open(p),
                    Err(e) => self.error = Some(e),
                }
            }
        }
        let busy = self.rx.is_some();
        let mut outcome = Outcome::Keep;
        let mut start = false;
        egui::Window::new("Clone repository")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width(480.0);
                ui.label("Repository URL");
                let r = ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.url)
                        .hint_text("https://host/owner/repo.git or git@host:owner/repo.git")
                        .desired_width(f32::INFINITY),
                );
                if r.changed() && !self.name_edited {
                    self.name = clone::repo_name_from_url(&self.url);
                }
                ui.add_space(6.0);
                ui.label("Clone into");
                ui.horizontal(|ui| {
                    ui.add_enabled(!busy, egui::TextEdit::singleline(&mut self.parent).desired_width(360.0));
                    if ui.add_enabled(!busy, egui::Button::new("Browse…")).clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.parent = p.display().to_string();
                        }
                    }
                });
                ui.add_space(6.0);
                ui.label("Folder name");
                if ui.add_enabled(!busy, egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY)).changed() {
                    self.name_edited = true;
                }
                ui.weak(self.dest().display().to_string());
                if let Some(e) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, e);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ok = !busy && !self.url.trim().is_empty() && !self.name.trim().is_empty();
                    if ui.add_enabled(ok, egui::Button::new("Clone")).clicked() {
                        start = true;
                    }
                    if ui.add_enabled(!busy, egui::Button::new("Cancel")).clicked() {
                        outcome = Outcome::Close;
                    }
                    if busy {
                        ui.spinner();
                        ui.weak("Cloning…");
                    }
                });
            });
        if start {
            self.error = None;
            let (url, dest) = (self.url.clone(), self.dest());
            self.rx = Some(spawn(ctx, move || clone::clone(&url, &dest).map(|_| dest)));
        }
        outcome
    }
}
