//! Working-tree panel: stage/unstage files, view their diffs, and commit.

use crate::detail::{spawn, Fetch};
use crate::diffview::{self, DiffOpts, HunkBtn};
use crate::filemenu::{self, FileRequest};
use crate::filerows;
use crate::filetree::{self, ViewMode};
use crate::icons;
use eframe::egui::{self, Color32, Sense, Vec2};
use gitgoing_core::diff::{ChangeKind, FileDiff};
use gitgoing_core::ignore;
use gitgoing_core::merge::{self, Side};
use gitgoing_core::rebase;
use gitgoing_core::status::{self, StatusEntry, WorkStatus};
use gitgoing_core::undo::{self, UndoEntry};
use std::collections::HashSet;
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
    /// A file-menu choice for the app to carry out.
    file_request: Option<FileRequest>,
    /// The Unstaged and Staged sections are expanded.
    unstaged_open: bool,
    staged_open: bool,
    /// Share of the list area the Unstaged section gets when both are open.
    split: f32,
    /// Show files as a directory tree instead of a flat path list.
    mode: ViewMode,
    /// Files picked with shift/ctrl+click, all from one list (`multi_staged`), to
    /// stage or unstage together. Empty when only `selected` is in play.
    multi: HashSet<String>,
    multi_staged: bool,
    /// Where a shift+click range starts.
    anchor: Option<String>,
    /// The "Ignored files" window: the list once loaded.
    ignored_win: Option<IgnoredWin>,
    ignored_rx: Fetch<Vec<String>>,
    /// Files the user asked to stash; the app opens its stash window.
    stash_request: Option<Vec<String>>,
}

struct IgnoredWin {
    list: Option<Result<Vec<String>, String>>,
    filter: String,
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
            file_request: None,
            unstaged_open: true,
            staged_open: true,
            split: 0.6,
            mode: ViewMode::default(),
            multi: HashSet::new(),
            multi_staged: false,
            anchor: None,
            ignored_win: None,
            ignored_rx: None,
            stash_request: None,
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

    pub fn take_stash_request(&mut self) -> Option<Vec<String>> {
        self.stash_request.take()
    }

    pub fn take_file_request(&mut self) -> Option<FileRequest> {
        self.file_request.take()
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

    fn open_ignored(&mut self, ctx: &egui::Context) {
        let repo = self.repo.clone();
        self.ignored_win = Some(IgnoredWin { list: None, filter: String::new() });
        self.ignored_rx = Some(spawn(ctx, move || ignore::ignored(&repo)));
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
                    let list = if self.multi_staged { &s.staged } else { &s.unstaged };
                    self.multi.retain(|p| list.iter().any(|e| e.path == *p));
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
        if let Some(rx) = &self.ignored_rx {
            if let Ok(r) = rx.try_recv() {
                self.ignored_rx = None;
                if let Some(w) = &mut self.ignored_win {
                    w.list = Some(r);
                }
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
    File(FileRequest),
    RebaseContinue,
    RebaseSkip,
    Discard(String, ChangeKind),
    DiscardAll,
    Resolve(String, Side),
    Abort,
    ConfirmAbort(bool),
    Select(String, bool),
    /// Shift/ctrl+click: extend or toggle the multi-selection.
    Pick(String, bool, Pick),
    Stage(Vec<String>),
    Unstage(Vec<String>),
    /// Add these patterns to `.gitignore` (or, if `true`, to `.git/info/exclude`).
    Ignore(Vec<String>, bool),
    /// Stop tracking these files but keep them on disk.
    Untrack(Vec<String>),
    ShowIgnored,
    Stash(Vec<String>),
    Commit,
}

#[derive(Clone, Copy)]
enum Pick {
    Range,
    Toggle,
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
            ui.label(egui::RichText::new("Working changes").size(15.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::icon_button(ui, icons::Icon::Close, false, "Close", true).clicked() {
                    close = true;
                }
                if icons::icon_button(ui, icons::Icon::Fetch, false, "Refresh", true).clicked() {
                    c.refresh(ctx);
                }
                if ui.small_button("Ignored…").on_hover_text("Show the files git is ignoring").clicked() {
                    act = Some(Act::ShowIgnored);
                }
            });
        });
        ui.add_space(2.0);
        filetree::mode_toggle(ui, &mut c.mode);
        ui.add_space(6.0);
        let mode = c.mode;
        match &c.status {
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

                // Two sections whose headings stay put: each has its own scrolling list,
                // and clicking a heading expands or collapses its section.
                const HEADER_H: f32 = 28.0;
                const DIVIDER_H: f32 = 8.0;
                let both = c.unstaged_open && c.staged_open;
                let body_total = (ui.available_height() - 2.0 * HEADER_H - if both { DIVIDER_H } else { 0.0 }).max(0.0);
                let (h_unstaged, h_staged) = section_heights(body_total, c.unstaged_open, c.staged_open, c.split);

                let none = HashSet::new();
                let (m_un, m_st) = if c.multi_staged { (&none, &c.multi) } else { (&c.multi, &none) };
                // The files "Stage selected" acts on: the picked ones, else the one selected file.
                let one = |staged: bool| -> HashSet<String> {
                    sel.iter().filter(|(_, st)| *st == staged).map(|(p, _)| p.clone()).collect()
                };
                let (p_un, p_st) = (if m_un.is_empty() { one(false) } else { m_un.clone() }, if m_st.is_empty() { one(true) } else { m_st.clone() });
                let list = FileList { title: "Unstaged", entries: &s.unstaged, staged: false };
                if section_head(ui, &list, &p_un, Some(c.unstaged_open), busy, &mut act) {
                    c.unstaged_open = !c.unstaged_open;
                }
                if c.unstaged_open {
                    list_body(ui, h_unstaged, list, sel, m_un, busy, mode, &mut act);
                }

                if both {
                    // Drag to change how the height is shared.
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), DIVIDER_H), Sense::drag());
                    let active = resp.hovered() || resp.dragged();
                    if active {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    let stroke = if active {
                        ui.visuals().widgets.active.fg_stroke
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke
                    };
                    ui.painter().hline(rect.x_range(), rect.center().y, stroke);
                    if resp.dragged() && body_total > 1.0 {
                        c.split = (c.split + resp.drag_delta().y / body_total).clamp(0.12, 0.88);
                    }
                }

                let list = FileList { title: "Staged", entries: &s.staged, staged: true };
                if section_head(ui, &list, &p_st, Some(c.staged_open), busy, &mut act) {
                    c.staged_open = !c.staged_open;
                }
                if c.staged_open {
                    list_body(ui, h_staged, list, sel, m_st, busy, mode, &mut act);
                }
            }
        }
    });

    match act {
        Some(Act::Select(p, staged)) => {
            // Line picks belong to one file's diff.
            if c.selected.as_ref() != Some(&(p.clone(), staged)) {
                c.opts.selected.clear();
            }
            c.multi.clear();
            c.anchor = Some(p.clone());
            c.selected = Some((p, staged));
            c.diff_open = true;
            c.diff = None;
            c.load_diff(ctx);
        }
        Some(Act::Pick(p, staged, how)) => {
            // Conflicted files are resolved one by one, so they are never part of a pick.
            let order: Vec<String> = match &c.status {
                Some(Ok(s)) => {
                    let list = if staged { &s.staged } else { &s.unstaged };
                    list.iter().filter(|e| e.kind != ChangeKind::Conflicted).map(|e| e.path.clone()).collect()
                }
                _ => Vec::new(),
            };
            let current = c.selected.as_ref().filter(|(_, st)| *st == staged).map(|(f, _)| f.clone());
            if c.multi_staged != staged {
                c.multi.clear();
                c.anchor = None;
            }
            c.multi_staged = staged;
            if c.multi.is_empty() {
                if let Some(cur) = current.filter(|f| order.contains(f)) {
                    c.anchor.get_or_insert_with(|| cur.clone());
                    c.multi.insert(cur);
                }
            }
            match how {
                Pick::Range => {
                    let anchor = c.anchor.clone().unwrap_or_else(|| p.clone());
                    let (a, b) = (order.iter().position(|x| *x == anchor), order.iter().position(|x| *x == p));
                    if let (Some(a), Some(b)) = (a, b) {
                        c.multi = order[a.min(b)..=a.max(b)].iter().cloned().collect();
                    }
                }
                Pick::Toggle => {
                    if !c.multi.remove(&p) && order.contains(&p) {
                        c.multi.insert(p.clone());
                    }
                    c.anchor = Some(p.clone());
                }
            }
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
        Some(Act::File(r)) => c.file_request = Some(r),
        Some(Act::RebaseContinue) => c.run_op(ctx, |r| rebase_step(rebase::continue_(r))),
        Some(Act::RebaseSkip) => c.run_op(ctx, |r| rebase_step(rebase::skip(r))),
        Some(Act::ConfirmAbort(v)) => c.confirm_abort = v,
        Some(Act::Abort) => c.run_op(ctx, |r| merge::abort(r).map(|_| OpDone::Staged)),
        Some(Act::Resolve(p, side)) => {
            c.run_op(ctx, move |r| merge::resolve(r, &p, side).map(|_| OpDone::Staged))
        }
        Some(Act::Stage(paths)) => {
            c.multi.clear();
            c.run_op(ctx, move |r| status::stage(r, &paths).map(|_| OpDone::Staged));
        }
        Some(Act::Unstage(paths)) => {
            c.multi.clear();
            c.run_op(ctx, move |r| status::unstage(r, &paths).map(|_| OpDone::Staged));
        }
        Some(Act::Ignore(patterns, local)) => {
            c.multi.clear();
            c.run_op(ctx, move |r| ignore::add_patterns(r, &patterns, local).map(|_| OpDone::Staged));
        }
        Some(Act::Untrack(paths)) => {
            c.multi.clear();
            c.run_op(ctx, move |r| ignore::untrack(r, &paths).map(|_| OpDone::Staged));
        }
        Some(Act::ShowIgnored) => c.open_ignored(ctx),
        Some(Act::Stash(paths)) => c.stash_request = Some(paths),
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
    // The files git is ignoring.
    let mut close_ignored = false;
    if let Some(w) = &mut c.ignored_win {
        let mut open = true;
        egui::Window::new("Ignored files")
            .open(&mut open)
            .default_size([420.0, 380.0])
            .show(ctx, |ui| match &w.list {
                None => {
                    ui.spinner();
                }
                Some(Err(e)) => {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
                Some(Ok(list)) => {
                    ui.add(egui::TextEdit::singleline(&mut w.filter).hint_text("Filter").desired_width(f32::INFINITY));
                    let needle = w.filter.to_lowercase();
                    let shown: Vec<&String> = list.iter().filter(|p| p.to_lowercase().contains(&needle)).collect();
                    ui.weak(format!("{} of {} ignored; a folder ending in / is ignored as a whole", shown.len(), list.len()));
                    egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, 18.0, shown.len(), |ui, range| {
                        for p in &shown[range] {
                            ui.label(p.as_str());
                        }
                    });
                }
            });
        close_ignored = !open;
    }
    if close_ignored {
        c.ignored_win = None;
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
#[derive(Clone, Copy)]
struct FileList<'a> {
    title: &'a str,
    entries: &'a [StatusEntry],
    staged: bool,
}

/// How the list height is shared: an open section alone gets all of it; two
/// open sections split it by `split` (the Unstaged share); a closed one gets none.
fn section_heights(total: f32, unstaged_open: bool, staged_open: bool, split: f32) -> (f32, f32) {
    match (unstaged_open, staged_open) {
        (true, true) => (total * split, total * (1.0 - split)),
        (true, false) => (total, 0.0),
        (false, true) => (0.0, total),
        (false, false) => (0.0, 0.0),
    }
}

/// A section's heading (pinned above its list). Returns true if the heading was
/// clicked to expand or collapse the section.
fn section_head(
    ui: &mut egui::Ui,
    list: &FileList,
    multi: &HashSet<String>,
    open: Option<bool>,
    busy: bool,
    act: &mut Option<Act>,
) -> bool {
    let FileList { title, entries, staged } = *list;
    let picked: Vec<String> = entries
        .iter()
        .filter(|e| e.kind != ChangeKind::Conflicted && multi.contains(&e.path))
        .map(|e| e.path.clone())
        .collect();
    let any_discardable = entries.iter().any(|e| e.kind != ChangeKind::Conflicted);
    let clicks = filerows::section_header(
        ui,
        title,
        entries.len(),
        open,
        Some((if staged { "Unstage all" } else { "Stage all" }, !busy && !entries.is_empty())),
        // Shown while any file of this list is selected.
        (!picked.is_empty()).then_some((if staged { "Unstage selected" } else { "Stage selected" }, !busy)),
        (!staged).then_some((icons::Icon::Trash, "Discard all unstaged changes", !busy && any_discardable)),
    );
    if clicks.text {
        let all = entries.iter().map(|e| e.path.clone()).collect();
        *act = Some(if staged { Act::Unstage(all) } else { Act::Stage(all) });
    }
    if clicks.extra {
        *act = Some(if staged { Act::Unstage(picked) } else { Act::Stage(picked) });
    }
    if clicks.icon {
        *act = Some(Act::DiscardAll);
    }
    clicks.toggle
}

/// A section's list inside its own scroll area of the given height.
fn list_body(
    ui: &mut egui::Ui,
    height: f32,
    list: FileList,
    selected: &Option<(String, bool)>,
    multi: &HashSet<String>,
    busy: bool,
    mode: ViewMode,
    act: &mut Option<Act>,
) {
    ui.allocate_ui(Vec2::new(ui.available_width(), height), |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("changes_list", list.staged))
            .auto_shrink(false)
            .show(ui, |ui| file_rows(ui, list, selected, multi, busy, mode, act));
    });
}

fn file_rows(
    ui: &mut egui::Ui,
    list: FileList,
    selected: &Option<(String, bool)>,
    multi: &HashSet<String>,
    busy: bool,
    mode: ViewMode,
    act: &mut Option<Act>,
) {
    let FileList { title, entries, staged } = list;
    if entries.is_empty() {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.weak(if staged { "Nothing staged" } else { "No unstaged changes" });
        });
        return;
    }

    let warn = Color32::from_rgb(0xe5, 0xc0, 0x7b);
    let mut row = |ui: &mut egui::Ui, e: &StatusEntry, name: &str| {
        let in_multi = multi.len() > 1 && multi.contains(&e.path);
        let sel = selected.as_ref().is_some_and(|(p, s)| *p == e.path && *s == staged) || in_multi;
        // Stage / unstage acts on every picked file when this row is one of them.
        let targets = || -> Vec<String> {
            if in_multi {
                entries.iter().filter(|x| multi.contains(&x.path)).map(|x| x.path.clone()).collect()
            } else {
                vec![e.path.clone()]
            }
        };
        let many = if in_multi { format!(" {} selected files", multi.len()) } else { String::new() };
        let conflicted = e.kind == ChangeKind::Conflicted;
        let salt = format!("{}|{staged}", e.path);
        let (idle, active) = if conflicted { (96.0, 96.0) } else if staged { (8.0, 40.0) } else { (8.0, 66.0) };
        let r = filerows::file_row(ui, e.kind, name, &e.path, sel, idle, active);

        if conflicted {
            if filerows::pill(ui, &r, &salt, "Resolve…", !busy, warn) {
                *act = Some(Act::OpenResolver(e.path.clone()));
                return;
            }
        } else {
            let (stage_tip, unstage_tip) = (format!("Stage{many}"), format!("Unstage{many}"));
            let buttons: Vec<(icons::Icon, &str, bool)> = if staged {
                vec![(icons::Icon::Minus, unstage_tip.as_str(), !busy)]
            } else {
                vec![(icons::Icon::Plus, stage_tip.as_str(), !busy), (icons::Icon::Trash, "Discard changes…", !busy)]
            };
            match filerows::hover_buttons(ui, &r, sel, &salt, &buttons) {
                Some(0) => {
                    let p = targets();
                    *act = Some(if staged { Act::Unstage(p) } else { Act::Stage(p) });
                    return;
                }
                Some(_) => {
                    *act = Some(Act::Discard(e.path.clone(), e.kind));
                    return;
                }
                None => {}
            }
        }

        if r.resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            *act = Some(if m.shift {
                Act::Pick(e.path.clone(), staged, Pick::Range)
            } else if m.command {
                Act::Pick(e.path.clone(), staged, Pick::Toggle)
            } else {
                Act::Select(e.path.clone(), staged)
            });
        }
        if r.resp.double_clicked() && conflicted {
            *act = Some(Act::OpenResolver(e.path.clone()));
        }
        r.resp.context_menu(|ui| {
            if conflicted {
                if ui.button("Resolve in editor…").clicked() {
                    *act = Some(Act::OpenResolver(e.path.clone()));
                    ui.close_menu();
                }
                if ui.button("Use ours (keep the current branch's version)").clicked() {
                    *act = Some(Act::Resolve(e.path.clone(), Side::Ours));
                    ui.close_menu();
                }
                if ui.button("Use theirs (take the incoming version)").clicked() {
                    *act = Some(Act::Resolve(e.path.clone(), Side::Theirs));
                    ui.close_menu();
                }
                if ui.button("Mark resolved as it is now").clicked() {
                    *act = Some(Act::Stage(vec![e.path.clone()]));
                    ui.close_menu();
                }
            } else {
                let label = format!("{}{many}", if staged { "Unstage" } else { "Stage" });
                if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                    let p = targets();
                    *act = Some(if staged { Act::Unstage(p) } else { Act::Stage(p) });
                    ui.close_menu();
                }
                if !staged && ui.add_enabled(!busy, egui::Button::new("Discard changes…")).clicked() {
                    *act = Some(Act::Discard(e.path.clone(), e.kind));
                    ui.close_menu();
                }
                if ui.add_enabled(!busy, egui::Button::new(format!("Stash{many}…"))).clicked() {
                    *act = Some(Act::Stash(targets()));
                    ui.close_menu();
                }
            }
            if !conflicted && !staged && e.kind == ChangeKind::Untracked {
                ui.separator();
                for (local, title) in [(false, "Ignore"), (true, "Ignore on this computer only")] {
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.menu_button(title, |ui| {
                            for s in ignore::suggestions(&e.path) {
                                if ui.button(&s.label).on_hover_text(&s.pattern).clicked() {
                                    *act = Some(Act::Ignore(vec![s.pattern], local));
                                    ui.close_menu();
                                }
                            }
                            if in_multi && ui.button(format!("The{many}")).clicked() {
                                *act = Some(Act::Ignore(targets().iter().map(|p| ignore::exact_pattern(p)).collect(), local));
                                ui.close_menu();
                            }
                        });
                    });
                }
            } else if e.kind == ChangeKind::Modified {
                ui.separator();
                let label = format!("Stop tracking{many} (keep on disk)");
                if ui
                    .add_enabled(!busy, egui::Button::new(label))
                    .on_hover_text("Remove from git but keep the file; then ignore it so it stays out")
                    .clicked()
                {
                    *act = Some(Act::Untrack(targets()));
                    ui.close_menu();
                }
            }
            ui.separator();
            if let Some(action) = filemenu::items(ui, &e.path) {
                *act = Some(Act::File(FileRequest { action, path: e.path.clone(), commit: None, staged }));
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

#[cfg(test)]
mod tests {
    use super::section_heights;

    #[test]
    fn open_sections_share_the_height_and_a_closed_one_gives_it_up() {
        let (a, b) = section_heights(100.0, true, true, 0.6);
        assert!((a - 60.0).abs() < 1e-3 && (b - 40.0).abs() < 1e-3, "{a} {b}");
        assert_eq!(section_heights(100.0, true, false, 0.6), (100.0, 0.0));
        assert_eq!(section_heights(100.0, false, true, 0.6), (0.0, 100.0));
        assert_eq!(section_heights(100.0, false, false, 0.6), (0.0, 0.0));
    }
}
