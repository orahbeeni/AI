//! Bottom panel: commit metadata, changed files, and the selected file's diff.

use crate::diffview::{self, DiffOpts};
use crate::filetree::{self, ViewMode};
use crate::icons;
use crate::timefmt;
use eframe::egui::{self, Color32, RichText, Sense};
use gitgoing_core::diff::{self, ChangeKind, CommitDetail, FileDiff};
use gitgoing_core::Oid;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

pub(crate) type Fetch<T> = Option<Receiver<Result<T, String>>>;

pub struct Detail {
    id: Oid,
    repo: PathBuf,
    commit_rx: Fetch<CommitDetail>,
    commit: Option<Result<CommitDetail, String>>,
    file: Option<usize>,
    /// Whether the centre pane shows the selected file's diff.
    pub diff_open: bool,
    opts: DiffOpts,
    diff_rx: Fetch<FileDiff>,
    diff: Option<Result<FileDiff, String>>,
    mode: ViewMode,
}

/// Error types that can be shown to the user as a message.
pub(crate) trait ErrorText {
    fn text(self) -> String;
}

impl ErrorText for gitgoing_core::GitError {
    fn text(self) -> String {
        self.message().to_owned()
    }
}

impl ErrorText for String {
    fn text(self) -> String {
        self
    }
}

pub(crate) fn spawn<T: Send + 'static, E: ErrorText>(
    ctx: &egui::Context,
    f: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> Receiver<Result<T, String>> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        // A panic must not leave the UI waiting forever for a result that never comes.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f().map_err(ErrorText::text)))
            .unwrap_or_else(|p| Err(format!("internal error: {}", panic_message(&*p))));
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

/// The text of a caught panic payload.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned())
}

impl Detail {
    pub fn new(ctx: &egui::Context, repo: &Path, id: Oid) -> Self {
        let r = repo.to_path_buf();
        Detail {
            id,
            repo: repo.to_path_buf(),
            commit_rx: Some(spawn(ctx, move || diff::commit_detail(&r, id))),
            commit: None,
            file: None,
            diff_open: false,
            opts: DiffOpts::default(),
            diff_rx: None,
            diff: None,
            mode: ViewMode::default(),
        }
    }

    pub fn id(&self) -> Oid {
        self.id
    }

    fn select_file(&mut self, ctx: &egui::Context, idx: usize) {
        let Some(Ok(c)) = &self.commit else { return };
        let Some(f) = c.files.get(idx) else { return };
        let (repo, id, path, old) = (self.repo.clone(), self.id, f.path.clone(), f.old_path.clone());
        self.file = Some(idx);
        self.diff = None;
        let (file_view, ignore_ws) = (self.opts.file_view, self.opts.ignore_ws);
        self.diff_rx = Some(spawn(ctx, move || {
            if file_view {
                diff::file_text(&repo, diff::FileSource::Commit(id), &path)
            } else {
                diff::file_diff(&repo, id, &path, old.as_deref(), ignore_ws)
            }
        }));
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.commit_rx {
            if let Ok(r) = rx.try_recv() {
                self.commit_rx = None;
                let has_files = matches!(&r, Ok(c) if !c.files.is_empty());
                self.commit = Some(r);
                if has_files {
                    self.select_file(ctx, 0);
                }
            }
        }
        if let Some(rx) = &self.diff_rx {
            if let Ok(r) = rx.try_recv() {
                self.diff_rx = None;
                self.diff = Some(r);
            }
        }
    }
}

/// Right-hand panel content. Returns `true` if the user closed it.
pub fn panel(ctx: &egui::Context, ui: &mut egui::Ui, d: &mut Detail) -> bool {
    let mut close = false;
    let mut pick = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| match &d.commit {
        None => {
            ui.spinner();
        }
        Some(Err(e)) => {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
        Some(Ok(c)) => {
            ui.horizontal(|ui| {
                hash_link(ui, &short(c.id), &c.id.to_string());
                if ui.small_button("Close").clicked() {
                    close = true;
                }
            });
            ui.strong(format!("{} <{}>", c.author, c.email));
            ui.weak(timefmt::absolute(c.time, c.offset_minutes));
            if !c.parents.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.weak("parents:");
                    for p in &c.parents {
                        hash_link(ui, &short(*p), &p.to_string());
                    }
                });
            }
            ui.add_space(4.0);
            message_with_hashes(ui, &c.message);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(format!("{} changed file(s)", c.files.len()));
                filetree::mode_toggle(ui, &mut d.mode);
            });
            let selected = if d.diff_open { d.file } else { None };
            let mode = d.mode;
            let mut row = |ui: &mut egui::Ui, i: usize, name: &str| {
                let f = &c.files[i];
                let full = match &f.old_path {
                    Some(o) => format!("{o} → {}", f.path),
                    None => f.path.clone(),
                };
                // Path view shows the whole (rename) text; the others show `name`.
                let name = if mode == ViewMode::Path { full.clone() } else { name.to_owned() };
                ui.horizontal(|ui| {
                    icons::change_badge(ui, f.kind);
                    if ui.selectable_label(selected == Some(i), name).on_hover_text(full).clicked() {
                        pick = Some(i);
                    }
                    if f.binary {
                        ui.weak("bin");
                    } else {
                        ui.colored_label(Color32::from_rgb(0x4c, 0xaf, 0x50), format!("+{}", f.additions));
                        ui.colored_label(Color32::from_rgb(0xe5, 0x53, 0x4b), format!("-{}", f.deletions));
                    }
                });
            };
            match mode {
                ViewMode::Tree => {
                    let items: Vec<(&str, usize)> = c.files.iter().enumerate().map(|(i, f)| (f.path.as_str(), i)).collect();
                    filetree::show(ui, "commit_files", &items, &mut |ui, i, name| row(ui, i, name));
                }
                ViewMode::Path => {
                    for (i, f) in c.files.iter().enumerate() {
                        row(ui, i, &f.path);
                    }
                }
                ViewMode::File => {
                    for (i, f) in c.files.iter().enumerate() {
                        row(ui, i, filetree::file_name(&f.path));
                    }
                }
            }
        }
    });
    if let Some(i) = pick {
        d.diff_open = true;
        d.select_file(ctx, i);
    }
    close
}

/// Centre pane showing the selected file's diff. Returns `true` to close it.
pub fn diff_pane(ui: &mut egui::Ui, d: &mut Detail) -> bool {
    let ctx = ui.ctx().clone();
    let path = match (&d.commit, d.file) {
        (Some(Ok(c)), Some(i)) => c.files.get(i).map(|f| f.path.clone()),
        _ => None,
    }
    .unwrap_or_default();
    let tag = short(d.id);
    let kind = match (&d.commit, d.file) {
        (Some(Ok(c)), Some(i)) => c.files.get(i).map(|f| f.kind),
        _ => None,
    };
    let out = diffview::show(
        ui,
        &mut d.opts,
        &d.diff,
        d.diff_rx.is_some(),
        &diffview::Header { title: &path, kind, tag: &tag, edit_label: None, hunk_btn: None },
    );
    if out.refetch {
        if let Some(i) = d.file {
            d.select_file(&ctx, i);
        }
    }
    out.close
}

/// A git hash that copies `full` to the clipboard when clicked, briefly
/// showing "copied!" in its place as confirmation.
pub(crate) fn hash_link(ui: &mut egui::Ui, shown: &str, full: &str) {
    let key = egui::Id::new(("hash_copied", full));
    let now = ui.input(|i| i.time);
    let copied_at: Option<f64> = ui.data(|d| d.get_temp(key));
    let recent = copied_at.is_some_and(|t| now - t < 1.0);
    let text = if recent {
        RichText::new("copied!").monospace().color(Color32::from_rgb(0x4c, 0xaf, 0x50))
    } else {
        RichText::new(shown).monospace().color(ui.visuals().hyperlink_color)
    };
    let r = ui
        .add(egui::Label::new(text).sense(Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Click to copy {full}"));
    if r.clicked() {
        ui.ctx().copy_text(full.to_owned());
        ui.data_mut(|d| d.insert_temp(key, now));
    }
    if recent || r.clicked() {
        // Repaint to switch the label back once the confirmation expires.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Split `word` into (leading punctuation, hash, trailing punctuation) if its
/// core looks like an abbreviated or full git hash.
fn split_hash(word: &str) -> Option<(&str, &str, &str)> {
    let is_punct = |c: char| !c.is_ascii_alphanumeric();
    let start = word.find(|c: char| !is_punct(c))?;
    let end = word.rfind(|c: char| !is_punct(c))? + 1;
    let core = &word[start..end];
    // Require a digit so ordinary words made of a-f letters ("defaced") don't match.
    let hashy = (7..=40).contains(&core.len())
        && core.bytes().all(|b| b.is_ascii_hexdigit())
        && core.bytes().any(|b| b.is_ascii_digit());
    hashy.then(|| (&word[..start], core, &word[end..]))
}

/// Show a commit message, making any hashes inside it click-to-copy.
fn message_with_hashes(ui: &mut egui::Ui, message: &str) {
    for line in message.lines() {
        if !line.split_whitespace().any(|w| split_hash(w).is_some()) {
            ui.label(line);
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for word in line.split_inclusive(char::is_whitespace) {
                let trimmed = word.trim_end();
                let space = &word[trimmed.len()..];
                match split_hash(trimmed) {
                    Some((pre, hash, post)) => {
                        ui.label(pre);
                        hash_link(ui, hash, hash);
                        ui.label(format!("{post}{space}"));
                    }
                    None => {
                        ui.label(word);
                    }
                }
            }
        });
    }
}

fn short(id: Oid) -> String {
    id.to_string()[..8].to_owned()
}

pub(crate) fn kind_color(k: ChangeKind) -> Color32 {
    match k {
        ChangeKind::Added => Color32::from_rgb(0x4c, 0xaf, 0x50),
        ChangeKind::Deleted => Color32::from_rgb(0xe5, 0x53, 0x4b),
        ChangeKind::Modified => Color32::from_rgb(0xe5, 0xc0, 0x7b),
        ChangeKind::Renamed | ChangeKind::Copied => Color32::from_rgb(0x56, 0xb6, 0xc2),
        ChangeKind::Untracked => Color32::from_rgb(0x4c, 0xaf, 0x50),
        ChangeKind::Conflicted => Color32::from_rgb(0xe5, 0x53, 0x4b),
        ChangeKind::Other => Color32::GRAY,
    }
}


#[cfg(test)]
mod tests {
    use super::{panic_message, spawn, split_hash};

    #[test]
    fn a_panicking_task_still_reports_back() {
        let ctx = eframe::egui::Context::default();
        let rx = spawn(&ctx, || -> Result<(), String> { panic!("boom") });
        let r = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("a result arrives");
        assert_eq!(r.unwrap_err(), "internal error: boom");
        let rx = spawn(&ctx, || Ok::<_, String>(7));
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap(), 7);
        assert_eq!(panic_message(&*Box::new(String::from("owned"))), "owned");
    }

    #[test]
    fn finds_hashes_in_text() {
        assert_eq!(split_hash("abc1234"), Some(("", "abc1234", "")));
        assert_eq!(split_hash("(abc1234)."), Some(("(", "abc1234", ").")));
        assert_eq!(
            split_hash("0123456789abcdef0123456789abcdef01234567,"),
            Some(("", "0123456789abcdef0123456789abcdef01234567", ","))
        );
        assert_eq!(split_hash("defaced"), None, "no digit");
        assert_eq!(split_hash("abc123"), None, "too short");
        assert_eq!(split_hash("ghijklm1"), None, "not hex");
        assert_eq!(split_hash("..."), None);
    }
}
