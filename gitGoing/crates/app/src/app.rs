use crate::changes::{self, Changes};
use crate::clonedlg;
use crate::conflictwin::{self, ConflictEditor};
use crate::settings::{Settings, SettingsWindow};
use crate::detail::{self, Detail};
use crate::icons::{self, Icon};
use crate::sidebar::{SidebarAction, Sidebar};
use crate::timefmt;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use gitgoing_core::branches;
use gitgoing_core::commit_ops::{self, Applied, ResetMode};
use gitgoing_core::merge::{self, MergeOutcome};
use gitgoing_core::undo::{self, UndoEntry};
use gitgoing_core::remote::{self, PullMode};
use gitgoing_core::rebase::{self, Action as RebaseAction, TodoItem};
use gitgoing_core::remotes;
use gitgoing_core::stash;
use gitgoing_core::graph::{SegKind, PALETTE_SIZE};
use gitgoing_core::history::{self, CommitRow, LoadEvent, RefKind, RepoInfo};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

const ROW_H: f32 = 24.0;
const LANE_W: f32 = 16.0;
const MAX_GRAPH_COLS: u16 = 14;
const AUTHOR_W: f32 = 140.0;
const DATE_W: f32 = 100.0;
const HEADER_H: f32 = 22.0;

const PALETTE: [Color32; PALETTE_SIZE as usize] = [
    Color32::from_rgb(0x4e, 0xa8, 0xde),
    Color32::from_rgb(0xe0, 0x6c, 0x75),
    Color32::from_rgb(0x98, 0xc3, 0x79),
    Color32::from_rgb(0xe5, 0xc0, 0x7b),
    Color32::from_rgb(0xc6, 0x78, 0xdd),
    Color32::from_rgb(0x56, 0xb6, 0xc2),
    Color32::from_rgb(0xd1, 0x9a, 0x66),
    Color32::from_rgb(0x9a, 0xa5, 0xce),
];

struct Loaded {
    info: Option<RepoInfo>,
    sidebar: Option<Sidebar>,
    rows: Vec<CommitRow>,
    max_cols: u16,
    loading: bool,
    rx: Receiver<LoadEvent>,
    /// Selected commit (index into `rows`).
    selected: Option<usize>,
    detail: Option<Detail>,
    changes: Option<Changes>,
    /// The WIP row is selected and the right panel shows working changes.
    show_changes: bool,
    search: Search,
    /// Display row to bring into view on the next frame, and whether to centre it.
    scroll_to: Option<(usize, bool)>,
    /// Row of the HEAD commit, once it has been loaded (where the WIP row attaches).
    head_row: Option<usize>,
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
}

impl Search {
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
        self.show_changes = false;
        if let Some(info) = &self.info {
            if self.detail.as_ref().map(Detail::id) != Some(row.id) {
                self.detail = Some(Detail::new(ctx, &info.path, row.id));
            }
        }
    }

    /// Scroll so commit `idx` is visible (centred if `center`).
    fn reveal(&mut self, idx: usize, center: bool) {
        let disp = idx + self.has_wip() as usize;
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

enum StashOp {
    Save,
    Apply(usize),
    Pop(usize),
}

enum Confirm {
    DeleteBranch(String),
    DropStash(usize),
    DeleteTag(String),
    HardReset(gitgoing_core::Oid),
    DeleteRemoteBranch(String),
    RemoveRemote(String),
    DeleteRemoteTag(String),
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

/// The interactive-rebase planner. `items` are listed newest first, like the graph.
struct RebaseDlg {
    from: gitgoing_core::Oid,
    items: Vec<TodoItem>,
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
    stash: Option<StashOp>,
    undo: bool,
    redo: bool,
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
    /// Text of the open "new branch" dialog.
    new_branch: Option<String>,
    /// Commit the new-branch dialog creates the branch at (HEAD if `None`).
    branch_at: Option<gitgoing_core::Oid>,
    /// (commit, name, message) of the open "new tag" dialog.
    new_tag: Option<(gitgoing_core::Oid, String, String)>,
    remote_dlg: Option<RemoteDlg>,
    upstream_dlg: Option<UpstreamDlg>,
    rebase_dlg: Option<RebaseDlg>,
    conflict: Option<ConflictEditor>,
    confirm: Option<Confirm>,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    pending: Option<Pending>,
}

impl RepoTab {
    fn new(id: u64, path: PathBuf) -> Self {
        let mut app = RepoTab {
            id,
            path: path.clone(),
            repo: None,
            error: None,
            op_rx: None,
            notice: None,
            op_label: "",
            new_branch: None,
            branch_at: None,
            new_tag: None,
            remote_dlg: None,
            upstream_dlg: None,
            rebase_dlg: None,
            conflict: None,
            confirm: None,
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
        };
        app.open(path);
        app
    }

    fn title(&self) -> String {
        let p = self.repo_path().unwrap_or_else(|| self.path.clone());
        p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    fn is_at(&self, path: &std::path::Path) -> bool {
        self.path == path || self.repo_path().as_deref() == Some(path)
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
            detail: None,
            changes,
            show_changes,
            search: Search::default(),
            scroll_to: None,
            head_row: None,
            auto_opened: false,
            visible: 0..0,
        });
        self.error = None;
        std::thread::spawn(move || {
            history::load(&path, |ev| tx.send(ev).is_ok());
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
                    l.sidebar = Some(Sidebar::from_info(&i));
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
                    l.rows.append(&mut r);
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
        self.notice = None;
        self.op_label = label;
        // Snapshot refs around the operation so it can be undone if it moved any.
        self.op_rx = Some(detail::spawn(ctx, move || {
            let before = undo::snapshot(&path).ok();
            f(&path).map_err(detail::ErrorText::text)?;
            let after = undo::snapshot(&path).ok();
            Ok::<_, String>(before.zip(after).filter(|(b, a)| b != a).map(|(before, after)| {
                UndoEntry { label, soft: false, before, after }
            }))
        }));
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

    /// Re-read history, sidebar and status after the repo changed.
    fn reload(&mut self, ctx: &egui::Context) {
        let Some(path) = self.repo_path() else { return };
        let (mut ch, show) = self.repo.take().map_or((None, false), |l| (l.changes, l.show_changes));
        if let Some(c) = &mut ch {
            c.refresh(ctx);
        }
        self.open_with(path, ch, show);
    }

    fn poll_op(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.op_rx else { return };
        if let Ok(r) = rx.try_recv() {
            self.op_rx = None;
            let pending = self.pending.take();
            match r {
                Ok(entry) => {
                    match pending {
                        Some(Pending::Undo(e)) => self.redo.push(e),
                        Some(Pending::Redo(e)) => self.undo.push(e),
                        None => {
                            if let Some(e) = entry {
                                self.undo.push(e);
                                self.redo.clear();
                            }
                        }
                    }
                    self.notice = Some((format!("{} done", self.op_label), false));
                }
                Err(e) => {
                    // A failed undo/redo leaves the entry where it was.
                    match pending {
                        Some(Pending::Undo(en)) => self.undo.push(en),
                        Some(Pending::Redo(en)) => self.redo.push(en),
                        None => {}
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
        self.background_updates(ctx, regained);
    }

    fn background_updates(&mut self, ctx: &egui::Context, regained: bool) {
        let mut reload = None;
        if let Some(l) = &mut self.repo {
            if let Some(c) = &mut l.changes {
                if regained {
                    c.refresh(ctx);
                }
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
        if let Some(path) = reload {
            let (ch, show) = self.repo.take().map_or((None, false), |l| (l.changes, l.show_changes));
            self.open_with(path, ch, show);
        }
    }

    fn toolbar(&mut self, ctx: &egui::Context, pull_mode: PullMode) -> ToolbarOut {
        let mut out = ToolbarOut::default();
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if icons::tool_button(ui, "open", Icon::Folder, true).on_hover_text("Open a repository").clicked() {
                    out.open = rfd::FileDialog::new().pick_folder();
                }
                ui.separator();

                // Repository and branch, each with a small caption like the buttons.
                if let Some(info) = self.repo.as_ref().and_then(|l| l.info.as_ref()) {
                    let name = info.path.file_name().map_or_else(
                        || info.path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    ui.vertical(|ui| {
                        ui.small("repository");
                        ui.label(egui::RichText::new(name).size(18.0).strong());
                    });
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.small("branch");
                        let b = info.head_branch.clone().unwrap_or_else(|| "(detached HEAD)".to_owned());
                        ui.label(egui::RichText::new(b).size(18.0).strong());
                    });
                    if let Some(u) = &info.upstream {
                        let sync = match (u.ahead, u.behind) {
                            (0, 0) => String::new(),
                            (a, 0) => format!("{a} ahead"),
                            (0, b) => format!("{b} behind"),
                            (a, b) => format!("{a} ahead, {b} behind"),
                        };
                        ui.vertical(|ui| {
                            ui.small(" ");
                            ui.weak(format!("tracking {}", u.name));
                            if !sync.is_empty() {
                                ui.weak(sync);
                            }
                        });
                    }
                    ui.separator();
                }

                let can = self.repo.as_ref().is_some_and(|l| l.info.is_some()) && self.op_rx.is_none();
                let tip = |v: &[UndoEntry], verb: &str| {
                    v.last().map_or_else(|| format!("Nothing to {}", verb.to_lowercase()), |e| format!("{verb} {}", e.label))
                };
                if icons::tool_button(ui, "Undo", Icon::Undo, can && !self.undo.is_empty())
                    .on_hover_text(tip(&self.undo, "Undo"))
                    .on_disabled_hover_text(tip(&self.undo, "Undo"))
                    .clicked()
                {
                    out.undo = true;
                }
                if icons::tool_button(ui, "Redo", Icon::Redo, can && !self.redo.is_empty())
                    .on_hover_text(tip(&self.redo, "Redo"))
                    .on_disabled_hover_text(tip(&self.redo, "Redo"))
                    .clicked()
                {
                    out.redo = true;
                }
                ui.separator();
                if icons::tool_button(ui, "Fetch", Icon::Fetch, can).on_hover_text("Fetch all remotes and prune").clicked() {
                    out.remote = Some(("Fetch", Box::new(remote::fetch)));
                }
                if icons::tool_button(ui, "Pull", Icon::Pull, can).on_hover_text(pull_mode.label()).clicked() {
                    out.remote = Some(("Pull", Box::new(move |p| remote::pull_with(p, pull_mode))));
                }
                // Drop-down next to Pull to choose how to integrate for this one pull.
                ui.spacing_mut().item_spacing.x = 0.0;
                let arrow = egui::menu::menu_custom_button(
                    ui,
                    egui::Button::new("").frame(false).min_size(Vec2::new(14.0, 50.0)),
                    |ui| {
                        for mode in PullMode::ALL {
                            if ui.add_enabled(can, egui::Button::new(mode.label())).clicked() {
                                out.remote = Some(("Pull", Box::new(move |p| remote::pull_with(p, mode))));
                                ui.close_menu();
                            }
                        }
                    },
                );
                let r = arrow.response.rect;
                let tri = ui.visuals().weak_text_color();
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(r.center().x - 3.5, r.center().y + 4.0),
                        Pos2::new(r.center().x + 3.5, r.center().y + 4.0),
                        Pos2::new(r.center().x, r.center().y + 8.0),
                    ],
                    tri,
                    Stroke::NONE,
                ));
                ui.spacing_mut().item_spacing.x = 4.0;
                if icons::tool_button(ui, "Push", Icon::Push, can).on_hover_text("Push the current branch").clicked() {
                    out.remote = Some(("Push", Box::new(remote::push)));
                }
                ui.separator();
                if icons::tool_button(ui, "Branch", Icon::Branch, can).on_hover_text("Create a branch").clicked() {
                    self.new_branch = Some(String::new());
                }
                let has_stash = self
                    .repo
                    .as_ref()
                    .and_then(|l| l.info.as_ref())
                    .is_some_and(|i| !i.stashes.is_empty());
                if icons::tool_button(ui, "Stash", Icon::Stash, can).on_hover_text("Stash all changes, including untracked files").clicked() {
                    out.stash = Some(StashOp::Save);
                }
                if icons::tool_button(ui, "Pop", Icon::Pop, can && has_stash).on_hover_text("Apply and remove the latest stash").clicked() {
                    out.stash = Some(StashOp::Pop(0));
                }
                ui.separator();
                icons::tool_button(ui, "Terminal", Icon::Terminal, false).on_disabled_hover_text("Coming soon");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::tool_button(ui, "search", Icon::Search, true)
                        .on_hover_text("Search commits (Cmd/Ctrl+F)")
                        .clicked()
                    {
                        ui.ctx().memory_mut(|m| m.request_focus(egui::Id::new(("search", self.id))));
                    }
                });

                if self.op_rx.is_some() {
                    ui.add_space(8.0);
                    ui.spinner();
                    ui.weak(format!("{}…", self.op_label));
                }
            });
            ui.add_space(3.0);
        });
        out
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let mut create = None;
        let mut close = false;
        let at = self.branch_at;
        if let Some(name) = &mut self.new_branch {
            egui::Window::new("New branch")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(match at {
                        Some(id) => format!("Create a branch at {} and check it out.", &id.to_string()[..8]),
                        None => "Create a branch at the current commit and check it out.".to_owned(),
                    });
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text("branch-name"));
                    r.request_focus();
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        let ok = !name.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new("Create")).clicked() || (enter && ok) {
                            create = Some(name.trim().to_owned());
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
        }
        if close || create.is_some() {
            self.new_branch = None;
            self.branch_at = None;
        }
        if let Some(n) = create {
            match at {
                Some(id) => self.run_op(ctx, "Create branch", move |p| branches::create_at(p, &n, id, true)),
                None => self.run_op(ctx, "Create branch", move |p| branches::create_and_checkout(p, &n)),
            }
        }

        let mut tag_create = None;
        let mut tag_close = false;
        if let Some((id, name, msg)) = &mut self.new_tag {
            egui::Window::new("New tag")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!("Tag commit {}.", &id.to_string()[..8]));
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text("tag-name"));
                    r.request_focus();
                    ui.add(egui::TextEdit::multiline(msg).hint_text("Message (optional; makes an annotated tag)").desired_rows(2));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Create")).clicked() {
                            tag_create = Some((*id, name.trim().to_owned(), msg.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            tag_close = true;
                        }
                    });
                });
        }
        if tag_close || tag_create.is_some() {
            self.new_tag = None;
        }
        if let Some((id, n, m)) = tag_create {
            self.run_op(ctx, "Create tag", move |p| commit_ops::create_tag(p, &n, id, &m));
        }

        // Interactive rebase planner.
        let mut rebase_go = false;
        let mut rebase_close = false;
        if let Some(d) = &mut self.rebase_dlg {
            egui::Window::new("Interactive rebase")
                .collapsible(false)
                .resizable(true)
                .default_width(560.0)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Newest first. Commits are applied from the bottom up; choose what happens to each.");
                    ui.add_space(4.0);
                    let n = d.items.len();
                    let mut swap = None;
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for i in 0..n {
                            ui.horizontal(|ui| {
                                if ui.add_enabled(i > 0, egui::Button::new("Up").small()).on_hover_text("Apply later").clicked() {
                                    swap = Some((i, i - 1));
                                }
                                if ui.add_enabled(i + 1 < n, egui::Button::new("Down").small()).on_hover_text("Apply earlier").clicked() {
                                    swap = Some((i, i + 1));
                                }
                                let item = &mut d.items[i];
                                egui::ComboBox::from_id_salt(("rebase_action", i))
                                    .width(80.0)
                                    .selected_text(item.action.label())
                                    .show_ui(ui, |ui| {
                                        for a in RebaseAction::ALL {
                                            ui.selectable_value(&mut item.action, a, a.label());
                                        }
                                    });
                                ui.monospace(&item.id.to_string()[..7]);
                                let dim = item.action == RebaseAction::Drop;
                                let text = egui::RichText::new(&item.summary);
                                ui.add(egui::Label::new(if dim { text.strikethrough().weak() } else { text }).truncate());
                            });
                            if d.items[i].action == RebaseAction::Reword {
                                ui.add(
                                    egui::TextEdit::multiline(&mut d.items[i].message)
                                        .desired_rows(2)
                                        .desired_width(f32::INFINITY),
                                );
                            }
                        }
                    });
                    if let Some((a, b)) = swap {
                        d.items.swap(a, b);
                    }
                    let last_ok = !matches!(d.items.last().map(|i| i.action), Some(RebaseAction::Squash | RebaseAction::Fixup));
                    if !last_ok {
                        ui.colored_label(Color32::LIGHT_RED, "The oldest commit (bottom) cannot be squashed or fixed up.");
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(last_ok, egui::Button::new("Start rebase")).clicked() {
                            rebase_go = true;
                        }
                        if ui.button("Cancel").clicked() {
                            rebase_close = true;
                        }
                    });
                });
        }
        if rebase_close {
            self.rebase_dlg = None;
        }
        if rebase_go {
            if let Some(d) = self.rebase_dlg.take() {
                let mut items = d.items;
                items.reverse(); // git wants oldest first
                let from = d.from;
                self.run_op(ctx, "Rebase", move |p| rebase_outcome(rebase::start_interactive(p, from, &items)));
            }
        }

        // Add / edit / rename remote.
        let mut remote_go = None;
        let mut remote_close = false;
        if let Some(d) = &mut self.remote_dlg {
            let title = match d.kind {
                RemoteDlgKind::Add => "Add remote",
                RemoteDlgKind::EditUrl => "Edit remote URL",
                RemoteDlgKind::Rename => "Rename remote",
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_width(420.0);
                    if d.kind == RemoteDlgKind::Add {
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY));
                    }
                    ui.label(if d.kind == RemoteDlgKind::Rename { "New name" } else { "URL" });
                    let r = ui.add(egui::TextEdit::singleline(&mut d.text).desired_width(f32::INFINITY));
                    if d.kind != RemoteDlgKind::Add {
                        r.request_focus();
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!d.text.trim().is_empty(), egui::Button::new("OK")).clicked() {
                            remote_go = Some((d.kind, d.name.clone(), d.text.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            remote_close = true;
                        }
                    });
                });
        }
        if remote_close || remote_go.is_some() {
            self.remote_dlg = None;
        }
        if let Some((kind, name, text)) = remote_go {
            match kind {
                RemoteDlgKind::Add => self.run_op(ctx, "Add remote", move |p| remotes::add(p, &name, &text)),
                RemoteDlgKind::EditUrl => self.run_op(ctx, "Edit remote", move |p| remotes::set_url(p, &name, &text)),
                RemoteDlgKind::Rename => self.run_op(ctx, "Rename remote", move |p| remotes::rename(p, &name, &text)),
            }
        }

        // Set upstream.
        let mut up_go = None;
        let mut up_close = false;
        if let Some(d) = &mut self.upstream_dlg {
            egui::Window::new("Set upstream")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_width(380.0);
                    ui.label(format!("Branch '{}' should track:", d.local));
                    egui::ComboBox::from_id_salt("upstream_choice")
                        .width(340.0)
                        .selected_text(d.choice.as_deref().unwrap_or("(no upstream)"))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut d.choice, None, "(no upstream)");
                            for o in &d.options {
                                ui.selectable_value(&mut d.choice, Some(o.clone()), o);
                            }
                        });
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            up_go = Some((d.local.clone(), d.choice.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            up_close = true;
                        }
                    });
                });
        }
        if up_close || up_go.is_some() {
            self.upstream_dlg = None;
        }
        if let Some((local, up)) = up_go {
            self.run_op(ctx, "Set upstream", move |p| remotes::set_upstream(p, &local, up.as_deref()));
        }

        let mut confirmed = false;
        let mut cancel = false;
        if let Some(c) = &self.confirm {
            let (title, text) = match c {
                Confirm::DeleteBranch(n) => (
                    "Delete branch",
                    format!("Delete local branch '{n}'? Commits only reachable from it will be lost."),
                ),
                Confirm::DropStash(i) => (
                    "Delete stash",
                    format!("Permanently delete stash {i}? Its changes will be lost."),
                ),
                Confirm::DeleteTag(n) => ("Delete tag", format!("Delete local tag '{n}'?")),
                Confirm::DeleteRemoteBranch(n) => (
                    "Delete remote branch",
                    format!("Delete '{n}' on the remote? Others who use it will lose it."),
                ),
                Confirm::RemoveRemote(n) => (
                    "Remove remote",
                    format!("Remove the remote '{n}' and its remote-tracking branches from this repository?"),
                ),
                Confirm::DeleteRemoteTag(n) => ("Delete remote tag", format!("Delete tag '{n}' on the remote?")),
                Confirm::HardReset(id) => (
                    "Hard reset",
                    format!(
                        "Reset the current branch to {} and DISCARD all uncommitted changes? This cannot be undone for uncommitted work.",
                        &id.to_string()[..8]
                    ),
                ),
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(text);
                    ui.horizontal(|ui| {
                        confirmed = ui.button("Delete").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
        }
        if confirmed || cancel {
            let c = self.confirm.take();
            if confirmed {
                match c {
                    Some(Confirm::DeleteBranch(n)) => {
                        self.run_op(ctx, "Delete branch", move |p| branches::delete(p, &n))
                    }
                    Some(Confirm::DeleteRemoteBranch(full)) => {
                        let (remote, branch) = full.split_once('/').map(|(r, b)| (r.to_owned(), b.to_owned())).unwrap_or_default();
                        self.run_op(ctx, "Delete remote branch", move |p| remotes::delete_remote_branch(p, &remote, &branch))
                    }
                    Some(Confirm::RemoveRemote(n)) => self.run_op(ctx, "Remove remote", move |p| remotes::remove(p, &n)),
                    Some(Confirm::DeleteRemoteTag(n)) => {
                        self.run_op(ctx, "Delete remote tag", move |p| remotes::delete_remote_tag(p, &n))
                    }
                    Some(Confirm::DeleteTag(n)) => {
                        self.run_op(ctx, "Delete tag", move |p| commit_ops::delete_tag(p, &n))
                    }
                    Some(Confirm::HardReset(id)) => {
                        self.run_op(ctx, "Hard reset", move |p| commit_ops::reset(p, id, ResetMode::Hard))
                    }
                    Some(Confirm::DropStash(i)) => {
                        self.run_op(ctx, "Drop stash", move |p| stash::drop(p, i))
                    }
                    None => {}
                }
            }
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some((n, is_err)) = &self.notice {
                    if *is_err {
                        ui.colored_label(Color32::LIGHT_RED, n.lines().last().unwrap_or(n)).on_hover_text(n);
                    } else {
                        ui.label(n);
                    }
                } else if let Some(info) = self.repo.as_ref().and_then(|l| l.info.as_ref()) {
                    ui.weak(info.path.display().to_string());
                }
                if let Some(l) = &self.repo {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.notice.is_some() && ui.small_button("Dismiss").clicked() {
                            self.notice = None;
                        }
                        ui.weak(format!(
                            "{} commits{}",
                            l.rows.len(),
                            if l.loading { " (loading…)" } else { "" }
                        ));
                    });
                }
            });
        });
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
        if tb.undo {
            self.run_restore(ctx, true);
        } else if tb.redo {
            self.run_restore(ctx, false);
        }
        self.status_bar(ctx);
        let mut branch_act = None;
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
                        branch_act = s.show(ui);
                    }
                });
            let rect = r.response.rect;
            widths.sidebar += drag_handle(ctx, id.with("grip"), rect, rect.right());

            if l.show_changes || l.detail.is_some() {
                widths.right = widths.right.clamp(300.0, 900.0_f32.min(ctx.screen_rect().width() * 0.7));
                let id = egui::Id::new(("right", tab_id));
                let r = egui::SidePanel::right(id)
                    .resizable(false)
                    .exact_width(widths.right)
                    .show(ctx, |ui| {
                        if l.show_changes {
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

        if self.op_rx.is_none() {
            match branch_act {
                Some(SidebarAction::Checkout(n)) => self.run_op(ctx, "Checkout", move |p| branches::checkout(p, &n)),
                Some(SidebarAction::CheckoutRemote(n)) => {
                    self.run_op(ctx, "Checkout", move |p| branches::checkout_remote(p, &n))
                }
                Some(SidebarAction::DeleteTag(n)) => self.confirm = Some(Confirm::DeleteTag(n)),
                Some(SidebarAction::Delete(n)) => self.confirm = Some(Confirm::DeleteBranch(n)),
                Some(SidebarAction::Merge(n)) => self.run_op(ctx, "Merge", move |p| {
                    match merge::merge(p, &n).map_err(detail::ErrorText::text)? {
                        MergeOutcome::UpToDate => Err("Already up to date".to_owned()),
                        MergeOutcome::Conflicts(n) => Err(format!(
                            "Merge stopped: {n} conflicted file(s). Resolve them in Changes, then commit (or abort the merge)."
                        )),
                        MergeOutcome::FastForward | MergeOutcome::Merged => Ok(()),
                    }
                }),
                Some(SidebarAction::RebaseOnto(n)) => self.run_op(ctx, "Rebase", move |p| rebase_outcome(rebase::rebase_onto(p, &n))),
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
                Some(StashOp::Save) => self.run_op(ctx, "Stash", stash::save),
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
        // The conflict editor lives in its own window.
        if let Some(ed) = &mut self.conflict {
            match ed.show(ctx) {
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
                Some(CommitAction::InteractiveRebase(id)) => {
                    let plan = self.repo_path().ok_or_else(|| "no repository".to_owned()).and_then(|p| rebase::plan(&p, id));
                    match plan {
                        Ok(mut items) => {
                            items.reverse();
                            self.rebase_dlg = Some(RebaseDlg { from: id, items });
                        }
                        Err(e) => self.notice = Some((e, true)),
                    }
                }
                None => {}
            }
        }
        to_open
    }
}

fn history_view(
    ui: &mut egui::Ui,
    l: &mut Loaded,
    tab_id: u64,
    widths: &mut PanelWidths,
    act: &mut Option<CommitAction>,
) {
    let now = timefmt::now();
    // Column widths: both are user-resizable by dragging the header dividers;
    // the graph column defaults to fitting the lanes seen so far.
    let auto_w = l.max_cols.clamp(3, MAX_GRAPH_COLS) as f32 * LANE_W + 12.0;
    let graph_w = widths.graph_col.unwrap_or(auto_w).clamp(40.0, 1200.0);
    let ref_w = widths.ref_col.clamp(60.0, 600.0);
    let wip_lane = l
        .head_row
        .or(Some(0))
        .and_then(|h| l.rows.get(h))
        .map_or((0, 0), |r| (r.graph.node_col, r.graph.node_color));
    // Highest lane index that still fits; wider lanes are squeezed into it.
    let max_col = (((graph_w - 12.0) / LANE_W).floor() as u16).max(1) - 1;
    let wip = l.has_wip() as usize;
    let wip_count = l.changes.as_ref().and_then(Changes::count).unwrap_or(0);
    let total = l.rows.len() + wip;
    let visuals = ui.visuals().clone();
    let text_color = visuals.text_color();
    let weak = visuals.weak_text_color();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());

    let ctx = ui.ctx().clone();
    let search_id = egui::Id::new(("search", tab_id));
    let mut pending: Option<(usize, bool)> = None;

    // Keyboard: Cmd/Ctrl+F focuses search; arrows move the selection.
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
        ctx.memory_mut(|m| m.request_focus(search_id));
    }
    if !ctx.wants_keyboard_input() && !l.rows.is_empty() {
        let (up, down) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)));
        let last = l.rows.len() - 1;
        let next = match (l.selected, up, down) {
            (None, true, _) | (None, _, true) => Some(0),
            (Some(i), true, _) => Some(i.saturating_sub(1)),
            (Some(i), _, true) => Some((i + 1).min(last)),
            _ => None,
        };
        if let Some(n) = next.filter(|n| Some(*n) != l.selected) {
            pending = Some((n, true));
        }
    }

    // Search bar.
    let text_changed = l.search.update(&l.rows);
    let mut nav = 0i32;
    let mut goto_head = false;
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut l.search.text)
                .id(search_id)
                .hint_text("Search message, author or SHA")
                .desired_width(280.0),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            nav = if ui.input(|i| i.modifiers.shift) { -1 } else { 1 };
            r.request_focus();
        }
        goto_head = ui
            .button("HEAD")
            .on_hover_text("Scroll to the current commit")
            .clicked();
        let n = l.search.matches.len();
        if !l.search.text.trim().is_empty() {
            if n == 0 {
                ui.weak("No results");
            } else {
                ui.weak(format!("{}/{}", l.search.cursor.min(n - 1) + 1, n));
                if ui.small_button("Prev").clicked() {
                    nav = -1;
                }
                if ui.small_button("Next").clicked() {
                    nav = 1;
                }
            }
            if ui.small_button("Clear").clicked() {
                l.search.text.clear();
            }
        }
    });
    if goto_head {
        let head = l.info.as_ref().and_then(|i| i.head_oid);
        if let Some(idx) = head.and_then(|h| l.rows.iter().position(|r| r.id == h)) {
            l.reveal(idx, true);
            pending = Some((idx, false));
        }
    }
    let n = l.search.matches.len();
    if n > 0 {
        if text_changed {
            l.reveal(l.search.matches[0] as usize, true);
        }
        if nav != 0 {
            let cur = l.search.matches[l.search.cursor.min(n - 1)] as usize;
            if nav > 0 {
                if l.selected == Some(cur) {
                    l.search.cursor = (l.search.cursor + 1) % n;
                }
            } else {
                l.search.cursor = (l.search.cursor + n - 1) % n;
            }
            pending = Some((l.search.matches[l.search.cursor] as usize, true));
        }
    }

    // Column headers.
    let (hrect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HEADER_H), Sense::hover());
    let hp = ui.painter_at(hrect);
    let hfont = egui::TextStyle::Small.resolve(ui.style());
    for (x, label) in [
        (hrect.left() + 6.0, "BRANCH / TAG"),
        (hrect.left() + ref_w + 6.0, "GRAPH"),
        (hrect.left() + ref_w + graph_w + 6.0, "COMMIT MESSAGE"),
    ] {
        hp.text(Pos2::new(x, hrect.center().y), egui::Align2::LEFT_CENTER, label, hfont.clone(), weak);
    }
    // Draggable dividers on the right edge of the BRANCH/TAG and GRAPH columns.
    for (i, x) in [hrect.left() + ref_w, hrect.left() + ref_w + graph_w].into_iter().enumerate() {
        let strip = Rect::from_center_size(Pos2::new(x, hrect.center().y), Vec2::new(8.0, HEADER_H));
        let r = ui.interact(strip, egui::Id::new(("colgrip", tab_id, i)), Sense::drag());
        let active = r.hovered() || r.dragged();
        if active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        let stroke = if active {
            ui.visuals().widgets.active.fg_stroke
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };
        hp.vline(x, hrect.y_range(), stroke);
        if r.dragged() {
            let dx = r.drag_delta().x;
            if i == 0 {
                widths.ref_col = (ref_w + dx).clamp(60.0, 600.0);
            } else {
                widths.graph_col = Some((graph_w + dx).clamp(40.0, 1200.0));
            }
        }
        if i == 1 && r.double_clicked() {
            widths.graph_col = None; // back to automatic width
        }
    }
    ui.separator();

    let mut area = egui::ScrollArea::vertical().id_salt(tab_id).auto_shrink(false);
    if let Some((disp, center)) = l.scroll_to.take() {
        let view_h = (l.visible.len().max(4) - 1) as f32 * ROW_H;
        let y = disp as f32 * ROW_H;
        let offset = if center {
            y - view_h / 2.0
        } else if disp < l.visible.start {
            y
        } else {
            y + ROW_H - view_h
        };
        area = area.vertical_scroll_offset(offset.max(0.0));
    }
    area.show_rows(ui, ROW_H, total, |ui, range| {
            l.visible = range.clone();
            for i in range {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                let p = ui.painter_at(rect);
                let cy = rect.center().y;
                let graph_x = rect.left() + ref_w;

                if wip == 1 && i == 0 {
                    // Working-tree row.
                    if resp.clicked() {
                        l.show_changes = true;
                        l.detail = None;
                        l.selected = None;
                    }
                    if l.show_changes {
                        p.rect_filled(rect, 0.0, visuals.selection.bg_fill);
                    } else if resp.hovered() {
                        p.rect_filled(rect, 0.0, visuals.widgets.hovered.weak_bg_fill);
                    }
                    // The WIP node sits in the HEAD commit's lane, joined to it by a dashed line.
                    let (col, color) = wip_lane;
                    let x = graph_x + 8.0 + (col.min(max_col) as f32 + 0.5) * LANE_W;
                    let c = PALETTE[color as usize % PALETTE.len()];
                    dashed_vline(&p, x, cy + 5.5, rect.bottom(), c);
                    let ring: Vec<Pos2> = (0..=24)
                        .map(|k| {
                            let t = k as f32 / 24.0 * std::f32::consts::TAU;
                            Pos2::new(x + 5.0 * t.cos(), cy + 5.0 * t.sin())
                        })
                        .collect();
                    p.extend(egui::Shape::dashed_line(&ring, Stroke::new(1.8_f32, c), 3.0, 2.5));
                    p.text(
                        Pos2::new(graph_x + graph_w + 2.0, cy),
                        egui::Align2::LEFT_CENTER,
                        match l.changes.as_ref().and_then(|c| c.operation()) {
                            Some((op, 0)) => format!("// WIP  ({op} in progress: resolved, ready to continue)"),
                            Some((op, n)) => format!("// WIP  ({op} in progress: {n} file(s) with conflicts, click to resolve)"),
                            None => format!("// WIP  ({wip_count} changed)"),
                        },
                        font.clone(),
                        if l.changes.as_ref().and_then(|c| c.operation()).is_some_and(|(_, n)| n > 0) { Color32::from_rgb(0xe5, 0x53, 0x4b) } else { weak },
                    );
                    continue;
                }

                let idx = i - wip;
                let row = &l.rows[idx];
                if resp.clicked() {
                    pending = Some((idx, false));
                }
                let id = row.id;
                resp.context_menu(|ui| {
                    let mut pick = |a: CommitAction, ui: &mut egui::Ui| {
                        *act = Some(a);
                        ui.close_menu();
                    };
                    if ui.button("Create branch here…").clicked() {
                        pick(CommitAction::CreateBranch(id), ui);
                    }
                    if ui.button("Create tag here…").clicked() {
                        pick(CommitAction::CreateTag(id), ui);
                    }
                    if ui.button("Check out this commit (detached)").clicked() {
                        pick(CommitAction::Checkout(id), ui);
                    }
                    ui.separator();
                    if ui.button("Cherry-pick").clicked() {
                        pick(CommitAction::CherryPick(id), ui);
                    }
                    if ui.button("Revert").clicked() {
                        pick(CommitAction::Revert(id), ui);
                    }
                    if ui.button("Interactive rebase from here…").clicked() {
                        pick(CommitAction::InteractiveRebase(id), ui);
                    }
                    ui.menu_button("Reset current branch to here", |ui| {
                        if ui.button("Soft (keep index and files)").clicked() {
                            pick(CommitAction::Reset(id, ResetMode::Soft), ui);
                        }
                        if ui.button("Mixed (keep files)").clicked() {
                            pick(CommitAction::Reset(id, ResetMode::Mixed), ui);
                        }
                        if ui.button("Hard (discard changes)…").clicked() {
                            pick(CommitAction::Reset(id, ResetMode::Hard), ui);
                        }
                    });
                    ui.separator();
                    if ui.button("Copy commit SHA").clicked() {
                        pick(CommitAction::CopySha(id), ui);
                    }
                });
                if l.search.matches.binary_search(&(idx as u32)).is_ok() {
                    p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0xe5, 0xc0, 0x7b, 40));
                }
                if l.selected == Some(idx) {
                    p.rect_filled(rect, 0.0, visuals.selection.bg_fill);
                } else if resp.hovered() {
                    p.rect_filled(rect, 0.0, visuals.widgets.hovered.weak_bg_fill);
                }

                let wip_line = (wip == 1)
                    .then_some(l.head_row)
                    .flatten()
                    .filter(|&h| idx <= h)
                    .map(|h| (wip_lane.0, wip_lane.1, idx == h));
                draw_graph(&p, rect, graph_x, row, l.info.as_ref().map(|i| &i.refs), graph_w, max_col, wip_line);

                // Right-aligned fixed columns, flexible message in between.
                let date_x = rect.right() - DATE_W;
                let author_x = date_x - AUTHOR_W;
                p.text(
                    Pos2::new(date_x, cy),
                    egui::Align2::LEFT_CENTER,
                    timefmt::relative(row.time, now),
                    font.clone(),
                    weak,
                );
                let author_clip = Rect::from_min_max(
                    Pos2::new(author_x, rect.top()),
                    Pos2::new(date_x - 6.0, rect.bottom()),
                );
                ui.painter_at(author_clip).text(
                    Pos2::new(author_x, cy),
                    egui::Align2::LEFT_CENTER,
                    &*row.author,
                    font.clone(),
                    weak,
                );

                // Ref chips live in their own column left of the graph.
                let ref_clip = Rect::from_min_max(
                    Pos2::new(rect.left(), rect.top()),
                    Pos2::new(graph_x - 4.0, rect.bottom()),
                );
                let rp = ui.painter_at(ref_clip);
                let mut x = rect.left() + 4.0;
                if let Some(labels) = l.info.as_ref().and_then(|i| i.refs.get(&row.id)) {
                    for label in labels {
                        let (bg, fg) = match label.kind {
                            RefKind::Head => (Color32::from_rgb(0x2e, 0x7d, 0x32), Color32::WHITE),
                            RefKind::Local => (Color32::from_rgb(0x1f, 0x6f, 0xa8), Color32::WHITE),
                            RefKind::Remote => (Color32::from_rgb(0x6a, 0x4c, 0x93), Color32::WHITE),
                            RefKind::Tag => (Color32::from_rgb(0xb2, 0x8a, 0x1e), Color32::BLACK),
                        };
                        let galley = rp.layout_no_wrap(label.name.clone(), small.clone(), fg);
                        let chip = Rect::from_min_size(
                            Pos2::new(x, cy - 8.0),
                            galley.size() + Vec2::new(10.0, 4.0),
                        );
                        rp.rect_filled(chip, 4.0, bg);
                        rp.galley(chip.min + Vec2::new(5.0, 2.0), galley, fg);
                        x = chip.right() + 4.0;
                    }
                }

                let msg_x = graph_x + graph_w + 2.0;
                let msg_clip = Rect::from_min_max(
                    Pos2::new(msg_x, rect.top()),
                    Pos2::new(author_x - 6.0, rect.bottom()),
                );
                ui.painter_at(msg_clip).text(
                    Pos2::new(msg_x, cy),
                    egui::Align2::LEFT_CENTER,
                    &*row.summary,
                    font.clone(),
                    text_color,
                );
            }
        });

    if let Some((idx, reveal)) = pending {
        l.select(&ctx, idx);
        if reveal {
            l.reveal(idx, false);
        }
    }
}

/// A vertical dashed line (dash and gap divide the row height evenly, so it
/// stays continuous across rows).
fn dashed_vline(p: &egui::Painter, x: f32, y0: f32, y1: f32, color: Color32) {
    let stroke = Stroke::new(2.0_f32, color);
    let mut y = y0;
    while y < y1 {
        p.line_segment([Pos2::new(x, y), Pos2::new(x, (y + 3.0).min(y1))], stroke);
        y += 6.0;
    }
}

fn draw_graph(
    p: &egui::Painter,
    rect: Rect,
    graph_x: f32,
    row: &CommitRow,
    refs: Option<&std::collections::HashMap<gitgoing_core::Oid, Vec<history::RefLabel>>>,
    graph_w: f32,
    max_col: u16,
    wip_line: Option<(u16, u8, bool)>,
) {
    let clip = Rect::from_min_size(Pos2::new(graph_x, rect.top()), Vec2::new(graph_w, ROW_H));
    let p = p.with_clip_rect(clip.intersect(p.clip_rect()));
    let cx = |col: u16| graph_x + 8.0 + (col.min(max_col) as f32 + 0.5) * LANE_W;
    let top = rect.top();
    let mid = rect.center().y;
    let bot = rect.bottom();
    let color = |c: u8| PALETTE[c as usize % PALETTE.len()];
    let node = cx(row.graph.node_col);

    // Dashed line from the WIP row down to the HEAD commit.
    if let Some((col, color, ends_here)) = wip_line {
        let c = PALETTE[color as usize % PALETTE.len()];
        dashed_vline(&p, cx(col), top, if ends_here { mid - 6.0 } else { bot }, c);
    }

    for s in &row.graph.segs {
        let stroke = Stroke::new(2.0_f32, color(s.color));
        let (a, b) = match s.kind {
            SegKind::Pass => (Pos2::new(cx(s.from), top), Pos2::new(cx(s.to), bot)),
            SegKind::In => (Pos2::new(cx(s.from), top), Pos2::new(node, mid)),
            SegKind::Out => (Pos2::new(node, mid), Pos2::new(cx(s.to), bot)),
        };
        p.line_segment([a, b], stroke);
    }

    let c = color(row.graph.node_color);
    let is_head = refs
        .and_then(|r| r.get(&row.id))
        .is_some_and(|l| l.iter().any(|l| l.kind == RefKind::Head));
    let center = Pos2::new(node, mid);
    if is_head {
        p.circle_filled(center, 6.0, c);
        p.circle_stroke(center, 6.0, Stroke::new(2.0_f32, Color32::WHITE));
    } else {
        p.circle_filled(center, 4.5, c);
    }
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
    settings: Settings,
    settings_win: SettingsWindow,
    /// A problem to show the user (e.g. a failed init).
    app_error: Option<String>,
}

fn paths_from(s: Option<String>) -> Vec<PathBuf> {
    s.unwrap_or_default().lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>, resolve: Option<String>) -> Self {
        let storage = cc.storage;
        let get = |k: &str| storage.and_then(|s| s.get_string(k));
        let mut app = App {
            tabs: Vec::new(),
            active: 0,
            next_id: 0,
            was_focused: true,
            recent: paths_from(get("recent")),
            clone_dlg: None,
            settings: Settings::load(&get),
            settings_win: SettingsWindow::default(),
            app_error: None,
            widths: PanelWidths {
                sidebar: get("sidebar_w").and_then(|v| v.parse().ok()).unwrap_or(240.0),
                right: get("right_w").and_then(|v| v.parse().ok()).unwrap_or(420.0),
                ref_col: get("ref_col").and_then(|v| v.parse().ok()).unwrap_or(190.0),
                graph_col: get("graph_col").and_then(|v| v.parse().ok()),
            },
        };
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
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path.clone());
        self.recent.truncate(MAX_RECENT);
        if let Some(i) = self.tabs.iter().position(|t| t.is_at(&path)) {
            self.active = i;
            return;
        }
        self.next_id += 1;
        self.tabs.push(RepoTab::new(self.next_id, path));
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
                ui.heading("gitgoing");
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
        self.tab_bar(ctx);
        if self.settings_win.show(ctx, &mut self.settings) {
            self.settings.apply(ctx);
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
        self.settings.save(|k, v| storage.set_string(k, v));
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
    fn search_matches_message_author_and_sha_incrementally() {
        let mut rows = vec![row(0xab, "Fix login bug", "Ann"), row(2, "Add feature", "Bob")];
        let mut s = Search::default();
        s.text = "LOGIN".into();
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
}
