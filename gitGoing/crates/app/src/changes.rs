//! Working-tree panel: stage/unstage files, view their diffs, and commit.

use crate::detail::{spawn, Fetch};
use crate::diffview::{self, DiffOpts, HunkBtn};
use crate::filetree::{self, ViewMode};
use crate::icons;
use eframe::egui::{self, Color32};
use gitgoing_core::diff::{ChangeKind, FileDiff};
use gitgoing_core::merge::{self, Side};
use gitgoing_core::rebase;
use gitgoing_core::status::{self, StatusEntry, WorkStatus};
use gitgoing_core::undo::{self, UndoEntry};
use std::path::PathBuf;

pub struct Changes {
    repo: PathBuf,
    status: Option<Result<WorkStatus, String>>,
    status_rx: Fetch<WorkStatus>,
    /// (path, staged?)
    selected: Option<(String, bool)>,
    diff: Option<Result<FileDiff, String>>,
    diff_rx: Fetch<FileDiff>,
    summary: String,
    description: String,
    /// Whether the centre pane shows the selected file's diff.
    pub diff_open: bool,
    /// A stage/unstage/commit running in the background; `Some(oid)` on commit.
    op_rx: Fetch<OpDone>,
    error: Option<String>,
    confirm_abort: bool,
    /// Amend the HEAD commit instead of creating a new one.
    amend: bool,
    opts: DiffOpts,
    /// A pending "discard changes" confirmation.
    confirm: Option<Discard>,
    /// A conflicted file the user asked to resolve in the editor window.
    resolve_request: Option<String>,
    /// Show files as a directory tree instead of a flat path list.
    mode: ViewMode,
}

enum OpDone {
    Staged,
    Committed(Option<UndoEntry>),
}

pub enum Outcome {
    None,
    Committed(Option<UndoEntry>),
}

impl Changes {
    pub fn new(ctx: &egui::Context, repo: PathBuf) -> Self {
        let mut c = Changes {
            repo,
            status: None,
            status_rx: None,
            selected: None,
            diff: None,
            diff_rx: None,
            summary: String::new(),
            description: String::new(),
            diff_open: false,
            op_rx: None,
            error: None,
            confirm_abort: false,
            amend: false,
            opts: DiffOpts::default(),
            confirm: None,
            resolve_request: None,
            mode: ViewMode::default(),
        };
        c.refresh(ctx);
        c
    }

    /// The operation in progress (merge, rebase, …) and how many files still conflict.
    pub fn operation(&self) -> Option<(&'static str, usize)> {
        let Some(Ok(s)) = &self.status else { return None };
        let op = s.operation?;
        Some((op, s.unstaged.iter().filter(|e| e.kind == ChangeKind::Conflicted).count()))
    }

    pub fn take_resolve_request(&mut self) -> Option<String> {
        self.resolve_request.take()
    }

    pub fn repo(&self) -> &PathBuf {
        &self.repo
    }

    /// Number of changed paths, once the first status has arrived.
    pub fn count(&self) -> Option<usize> {
        match &self.status {
            Some(Ok(s)) => Some(s.changed_paths()),
            _ => None,
        }
    }

    pub fn refresh(&mut self, ctx: &egui::Context) {
        let repo = self.repo.clone();
        self.status_rx = Some(spawn(ctx, move || status::status(&repo)));
    }

    fn load_diff(&mut self, ctx: &egui::Context) {
        let Some((path, staged)) = self.selected.clone() else {
            self.diff = None;
            self.diff_rx = None;
            return;
        };
        let repo = self.repo.clone();
        let conflicted = matches!(&self.status, Some(Ok(s))
            if !staged && s.unstaged.iter().any(|e| e.path == path && e.kind == ChangeKind::Conflicted));
        let (file_view, ignore_ws) = (self.opts.file_view, self.opts.ignore_ws);
        self.diff_rx = Some(spawn(ctx, move || {
            use gitgoing_core::diff::{file_text, FileSource};
            if conflicted {
                status::conflict_view(&repo, &path)
            } else if file_view {
                file_text(&repo, if staged { FileSource::Index } else { FileSource::Workdir }, &path)
            } else {
                status::workdir_diff(&repo, &path, staged, ignore_ws)
            }
        }));
    }

    fn run_op<E: crate::detail::ErrorText>(
        &mut self,
        ctx: &egui::Context,
        f: impl FnOnce(&std::path::Path) -> Result<OpDone, E> + Send + 'static,
    ) {
        let repo = self.repo.clone();
        self.op_rx = Some(spawn(ctx, move || f(&repo)));
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> Outcome {
        let mut outcome = Outcome::None;
        if let Some(rx) = &self.status_rx {
            if let Ok(r) = rx.try_recv() {
                self.status_rx = None;
                if let Ok(s) = &r {
                    let exists = |sel: &(String, bool)| {
                        let list = if sel.1 { &s.staged } else { &s.unstaged };
                        list.iter().any(|e| e.path == sel.0)
                    };
                    if !self.selected.as_ref().is_some_and(exists) {
                        self.opts.selected.clear();
                        self.selected = s
                            .unstaged
                            .first()
                            .map(|e| (e.path.clone(), false))
                            .or_else(|| s.staged.first().map(|e| (e.path.clone(), true)));
                    }
                }
                if let Ok(s) = &r {
                    // Start a merge commit message from the one git prepared.
                    if s.merging && self.summary.is_empty() && self.description.is_empty() {
                        if let Some(m) = &s.merge_message {
                            let (first, rest) = m.trim().split_once('\n').unwrap_or((m.trim(), ""));
                            self.summary = first.to_owned();
                            self.description = rest.trim().to_owned();
                        }
                    }
                    if !s.merging {
                        self.confirm_abort = false;
                    }
                }
                self.status = Some(r);
                self.load_diff(ctx);
            }
        }
        if let Some(rx) = &self.diff_rx {
            if let Ok(r) = rx.try_recv() {
                self.diff_rx = None;
                self.diff = Some(r);
            }
        }
        if let Some(rx) = &self.op_rx {
            if let Ok(r) = rx.try_recv() {
                self.op_rx = None;
                match r {
                    Ok(done) => {
                        self.error = None;
                        if let OpDone::Committed(entry) = done {
                            self.summary.clear();
                            self.description.clear();
                            outcome = Outcome::Committed(entry);
                        }
                    }
                    Err(e) => self.error = Some(e),
                }
                self.refresh(ctx);
            }
        }
        outcome
    }
}

/// A continue/skip that stops on more conflicts is reported as an error, so the
/// panel stays open for the user to resolve them.
fn rebase_step(r: Result<rebase::Outcome, String>) -> Result<OpDone, String> {
    match r? {
        rebase::Outcome::Done => Ok(OpDone::Staged),
        rebase::Outcome::Conflicts => Err("The rebase stopped again with conflicts. Resolve them, then continue.".into()),
    }
}

/// What a confirmed "discard changes" will throw away.
enum Discard {
    File(String, bool),
    All { tracked: usize, untracked: usize },
    Hunk(String, usize),
}

enum Act {
    OpenResolver(String),
    RebaseContinue,
    RebaseSkip,
    Discard(String, ChangeKind),
    DiscardAll,
    Resolve(String, Side),
    Abort,
    ConfirmAbort(bool),
    Select(String, bool),
    Stage(Vec<String>),
    Unstage(Vec<String>),
    Commit,
}

/// Right-hand panel content. Returns `true` if the user closed it.
pub fn panel(ctx: &egui::Context, ui: &mut egui::Ui, c: &mut Changes) -> bool {
    let mut close = false;
    let mut act = None;
    let busy = c.op_rx.is_some();

    let rebasing = matches!(&c.status, Some(Ok(s)) if s.operation == Some("Rebase"));
    egui::TopBottomPanel::bottom("commit_box")
        .resizable(false)
        .show_inside(ui, |ui| {
            ui.add_space(4.0);
            if rebasing {
                ui.weak("A rebase is in progress. Use Continue, Skip or Abort above instead of committing.");
                if let Some(e) = &c.error {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
                ui.add_space(4.0);
                return;
            }
            ui.horizontal(|ui| {
                ui.strong("Commit message");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let head_msg = match &c.status {
                        Some(Ok(s)) => s.head_message.clone(),
                        _ => None,
                    };
                    let merging = matches!(&c.status, Some(Ok(s)) if s.merging);
                    let was = c.amend;
                    ui.add_enabled(!merging && head_msg.is_some(), egui::Checkbox::new(&mut c.amend, "Amend"))
                        .on_hover_text("Replace the last commit instead of making a new one");
                    if c.amend != was {
                        let head_msg = head_msg.unwrap_or_default();
                        let (first, rest) = head_msg.split_once('\n').unwrap_or((&head_msg, ""));
                        if c.amend && c.summary.is_empty() && c.description.is_empty() {
                            c.summary = first.to_owned();
                            c.description = rest.trim().to_owned();
                        } else if !c.amend && c.summary == first && c.description == rest.trim() {
                            c.summary.clear();
                            c.description.clear();
                        }
                    }
                });
            });
            ui.add(
                egui::TextEdit::singleline(&mut c.summary)
                    .hint_text("Summary")
                    .desired_width(f32::INFINITY),
            );
            ui.add(
                egui::TextEdit::multiline(&mut c.description)
                    .hint_text("Description")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            let (staged, merging, conflicts) = match &c.status {
                Some(Ok(s)) => (
                    !s.staged.is_empty(),
                    s.merging,
                    s.unstaged.iter().any(|e| e.kind == ChangeKind::Conflicted),
                ),
                _ => (false, false, false),
            };
            let ready = (staged || merging || c.amend) && !conflicts;
            let can_commit = ready && !c.summary.trim().is_empty() && !busy;
            ui.horizontal(|ui| {
                let label = if conflicts {
                    "Resolve conflicts to commit"
                } else if c.amend {
                    "Amend commit"
                } else if merging {
                    "Commit"
                } else if staged {
                    "Commit changes"
                } else {
                    "Stage files/changes to commit"
                };
                let w = ui.available_width() - if busy { 28.0 } else { 0.0 };
                if ui
                    .add_enabled(can_commit, egui::Button::new(label).min_size(egui::vec2(w, 26.0)))
                    .clicked()
                {
                    act = Some(Act::Commit);
                }
                if busy {
                    ui.spinner();
                }
            });
            if c.amend && matches!(&c.status, Some(Ok(s)) if s.head_pushed) {
                ui.colored_label(
                    Color32::from_rgb(0xe5, 0xc0, 0x7b),
                    "This commit is already pushed. After amending it you will need to force push.",
                );
            }
            if let Some(e) = &c.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.add_space(4.0);
        });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            ui.strong("Working changes");
            if ui.small_button("Refresh").clicked() {
                c.refresh(ctx);
            }
            if ui.small_button("Close").clicked() {
                close = true;
            }
        });
        ui.horizontal(|ui| {
            filetree::mode_toggle(ui, &mut c.mode);
        });
        let mode = c.mode;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| match &c.status {
            None => {
                ui.spinner();
            }
            Some(Err(e)) => {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            Some(Ok(s)) => {
                if s.merging {
                    ui.group(|ui| {
                        ui.strong(format!("{} in progress", s.operation.unwrap_or("Merge")));
                        let rebasing = s.operation == Some("Rebase");
                        let conflicts = s.unstaged.iter().any(|e| e.kind == ChangeKind::Conflicted);
                        if rebasing {
                            ui.label("Resolve the conflicted files (warning icon) and stage them, then continue.");
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(!busy && !conflicts, egui::Button::new("Continue rebase"))
                                    .on_disabled_hover_text("Resolve and stage all conflicts first")
                                    .clicked()
                                {
                                    act = Some(Act::RebaseContinue);
                                }
                                if ui
                                    .add_enabled(!busy, egui::Button::new("Skip this commit"))
                                    .on_hover_text("Drop the commit being applied and move on")
                                    .clicked()
                                {
                                    act = Some(Act::RebaseSkip);
                                }
                            });
                        } else {
                            ui.label("Resolve the conflicted files (warning icon), stage them, then commit.");
                        }
                        if c.confirm_abort {
                            ui.label("Discard it and all resolutions?");
                            ui.horizontal(|ui| {
                                if ui.add_enabled(!busy, egui::Button::new("Yes, abort")).clicked() {
                                    act = Some(Act::Abort);
                                }
                                if ui.button("Cancel").clicked() {
                                    act = Some(Act::ConfirmAbort(false));
                                }
                            });
                        } else if ui.add_enabled(!busy, egui::Button::new(format!("Abort {}", s.operation.unwrap_or("merge").to_lowercase()))).clicked() {
                            act = Some(Act::ConfirmAbort(true));
                        }
                    });
                    ui.add_space(6.0);
                }
                let sel = if c.diff_open { &c.selected } else { &None };
                file_list(ui, FileList { title: "Unstaged Files", entries: &s.unstaged, staged: false }, sel, busy, mode, &mut act);
                ui.add_space(6.0);
                file_list(ui, FileList { title: "Staged Files", entries: &s.staged, staged: true }, sel, busy, mode, &mut act);
            }
        });
    });

    match act {
        Some(Act::Select(p, staged)) => {
            // Line picks belong to one file's diff.
            if c.selected.as_ref() != Some(&(p.clone(), staged)) {
                c.opts.selected.clear();
            }
            c.selected = Some((p, staged));
            c.diff_open = true;
            c.diff = None;
            c.load_diff(ctx);
        }
        Some(Act::Discard(p, kind)) => c.confirm = Some(Discard::File(p, kind == ChangeKind::Untracked)),
        Some(Act::DiscardAll) => {
            if let Some(Ok(s)) = &c.status {
                let untracked = s.unstaged.iter().filter(|e| e.kind == ChangeKind::Untracked).count();
                let tracked = s.unstaged.iter().filter(|e| !matches!(e.kind, ChangeKind::Untracked | ChangeKind::Conflicted)).count();
                c.confirm = Some(Discard::All { tracked, untracked });
            }
        }
        Some(Act::OpenResolver(p)) => c.resolve_request = Some(p),
        Some(Act::RebaseContinue) => c.run_op(ctx, |r| rebase_step(rebase::continue_(r))),
        Some(Act::RebaseSkip) => c.run_op(ctx, |r| rebase_step(rebase::skip(r))),
        Some(Act::ConfirmAbort(v)) => c.confirm_abort = v,
        Some(Act::Abort) => c.run_op(ctx, |r| merge::abort(r).map(|_| OpDone::Staged)),
        Some(Act::Resolve(p, side)) => {
            c.run_op(ctx, move |r| merge::resolve(r, &p, side).map(|_| OpDone::Staged))
        }
        Some(Act::Stage(paths)) => c.run_op(ctx, move |r| status::stage(r, &paths).map(|_| OpDone::Staged)),
        Some(Act::Unstage(paths)) => c.run_op(ctx, move |r| status::unstage(r, &paths).map(|_| OpDone::Staged)),
        Some(Act::Commit) => {
            let msg = match c.description.trim() {
                "" => c.summary.trim().to_owned(),
                d => format!("{}\n\n{d}", c.summary.trim()),
            };
            let amend = c.amend;
            c.amend = false;
            c.run_op(ctx, move |r| {
                let before = undo::snapshot(r).ok();
                if amend {
                    status::amend(r, &msg)?;
                } else {
                    status::commit(r, &msg)?;
                }
                let after = undo::snapshot(r).ok();
                let entry = before
                    .zip(after)
                    .map(|(before, after)| UndoEntry {
                        label: if amend { "Amend" } else { "Commit" },
                        soft: true,
                        before,
                        after,
                    });
                Ok::<OpDone, gitgoing_core::GitError>(OpDone::Committed(entry))
            });
        }
        None => {}
    }
    // Confirmation for destructive discards.
    let mut decision = None;
    if let Some(d) = &c.confirm {
        let text = match d {
            Discard::File(p, true) => format!("Delete the new file '{p}'? It is not tracked, so it cannot be recovered."),
            Discard::File(p, false) => format!("Discard all unstaged changes to '{p}'? This cannot be undone."),
            Discard::All { tracked, untracked } => format!(
                "Discard unstaged changes to {tracked} file(s) and DELETE {untracked} new file(s)? This cannot be undone."
            ),
            Discard::Hunk(p, _) => format!("Discard this hunk of '{p}'? This cannot be undone."),
        };
        egui::Window::new("Discard changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(text);
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Discard").color(Color32::from_rgb(0xe5, 0x53, 0x4b))).clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
    }
    if let Some(go) = decision {
        c.opts.selected.clear();
        let d = c.confirm.take();
        if go {
            match d {
                Some(Discard::File(p, _)) => c.run_op(ctx, move |r| status::discard_file(r, &p).map(|_| OpDone::Staged)),
                Some(Discard::All { .. }) => c.run_op(ctx, |r| status::discard_all(r, true).map(|_| OpDone::Staged)),
                Some(Discard::Hunk(p, h)) => c.run_op(ctx, move |r| status::discard_hunk(r, &p, h).map(|_| OpDone::Staged)),
                None => {}
            }
        }
    }
    close
}

/// Centre pane showing the selected file's diff. Returns `true` to close it.
pub fn diff_pane(ui: &mut egui::Ui, c: &mut Changes) -> bool {
    let ctx = ui.ctx().clone();
    let (path, staged) = c.selected.clone().unwrap_or_default();
    // Hunks can be (un)staged for modified files; new, deleted and conflicted
    // files are staged as a whole with the +/- buttons.
    let kind = match &c.status {
        Some(Ok(s)) => {
            let list = if staged { &s.staged } else { &s.unstaged };
            list.iter().find(|e| e.path == path).map(|e| e.kind)
        }
        _ => None,
    };
    let hunk_btn = (kind == Some(ChangeKind::Modified) && c.op_rx.is_none())
        .then_some(if staged { HunkBtn::Unstage } else { HunkBtn::Stage });
    let out = diffview::show(
        ui,
        &mut c.opts,
        &c.diff,
        c.diff_rx.is_some(),
        &diffview::Header {
            title: &path,
            tag: if staged { "Staged" } else { "Unstaged" },
            kind,
            edit_label: (kind != Some(ChangeKind::Deleted))
                .then_some(if staged { "Edit in working directory" } else { "Edit this file" }),
            hunk_btn,
        },
    );
    if out.refetch {
        c.load_diff(&ctx);
    }
    if out.edit {
        if let Err(e) = gitgoing_core::system::open_file(&c.repo.join(&path)) {
            c.error = Some(format!("could not open {path}: {e}"));
        }
    }
    if let Some(h) = out.discard_hunk {
        c.confirm = Some(Discard::Hunk(path.clone(), h));
    }
    if let Some(ids) = out.lines {
        c.opts.selected.clear(); // hunks are renumbered once lines are staged
        let p = path.clone();
        c.run_op(&ctx, move |r| {
            if staged { status::unstage_lines(r, &p, &ids) } else { status::stage_lines(r, &p, &ids) }
                .map(|_| OpDone::Staged)
        });
    }
    if let Some(h) = out.hunk {
        c.opts.selected.clear();
        c.run_op(&ctx, move |r| {
            if staged { status::unstage_hunk(r, &path, h) } else { status::stage_hunk(r, &path, h) }
                .map(|_| OpDone::Staged)
        });
    }
    out.close
}

/// One of the two file lists (unstaged or staged).
struct FileList<'a> {
    title: &'a str,
    entries: &'a [StatusEntry],
    staged: bool,
}

fn file_list(
    ui: &mut egui::Ui,
    list: FileList,
    selected: &Option<(String, bool)>,
    busy: bool,
    mode: ViewMode,
    act: &mut Option<Act>,
) {
    let FileList { title, entries, staged } = list;
    ui.horizontal(|ui| {
        ui.strong(format!("{title} ({})", entries.len()));
        if !staged {
            let any = entries.iter().any(|e| e.kind != ChangeKind::Conflicted);
            if ui
                .add_enabled(!busy && any, egui::Button::new("Discard all").small())
                .on_hover_text("Throw away all unstaged changes")
                .clicked()
            {
                *act = Some(Act::DiscardAll);
            }
        }
        let label = if staged { "Unstage all" } else { "Stage all" };
        if ui.add_enabled(!busy && !entries.is_empty(), egui::Button::new(label).small()).clicked() {
            let all = entries.iter().map(|e| e.path.clone()).collect();
            *act = Some(if staged { Act::Unstage(all) } else { Act::Stage(all) });
        }
    });
    let mut row = |ui: &mut egui::Ui, e: &StatusEntry, name: &str| {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new(if staged { "-" } else { "+" }).small())
                .on_hover_text(match (staged, e.kind) {
                    (true, _) => "Unstage",
                    (false, ChangeKind::Conflicted) => "Mark resolved (keep the file as it is now)",
                    (false, _) => "Stage",
                })
                .clicked()
            {
                let p = vec![e.path.clone()];
                *act = Some(if staged { Act::Unstage(p) } else { Act::Stage(p) });
            }
            if e.kind == ChangeKind::Conflicted {
                if ui
                    .add_enabled(!busy, egui::Button::new("Resolve…").small())
                    .on_hover_text("Open the conflict editor")
                    .clicked()
                {
                    *act = Some(Act::OpenResolver(e.path.clone()));
                }
                for (label, side, tip) in [
                    ("Ours", Side::Ours, "Keep the current branch's version"),
                    ("Theirs", Side::Theirs, "Take the incoming version"),
                ] {
                    if ui.add_enabled(!busy, egui::Button::new(label).small()).on_hover_text(tip).clicked() {
                        *act = Some(Act::Resolve(e.path.clone(), side));
                    }
                }
            }
            icons::change_badge(ui, e.kind);
            if !staged && e.kind != ChangeKind::Conflicted
                && icons::small_icon_button(ui, icons::Icon::Trash, "Discard changes", !busy).clicked()
            {
                *act = Some(Act::Discard(e.path.clone(), e.kind));
            }
            let sel = selected.as_ref().is_some_and(|(p, s)| *p == e.path && *s == staged);
            let label = ui.selectable_label(sel, name).on_hover_text(&e.path);
            if label.clicked() {
                *act = Some(Act::Select(e.path.clone(), staged));
            }
            if label.double_clicked() && e.kind == ChangeKind::Conflicted {
                *act = Some(Act::OpenResolver(e.path.clone()));
            }
        });
    };
    match mode {
        ViewMode::Tree => {
            let items: Vec<(&str, usize)> = entries.iter().enumerate().map(|(i, e)| (e.path.as_str(), i)).collect();
            filetree::show(ui, title, &items, &mut |ui, i, name| row(ui, &entries[i], name));
        }
        ViewMode::Path => {
            for e in entries {
                row(ui, e, &e.path);
            }
        }
        ViewMode::File => {
            for e in entries {
                row(ui, e, filetree::file_name(&e.path));
            }
        }
    }
}
