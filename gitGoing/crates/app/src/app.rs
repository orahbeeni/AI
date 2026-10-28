use crate::about::AboutWindow;
use crate::assets::{self, Assets};
use crate::changes::{self, Changes};
use crate::clonedlg;
use crate::comparewin::CompareWin;
use crate::conflictwin::{self, ConflictEditor};
use crate::consolewin::Console;
use crate::settings::{Settings, SettingsWindow};
use crate::startup::{self, WindowGeom};
use crate::detail::{self, Detail};
use crate::filemenu::FileRequest;
use crate::icons::{self, Icon};
use crate::intro::Intro;
use crate::sidebar::{SidebarAction, Sidebar};
use crate::timefmt;
use crate::toasts::Toasts;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use gitgoing_core::branches;
use gitgoing_core::commit_ops::{self, Applied, ResetMode};
use gitgoing_core::merge::{self, MergeOutcome};
use gitgoing_core::undo::{self, UndoEntry};
use gitgoing_core::remote::{self, PullMode};
use gitgoing_core::rebase::{self, Action as RebaseAction, TodoItem};
use gitgoing_core::remotes;
use gitgoing_core::changesets;
use gitgoing_core::stash;
use gitgoing_core::watch::Watch;
use gitgoing_core::graph::{SegKind, PALETTE_SIZE};
use gitgoing_core::history::{self, CommitRow, LoadEvent, RefKind, RepoInfo};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

mod dialogs;
mod filedlgs;
mod graphview;
mod toolbar;
use graphview::{history_view, multi_panel};

struct Loaded {
    info: Option<RepoInfo>,
    sidebar: Option<Sidebar>,
    rows: Vec<CommitRow>,
    max_cols: u16,
    loading: bool,
    rx: Receiver<LoadEvent>,
    /// Selected commit (index into `rows`).
    selected: Option<usize>,
    /// Several commits selected with Cmd/Ctrl- or Shift-click (indices into `rows`).
    multi: std::collections::BTreeSet<usize>,
    /// Where a Shift-click range starts.
    anchor: Option<usize>,
    /// Hidden branches/tags this history was loaded without (their chips are not drawn).
    hidden: BTreeSet<String>,
    detail: Option<Detail>,
    changes: Option<Changes>,
    /// The WIP row is selected and the right panel shows working changes.
    show_changes: bool,
    search: Search,
    /// Display row to bring into view on the next frame, and whether to centre it.
    scroll_to: Option<(usize, bool)>,
    /// Row of the HEAD commit, once it has been loaded (where the WIP row attaches).
    head_row: Option<usize>,
    /// A commit to select again once the history has reloaded (after an operation).
    restore: Option<gitgoing_core::Oid>,
    /// The Changes panel was opened automatically for the current merge/rebase.
    auto_opened: bool,
    /// Display rows that were visible last frame.
    visible: std::ops::Range<usize>,
}

/// Incremental commit search (message, author, or SHA prefix).
#[derive(Default)]
struct Search {
    text: String,
    /// Text and row count the current `matches` were computed for.
    scanned_text: String,
    scanned_rows: usize,
    /// Indices into `Loaded::rows`, ascending.
    matches: Vec<u32>,
    /// Position in `matches` of the current result.
    cursor: usize,
    /// Show only the matching commits in the list.
    only: bool,
    /// Whether the list was filtered last frame, to notice when that changes.
    was_filtering: bool,
}

impl Search {
    /// The list shows only the matches (a search is typed and "only matches" is ticked).
    fn filtering(&self) -> bool {
        self.only && !self.text.trim().is_empty()
    }

    /// Bring `matches` up to date with the text and any newly loaded rows.
    /// Returns true if the text changed (so the view should jump to a result).
    fn update(&mut self, rows: &[CommitRow]) -> bool {
        let changed = self.text != self.scanned_text;
        if changed {
            self.matches.clear();
            self.scanned_rows = 0;
            self.cursor = 0;
            self.scanned_text = self.text.clone();
        }
        let q = self.text.trim().to_lowercase();
        if q.is_empty() {
            self.scanned_rows = rows.len();
            return changed;
        }
        let hex = q.len() >= 4 && q.bytes().all(|b| b.is_ascii_hexdigit());
        for (i, r) in rows.iter().enumerate().skip(self.scanned_rows) {
            let hit = (hex && r.id.to_string().starts_with(&q))
                || r.summary.to_lowercase().contains(&q)
                || r.author.to_lowercase().contains(&q);
            if hit {
                self.matches.push(i as u32);
            }
        }
        self.scanned_rows = rows.len();
        changed
    }
}

impl Loaded {
    /// Ids of the multi-selected commits.
    fn multi_ids(&self) -> Vec<gitgoing_core::Oid> {
        self.multi.iter().filter_map(|&i| self.rows.get(i).map(|r| r.id)).collect()
    }

    fn has_wip(&self) -> bool {
        self.changes.as_ref().and_then(Changes::count).is_some_and(|n| n > 0)
    }

    fn diff_open(&self) -> bool {
        if self.show_changes {
            self.changes.as_ref().is_some_and(|c| c.diff_open)
        } else {
            self.detail.as_ref().is_some_and(|d| d.diff_open)
        }
    }

    fn select(&mut self, ctx: &egui::Context, idx: usize) {
        let Some(row) = self.rows.get(idx) else { return };
        self.selected = Some(idx);
        self.multi.clear();
        self.anchor = Some(idx);
        self.show_changes = false;
        if let Some(info) = &self.info {
            if self.detail.as_ref().map(Detail::id) != Some(row.id) {
                self.detail = Some(Detail::new(ctx, &info.path, row.id));
            }
        }
    }

    /// Scroll so commit `idx` is visible (centred if `center`).
    /// The list row a commit is shown in: its own row, or its place among the matches when filtering.
    fn display_of(&self, idx: usize) -> Option<usize> {
        if self.search.filtering() {
            self.search.matches.binary_search(&(idx as u32)).ok()
        } else {
            Some(idx + self.has_wip() as usize)
        }
    }

    fn reveal(&mut self, idx: usize, center: bool) {
        let Some(disp) = self.display_of(idx) else { return };
        let inside = disp > self.visible.start && disp + 1 < self.visible.end;
        if center || !inside {
            self.scroll_to = Some((disp, center));
        }
    }

    fn close_diff(&mut self) {
        if self.show_changes {
            if let Some(c) = &mut self.changes {
                c.diff_open = false;
            }
        } else if let Some(d) = &mut self.detail {
            d.diff_open = false;
        }
    }
}

/// What the "Stash changes" window will stash.
#[derive(Clone, PartialEq, Eq)]
enum StashScope {
    All,
    /// Only what is staged.
    Staged,
    /// The files picked in the Changes panel.
    Paths(Vec<String>),
}

/// The open "Stash changes" window.
struct StashDlg {
    scope: StashScope,
    message: String,
    include_untracked: bool,
    keep_index: bool,
    /// The message field has been given focus once.
    focused: bool,
}

impl StashDlg {
    fn new(scope: StashScope) -> Self {
        StashDlg { scope, message: String::new(), include_untracked: true, keep_index: false, focused: false }
    }
}

/// The window to save the current changes as a named set, or to rename one.
struct ChangeSetDlg {
    /// Renaming this set (reference name) instead of saving.
    rename: Option<String>,
    name: String,
    include_untracked: bool,
    /// Names of the saved sets, offered for updating.
    existing: Vec<String>,
    focused: bool,
}

enum StashOp {
    Save,
    Apply(usize),
    Pop(usize),
}

enum Confirm {
    DeleteBranch(String),
    DropStash(usize),
    /// Delete a saved change set: (reference name, display name).
    DeleteChangeSet(String, String),
    /// Replace a saved change set's contents with the current changes (display name).
    UpdateChangeSet(String),
    DeleteTag(String),
    HardReset(gitgoing_core::Oid),
    DeleteRemoteBranch(String),
    RemoveRemote(String),
    DeleteRemoteTag(String),
    DeleteFile(String),
}

/// The add / edit-URL / rename remote window.
struct RemoteDlg {
    kind: RemoteDlgKind,
    /// Remote name (new name when adding).
    name: String,
    /// URL when adding or editing; new name when renaming.
    text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RemoteDlgKind {
    Add,
    EditUrl,
    Rename,
}

/// The window to add or edit a branch's local comment.
struct CommentDlg {
    /// Short branch name.
    branch: String,
    text: String,
}

/// The "squash commits" window: the commits and the message for the result.
struct SquashDlg {
    ids: Vec<gitgoing_core::Oid>,
    message: String,
    plan: rebase::SquashPlan,
    /// The user acknowledged the predicted conflicts.
    confirmed: bool,
}

/// A checkout to repeat once uncommitted changes are out of the way.
#[derive(Clone)]
enum Retry {
    Checkout(String),
    CheckoutRemote(String),
    Pull(PullMode),
}

/// Offered when a checkout or pull was stopped by uncommitted changes.
struct StashOffer {
    message: String,
    retry: Retry,
}

/// The force-push confirmation.
struct ForceDlg {
    branch: String,
    upstream: String,
    ahead: usize,
    behind: usize,
}

/// The result of a background planning task, ready to open its window.
enum PlanResult {
    Squash(Vec<gitgoing_core::Oid>, rebase::SquashPlan),
    Rebase(RebaseDlg),
}

/// What the multi-selection panel asked for.
enum MultiAction {
    Squash,
    Clear,
}

/// The interactive-rebase planner. `items` are listed newest first, like the graph.
struct RebaseDlg {
    from: gitgoing_core::Oid,
    items: Vec<TodoItem>,
    /// How many of these commits are already pushed.
    pushed: usize,
}

/// The "change commit date" window.
struct RedateDlg {
    id: gitgoing_core::Oid,
    plan: gitgoing_core::redate::Plan,
    /// The new date as typed (local time).
    text: String,
    author: bool,
    committer: bool,
    /// Delete every trace of the old commits from this repository afterwards.
    purge: bool,
}

/// The "set upstream" window for a local branch.
struct UpstreamDlg {
    local: String,
    choice: Option<String>,
    options: Vec<String>,
}

/// Right-click actions on a commit row.
enum CommitAction {
    CherryPick(gitgoing_core::Oid),
    Revert(gitgoing_core::Oid),
    Reset(gitgoing_core::Oid, ResetMode),
    CreateBranch(gitgoing_core::Oid),
    CreateTag(gitgoing_core::Oid),
    Checkout(gitgoing_core::Oid),
    CopySha(gitgoing_core::Oid),
    InteractiveRebase(gitgoing_core::Oid),
    ChangeDate(gitgoing_core::Oid),
    /// Squash the multi-selected commits.
    Squash,
}

/// Turn a rebase result into an operation result; stopping on conflicts is
/// reported as an error so the user is pointed at the Changes panel.
fn rebase_outcome(r: Result<rebase::Outcome, String>) -> Result<(), String> {
    match r? {
        rebase::Outcome::Done => Ok(()),
        rebase::Outcome::Conflicts => Err(
            "Rebase stopped on conflicts. Resolve them in Changes, then continue (or abort) the rebase.".to_owned(),
        ),
    }
}

/// The message for a merge that had nothing to do (not a failure).
const ALREADY_UP_TO_DATE: &str = "Already up to date";

/// Remember that a checkout failed because uncommitted changes were in the way.
fn note_blocked(flag: &AtomicBool, e: gitgoing_core::GitError) -> gitgoing_core::GitError {
    if branches::blocked_by_local_changes(&e) {
        flag.store(true, Ordering::Relaxed);
    }
    e
}

fn applied(op: &str, r: Result<Applied, gitgoing_core::GitError>) -> Result<(), String> {
    match r.map_err(detail::ErrorText::text)? {
        Applied::Committed => Ok(()),
        Applied::Conflicts(n) => Err(format!(
            "{op} stopped: {n} conflicted file(s). Resolve them in Changes, then commit (or abort)."
        )),
    }
}

type OpRx = Receiver<Result<Option<UndoEntry>, String>>;

/// An undo or redo in flight; its entry returns to the right stack on completion.
enum Pending {
    Undo(UndoEntry),
    Redo(UndoEntry),
}

#[derive(Default)]
struct ToolbarOut {
    open: Option<PathBuf>,
    remote: Option<RemoteOp>,
    /// Start a pull with this mode.
    pull: Option<PullMode>,
    stash: Option<StashOp>,
    undo: bool,
    redo: bool,
    /// Open the force-push confirmation.
    force_push: bool,
}
type RemoteFn = Box<dyn FnOnce(&std::path::Path) -> Result<(), String> + Send>;
type RemoteOp = (&'static str, RemoteFn);

pub struct RepoTab {
    id: u64,
    /// The path this tab was opened with.
    path: PathBuf,
    repo: Option<Loaded>,
    error: Option<String>,
    /// A branch operation running in the background.
    op_rx: Option<OpRx>,
    /// Last operation error, shown in the status bar.
    notice: Option<(String, bool)>,
    /// What the running operation is called, for the toolbar.
    op_label: &'static str,
    /// The local branch the running operation is checking out, if any.
    op_branch: Option<String>,
    /// Set by the running operation when it moved any reference (branch, remote-tracking branch, tag).
    refs_moved: Arc<AtomicBool>,
    /// Transient notifications in the bottom-left corner.
    toasts: Toasts,
    /// Text of the open "new branch" dialog.
    new_branch: Option<String>,
    /// Commit the new-branch dialog creates the branch at (HEAD if `None`).
    branch_at: Option<gitgoing_core::Oid>,
    /// (commit, name, message) of the open "new tag" dialog.
    new_tag: Option<(gitgoing_core::Oid, String, String)>,
    /// The open "change commit date" dialog.
    redate_dlg: Option<RedateDlg>,
    /// The running operation rewrote history for good: drop the undo and redo stacks when it ends.
    forget_undo: bool,
    remote_dlg: Option<RemoteDlg>,
    upstream_dlg: Option<UpstreamDlg>,
    rebase_dlg: Option<RebaseDlg>,
    squash_dlg: Option<SquashDlg>,
    force_dlg: Option<ForceDlg>,
    /// The last push was rejected because the branches diverged.
    push_rejected: bool,
    /// The status-bar "Force push…" button was clicked.
    force_requested: bool,
    /// A squash/rebase plan being computed in the background.
    plan_rx: Option<Receiver<Result<PlanResult, String>>>,
    /// Set by a checkout that failed because uncommitted changes were in the way.
    blocked: Arc<AtomicBool>,
    /// The checkout in flight, so it can be repeated after stashing.
    retry: Option<Retry>,
    stash_offer: Option<StashOffer>,
    stash_dlg: Option<StashDlg>,
    change_set_dlg: Option<ChangeSetDlg>,
    /// The history of some lines, opened from a diff.
    line_hist: Option<crate::linehist::LineHistWin>,
    line_hist_serial: u64,
    /// A failed operation to tell the user about in a window (the status bar is easy to miss).
    error_dlg: Option<(String, String)>,
    file_hist: Option<filedlgs::FileHistDlg>,
    blame: Option<filedlgs::BlameDlg>,
    edit: Option<filedlgs::EditDlg>,
    /// Slow external actions (diff tool) whose failure is reported when they end.
    file_bg: Vec<(&'static str, Receiver<Result<(), String>>)>,
    /// Branches and tags (full reference names) hidden from the graph.
    hidden: BTreeSet<String>,
    /// Pinned local branches (full reference names), in the order they were pinned.
    pinned: Vec<String>,
    /// Local comments on branches (full reference name -> text).
    comments: BTreeMap<String, String>,
    comment_dlg: Option<CommentDlg>,
    /// The previous sidebar, whose filter and layout the next one inherits.
    carry_sidebar: Option<Sidebar>,
    conflict: Option<ConflictEditor>,
    /// What the conflict editor reported last frame.
    conflict_outcome: Option<conflictwin::Outcome>,
    compare: Option<CompareWin>,
    confirm: Option<Confirm>,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    pending: Option<Pending>,
    /// Notices changes made outside the app; see [`Watch`].
    watch: Option<Watch>,
    /// Starting the watcher failed; don't retry every frame.
    watch_failed: bool,
}

impl RepoTab {
    fn new(id: u64, path: PathBuf, hidden: BTreeSet<String>, pinned: Vec<String>, comments: BTreeMap<String, String>) -> Self {
        let mut app = RepoTab {
            id,
            hidden,
            pinned,
            comments,
            comment_dlg: None,
            carry_sidebar: None,
            path: path.clone(),
            repo: None,
            error: None,
            op_rx: None,
            notice: None,
            op_label: "",
            op_branch: None,
            refs_moved: Arc::new(AtomicBool::new(true)),
            toasts: Toasts::default(),
            new_branch: None,
            branch_at: None,
            new_tag: None,
            redate_dlg: None,
            forget_undo: false,
            remote_dlg: None,
            upstream_dlg: None,
            rebase_dlg: None,
            squash_dlg: None,
            force_dlg: None,
            push_rejected: false,
            force_requested: false,
            plan_rx: None,
            blocked: Arc::new(AtomicBool::new(false)),
            retry: None,
            stash_offer: None,
            stash_dlg: None,
            change_set_dlg: None,
            line_hist: None,
            line_hist_serial: 0,
            error_dlg: None,
            file_hist: None,
            blame: None,
            edit: None,
            file_bg: Vec::new(),
            conflict: None,
            conflict_outcome: None,
            compare: None,
            confirm: None,
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
            watch: None,
            watch_failed: false,
        };
        app.open(path);
        app
    }

    fn title(&self) -> String {
        let p = self.repo_path().unwrap_or_else(|| self.path.clone());
        p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// Is this tab showing the repository at `path`? Compared in canonical form,
    /// since git reports working directories with a trailing slash and users may
    /// pass relative paths.
    fn is_at(&self, path: &std::path::Path) -> bool {
        let want = normalise(path);
        normalise(&self.path) == want || self.repo_path().is_some_and(|p| normalise(&p) == want)
    }

    fn open(&mut self, path: PathBuf) {
        self.open_with(path, None, false);
    }

    fn open_with(&mut self, path: PathBuf, changes: Option<Changes>, show_changes: bool) {
        let (tx, rx) = mpsc::channel();
        // Dropping the old `Loaded` drops its receiver, which cancels its
        // loader thread on the next send.
        self.repo = Some(Loaded {
            info: None,
            sidebar: None,
            rows: Vec::new(),
            max_cols: 1,
            loading: true,
            rx,
            selected: None,
            multi: Default::default(),
            anchor: None,
            hidden: self.hidden.clone(),
            detail: None,
            changes,
            show_changes,
            search: Search::default(),
            scroll_to: None,
            head_row: None,
            restore: None,
            auto_opened: false,
            visible: 0..0,
        });
        self.error = None;
        let opts = history::LoadOptions { hidden: self.hidden.clone() };
        std::thread::spawn(move || {
            let fail = tx.clone();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                history::load(&path, &opts, |ev| tx.send(ev).is_ok());
            }));
            if let Err(p) = r {
                let _ = fail.send(LoadEvent::Error(format!("internal error: {}", detail::panic_message(&*p))));
            }
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(l) = &mut self.repo else { return };
        if !l.loading {
            return;
        }
        loop {
            match l.rx.try_recv() {
                Ok(LoadEvent::Info(i)) => {
                    if l.changes.is_none() {
                        l.changes = Some(Changes::new(ctx, i.path.clone()));
                    }
                    let mut sidebar = Sidebar::from_info(&i);
                    if let Some(old) = self.carry_sidebar.take() {
                        sidebar.adopt_ui(old);
                    }
                    l.sidebar = Some(sidebar);
                    l.info = Some(i);
                }
                Ok(LoadEvent::Rows(mut r)) => {
                    for row in &r {
                        for s in &row.graph.segs {
                            l.max_cols = l.max_cols.max(s.from.max(s.to) + 1);
                        }
                    }
                    if l.head_row.is_none() {
                        let head = l.info.as_ref().and_then(|i| i.head_oid);
                        l.head_row = head.and_then(|h| r.iter().position(|row| row.id == h)).map(|k| l.rows.len() + k);
                    }
                    let base = l.rows.len();
                    let restored = l.restore.and_then(|want| r.iter().position(|row| row.id == want));
                    l.rows.append(&mut r);
                    if let Some(k) = restored {
                        l.restore = None;
                        l.select(ctx, base + k);
                        l.reveal(base + k, true);
                    }
                }
                Ok(LoadEvent::Done { .. }) => l.loading = false,
                Ok(LoadEvent::Error(e)) => {
                    l.loading = false;
                    self.error = Some(e);
                    self.repo = None;
                    return;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    l.loading = false;
                    break;
                }
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }

    fn repo_path(&self) -> Option<PathBuf> {
        self.repo.as_ref()?.info.as_ref().map(|i| i.path.clone())
    }

    fn run_op<E: detail::ErrorText>(
        &mut self,
        ctx: &egui::Context,
        label: &'static str,
        f: impl FnOnce(&std::path::Path) -> Result<(), E> + Send + 'static,
    ) {
        let Some(path) = self.repo_path() else { return };
        if self.op_rx.is_some() {
            // Starting a second operation would orphan the first one's result.
            self.notice = Some((format!("Wait for '{}' to finish first.", self.op_label), true));
            return;
        }
        self.notice = None;
        self.push_rejected = false;
        self.op_label = label;
        let moved = self.refs_moved.clone();
        // Snapshot refs around the operation so it can be undone if it moved any.
        self.op_rx = Some(detail::spawn(ctx, move || {
            let before = undo::snapshot(&path).ok();
            let fingerprint = remote::refs_fingerprint(&path);
            f(&path).map_err(detail::ErrorText::text)?;
            // If either side could not be read, assume something moved.
            moved.store(fingerprint.is_none() || fingerprint != remote::refs_fingerprint(&path), Ordering::Relaxed);
            let after = undo::snapshot(&path).ok();
            Ok::<_, String>(before.zip(after).filter(|(b, a)| b != a).map(|(before, after)| {
                UndoEntry { label, soft: false, before, after }
            }))
        }));
    }

    /// Pull; if uncommitted changes block it, offer to stash them first.
    fn start_pull(&mut self, ctx: &egui::Context, mode: PullMode) {
        if self.op_rx.is_some() {
            self.notice = Some((format!("Wait for '{}' to finish first.", self.op_label), true));
            return;
        }
        self.retry = Some(Retry::Pull(mode));
        self.blocked.store(false, Ordering::Relaxed);
        let flag = self.blocked.clone();
        self.run_op(ctx, "Pull", move |p| {
            remote::pull_with(p, mode).map_err(|e| {
                if remote::blocked_by_local_changes(&e) {
                    flag.store(true, Ordering::Relaxed);
                }
                e
            })
        });
    }

    /// Open the force-push confirmation for the current branch.
    fn open_force_push(&mut self) {
        let info = self.repo.as_ref().and_then(|l| l.info.as_ref());
        match info.and_then(|i| Some((i.head_branch.clone()?, i.upstream.clone()?))) {
            Some((branch, up)) => {
                self.force_dlg = Some(ForceDlg { branch, upstream: up.name, ahead: up.ahead, behind: up.behind })
            }
            None => {
                self.notice = Some(("This branch has no upstream yet; use a normal Push first.".to_owned(), true))
            }
        }
    }

    /// Validate the multi-selection and open the squash window.
    ///
    /// The check replays commits to predict conflicts, which can take a while on
    /// a long range, so it runs on a background thread.
    fn start_squash(&mut self, ctx: &egui::Context) {
        let Some(path) = self.repo_path() else { return };
        let ids = self.repo.as_ref().map(Loaded::multi_ids).unwrap_or_default();
        self.notice = None;
        self.plan_rx = Some(detail::spawn(ctx, move || {
            let plan = rebase::plan_squash(&path, &ids)?;
            Ok::<_, String>(PlanResult::Squash(ids, plan))
        }));
    }

    /// Load the commits for the interactive-rebase planner on a background thread.
    fn start_interactive_rebase(&mut self, ctx: &egui::Context, from: gitgoing_core::Oid) {
        let Some(path) = self.repo_path() else { return };
        self.notice = None;
        self.plan_rx = Some(detail::spawn(ctx, move || {
            let mut items = rebase::plan(&path, from)?;
            items.reverse(); // newest first, like the graph
            let pushed = rebase::pushed_count_from(&path, from);
            Ok::<_, String>(PlanResult::Rebase(RebaseDlg { from, items, pushed }))
        }));
    }

    /// Pick up a finished planning task and open its window.
    fn poll_plan(&mut self) {
        let Some(rx) = &self.plan_rx else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.plan_rx = None;
        match r {
            Ok(PlanResult::Squash(ids, plan)) => {
                self.squash_dlg = Some(SquashDlg { ids, message: plan.message.clone(), plan, confirmed: false })
            }
            Ok(PlanResult::Rebase(dlg)) => self.rebase_dlg = Some(dlg),
            Err(e) => self.notice = Some((e, true)),
        }
    }

    fn run_restore(&mut self, ctx: &egui::Context, is_undo: bool) {
        let Some(path) = self.repo_path() else { return };
        let Some(entry) = (if is_undo { self.undo.pop() } else { self.redo.pop() }) else { return };
        let verb = if is_undo { "Undo" } else { "Redo" };
        self.op_label = if is_undo { "Undo" } else { "Redo" };
        self.notice = None;
        let e = entry.clone();
        self.pending = Some(if is_undo { Pending::Undo(entry) } else { Pending::Redo(entry) });
        self.op_rx = Some(detail::spawn(ctx, move || {
            let (expected, target) = if is_undo { (&e.after, &e.before) } else { (&e.before, &e.after) };
            // Redoing a soft commit must not discard staged work.
            undo::restore(&path, expected, target, e.soft, !is_undo && e.soft)
                .map(|_| None)
                .map_err(|err| format!("{verb} {}: {}", e.label, err.message()))
        }));
    }

    /// Change which branches and tags the graph shows (reloads the history).
    fn set_hidden(&mut self, hidden: BTreeSet<String>, ctx: &egui::Context) {
        if hidden != self.hidden {
            self.hidden = hidden;
            self.reload(ctx);
        }
    }

    /// Re-read history, sidebar and status after the repo changed.
    fn reload(&mut self, ctx: &egui::Context) {
        let Some(path) = self.repo_path() else { return };
        if let Some(w) = &self.watch {
            w.sync_refs();
        }
        // Keep what the user was looking at: the selected commit and the search.
        let (selected, search, mut ch, show) = match self.repo.take() {
            Some(l) => {
                self.carry_sidebar = l.sidebar;
                (l.selected.and_then(|i| l.rows.get(i)).map(|r| r.id), (l.search.text, l.search.only), l.changes, l.show_changes)
            }
            None => (None, (String::new(), false), None, false),
        };
        if let Some(c) = &mut ch {
            c.refresh(ctx);
        }
        self.open_with(path, ch, show);
        if let Some(l) = &mut self.repo {
            l.restore = selected;
            (l.search.text, l.search.only) = search;
        }
    }

    /// Say that the operation that just finished worked: a toast for the network
    /// operations, a status-bar line for everything else.
    fn report_done(&mut self, ctx: &egui::Context) {
        let moved = self.refs_moved.load(Ordering::Relaxed);
        let info = self.repo.as_ref().and_then(|l| l.info.as_ref());
        let branch = info.and_then(|i| i.head_branch.clone());
        let upstream = info.and_then(|i| i.upstream.as_ref().map(|u| u.name.clone()));
        let (title, detail) = match (self.op_label, moved) {
            ("Fetch", true) => ("Fetched Successfully", None),
            ("Fetch", false) => ("Already Up-to-Date", Some("Nothing new to fetch.".to_owned())),
            ("Pull", true) => ("Pulled Successfully", None),
            ("Pull", false) => (
                "Already Up-to-Date",
                Some(match (branch, upstream) {
                    (Some(b), Some(u)) => format!("No merge necessary. Branch {u} was not moved, so {b} is unchanged."),
                    _ => "No merge necessary.".to_owned(),
                }),
            ),
            ("Push", true) => ("Pushed Successfully", branch.zip(upstream).map(|(b, u)| format!("{b} to {u}"))),
            ("Push", false) => ("Already Up-to-Date", Some("Everything is already on the remote.".to_owned())),
            ("Force push", _) => ("Force Pushed Successfully", branch.zip(upstream).map(|(b, u)| format!("{b} to {u}"))),
            ("Push tag", _) => ("Tag Pushed Successfully", None),
            _ => {
                self.notice = Some((format!("{} done", self.op_label), false));
                return;
            }
        };
        self.toasts.push(ctx, title, detail);
    }

    fn poll_op(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.op_rx else { return };
        if let Ok(r) = rx.try_recv() {
            self.op_rx = None;
            self.op_branch = None;
            let pending = self.pending.take();
            match r {
                Ok(entry) => {
                    match pending {
                        Some(Pending::Undo(e)) => self.redo.push(e),
                        Some(Pending::Redo(e)) => self.undo.push(e),
                        None if self.forget_undo => {
                            // History was rewritten for good: the old snapshots point at deleted commits.
                            self.forget_undo = false;
                            self.undo.clear();
                            self.redo.clear();
                        }
                        None => {
                            if let Some(e) = entry {
                                self.undo.push(e);
                                self.redo.clear();
                            }
                        }
                    }
                    self.report_done(ctx);
                }
                Err(e) => {
                    self.forget_undo = false;
                    // A failed undo/redo leaves the entry where it was.
                    match pending {
                        Some(Pending::Undo(en)) => self.undo.push(en),
                        Some(Pending::Redo(en)) => self.redo.push(en),
                        None => {}
                    }
                    let lower = e.to_lowercase();
                    self.push_rejected = self.op_label == "Push"
                        && ["rejected", "non-fast-forward", "fetch first", "stale info"].iter().any(|k| lower.contains(k));
                    if self.blocked.swap(false, Ordering::Relaxed) && self.retry.is_some() {
                        // Uncommitted changes are in the way: explain, and offer to stash them.
                        let retry = self.retry.take().expect("checked above");
                        self.stash_offer = Some(StashOffer { message: e.clone(), retry });
                    } else if e == ALREADY_UP_TO_DATE {
                        self.toasts.push(ctx, "Already Up-to-Date", Some("No merge necessary.".to_owned()));
                        self.reload(ctx);
                        return;
                    } else {
                        // Say so in a window: a red line in the status bar is easy to miss.
                        let title = if e.contains(" stopped") {
                            "Needs your attention".to_owned() // e.g. a merge that stopped on conflicts
                        } else {
                            format!("{} failed", self.op_label)
                        };
                        self.error_dlg = Some((title, e.clone()));
                    }
                    self.notice = Some((e, true));
                }
            }
            // Reload even on failure: a partial operation may have changed state.
            self.reload(ctx);
        }
    }

    /// Poll everything that completes in the background; runs for every tab.
    /// `regained` is true on the frame the window regains focus: working-tree
    /// status goes stale when files change outside the app, so re-read it.
    fn background(&mut self, ctx: &egui::Context, regained: bool) {
        self.poll(ctx);
        self.poll_op(ctx);
        self.poll_plan();
        self.poll_files();
        self.poll_watch(ctx);
        self.background_updates(ctx, regained);
    }

    /// React to changes made outside the app (an editor, a terminal).
    fn poll_watch(&mut self, ctx: &egui::Context) {
        if !gitgoing_core::watch::enabled() {
            self.watch = None;
            self.watch_failed = false;
            return;
        }
        if self.watch.is_none() && !self.watch_failed {
            let Some(path) = self.repo_path() else { return };
            let repaint = ctx.clone();
            self.watch = Watch::start(&path, move || repaint.request_repaint());
            self.watch_failed = self.watch.is_none();
        }
        let Some(w) = &self.watch else { return };
        // Our own operation or load is in flight and will reload by itself; the flags wait.
        if self.op_rx.is_some() || self.repo.as_ref().is_none_or(|l| l.loading) {
            return;
        }
        let changed = w.take();
        if changed.refs {
            self.reload(ctx);
        } else if changed.work {
            if let Some(c) = self.repo.as_mut().and_then(|l| l.changes.as_mut()) {
                c.refresh(ctx);
            }
        }
    }

    fn background_updates(&mut self, ctx: &egui::Context, regained: bool) {
        let mut reload = None;
        let mut file_req: Option<FileRequest> = None;
        let mut stash_req: Option<Vec<String>> = None;
        if let Some(l) = &mut self.repo {
            file_req = file_req.or_else(|| l.detail.as_mut().and_then(|d| d.file_request.take()));
            if let Some(c) = &mut l.changes {
                if regained {
                    c.refresh(ctx);
                }
                file_req = file_req.or(c.take_file_request());
                stash_req = c.take_stash_request();
                if let Some(path) = c.take_resolve_request() {
                    match ConflictEditor::open(c.repo(), &path) {
                        Ok(ed) => self.conflict = Some(ed),
                        Err(e) => self.notice = Some((e, true)),
                    }
                }
                // When a merge/rebase stops on conflicts, bring the Changes panel up
                // once so the files to resolve are visible.
                match c.operation() {
                    Some((_, n)) if n > 0 && !l.auto_opened => {
                        l.auto_opened = true;
                        l.show_changes = true;
                        l.detail = None;
                        l.selected = None;
                    }
                    None => l.auto_opened = false,
                    _ => {}
                }
                if let changes::Outcome::Committed(entry) = c.poll(ctx) {
                    reload = Some(c.repo().clone());
                    if let Some(e) = entry {
                        self.undo.push(e);
                        self.redo.clear();
                    }
                }
            }
            if let Some(d) = &mut l.detail {
                d.poll(ctx);
            }
        }
        if let Some(req) = file_req {
            self.file_action(ctx, req);
        }
        if let Some(paths) = stash_req {
            self.stash_dlg = Some(StashDlg::new(StashScope::Paths(paths)));
        }
        if reload.is_some() {
            self.reload(ctx);
        }
    }

}

impl RepoTab {
    /// Draw this tab. Returns a repository the user asked to open.
    fn ui(&mut self, ctx: &egui::Context, widths: &mut PanelWidths, settings: &Settings) -> Option<PathBuf> {
        let tab_id = self.id;
        let tb = self.toolbar(ctx, settings.pull_mode);
        let (to_open, mut stash_op) = (tb.open, tb.stash);
        if let Some((label, f)) = tb.remote {
            self.run_op(ctx, label, f);
        }
        if let Some(mode) = tb.pull {
            self.start_pull(ctx, mode);
        }
        if tb.force_push {
            self.open_force_push();
        }
        if tb.undo {
            self.run_restore(ctx, true);
        } else if tb.redo {
            self.run_restore(ctx, false);
        }
        self.status_bar(ctx);
        // The toasts sit just above the status bar; this is its top edge.
        let toast_bottom = ctx.available_rect().bottom();
        if std::mem::take(&mut self.force_requested) {
            self.open_force_push();
        }
        let mut branch_act = None;
        let mut multi_action = None;
        let repo_path = self.repo_path();
        if let Some(l) = &mut self.repo {
            // Panel widths are set explicitly (egui's own resizing snaps back to
            // the width of the widest content), with our own drag handles.
            let max_w = ctx.screen_rect().width() * 0.5;
            widths.sidebar = widths.sidebar.clamp(150.0, 500.0_f32.min(max_w));
            let id = egui::Id::new(("sidebar", tab_id));
            let r = egui::SidePanel::left(id)
                .resizable(false)
                .exact_width(widths.sidebar)
                .show(ctx, |ui| {
                    if let Some(s) = &mut l.sidebar {
                        s.busy_branch = self.op_rx.as_ref().and(self.op_branch.clone());
                        let head = l.info.as_ref().and_then(|i| i.head_branch.as_deref());
                        branch_act = s.show(ui, &self.hidden, &self.pinned, &self.comments, head);
                    }
                });
            let rect = r.response.rect;
            widths.sidebar += drag_handle(ctx, id.with("grip"), rect, rect.right());

            if l.show_changes || l.detail.is_some() || l.multi.len() > 1 {
                widths.right = widths.right.clamp(300.0, 900.0_f32.min(ctx.screen_rect().width() * 0.7));
                let id = egui::Id::new(("right", tab_id));
                let r = egui::SidePanel::right(id)
                    .resizable(false)
                    .exact_width(widths.right)
                    .show(ctx, |ui| {
                        if l.multi.len() > 1 {
                            multi_action = multi_panel(ui, l);
                        } else if l.show_changes {
                            if let Some(c) = &mut l.changes {
                                if changes::panel(ctx, ui, c) {
                                    l.show_changes = false;
                                }
                            }
                        } else if let Some(d) = &mut l.detail {
                            if detail::panel(ctx, ui, d) {
                                l.detail = None;
                                l.selected = None;
                            }
                        }
                    });
                let rect = r.response.rect;
                widths.right -= drag_handle(ctx, id.with("grip"), rect, rect.left());
            }
        }

        match multi_action {
            Some(MultiAction::Clear) => {
                if let Some(l) = &mut self.repo {
                    l.multi.clear();
                }
            }
            Some(MultiAction::Squash) => self.start_squash(ctx),
            None => {}
        }

        if self.op_rx.is_none() {
            match branch_act {
                Some(SidebarAction::Checkout(n)) => {
                    self.op_branch = Some(n.clone());
                    self.retry = Some(Retry::Checkout(n.clone()));
                    let flag = self.blocked.clone();
                    self.run_op(ctx, "Checkout", move |p| branches::checkout(p, &n).map_err(|e| note_blocked(&flag, e)))
                }
                Some(SidebarAction::CheckoutRemote(n)) => {
                    self.op_branch = n.split_once('/').map(|(_, b)| b.to_owned());
                    self.retry = Some(Retry::CheckoutRemote(n.clone()));
                    let flag = self.blocked.clone();
                    self.run_op(ctx, "Checkout", move |p| {
                        branches::checkout_remote(p, &n).map_err(|e| note_blocked(&flag, e))
                    })
                }
                Some(SidebarAction::DeleteTag(n)) => self.confirm = Some(Confirm::DeleteTag(n)),
                Some(SidebarAction::Delete(n)) => self.confirm = Some(Confirm::DeleteBranch(n)),
                Some(SidebarAction::Merge(n)) => self.run_op(ctx, "Merge", move |p| {
                    match merge::merge(p, &n).map_err(detail::ErrorText::text)? {
                        MergeOutcome::UpToDate => Err(ALREADY_UP_TO_DATE.to_owned()),
                        MergeOutcome::Conflicts(n) => Err(format!(
                            "Merge stopped: {n} conflicted file(s). Resolve them in Changes, then commit (or abort the merge)."
                        )),
                        MergeOutcome::FastForward | MergeOutcome::Merged => Ok(()),
                    }
                }),
                Some(SidebarAction::Compare(other_ref, other)) => {
                    let head = self.repo.as_ref().and_then(|l| l.info.as_ref()).and_then(|i| i.head_branch.clone());
                    let (base_ref, base) = match head {
                        Some(h) => (format!("refs/heads/{h}"), h),
                        None => ("HEAD".to_owned(), "HEAD".to_owned()),
                    };
                    self.compare = Some(CompareWin::new(self.path.clone(), base_ref, base, other_ref, other));
                }
                Some(SidebarAction::RebaseOnto(n)) => self.run_op(ctx, "Rebase", move |p| rebase_outcome(rebase::rebase_onto(p, &n))),
                Some(SidebarAction::SetHidden(names, hide)) => {
                    let head_full = self
                        .repo
                        .as_ref()
                        .and_then(|l| l.info.as_ref())
                        .and_then(|i| i.head_branch.as_ref())
                        .map(|h| format!("refs/heads/{h}"));
                    let mut set = self.hidden.clone();
                    for n in names.into_iter().filter(|n| Some(n) != head_full.as_ref()) {
                        if hide {
                            set.insert(n);
                        } else {
                            set.remove(&n);
                        }
                    }
                    self.set_hidden(set, ctx);
                }
                Some(SidebarAction::Solo(only)) => {
                    let everything: BTreeSet<String> = self
                        .repo
                        .as_ref()
                        .and_then(|l| l.info.as_ref())
                        .map(|i| i.refs.values().flatten().map(history::RefLabel::full_name).collect())
                        .unwrap_or_default();
                    let set = everything.into_iter().filter(|n| *n != only).collect();
                    self.set_hidden(set, ctx);
                }
                Some(SidebarAction::ShowAll) => self.set_hidden(BTreeSet::new(), ctx),
                Some(SidebarAction::SetPinned(full, pin)) => {
                    if pin {
                        if !self.pinned.contains(&full) {
                            self.pinned.push(full);
                        }
                    } else {
                        self.pinned.retain(|p| *p != full);
                    }
                }
                Some(SidebarAction::MovePinned(full, delta)) => move_in_list(&mut self.pinned, &full, delta),
                Some(SidebarAction::ReorderPinned(from, to)) => reorder_in_list(&mut self.pinned, &from, &to),
                Some(SidebarAction::EditComment(branch)) => {
                    let text = self.comments.get(&format!("refs/heads/{branch}")).cloned().unwrap_or_default();
                    self.comment_dlg = Some(CommentDlg { branch, text });
                }
                Some(SidebarAction::AddRemote) => {
                    self.remote_dlg = Some(RemoteDlg { kind: RemoteDlgKind::Add, name: "origin".into(), text: String::new() })
                }
                Some(SidebarAction::EditRemote(n)) => {
                    let url = self
                        .repo
                        .as_ref()
                        .and_then(|l| l.info.as_ref())
                        .and_then(|i| i.remotes.iter().find(|r| r.name == n))
                        .map(|r| r.url.clone())
                        .unwrap_or_default();
                    self.remote_dlg = Some(RemoteDlg { kind: RemoteDlgKind::EditUrl, name: n, text: url });
                }
                Some(SidebarAction::RenameRemote(n)) => {
                    self.remote_dlg = Some(RemoteDlg { kind: RemoteDlgKind::Rename, text: n.clone(), name: n })
                }
                Some(SidebarAction::RemoveRemote(n)) => self.confirm = Some(Confirm::RemoveRemote(n)),
                Some(SidebarAction::FetchRemote(n)) => {
                    self.run_op(ctx, "Fetch", move |p| remotes::fetch_remote(p, &n))
                }
                Some(SidebarAction::FetchBranch(n, local)) => {
                    self.run_op(ctx, "Fetch", move |p| remotes::fetch_branch(p, &n, local))
                }
                Some(SidebarAction::PullBranch(n)) => {
                    let head = self.repo.as_ref().and_then(|l| l.info.as_ref()).and_then(|i| i.head_branch.clone());
                    if head.as_deref() == Some(n.as_str()) {
                        self.start_pull(ctx, settings.pull_mode);
                    } else {
                        self.run_op(ctx, "Pull", move |p| remotes::pull_other_branch(p, &n));
                    }
                }
                Some(SidebarAction::DeleteRemoteBranch(n)) => self.confirm = Some(Confirm::DeleteRemoteBranch(n)),
                Some(SidebarAction::SetUpstream(local)) => {
                    let options = self
                        .repo
                        .as_ref()
                        .and_then(|l| l.info.as_ref())
                        .map(|i| {
                            let mut v: Vec<String> = i
                                .refs
                                .values()
                                .flatten()
                                .filter(|l| l.kind == history::RefKind::Remote)
                                .map(|l| l.name.clone())
                                .collect();
                            v.sort();
                            v.dedup();
                            v
                        })
                        .unwrap_or_default();
                    self.upstream_dlg = Some(UpstreamDlg { local, choice: None, options });
                }
                Some(SidebarAction::PushTag(n)) => self.run_op(ctx, "Push tag", move |p| remotes::push_tag(p, &n)),
                Some(SidebarAction::DeleteRemoteTag(n)) => self.confirm = Some(Confirm::DeleteRemoteTag(n)),
                Some(SidebarAction::Goto(id)) => {
                    if let Some(l) = &mut self.repo {
                        if let Some(idx) = l.rows.iter().position(|r| r.id == id) {
                            l.select(ctx, idx);
                            l.reveal(idx, true);
                        }
                    }
                }
                Some(SidebarAction::ApplyStash(i)) => stash_op = Some(StashOp::Apply(i)),
                Some(SidebarAction::PopStash(i)) => stash_op = Some(StashOp::Pop(i)),
                Some(SidebarAction::DropStash(i)) => self.confirm = Some(Confirm::DropStash(i)),
                Some(SidebarAction::SaveChanges) => {
                    let existing = self.repo.as_ref().and_then(|l| l.info.as_ref()).map(|i| i.change_sets.iter().map(|c| c.name.clone()).collect()).unwrap_or_default();
                    self.change_set_dlg = Some(ChangeSetDlg { rename: None, name: String::new(), include_untracked: true, existing, focused: false });
                }
                Some(SidebarAction::RenameChangeSet(refname, name)) => {
                    self.change_set_dlg = Some(ChangeSetDlg { rename: Some(refname), name, include_untracked: true, existing: Vec::new(), focused: false });
                }
                Some(SidebarAction::ApplyChangeSet(refname)) => {
                    self.run_op(ctx, "Apply saved changes", move |p| changesets::apply(p, &refname))
                }
                Some(SidebarAction::UpdateChangeSet(name)) => self.confirm = Some(Confirm::UpdateChangeSet(name)),
                Some(SidebarAction::DeleteChangeSet(refname, name)) => self.confirm = Some(Confirm::DeleteChangeSet(refname, name)),
                Some(SidebarAction::ShowChangeSet(id)) => {
                    if let (Some(l), Some(path)) = (&mut self.repo, repo_path.as_ref()) {
                        l.show_changes = false;
                        l.selected = None;
                        l.detail = Some(Detail::new(ctx, path, id));
                    }
                }
                Some(SidebarAction::ShowStash(id)) => {
                    if let (Some(l), Some(path)) = (&mut self.repo, repo_path.as_ref()) {
                        l.show_changes = false;
                        l.selected = None;
                        l.detail = Some(Detail::new(ctx, path, id));
                    }
                }
                None => {}
            }
            match stash_op {
                Some(StashOp::Save) => self.stash_dlg = Some(StashDlg::new(StashScope::All)),
                Some(StashOp::Apply(i)) => self.run_op(ctx, "Apply stash", move |p| stash::apply(p, i)),
                Some(StashOp::Pop(i)) => self.run_op(ctx, "Pop stash", move |p| stash::pop(p, i)),
                None => {}
            }
        }
        self.dialogs(ctx);

        let mut commit_act = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(e) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            if let Some(ed) = self.conflict.as_mut().filter(|e| !e.popped()) {
                self.conflict_outcome = Some(ed.ui(ui));
                return;
            }
            let Some(l) = &mut self.repo else {
                if self.error.is_none() {
                    ui.label("Open a repository to begin.");
                }
                return;
            };
            if l.diff_open() {
                let close = if l.show_changes {
                    l.changes.as_mut().is_some_and(|c| changes::diff_pane(ui, c))
                } else {
                    l.detail.as_mut().is_some_and(|d| detail::diff_pane(ui, d))
                };
                if close {
                    l.close_diff();
                }
            } else {
                history_view(ui, l, tab_id, widths, &mut commit_act);
            }
        });
        // A diff asked for the history of some lines: open (or replace) the window.
        let req = self.repo.as_mut().and_then(|l| {
            l.changes.as_mut().and_then(|c| c.line_history.take()).or_else(|| l.detail.as_mut().and_then(|d| d.line_history.take()))
        });
        if let Some(r) = req {
            self.line_hist_serial += 1;
            self.line_hist = Some(crate::linehist::LineHistWin::new(ctx, r, self.line_hist_serial));
        }
        if let Some(w) = &mut self.line_hist {
            match w.show(ctx) {
                crate::linehist::Outcome::Open => {}
                crate::linehist::Outcome::Closed => self.line_hist = None,
                crate::linehist::Outcome::ShowCommit(id) => {
                    if let (Some(path), Some(l)) = (self.repo_path(), self.repo.as_mut()) {
                        l.show_changes = false;
                        l.selected = None;
                        l.detail = Some(Detail::new(ctx, &path, id));
                    }
                }
            }
        }
        // The conflict editor is drawn in the centre pane (above) or in a window of its own.
        if let Some(ed) = self.conflict.as_mut().filter(|e| e.popped()) {
            self.conflict_outcome = Some(ed.window(ctx));
        }
        if let Some(outcome) = self.conflict_outcome.take() {
            match outcome {
                conflictwin::Outcome::Open => {}
                conflictwin::Outcome::Cancelled => self.conflict = None,
                conflictwin::Outcome::Resolved => {
                    self.conflict = None;
                    if let Some(c) = self.repo.as_mut().and_then(|l| l.changes.as_mut()) {
                        c.refresh(ctx);
                    }
                    self.notice = Some(("File resolved and staged".to_owned(), false));
                }
            }
        }

        // So is the branch compare window.
        if let Some(w) = &mut self.compare {
            if !w.show(ctx) {
                self.compare = None;
            }
        }

        if self.op_rx.is_none() {
            match commit_act {
                Some(CommitAction::CherryPick(id)) => {
                    self.run_op(ctx, "Cherry-pick", move |p| applied("Cherry-pick", commit_ops::cherry_pick(p, id)))
                }
                Some(CommitAction::Revert(id)) => {
                    self.run_op(ctx, "Revert", move |p| applied("Revert", commit_ops::revert(p, id)))
                }
                Some(CommitAction::Reset(id, ResetMode::Hard)) => self.confirm = Some(Confirm::HardReset(id)),
                Some(CommitAction::Reset(id, mode)) => {
                    self.run_op(ctx, "Reset", move |p| commit_ops::reset(p, id, mode))
                }
                Some(CommitAction::Checkout(id)) => {
                    self.run_op(ctx, "Checkout", move |p| commit_ops::checkout_detached(p, id))
                }
                Some(CommitAction::CreateBranch(id)) => {
                    self.new_branch = Some(String::new());
                    self.branch_at = Some(id);
                }
                Some(CommitAction::CreateTag(id)) => self.new_tag = Some((id, String::new(), String::new())),
                Some(CommitAction::CopySha(id)) => ctx.copy_text(id.to_string()),
                Some(CommitAction::Squash) => self.start_squash(ctx),
                Some(CommitAction::InteractiveRebase(id)) => self.start_interactive_rebase(ctx, id),
                Some(CommitAction::ChangeDate(id)) => self.open_redate(id),
                None => {}
            }
        }
        self.toasts.show(ctx, toast_bottom);
        to_open
    }
}

/// Per-repository lists of reference names, one `path<TAB>ref<TAB>ref...` line each.
/// Works for any collection of names (a set of hidden refs, an ordered list of pins).
fn serialise_lists<'a, I, N>(store: I) -> String
where
    I: IntoIterator<Item = (&'a PathBuf, N)>,
    N: IntoIterator<Item = &'a String>,
{
    store
        .into_iter()
        .map(|(p, names)| (p, names.into_iter().map(String::as_str).collect::<Vec<_>>()))
        .filter(|(_, names)| !names.is_empty())
        .map(|(p, names)| format!("{}\t{}", p.display(), names.join("\t")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_lists<C: FromIterator<String> + Default + PartialEq>(text: &str) -> HashMap<PathBuf, C> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let path = PathBuf::from(parts.next()?);
            let refs: C = parts.filter(|r| !r.is_empty()).map(str::to_owned).collect();
            (refs != C::default()).then_some((path, refs))
        })
        .collect()
}

/// Move `name` up (`delta` < 0) or down (> 0) one place in `list`, if it can move.
fn move_in_list(list: &mut [String], name: &str, delta: i32) {
    let Some(i) = list.iter().position(|n| n == name) else { return };
    let j = i as i64 + delta.signum() as i64;
    if (0..list.len() as i64).contains(&j) {
        list.swap(i, j as usize);
    }
}

/// Move `name` to the place of `target` in `list`: below it if it came from above, else above it.
fn reorder_in_list(list: &mut Vec<String>, name: &str, target: &str) {
    let (Some(i), Some(j)) = (list.iter().position(|n| n == name), list.iter().position(|n| n == target)) else { return };
    let item = list.remove(i);
    list.insert(j, item);
}

/// Per-repository branch comments, one `path<TAB>ref<TAB>comment` line each; the
/// comment has `\`, newlines and tabs escaped.
fn serialise_comments(store: &HashMap<PathBuf, BTreeMap<String, String>>) -> String {
    let mut lines = Vec::new();
    for (p, map) in store {
        for (r, c) in map {
            let esc = c.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "").replace('\t', "\\t");
            lines.push(format!("{}\t{r}\t{esc}", p.display()));
        }
    }
    lines.join("\n")
}

fn parse_comments(text: &str) -> HashMap<PathBuf, BTreeMap<String, String>> {
    let mut out: HashMap<PathBuf, BTreeMap<String, String>> = HashMap::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(p), Some(r), Some(c)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let mut un = String::new();
        let mut chars = c.chars();
        while let Some(ch) = chars.next() {
            if ch != '\\' {
                un.push(ch);
                continue;
            }
            match chars.next() {
                Some('n') => un.push('\n'),
                Some('t') => un.push('\t'),
                Some(o) => un.push(o),
                None => {}
            }
        }
        out.entry(PathBuf::from(p)).or_default().insert(r.to_owned(), un);
    }
    out
}

/// An absolute path without `.`/`..` or a trailing slash (unchanged if the path
/// does not exist).
fn normalise(path: &std::path::Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

const MAX_RECENT: usize = 10;

enum NewRepo {
    Open,
    Clone,
    Init,
}

/// Side panel widths, shared by all tabs and remembered between runs.
#[derive(Clone, Copy)]
struct PanelWidths {
    sidebar: f32,
    right: f32,
    /// BRANCH / TAG column in the history.
    ref_col: f32,
    /// GRAPH column; `None` fits the lanes automatically.
    graph_col: Option<f32>,
}

impl Default for PanelWidths {
    fn default() -> Self {
        PanelWidths { sidebar: 240.0, right: 420.0, ref_col: 190.0, graph_col: None }
    }
}

/// A thin drag handle on a panel's inner edge. Returns the horizontal drag delta.
fn drag_handle(ctx: &egui::Context, id: egui::Id, panel: Rect, edge_x: f32) -> f32 {
    let resp = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(edge_x - 3.0, panel.top()))
        .show(ctx, |ui| ui.allocate_exact_size(Vec2::new(6.0, panel.height()), Sense::drag()).1)
        .inner;
    if resp.hovered() || resp.dragged() {
        ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if resp.dragged() {
        resp.drag_delta().x
    } else {
        0.0
    }
}

pub struct App {
    tabs: Vec<RepoTab>,
    active: usize,
    next_id: u64,
    was_focused: bool,
    recent: Vec<PathBuf>,
    widths: PanelWidths,
    clone_dlg: Option<clonedlg::CloneDialog>,
    /// Hidden branches/tags per repository, remembered between runs.
    hidden_store: HashMap<PathBuf, BTreeSet<String>>,
    /// Pinned branches per repository, in pin order.
    pinned_store: HashMap<PathBuf, Vec<String>>,
    /// Branch comments per repository.
    comments_store: HashMap<PathBuf, BTreeMap<String, String>>,
    assets: Assets,
    /// Splash screen state: shown at start until `Some(start)` + a second or so.
    splash_on: bool,
    /// The window was last at this place; the borderless intro window moves there when it ends.
    saved_geom: Option<WindowGeom>,
    /// The window's current place, written out when the app saves.
    last_geom: Option<WindowGeom>,
    /// When the window became a normal one (until then it is the intro's borderless window).
    normal_since: Option<f64>,
    splash_start: Option<f64>,
    /// The intro animation, if the embedded bundle is valid, and its current frame.
    intro: Option<Intro>,
    intro_tex: Option<egui::TextureHandle>,
    intro_frame: Option<usize>,
    settings: Settings,
    settings_win: SettingsWindow,
    about_win: AboutWindow,
    console: Console,
    /// A problem to show the user (e.g. a failed init).
    app_error: Option<String>,
}

fn paths_from(s: Option<String>) -> Vec<PathBuf> {
    s.unwrap_or_default().lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        initial: Option<PathBuf>,
        resolve: Option<String>,
        console: bool,
        splash: bool,
        saved_geom: Option<WindowGeom>,
    ) -> Self {
        let assets = Assets::load(&cc.egui_ctx);
        assets::share_mark(&cc.egui_ctx, &assets.mark);
        let storage = cc.storage;
        let get = |k: &str| storage.and_then(|s| s.get_string(k));
        let settings = Settings::load(get);
        startup::set_intro_enabled(settings.show_intro);
        let mut app = App {
            tabs: Vec::new(),
            active: 0,
            next_id: 0,
            was_focused: true,
            recent: paths_from(get("recent")),
            clone_dlg: None,
            hidden_store: parse_lists(&get("hidden_refs").unwrap_or_default()),
            pinned_store: parse_lists(&get("pinned_refs").unwrap_or_default()),
            comments_store: parse_comments(&get("branch_comments").unwrap_or_default()),
            assets,
            splash_on: splash,
            saved_geom,
            last_geom: None,
            normal_since: None,
            splash_start: None,
            intro: Intro::embedded(),
            intro_tex: None,
            intro_frame: None,
            settings,
            settings_win: SettingsWindow::default(),
            about_win: AboutWindow::default(),
            console: Console::default(),
            app_error: None,
            widths: PanelWidths {
                sidebar: get("sidebar_w").and_then(|v| v.parse().ok()).unwrap_or(240.0),
                right: get("right_w").and_then(|v| v.parse().ok()).unwrap_or(420.0),
                ref_col: get("ref_col").and_then(|v| v.parse().ok()).unwrap_or(190.0),
                graph_col: get("graph_col").and_then(|v| v.parse().ok()),
            },
        };
        gitgoing_core::gitlog::init();
        let repaint = cc.egui_ctx.clone();
        gitgoing_core::gitlog::set_notifier(Box::new(move || repaint.request_repaint()));
        if console {
            app.console.toggle();
        }
        app.settings.apply(&cc.egui_ctx);
        for p in paths_from(get("tabs")).into_iter().filter(|p| p.exists()) {
            app.open_tab(p);
        }
        app.active = get("active").and_then(|a| a.parse().ok()).filter(|a| *a < app.tabs.len()).unwrap_or(0);
        if let (Some(file), Some(repo)) = (resolve, initial.as_ref()) {
            app.open_tab(repo.clone());
            if let Some(tab) = app.tabs.get_mut(app.active) {
                tab.conflict = ConflictEditor::open(repo, &file).ok();
            }
        }
        match initial {
            Some(p) => app.open_tab(p),
            None if app.tabs.is_empty() => {
                if let Some(cwd) = std::env::current_dir().ok().filter(|d| gitgoing_core::is_repo(d)) {
                    app.open_tab(cwd);
                }
            }
            None => {}
        }
        app
    }

    /// Open `path` in a tab, switching to an existing tab if it's already open.
    fn open_tab(&mut self, path: PathBuf) {
        let path = normalise(&path);
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path.clone());
        self.recent.truncate(MAX_RECENT);
        if let Some(i) = self.tabs.iter().position(|t| t.is_at(&path)) {
            self.active = i;
            return;
        }
        self.next_id += 1;
        let hidden = self.hidden_store.get(&path).cloned().unwrap_or_default();
        let pinned = self.pinned_store.get(&path).cloned().unwrap_or_default();
        let comments = self.comments_store.get(&path).cloned().unwrap_or_default();
        self.tabs.push(RepoTab::new(self.next_id, path, hidden, pinned, comments));
        self.active = self.tabs.len() - 1;
    }

    fn close_tab(&mut self, i: usize) {
        self.tabs.remove(i);
        if self.active > i || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    fn tab_bar(&mut self, ctx: &egui::Context) {
        let mut close = None;
        let mut action = None;
        let bar_fill = ctx.style().visuals.extreme_bg_color;
        let border = ctx.style().visuals.widgets.noninteractive.bg_stroke.color;
        egui::TopBottomPanel::top("tabs")
            .frame(
                egui::Frame::new()
                    .fill(bar_fill)
                    .inner_margin(egui::Margin { left: 8, right: 8, top: 5, bottom: 0 }),
            )
            .show(ctx, |ui| {
                let bar = ui.max_rect();
                let mut active_rect = None;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for (i, t) in self.tabs.iter().enumerate() {
                        let path = t.repo_path().unwrap_or_else(|| t.path.clone());
                        let (act, rect) = icons::tab_button(ui, &t.title(), &path.display().to_string(), i == self.active);
                        if i == self.active {
                            active_rect = Some(rect);
                        }
                        match act {
                            icons::TabAction::Select => self.active = i,
                            icons::TabAction::Close => close = Some(i),
                            icons::TabAction::None => {}
                        }
                    }
                    ui.menu_button(egui::RichText::new(" + ").size(17.0), |ui| {
                        if ui.button("Open repository…").clicked() {
                            action = Some(NewRepo::Open);
                            ui.close_menu();
                        }
                        if ui.button("Clone repository…").clicked() {
                            action = Some(NewRepo::Clone);
                            ui.close_menu();
                        }
                        if ui.button("Initialise a new repository…").clicked() {
                            action = Some(NewRepo::Init);
                            ui.close_menu();
                        }
                    })
                    .response
                    .on_hover_text("Open, clone or create a repository");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::icon_button(ui, Icon::Gear, self.settings_win.open, "Settings", true).clicked() {
                            self.settings_win.open = !self.settings_win.open;
                        }
                        ui.menu_button("Help", |ui| {
                            if ui.button("About gitGoing").clicked() {
                                self.about_win.open = true;
                                ui.close_menu();
                            }
                        });
                    });
                });
                // A line under the bar, broken where the selected tab joins the toolbar.
                let y = bar.bottom() + 0.5;
                let stroke = Stroke::new(1.0_f32, border);
                let painter = ui.painter();
                match active_rect {
                    Some(r) => {
                        painter.hline(bar.left() - 8.0..=r.left(), y, stroke);
                        painter.hline(r.right()..=bar.right() + 8.0, y, stroke);
                    }
                    None => {
                        painter.hline(bar.left() - 8.0..=bar.right() + 8.0, y, stroke);
                    }
                }
            });
        if let Some(i) = close {
            self.close_tab(i);
        }
        self.new_repo(action);
    }

    /// The start-up intro: the animation (or, if it can't be played, the logo)
    /// over everything until it ends. A click, Esc, Enter or Space skips it.
    fn splash(&mut self, ctx: &egui::Context) {
        const STILL_HOLD: f64 = 1.1;
        const FADE: f64 = 0.4;
        if !self.splash_on {
            return;
        }
        let now = ctx.input(|i| i.time);
        let t = now - *self.splash_start.get_or_insert(now);
        let total = self.intro.as_ref().map_or(STILL_HOLD + FADE, |i| i.duration());
        let keys = ctx.input(|i| {
            i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space)
        });
        if t >= total || keys {
            self.finish_splash(ctx);
            return;
        }
        // Fade the whole thing out over the last moments.
        let alpha = (((total - t) / FADE).clamp(0.0, 1.0) * 255.0) as u8;

        // Decode the frame for this moment (only when it changes).
        if let Some(intro) = &self.intro {
            let idx = intro.frame_at(t);
            if self.intro_frame != Some(idx) {
                if let Some(img) = intro.decode(idx) {
                    match &mut self.intro_tex {
                        Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                        None => self.intro_tex = Some(ctx.load_texture("gitgoing-intro", img, egui::TextureOptions::LINEAR)),
                    }
                    self.intro_frame = Some(idx);
                }
            }
        }

        let screen = ctx.screen_rect();
        let mut skip = false;
        egui::Area::new(egui::Id::new("intro"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                // Covers the window, so clicks don't reach what is underneath.
                let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                skip = resp.clicked();
                let p = ui.painter();
                p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, alpha));
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                let tint = Color32::from_white_alpha(alpha);
                match (&self.intro, &self.intro_tex) {
                    (Some(intro), Some(tex)) => {
                        // Fit the video inside the window, keeping its proportions.
                        let scale = (rect.width() / intro.width as f32).min(rect.height() / intro.height as f32);
                        let size = Vec2::new(intro.width as f32, intro.height as f32) * scale;
                        p.image(tex.id(), Rect::from_center_size(rect.center(), size), uv, tint);
                    }
                    _ => {
                        let side = (rect.width().min(rect.height()) * 0.5).clamp(160.0, 340.0);
                        p.image(self.assets.logo.id(), Rect::from_center_size(rect.center(), Vec2::splat(side)), uv, tint);
                    }
                }
                if t > 0.8 && self.intro.is_some() {
                    p.text(
                        rect.right_bottom() - Vec2::new(14.0, 12.0),
                        egui::Align2::RIGHT_BOTTOM,
                        "Click or press Esc to skip",
                        egui::FontId::proportional(12.0),
                        Color32::from_white_alpha(alpha / 3),
                    );
                }
            });
        if skip {
            self.finish_splash(ctx);
            return;
        }
        ctx.request_repaint();
    }

    /// End the intro: free its frame, and turn the borderless intro window into the
    /// normal one (title bar, resizable, last size and place).
    fn finish_splash(&mut self, ctx: &egui::Context) {
        self.splash_on = false;
        self.intro_tex = None;
        self.intro_frame = None;
        let size = self.saved_geom.map_or(startup::DEFAULT_SIZE, |g| g.size);
        let pos = self.saved_geom.and_then(|g| g.pos).unwrap_or_else(|| {
            // Centre on the screen.
            let monitor = ctx.input(|i| i.viewport().monitor_size).unwrap_or(Vec2::new(1440.0, 900.0));
            [((monitor.x - size[0]) / 2.0).max(0.0), ((monitor.y - size[1]) / 2.0).max(0.0)]
        });
        ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
        // winit on Wayland implements "not resizable" as min size == max size == the
        // current size, and overwrites its stored hints doing so; turning resizable
        // back on only re-applies those, leaving the window pinned to the splash size.
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(Vec2::ZERO));
        ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(Vec2::INFINITY));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(size[0], size[1])));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(pos[0], pos[1])));
        if self.saved_geom.is_some_and(|g| g.maximized) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        self.normal_since = Some(ctx.input(|i| i.time));
    }

    /// Remember where the window is, once it is a normal window again.
    fn track_window(&mut self, ctx: &egui::Context) {
        if self.splash_on {
            return;
        }
        let now = ctx.input(|i| i.time);
        let since = *self.normal_since.get_or_insert(now - 10.0); // normal from the start
        if now - since < 1.0 {
            return; // let the intro window finish resizing
        }
        let (outer, inner, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (v.outer_rect, v.inner_rect, v.maximized.unwrap_or(false))
        });
        if let (Some(outer), Some(inner)) = (outer, inner) {
            if !maximized {
                self.last_geom = Some(WindowGeom {
                    pos: Some([outer.min.x, outer.min.y]),
                    size: [inner.width(), inner.height()],
                    maximized: false,
                });
            } else if let Some(g) = &mut self.last_geom {
                g.maximized = true;
            }
        }
    }

    fn new_repo(&mut self, action: Option<NewRepo>) {
        match action {
            Some(NewRepo::Open) => {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.open_tab(p);
                }
            }
            Some(NewRepo::Clone) => self.clone_dlg = Some(clonedlg::CloneDialog::new()),
            Some(NewRepo::Init) => {
                if let Some(p) = rfd::FileDialog::new().set_title("Choose a folder to initialise").pick_folder() {
                    match gitgoing_core::clone::init(&p) {
                        Ok(()) => self.open_tab(p),
                        Err(e) => self.app_error = Some(format!("could not initialise {}: {}", p.display(), e.message())),
                    }
                }
            }
            None => {}
        }
    }

    fn welcome(&mut self, ctx: &egui::Context) {
        let mut open = None;
        let mut action = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.add(egui::Image::new(&self.assets.logo).fit_to_exact_size(Vec2::splat(190.0)));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Open a repository…").clicked() {
                        open = rfd::FileDialog::new().pick_folder();
                    }
                    if ui.button("Clone…").clicked() {
                        action = Some(NewRepo::Clone);
                    }
                    if ui.button("Init…").clicked() {
                        action = Some(NewRepo::Init);
                    }
                });
                if !self.recent.is_empty() {
                    ui.add_space(16.0);
                    ui.weak("Recent");
                    for p in &self.recent {
                        if ui.link(p.display().to_string()).clicked() {
                            open = Some(p.clone());
                        }
                    }
                }
            });
        });
        if let Some(p) = open {
            self.open_tab(p);
        }
        self.new_repo(action);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let regained = focused && !self.was_focused;
        self.was_focused = focused;
        for t in &mut self.tabs {
            t.background(ctx, regained);
        }
        self.splash(ctx);
        self.track_window(ctx);
        // The toolbar's Console button asks for the console via egui's data store.
        if ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("toggle_console"))).unwrap_or(false) {
            self.console.toggle();
        }
        self.console.show(ctx);
        self.tab_bar(ctx);
        self.about_win.show(ctx);
        if self.settings_win.show(ctx, &mut self.settings) {
            self.settings.apply(ctx);
            startup::set_intro_enabled(self.settings.show_intro);
        }
        if let Some(d) = &mut self.clone_dlg {
            match d.show(ctx) {
                clonedlg::Outcome::Keep => {}
                clonedlg::Outcome::Close => self.clone_dlg = None,
                clonedlg::Outcome::Open(p) => {
                    self.clone_dlg = None;
                    self.open_tab(p);
                }
            }
        }
        if let Some(e) = &self.app_error {
            let mut close = false;
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.colored_label(Color32::LIGHT_RED, e);
                    close = ui.button("OK").clicked();
                });
            if close {
                self.app_error = None;
            }
        }
        if self.tabs.is_empty() {
            self.welcome(ctx);
            return;
        }
        if let Some(p) = self.tabs[self.active].ui(ctx, &mut self.widths, &self.settings) {
            self.open_tab(p);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let join = |v: Vec<PathBuf>| v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
        storage.set_string("tabs", join(self.tabs.iter().map(|t| t.repo_path().unwrap_or_else(|| t.path.clone())).collect()));
        storage.set_string("active", self.active.to_string());
        storage.set_string("sidebar_w", self.widths.sidebar.to_string());
        storage.set_string("right_w", self.widths.right.to_string());
        storage.set_string("ref_col", self.widths.ref_col.to_string());
        storage.set_string("graph_col", self.widths.graph_col.map_or_else(String::new, |w| w.to_string()));
        storage.set_string("recent", join(self.recent.clone()));
        for t in &self.tabs {
            let key = normalise(&t.repo_path().unwrap_or_else(|| t.path.clone()));
            if t.hidden.is_empty() {
                self.hidden_store.remove(&key);
            } else {
                self.hidden_store.insert(key.clone(), t.hidden.clone());
            }
            if t.pinned.is_empty() {
                self.pinned_store.remove(&key);
            } else {
                self.pinned_store.insert(key.clone(), t.pinned.clone());
            }
            if t.comments.is_empty() {
                self.comments_store.remove(&key);
            } else {
                self.comments_store.insert(key, t.comments.clone());
            }
        }
        storage.set_string("hidden_refs", serialise_lists(self.hidden_store.iter().map(|(p, s)| (p, s.iter()))));
        storage.set_string("branch_comments", serialise_comments(&self.comments_store));
        storage.set_string("pinned_refs", serialise_lists(self.pinned_store.iter().map(|(p, s)| (p, s.iter()))));
        self.settings.save(|k, v| storage.set_string(k, v));
        if let Some(g) = &self.last_geom {
            startup::save_geom(g);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: u8, summary: &str, author: &str) -> CommitRow {
        let mut b = [0u8; 20];
        b[0] = n;
        CommitRow {
            id: gitgoing_core::Oid::from_bytes(&b).unwrap(),
            summary: summary.into(),
            author: author.into(),
            time: 0,
            graph: Default::default(),
        }
    }

    #[test]
    fn hidden_refs_survive_serialisation() {
        let mut store = HashMap::new();
        store.insert(PathBuf::from("/a/b"), ["refs/heads/x".to_owned(), "refs/tags/v1".to_owned()].into_iter().collect());
        store.insert(PathBuf::from("/c d"), ["refs/remotes/origin/y".to_owned()].into_iter().collect());
        store.insert(PathBuf::from("/empty"), BTreeSet::new());
        let text = serialise_lists(store.iter().map(|(p, s)| (p, s.iter())));
        let back: HashMap<PathBuf, BTreeSet<String>> = parse_lists(&text);
        assert_eq!(back.len(), 2, "empty sets are not stored");
        assert_eq!(back[&PathBuf::from("/a/b")].len(), 2);
        assert!(back[&PathBuf::from("/c d")].contains("refs/remotes/origin/y"));
        assert!(parse_lists::<BTreeSet<String>>("").is_empty());
        assert!(parse_lists::<BTreeSet<String>>("garbage-line-without-refs").is_empty());
    }

    #[test]
    fn pinned_lists_keep_their_order_and_can_be_moved() {
        let mut store: HashMap<PathBuf, Vec<String>> = HashMap::new();
        let pins = vec!["refs/heads/zeta".to_owned(), "refs/heads/alpha".to_owned(), "refs/heads/mid".to_owned()];
        store.insert(PathBuf::from("/repo"), pins.clone());
        let back: HashMap<PathBuf, Vec<String>> =
            parse_lists(&serialise_lists(store.iter().map(|(p, s)| (p, s.iter()))));
        assert_eq!(back[&PathBuf::from("/repo")], pins, "pin order is kept, not sorted");

        let mut list = pins;
        move_in_list(&mut list, "refs/heads/mid", -1);
        assert_eq!(list, ["refs/heads/zeta", "refs/heads/mid", "refs/heads/alpha"]);
        move_in_list(&mut list, "refs/heads/zeta", -1); // already first
        move_in_list(&mut list, "refs/heads/alpha", 1); // already last
        move_in_list(&mut list, "refs/heads/missing", 1);
        assert_eq!(list, ["refs/heads/zeta", "refs/heads/mid", "refs/heads/alpha"]);
    }

    #[test]
    fn pins_reorder_by_drag_and_comments_round_trip() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut l = names(&["a", "b", "c"]);
        reorder_in_list(&mut l, "a", "c"); // dragged down: lands after c
        assert_eq!(l, names(&["b", "c", "a"]));
        reorder_in_list(&mut l, "a", "b"); // dragged up: lands before b
        assert_eq!(l, names(&["a", "b", "c"]));
        reorder_in_list(&mut l, "a", "missing");
        assert_eq!(l, names(&["a", "b", "c"]));

        let mut store = HashMap::new();
        let mut m = BTreeMap::new();
        m.insert("refs/heads/x".to_owned(), "line one\n\ttabbed \\ back\\n".to_owned());
        store.insert(PathBuf::from("/repo"), m);
        assert_eq!(parse_comments(&serialise_comments(&store)), store);
    }

    #[test]
    fn paths_compare_equal_with_or_without_trailing_slash() {
        let dir = tempfile::tempdir().unwrap();
        let with_slash = PathBuf::from(format!("{}/", dir.path().display()));
        assert_eq!(normalise(&with_slash), normalise(dir.path()));
        // Missing paths are left as they are.
        let gone = dir.path().join("nope");
        assert_eq!(normalise(&gone), gone);
    }

    #[test]
    fn search_matches_message_author_and_sha_incrementally() {
        let mut rows = vec![row(0xab, "Fix login bug", "Ann"), row(2, "Add feature", "Bob")];
        let mut s = Search { text: "LOGIN".into(), ..Default::default() };
        assert!(s.update(&rows));
        assert_eq!(s.matches, vec![0]);

        // New rows are scanned without rescanning old ones.
        rows.push(row(3, "login again", "Cy"));
        assert!(!s.update(&rows));
        assert_eq!(s.matches, vec![0, 2]);

        s.text = "bob".into();
        s.update(&rows);
        assert_eq!(s.matches, vec![1]);

        s.text = "ab00".into();
        s.update(&rows);
        assert_eq!(s.matches, vec![0]);

        s.text.clear();
        s.update(&rows);
        assert!(s.matches.is_empty());
    }

    #[test]
    fn only_matches_filters_just_while_a_search_is_typed() {
        let mut s = Search { text: "bob".into(), ..Default::default() };
        assert!(!s.filtering(), "off until ticked");
        s.only = true;
        assert!(s.filtering());
        s.text = "  ".into();
        assert!(!s.filtering(), "an empty search shows every commit again");
    }
}
