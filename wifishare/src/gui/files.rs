//! The Files tab: this computer's folders on the left, the other computer's shared folders on
//! the right, buttons to copy between them, and the copies in progress below.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::{Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, vec2};

use crate::clipnet::human;
use crate::config::{Config, ShareCfg};
use crate::files::{Files, Job, JobKind, JobState, Pending, join, split};
use crate::proto::{DirEntry, Share};

const ROW: f32 = 24.0;
const GREEN: Color32 = Color32::from_rgb(46, 160, 90);
const RED: Color32 = Color32::from_rgb(210, 70, 60);
const AMBER: Color32 = Color32::from_rgb(214, 140, 30);

/// What the window tells the tab about the world.
pub struct Ctx<'a> {
    /// The running engine's shared-folder service, if sharing is on.
    pub files: Option<Arc<Files>>,
    pub cfg: &'a Config,
    /// Computers connected right now (a client has its server; a server has its clients).
    pub peers: &'a [String],
}

#[derive(Clone)]
struct Item {
    name: String,
    dir: bool,
    size: u64,
}

#[derive(Default)]
struct Selection {
    names: HashSet<String>,
    anchor: Option<usize>,
}

/// The list of one pane: rows, selection, and double-click to open a folder.
fn list(ui: &mut Ui, id: &str, items: &[&Item], sel: &mut Selection) -> Option<String> {
    let mut opened = None;
    let mods = ui.input(|i| i.modifiers);
    egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show_rows(ui, ROW, items.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in range {
            let it = items[i];
            let selected = sel.names.contains(&it.name);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
            let visuals = ui.visuals();
            if selected {
                ui.painter().rect_filled(
                    rect.shrink2(vec2(2.0, 1.0)),
                    CornerRadius::same(4),
                    visuals.selection.bg_fill.gamma_multiply(0.55),
                );
            } else if resp.hovered() {
                ui.painter().rect_filled(rect.shrink2(vec2(2.0, 1.0)), CornerRadius::same(4), visuals.widgets.hovered.weak_bg_fill);
            }
            let text_color = visuals.text_color();
            icon(ui, Pos2::new(rect.left() + 16.0, rect.center().y), it.dir);
            let size_text = if it.dir { String::new() } else { human(it.size) };
            let font = FontId::proportional(14.0);
            let size_width = ui.painter().layout_no_wrap(size_text.clone(), font.clone(), text_color).size().x;
            ui.painter().text(
                Pos2::new(rect.right() - 10.0, rect.center().y),
                Align2::RIGHT_CENTER,
                size_text,
                font.clone(),
                text_color.gamma_multiply(0.6),
            );
            let name_area =
                Rect::from_min_max(Pos2::new(rect.left() + 32.0, rect.top()), Pos2::new(rect.right() - size_width - 20.0, rect.bottom()));
            ui.painter().with_clip_rect(name_area).text(
                Pos2::new(name_area.left(), rect.center().y),
                Align2::LEFT_CENTER,
                &it.name,
                font,
                text_color,
            );
            if resp.clicked() {
                if mods.shift
                    && let Some(a) = sel.anchor
                {
                    let (lo, hi) = (a.min(i), a.max(i));
                    if !mods.command {
                        sel.names.clear();
                    }
                    sel.names.extend(items[lo..=hi.min(items.len() - 1)].iter().map(|x| x.name.clone()));
                } else if mods.command {
                    if !sel.names.remove(&it.name) {
                        sel.names.insert(it.name.clone());
                    }
                    sel.anchor = Some(i);
                } else {
                    sel.names.clear();
                    sel.names.insert(it.name.clone());
                    sel.anchor = Some(i);
                }
            }
            if resp.double_clicked() && it.dir {
                opened = Some(it.name.clone());
            }
        }
    });
    opened
}

fn icon(ui: &Ui, c: Pos2, dir: bool) {
    let p = ui.painter();
    if dir {
        let color = Color32::from_rgb(222, 168, 54);
        p.rect_filled(Rect::from_min_size(c + vec2(-8.0, -6.0), vec2(7.0, 4.0)), CornerRadius::same(1), color);
        p.rect_filled(Rect::from_min_size(c + vec2(-8.0, -4.0), vec2(16.0, 11.0)), CornerRadius::same(2), color);
    } else {
        let color = ui.visuals().text_color().gamma_multiply(0.55);
        let r = Rect::from_min_size(c + vec2(-6.0, -8.0), vec2(12.0, 16.0));
        p.rect_stroke(r, CornerRadius::same(2), Stroke::new(1.3, color), egui::StrokeKind::Inside);
        for k in 0..3 {
            let y = r.top() + 5.0 + 3.5 * k as f32;
            p.line_segment([Pos2::new(r.left() + 3.0, y), Pos2::new(r.right() - 3.0, y)], Stroke::new(1.0, color));
        }
    }
}

fn visible(items: &[Item], hidden: bool) -> Vec<&Item> {
    items.iter().filter(|i| hidden || !i.name.starts_with('.')).collect()
}

fn sort(items: &mut [Item]) {
    items.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
}

fn from_entries(entries: Vec<DirEntry>) -> Vec<Item> {
    entries.into_iter().map(|e| Item { name: e.name, dir: e.dir, size: e.size }).collect()
}

/// A small "new folder" form shown above a pane's buttons.
fn new_folder_form(ui: &mut Ui, name: &mut Option<String>) -> Option<String> {
    let mut create = None;
    let mut close = false;
    if let Some(text) = name {
        ui.horizontal(|ui| {
            let edit =
                ui.add(egui::TextEdit::singleline(text).hint_text("Name of the new folder").desired_width(ui.available_width() - 150.0));
            edit.request_focus();
            let go = ui.add_enabled(!text.trim().is_empty(), egui::Button::new("Create")).clicked()
                || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty());
            if go {
                create = Some(text.trim().to_string());
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
    }
    if create.is_some() || close {
        *name = None;
    }
    create
}

// ---- this computer ----

struct LocalPane {
    dir: PathBuf,
    path_text: String,
    items: Vec<Item>,
    error: Option<String>,
    sel: Selection,
    new_folder: Option<String>,
}

impl LocalPane {
    fn new() -> LocalPane {
        let dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut pane =
            LocalPane { dir, path_text: String::new(), items: Vec::new(), error: None, sel: Selection::default(), new_folder: None };
        pane.reload();
        pane
    }

    fn go(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.sel = Selection::default();
        self.reload();
    }

    fn reload(&mut self) {
        self.path_text = self.dir.display().to_string();
        self.items.clear();
        self.error = None;
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let Ok(meta) = e.metadata() else { continue };
                    // Follow links to see what they point at; a broken link is skipped.
                    let meta = if meta.file_type().is_symlink() { std::fs::metadata(e.path()).ok() } else { Some(meta) };
                    let Some(meta) = meta else { continue };
                    self.items.push(Item { name: e.file_name().to_string_lossy().into_owned(), dir: meta.is_dir(), size: meta.len() });
                }
                sort(&mut self.items);
            }
            Err(e) => self.error = Some(format!("Cannot open this folder: {e}")),
        }
        let names: HashSet<_> = self.items.iter().map(|i| i.name.clone()).collect();
        self.sel.names.retain(|n| names.contains(n));
    }
}

enum LocalAction {
    Send,
    Folders,
}

impl LocalPane {
    fn show(&mut self, ui: &mut Ui, hidden: bool, send: Result<String, String>) -> Option<LocalAction> {
        let mut action = None;
        ui.label(RichText::new("This computer").strong().size(15.0));
        ui.horizontal(|ui| {
            if ui.add_enabled(self.dir.parent().is_some(), egui::Button::new("Up")).clicked() {
                let parent = self.dir.parent().map(|p| p.to_path_buf());
                if let Some(p) = parent {
                    self.go(p);
                }
            }
            if ui.button("Home").clicked() {
                self.go(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
            }
            let edit = ui.add(egui::TextEdit::singleline(&mut self.path_text).desired_width(ui.available_width() - 70.0));
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let wanted = PathBuf::from(self.path_text.trim());
                if wanted.is_dir() {
                    self.go(wanted);
                } else {
                    self.error = Some("There is no such folder.".into());
                }
            }
            if ui.button("Refresh").clicked() {
                self.reload();
            }
        });
        let footer = 74.0;
        let height = (ui.available_height() - footer).max(80.0);
        Frame::canvas(ui.style()).inner_margin(Margin::same(2)).show(ui, |ui| {
            ui.set_height(height);
            ui.set_width(ui.available_width());
            if let Some(e) = &self.error {
                ui.add_space(8.0);
                ui.colored_label(RED, e);
            } else {
                let rows = visible(&self.items, hidden);
                if rows.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new("  This folder is empty.").weak());
                }
                if let Some(name) = list(ui, "local-list", &rows, &mut self.sel) {
                    let next = self.dir.join(name);
                    self.go(next);
                }
            }
        });
        if let Some(name) = new_folder_form(ui, &mut self.new_folder) {
            match std::fs::create_dir(self.dir.join(&name)) {
                Ok(()) => self.reload(),
                Err(e) => self.error = Some(format!("Could not create the folder: {e}")),
            }
        }
        ui.horizontal(|ui| {
            let n = self.sel.names.len();
            let label = match &send {
                Ok(peer) => format!("Send to {peer}"),
                Err(_) => "Send".to_string(),
            };
            let button = ui.add_enabled(n > 0 && send.is_ok(), egui::Button::new(RichText::new(label).strong()));
            if button.clicked() {
                action = Some(LocalAction::Send);
            }
            if n == 0 {
                button.on_disabled_hover_text("Select files or folders here first");
            } else if let Err(why) = &send {
                button.on_disabled_hover_text(why);
            }
            if ui.button("New folder").clicked() {
                self.new_folder = Some(String::new());
            }
            if ui.button("Folders I share").on_hover_text("Choose which of your folders the other computer can see").clicked() {
                action = Some(LocalAction::Folders);
            }
        });
        action
    }
}

// ---- the other computer ----

enum Fetch {
    Shares(Pending<Vec<Share>>),
    Dir(Pending<Vec<DirEntry>>),
}

struct RemotePane {
    peer: Option<String>,
    /// The shared folder being browsed; `None` shows the list of shares.
    share: Option<String>,
    dir: String,
    items: Vec<Item>,
    /// Which shares allow adding files.
    writable: HashMap<String, bool>,
    fetch: Option<Fetch>,
    error: Option<String>,
    sel: Selection,
    new_folder: Option<String>,
    making: Option<Pending<()>>,
    was_connected: bool,
    loaded: bool,
}

enum RemoteAction {
    Get,
}

impl RemotePane {
    fn new() -> RemotePane {
        RemotePane {
            peer: None,
            share: None,
            dir: String::new(),
            items: Vec::new(),
            writable: HashMap::new(),
            fetch: None,
            error: None,
            sel: Selection::default(),
            new_folder: None,
            making: None,
            was_connected: false,
            loaded: false,
        }
    }

    fn can_write(&self) -> bool {
        self.share.as_ref().is_some_and(|s| self.writable.get(s).copied().unwrap_or(false))
    }

    fn go(&mut self, files: &Arc<Files>, share: Option<String>, dir: String) {
        self.share = share;
        self.dir = dir;
        self.sel = Selection::default();
        self.reload(files);
    }

    fn reload(&mut self, files: &Arc<Files>) {
        let Some(peer) = self.peer.clone() else { return };
        self.error = None;
        self.fetch = Some(match &self.share {
            None => Fetch::Shares(files.shares(&peer)),
            Some(share) => Fetch::Dir(files.list(&peer, share, &self.dir)),
        });
    }

    /// Follows connection changes and collects finished requests.
    fn poll(&mut self, cx: &Ctx<'_>) {
        let Some(files) = &cx.files else {
            self.fetch = None;
            self.was_connected = false;
            return;
        };
        if self.peer.as_ref().is_none_or(|p| !cx.peers.contains(p)) {
            self.peer = cx.peers.first().cloned();
        }
        let connected = self.peer.is_some();
        if connected && (!self.was_connected || !self.loaded) {
            self.share = None;
            self.dir.clear();
            self.sel = Selection::default();
            self.loaded = true;
            self.reload(files);
        }
        if !connected {
            self.items.clear();
            self.loaded = false;
        }
        self.was_connected = connected;

        match self.fetch.as_ref().map(|f| match f {
            Fetch::Shares(p) => p.take().map(|r| r.map(|v| (Some(v), None))),
            Fetch::Dir(p) => p.take().map(|r| r.map(|v| (None, Some(v)))),
        }) {
            Some(Some(Ok((shares, entries)))) => {
                self.fetch = None;
                self.error = None;
                if let Some(shares) = shares {
                    self.writable = shares.iter().map(|s| (s.name.clone(), s.write)).collect();
                    self.items = shares.into_iter().map(|s| Item { name: s.name, dir: true, size: 0 }).collect();
                    sort(&mut self.items);
                }
                if let Some(entries) = entries {
                    self.items = from_entries(entries);
                }
                let names: HashSet<_> = self.items.iter().map(|i| i.name.clone()).collect();
                self.sel.names.retain(|n| names.contains(n));
            }
            Some(Some(Err(e))) => {
                self.fetch = None;
                self.items.clear();
                self.error = Some(e);
            }
            _ => {}
        }
        if let Some(Some(result)) = self.making.as_ref().map(|m| m.take()) {
            self.making = None;
            match result {
                Ok(()) => self.reload(files),
                Err(e) => self.error = Some(e),
            }
        }
    }

    fn show(&mut self, ui: &mut Ui, cx: &Ctx<'_>, hidden: bool, local_dir: &str) -> Option<RemoteAction> {
        let mut action = None;
        let files = cx.files.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("The other computer").strong().size(15.0));
            if cx.peers.len() > 1 {
                let before = self.peer.clone();
                egui::ComboBox::from_id_salt("files-peer").selected_text(self.peer.clone().unwrap_or_default()).show_ui(ui, |ui| {
                    for p in cx.peers {
                        ui.selectable_value(&mut self.peer, Some(p.clone()), p);
                    }
                });
                if self.peer != before
                    && let Some(files) = &files
                {
                    self.go(files, None, String::new());
                }
            } else if let Some(p) = &self.peer {
                ui.label(RichText::new(p).weak());
            }
        });
        ui.horizontal(|ui| {
            let up = self.share.is_some();
            if ui.add_enabled(up && files.is_some(), egui::Button::new("Up")).clicked()
                && let Some(files) = &files
            {
                if self.dir.is_empty() {
                    self.go(files, None, String::new());
                } else {
                    let parent = split(&self.dir).0.to_string();
                    let share = self.share.clone();
                    self.go(files, share, parent);
                }
            }
            let place = match &self.share {
                None => "Shared folders".to_string(),
                Some(s) => join(s, &self.dir),
            };
            ui.label(RichText::new(place).monospace());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(files.is_some() && self.peer.is_some(), egui::Button::new("Refresh")).clicked()
                    && let Some(files) = &files
                {
                    self.reload(files);
                }
            });
        });
        let footer = 74.0;
        let height = (ui.available_height() - footer).max(80.0);
        Frame::canvas(ui.style()).inner_margin(Margin::same(2)).show(ui, |ui| {
            ui.set_height(height);
            ui.set_width(ui.available_width());
            ui.add_space(2.0);
            if files.is_none() {
                ui.add_space(10.0);
                ui.label(RichText::new("  Sharing is stopped. Start it (top right) to see the other computer's files.").weak());
            } else if self.peer.is_none() {
                ui.add_space(10.0);
                ui.label(RichText::new("  Not connected to the other computer yet.").weak());
            } else if let Some(e) = &self.error {
                ui.add_space(8.0);
                ui.colored_label(RED, format!("  {e}"));
            } else if self.fetch.is_some() && self.items.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new("  Loading...").weak());
            } else {
                let rows = visible(&self.items, hidden);
                if rows.is_empty() {
                    ui.add_space(10.0);
                    let text = if self.share.is_none() {
                        "  The other computer shares no folders yet. It chooses them in its Files tab."
                    } else {
                        "  This folder is empty."
                    };
                    ui.label(RichText::new(text).weak());
                }
                if let Some(name) = list(ui, "remote-list", &rows, &mut self.sel)
                    && let Some(files) = &files
                {
                    match &self.share {
                        None => self.go(files, Some(name), String::new()),
                        Some(_) => {
                            let next = join(&self.dir, &name);
                            let share = self.share.clone();
                            self.go(files, share, next);
                        }
                    }
                }
            }
        });
        if let Some(name) = new_folder_form(ui, &mut self.new_folder)
            && let (Some(files), Some(peer), Some(share)) = (&files, &self.peer, &self.share)
        {
            self.making = Some(files.mkdir(peer, share, &join(&self.dir, &name)));
        }
        ui.horizontal(|ui| {
            let n = self.sel.names.len();
            let ready = files.is_some() && self.share.is_some() && n > 0;
            let button = ui.add_enabled(ready, egui::Button::new(RichText::new("Get").strong()));
            if button.clicked() {
                action = Some(RemoteAction::Get);
            }
            if self.share.is_none() {
                button.on_disabled_hover_text("Open a shared folder first");
            } else if n == 0 {
                button.on_disabled_hover_text("Select files or folders here first");
            } else {
                button.on_hover_text(format!("Copy the selection into {local_dir}"));
            }
            let can_make = files.is_some() && self.can_write();
            let make = ui.add_enabled(can_make, egui::Button::new("New folder"));
            if make.clicked() {
                self.new_folder = Some(String::new());
            }
            if !can_make && self.share.is_some() {
                make.on_disabled_hover_text("This folder is shared read-only");
            }
        });
        action
    }
}

// ---- the tab ----

pub struct FilesView {
    local: LocalPane,
    remote: RemotePane,
    hidden: bool,
    folders_open: bool,
    share_name: String,
    share_write: bool,
    error: Option<String>,
    /// Copies whose result changes what a pane shows.
    watch: Vec<Arc<Job>>,
}

impl FilesView {
    pub fn new() -> FilesView {
        FilesView {
            local: LocalPane::new(),
            remote: RemotePane::new(),
            hidden: false,
            folders_open: false,
            share_name: String::new(),
            share_write: false,
            error: None,
            watch: Vec::new(),
        }
    }

    /// Draws the tab. Returns a changed config to save, if the user changed what is shared.
    pub fn show(&mut self, ui: &mut Ui, cx: &Ctx<'_>) -> Option<Config> {
        self.remote.poll(cx);
        self.after_jobs(cx);
        let jobs = cx.files.as_ref().map(|f| f.jobs()).unwrap_or_default();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Browse the other computer's shared folders and copy files in either direction.").weak());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.checkbox(&mut self.hidden, "Show hidden files").changed() {
                    self.local.sel.names.clear();
                    self.remote.sel.names.clear();
                }
            });
        });
        ui.add_space(4.0);

        let jobs_height = if jobs.is_empty() { 0.0 } else { (jobs.len() as f32 * 50.0 + 36.0).min(190.0) };
        let send = self.send_target(cx);
        let local_dir = self.local.dir.display().to_string();
        let (mut send_clicked, mut get_clicked, mut folders) = (false, false, false);
        let panes = vec2(ui.available_width(), (ui.available_height() - jobs_height).max(240.0));
        ui.allocate_ui(panes, |ui| {
            let FilesView { local, remote, hidden, .. } = self;
            ui.columns(2, |cols| {
                match local.show(&mut cols[0], *hidden, send) {
                    Some(LocalAction::Send) => send_clicked = true,
                    Some(LocalAction::Folders) => folders = true,
                    None => {}
                }
                if let Some(RemoteAction::Get) = remote.show(&mut cols[1], cx, *hidden, &local_dir) {
                    get_clicked = true;
                }
            });
        });
        if !jobs.is_empty() {
            ui.allocate_ui(vec2(ui.available_width(), jobs_height), |ui| self.jobs_panel(ui, cx, &jobs));
        }
        if folders {
            self.folders_open = true;
            self.share_name = self.local.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.share_write = false;
        }
        if let (Some(files), Some(peer)) = (&cx.files, self.remote.peer.clone()) {
            if send_clicked && let Some(share) = self.remote.share.clone() {
                let sources: Vec<PathBuf> = self.local.sel.names.iter().map(|n| self.local.dir.join(n)).collect();
                self.watch.push(files.upload(&peer, &share, &self.remote.dir.clone(), sources));
                self.local.sel = Selection::default();
            }
            if get_clicked && let Some(share) = self.remote.share.clone() {
                let names: Vec<String> = self.remote.sel.names.iter().cloned().collect();
                self.watch.push(files.download(&peer, &share, &self.remote.dir.clone(), names, self.local.dir.clone()));
                self.remote.sel = Selection::default();
            }
        }
        if let Some(e) = &self.error {
            ui.colored_label(RED, e);
        }
        self.folders_dialog(ui.ctx(), cx)
    }

    /// Opens a shared folder of the other computer (for screenshots).
    #[cfg(test)]
    pub fn browse_remote(&mut self, files: &Arc<Files>, share: &str, dir: &str) {
        self.remote.go(files, Some(share.into()), dir.into());
    }

    #[cfg(test)]
    pub fn open_folders(&mut self) {
        self.folders_open = true;
        self.share_name = "Downloads".into();
    }

    #[cfg(test)]
    pub fn browse_local(&mut self, dir: PathBuf) {
        self.local.go(dir);
    }

    /// Where "Send" would copy to, or why it cannot.
    fn send_target(&self, cx: &Ctx<'_>) -> Result<String, String> {
        let peer = self.remote.peer.clone().filter(|_| cx.files.is_some()).ok_or("Not connected to the other computer")?;
        if self.remote.share.is_none() {
            return Err("Open one of the other computer's shared folders on the right first".into());
        }
        if !self.remote.can_write() {
            return Err("That shared folder is read-only. The other computer can allow adding files in \"Folders I share\"".into());
        }
        Ok(peer)
    }

    /// Refreshes a pane when a copy into it finishes.
    fn after_jobs(&mut self, cx: &Ctx<'_>) {
        let mut done = Vec::new();
        self.watch.retain(|j| {
            if j.finished() {
                done.push(j.clone());
                false
            } else {
                true
            }
        });
        for j in done {
            match j.kind {
                JobKind::Download => self.local.reload(),
                JobKind::Upload => {
                    if let Some(files) = &cx.files {
                        self.remote.reload(files);
                    }
                }
            }
        }
    }

    fn jobs_panel(&mut self, ui: &mut Ui, cx: &Ctx<'_>, jobs: &[Arc<Job>]) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Copies").strong().size(15.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if jobs.iter().any(|j| j.finished())
                    && ui.button("Clear finished").clicked()
                    && let Some(files) = &cx.files
                {
                    files.clear_finished();
                }
            });
        });
        egui::ScrollArea::vertical().id_salt("jobs").auto_shrink([false, false]).show(ui, |ui| {
            for job in jobs.iter().rev() {
                let (done, total) = (job.done.load(Ordering::Relaxed), job.total.load(Ordering::Relaxed));
                let state = job.state();
                ui.horizontal(|ui| {
                    let verb = match (job.kind, job.finished()) {
                        (JobKind::Download, false) => "Getting",
                        (JobKind::Download, true) => "Got",
                        (JobKind::Upload, false) => "Sending",
                        (JobKind::Upload, true) => "Sent",
                    };
                    let dir = match job.kind {
                        JobKind::Download => format!("from {}", job.peer),
                        JobKind::Upload => format!("to {}", job.peer),
                    };
                    ui.label(RichText::new(format!("{verb} {} {dir}", job.what)).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !job.finished() {
                            if ui.button("Cancel").clicked() {
                                job.cancel();
                            }
                        } else if state == JobState::Done
                            && job.kind == JobKind::Download
                            && ui.button("Show").on_hover_text("Open the folder it was copied to").clicked()
                        {
                            open_folder(&job.place);
                        }
                    });
                });
                match &state {
                    JobState::Running => {
                        let frac = if total == 0 { 0.0 } else { done as f32 / total as f32 };
                        ui.add(egui::ProgressBar::new(frac).text(format!("{} of {}", human(done), human(total))));
                    }
                    JobState::Waiting(why) => {
                        ui.add(egui::ProgressBar::new(if total == 0 { 0.0 } else { done as f32 / total as f32 }).text(format!(
                            "{} of {}",
                            human(done),
                            human(total)
                        )));
                        ui.colored_label(AMBER, format!("Paused: {why}. It carries on by itself when the other computer is back."));
                    }
                    JobState::Done => {
                        ui.colored_label(GREEN, format!("Finished, {}", human(total)));
                    }
                    JobState::Failed(why) => {
                        ui.colored_label(RED, why);
                    }
                    JobState::Cancelled => {
                        ui.label(RichText::new("Cancelled").weak());
                    }
                }
                ui.add_space(4.0);
            }
        });
    }

    fn folders_dialog(&mut self, ctx: &egui::Context, cx: &Ctx<'_>) -> Option<Config> {
        if !self.folders_open {
            return None;
        }
        let mut changed: Option<Config> = None;
        let mut close = false;
        egui::Modal::new(egui::Id::new("folders-i-share")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new("Folders I share").strong().size(16.0));
            ui.label(RichText::new("The other computer can browse these and copy files out of them. It can only add files if you allow it, and it can never overwrite or delete anything.").weak());
            ui.add_space(6.0);
            let mut cfg = cx.cfg.clone();
            let mut edited = false;
            if cfg.files.shares.is_empty() {
                ui.label(RichText::new("Nothing is shared yet.").weak());
            }
            let mut remove = None;
            for (i, s) in cfg.files.shares.iter_mut().enumerate() {
                Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&s.name).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Stop sharing").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                    ui.label(RichText::new(s.path.display().to_string()).weak().size(12.0));
                    edited |= ui.checkbox(&mut s.write, "Let the other computer add files and folders").changed();
                });
            }
            if let Some(i) = remove {
                cfg.files.shares.remove(i);
                edited = true;
            }
            ui.add_space(6.0);
            ui.separator();
            ui.label(RichText::new("Share the folder open on the left").strong());
            ui.label(RichText::new(self.local.dir.display().to_string()).weak().size(12.0));
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut self.share_name).desired_width(180.0));
                ui.checkbox(&mut self.share_write, "Allow adding files");
            });
            let name = self.share_name.trim().to_string();
            let taken = cfg.files.shares.iter().any(|s| s.name == name);
            let already = cfg.files.shares.iter().any(|s| s.path == self.local.dir);
            ui.horizontal(|ui| {
                let button = ui.add_enabled(!name.is_empty() && !taken && !already, egui::Button::new("Share this folder"));
                if button.clicked() {
                    cfg.files.shares.push(ShareCfg { name: name.clone(), path: self.local.dir.clone(), write: self.share_write });
                    edited = true;
                }
                if already {
                    ui.label(RichText::new("This folder is already shared.").weak());
                } else if taken {
                    ui.colored_label(AMBER, "Another shared folder already has that name.");
                }
            });
            ui.add_space(6.0);
            if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            if edited {
                changed = Some(cfg);
            }
        });
        if close {
            self.folders_open = false;
        }
        changed
    }
}

fn open_folder(path: &str) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(path).spawn();
}
