//! File-level windows: history, blame, a small editor, and the external-tool actions.

use super::*;
use crate::filemenu::{FileAction, FileRequest};
use gitgoing_core::filehist::{self, BlameLine, FileCommit};
use gitgoing_core::gitcmd;
use gitgoing_core::system;

type Task<T> = Receiver<Result<T, String>>;

/// "File History" window.
pub(super) struct FileHistDlg {
    path: String,
    rx: Option<Task<Vec<FileCommit>>>,
    items: Option<Result<Vec<FileCommit>, String>>,
}

/// "File Blame" window.
pub(super) struct BlameDlg {
    path: String,
    at: Option<gitgoing_core::Oid>,
    rx: Option<Task<Vec<BlameLine>>>,
    lines: Option<Result<Vec<BlameLine>, String>>,
}

/// The small built-in text editor.
pub(super) struct EditDlg {
    path: String,
    abs: PathBuf,
    text: String,
    saved: String,
    error: Option<String>,
}

const EDIT_LIMIT: u64 = 2 * 1024 * 1024;

fn short(id: gitgoing_core::Oid) -> String {
    id.to_string()[..8].to_owned()
}

/// Colour that is stable per commit, to band the blame view.
fn band(id: gitgoing_core::Oid, dark: bool) -> Color32 {
    let b = id.as_bytes();
    let (r, g, c) = (b[0] as u32, b[1] as u32, b[2] as u32);
    let base = if dark { 40 } else { 215 };
    let spread = 28;
    let f = |v: u32| (base + (v * spread / 255)) as u8;
    Color32::from_rgb(f(r), f(g), f(c))
}

impl RepoTab {
    /// A repository-relative path as a file in the working folder; refuses anything that escapes it.
    pub(super) fn workdir_file(&self, rel: &str) -> Result<PathBuf, String> {
        let root = self.repo_path().ok_or("No repository is open.")?;
        let p = std::path::Path::new(rel);
        if p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(format!("'{rel}' is not a path inside the repository."));
        }
        Ok(root.join(p))
    }

    /// An existing working-folder file, or an explanation of why there is none.
    fn existing_file(&mut self, rel: &str) -> Option<PathBuf> {
        match self.workdir_file(rel) {
            Ok(p) if p.is_file() => Some(p),
            Ok(_) => {
                self.error_dlg = Some((
                    "File not in your working folder".into(),
                    format!("'{rel}' doesn't exist on disk right now (it may have been deleted, renamed or only exist in that commit)."),
                ));
                None
            }
            Err(e) => {
                self.error_dlg = Some(("Can't open file".into(), e));
                None
            }
        }
    }

    pub(super) fn file_action(&mut self, ctx: &egui::Context, req: FileRequest) {
        let Some(repo) = self.repo_path() else { return };
        let FileRequest { action, path, commit, staged } = req;
        match action {
            FileAction::History => {
                let (p, r) = (path.clone(), repo.clone());
                self.file_hist = Some(FileHistDlg {
                    path,
                    rx: Some(detail::spawn(ctx, move || filehist::history(&r, &p, commit, 500))),
                    items: None,
                });
            }
            FileAction::Blame => self.open_blame(ctx, path, commit),
            FileAction::DiffTool => {
                let label = "Open in external diff tool";
                self.file_bg.push((
                    label,
                    detail::spawn(ctx, move || {
                        let mut cmd = gitcmd::command(Some(&repo));
                        cmd.args(["difftool", "-y"]);
                        if let Some(c) = commit {
                            cmd.arg(filehist::diff_base(&repo, c)).arg(c.to_string());
                        } else if staged {
                            cmd.arg("--cached");
                        }
                        cmd.arg("--").arg(&path);
                        gitcmd::run_cmd(&mut cmd, "difftool").map(|_| ())
                    }),
                ));
            }
            FileAction::Editor | FileAction::DefaultProgram | FileAction::Reveal => {
                let Some(abs) = self.existing_file(&path) else { return };
                let r = match action {
                    FileAction::Editor => system::open_file(&abs),
                    FileAction::DefaultProgram => system::open_default(&abs),
                    _ => system::reveal(&abs),
                };
                if let Err(e) = r {
                    self.error_dlg = Some(("Couldn't open the file".into(), e.to_string()));
                }
            }
            FileAction::Edit => {
                let Some(abs) = self.existing_file(&path) else { return };
                let too_big = std::fs::metadata(&abs).map(|m| m.len() > EDIT_LIMIT).unwrap_or(false);
                match std::fs::read_to_string(&abs) {
                    Ok(_) if too_big => {
                        self.error_dlg = Some(("File too large".into(), "This file is over 2 MB; use an external editor.".into()))
                    }
                    Ok(text) => self.edit = Some(EditDlg { path, abs, saved: text.clone(), text, error: None }),
                    Err(_) => {
                        self.error_dlg = Some((
                            "Can't edit this file".into(),
                            "It isn't valid UTF-8 text (binary file?). Use an external editor.".into(),
                        ))
                    }
                }
            }
            FileAction::Delete => self.confirm = Some(Confirm::DeleteFile(path)),
        }
    }

    fn open_blame(&mut self, ctx: &egui::Context, path: String, at: Option<gitgoing_core::Oid>) {
        let Some(repo) = self.repo_path() else { return };
        let p = path.clone();
        self.blame = Some(BlameDlg { path, at, rx: Some(detail::spawn(ctx, move || filehist::blame(&repo, &p, at))), lines: None });
    }

    /// Collect finished background file tasks.
    pub(super) fn poll_files(&mut self) {
        if let Some(d) = &mut self.file_hist {
            if let Some(r) = d.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
                d.rx = None;
                d.items = Some(r);
            }
        }
        if let Some(d) = &mut self.blame {
            if let Some(r) = d.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
                d.rx = None;
                d.lines = Some(r);
            }
        }
        let mut done = Vec::new();
        self.file_bg.retain(|(label, rx)| match rx.try_recv() {
            Ok(r) => {
                done.push((*label, r));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => false,
        });
        for (label, r) in done {
            if let Err(e) = r {
                self.error_dlg = Some((format!("{label} failed"), e));
            }
        }
    }

    /// Draw the history, blame and editor windows.
    pub(super) fn file_windows(&mut self, ctx: &egui::Context) {
        self.window_file_history(ctx);
        self.window_blame(ctx);
        self.window_edit(ctx);
    }

    fn window_file_history(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut jump = None;
        let mut blame_at = None;
        if let Some(d) = &self.file_hist {
            egui::Window::new(format!("History of {}", d.path))
                .id(egui::Id::new("file_history"))
                .open(&mut open)
                .default_size([640.0, 420.0])
                .show(ctx, |ui| match &d.items {
                    None => {
                        ui.spinner();
                    }
                    Some(Err(e)) => {
                        ui.colored_label(Color32::LIGHT_RED, e);
                    }
                    Some(Ok(items)) if items.is_empty() => {
                        ui.weak("No commits touch this file.");
                    }
                    Some(Ok(items)) => {
                        ui.weak(format!("{} commit(s) · click one to show it in the graph", items.len()));
                        ui.add_space(4.0);
                        let now = timefmt::now();
                        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, 24.0, items.len(), |ui, range| {
                            for c in &items[range] {
                                ui.horizontal(|ui| {
                                    detail::hash_link(ui, &short(c.id), &c.id.to_string());
                                    if ui.small_button("Blame").on_hover_text("Blame the file as of this commit").clicked() {
                                        blame_at = Some(c.id);
                                    }
                                    ui.weak(&c.author);
                                    ui.weak(timefmt::relative(c.time, now));
                                    let r = ui.add(egui::Label::new(&c.summary).truncate().sense(Sense::click()));
                                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                        jump = Some(c.id);
                                    }
                                });
                            }
                        });
                    }
                });
        }
        if !open {
            self.file_hist = None;
        }
        if let Some(id) = jump {
            if let Some(l) = &mut self.repo {
                if let Some(i) = l.rows.iter().position(|r| r.id == id) {
                    l.select(ctx, i);
                    l.reveal(i, true);
                }
            }
        }
        if let (Some(id), Some(d)) = (blame_at, &self.file_hist) {
            let path = d.path.clone();
            self.open_blame(ctx, path, Some(id));
        }
    }

    fn window_blame(&mut self, ctx: &egui::Context) {
        let mut open = true;
        if let Some(d) = &self.blame {
            let at = d.at.map_or("HEAD".to_owned(), short);
            let dark = ctx.style().visuals.dark_mode;
            egui::Window::new(format!("Blame: {} @ {at}", d.path))
                .id(egui::Id::new("file_blame"))
                .open(&mut open)
                .default_size([820.0, 520.0])
                .show(ctx, |ui| match &d.lines {
                    None => {
                        ui.spinner();
                    }
                    Some(Err(e)) => {
                        ui.colored_label(Color32::LIGHT_RED, e);
                    }
                    Some(Ok(lines)) => {
                        let now = timefmt::now();
                        let mono = egui::TextStyle::Monospace.resolve(ui.style());
                        egui::ScrollArea::both().auto_shrink(false).show_rows(ui, 18.0, lines.len(), |ui, range| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for i in range {
                                let l = &lines[i];
                                let first = i == 0 || lines[i - 1].id != l.id;
                                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(700.0), 18.0), Sense::click());
                                let p = ui.painter_at(rect);
                                p.rect_filled(rect, 0.0, band(l.id, dark));
                                let weak = ui.visuals().weak_text_color();
                                if first {
                                    p.text(rect.left_center() + Vec2::new(6.0, 0.0), egui::Align2::LEFT_CENTER, short(l.id), mono.clone(), ui.visuals().hyperlink_color);
                                    p.text(rect.left_center() + Vec2::new(80.0, 0.0), egui::Align2::LEFT_CENTER, &l.author, mono.clone(), weak);
                                    p.text(rect.left_center() + Vec2::new(200.0, 0.0), egui::Align2::LEFT_CENTER, timefmt::relative(l.time, now), mono.clone(), weak);
                                }
                                p.text(rect.left_center() + Vec2::new(290.0, 0.0), egui::Align2::LEFT_CENTER, &l.text, mono.clone(), ui.visuals().text_color());
                                if resp.on_hover_text(format!("{}\n{}", l.id, l.summary)).clicked() {
                                    ui.ctx().copy_text(l.id.to_string());
                                }
                            }
                        });
                    }
                });
        }
        if !open {
            self.blame = None;
        }
    }

    fn window_edit(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut save = false;
        let mut close = false;
        if let Some(d) = &mut self.edit {
            let dirty = d.text != d.saved;
            egui::Window::new(format!("Edit {}{}", d.path, if dirty { " •" } else { "" }))
                .id(egui::Id::new("file_edit"))
                .open(&mut open)
                .default_size([720.0, 520.0])
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        save = ui.add_enabled(dirty, egui::Button::new("Save")).clicked();
                        close = ui.button("Close").clicked();
                        if let Some(e) = &d.error {
                            ui.colored_label(Color32::LIGHT_RED, e);
                        }
                    });
                    ui.add_space(4.0);
                    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                        ui.add(egui::TextEdit::multiline(&mut d.text).code_editor().desired_width(f32::INFINITY).desired_rows(30));
                    });
                });
            if save {
                match std::fs::write(&d.abs, &d.text) {
                    Ok(()) => {
                        d.saved = d.text.clone();
                        d.error = None;
                    }
                    Err(e) => d.error = Some(format!("Couldn't save: {e}")),
                }
            }
        }
        if save {
            if let Some(c) = self.repo.as_mut().and_then(|l| l.changes.as_mut()) {
                c.refresh(ctx);
            }
        }
        if !open || close {
            self.edit = None;
        }
    }
}
