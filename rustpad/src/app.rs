//! Application state and command execution.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use eframe::egui;

use crate::cmd::{Cmd, SearchOp};
use crate::doc::{Doc, EditKind, EditOpts, Eol, Mark, Move, Sel, TextEdit};
use crate::editor::{self, EditorState};
use crate::encoding::{self, Enc};
use crate::lang;
use crate::search::{self, FifQuery, FileHits, Mode, Query};
use crate::settings::{SavedMacro, Settings};
use crate::theme::{self, Theme};
use crate::ui::dialogs::Dialogs;

pub struct ResultFile {
    pub path: Option<PathBuf>,
    pub name: String,
    pub hits: Vec<search::Hit>,
}

pub struct ResultSet {
    pub title: String,
    pub files: Vec<ResultFile>,
    pub collapsed: bool,
}

pub enum FifMsg {
    File(FileHits),
    Progress(usize),
    Done(Result<usize, String>),
    Replaced(Result<(usize, usize), String>),
}

/// Modal questions awaiting an answer.
pub enum Pending {
    /// Close these docs (by id) one by one, asking to save modified ones.
    ConfirmClose { queue: Vec<u64>, exit_after: bool },
    ExternalChange(u64),
    ExternalDeleted(u64),
    Message(String),
    ConfirmReload(u64),
    Rename { doc: u64, name: String },
    ConfirmTrash(u64),
    Hash { title: String, text: String },
}

pub struct Closed {
    pub path: PathBuf,
}

pub struct App {
    pub settings: Settings,
    pub themes: Vec<Theme>,
    pub tabs: Vec<Doc>,
    pub active: usize,
    pub ed: EditorState,
    pub ed2: EditorState,
    pub split: bool,
    pub split_vertical: bool,
    pub view2: usize,
    pub dialogs: Dialogs,
    pub pending: Option<Pending>,
    pub status_msg: Option<(String, f64)>,
    pub results: Vec<ResultSet>,
    pub show_results: bool,
    pub fif_rx: Option<Receiver<FifMsg>>,
    pub fif_cancel: Arc<AtomicBool>,
    pub fif_running: bool,
    pub fif_progress: usize,
    pub fif_current: Option<usize>,
    pub recording: bool,
    pub macro_buf: Vec<Cmd>,
    pub last_macro: Vec<Cmd>,
    pub playing: bool,
    pub closed: Vec<Closed>,
    pub clipboard: Option<arboard::Clipboard>,
    pub next_untitled: u32,
    pub fullscreen: bool,
    pub post_it: bool,
    pub always_on_top: bool,
    pub should_exit: bool,
    pub last_session_save: f64,
    pub last_mtime_check: f64,
    pub time: f64,
    pub inc_search: Option<String>,
    pub inc_focus: bool,
    pub tab_drag: Option<usize>,
    pub want_editor_focus: bool,
    pub fn_list_next: f64,
    pub fn_job: Option<(u64, u64, std::sync::mpsc::Receiver<Vec<(usize, String)>>)>,
    pub fn_list_cache: Option<(u64, u64, Vec<(usize, String)>)>,
    pub tree: crate::ui::panels::Tree,
    pub saved_window_state: Option<(bool, bool)>,
    pub frame_count: u64,
    pub tab_scroll_to_active: bool,
    pub pane_swapped: bool,
    pub last_title: String,
    pub script: Option<crate::debug::Script>,
    pub snapshot_thread: Option<std::thread::JoinHandle<()>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Vec<String>) -> App {
        let settings = Settings::load();
        crate::fonts::install(&cc.egui_ctx, &settings);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let themes = theme::load_all(&crate::settings::config_dir());
        let mut app = App {
            settings,
            themes,
            tabs: Vec::new(),
            active: 0,
            ed: EditorState::new(egui::Id::new("editor-main")),
            ed2: EditorState::new(egui::Id::new("editor-second")),
            split: false,
            split_vertical: true,
            view2: 0,
            dialogs: Dialogs::default(),
            pending: None,
            status_msg: None,
            results: Vec::new(),
            show_results: false,
            fif_rx: None,
            fif_cancel: Arc::new(AtomicBool::new(false)),
            fif_running: false,
            fif_progress: 0,
            fif_current: None,
            recording: false,
            macro_buf: Vec::new(),
            last_macro: Vec::new(),
            playing: false,
            closed: Vec::new(),
            clipboard: arboard::Clipboard::new().ok(),
            next_untitled: 1,
            fullscreen: false,
            post_it: false,
            always_on_top: false,
            should_exit: false,
            last_session_save: 0.0,
            last_mtime_check: 0.0,
            time: 0.0,
            inc_search: None,
            inc_focus: false,
            tab_drag: None,
            want_editor_focus: true,
            fn_list_next: 0.0,
            fn_job: None,
            fn_list_cache: None,
            tree: Default::default(),
            saved_window_state: None,
            frame_count: 0,
            tab_scroll_to_active: false,
            pane_swapped: false,
            last_title: String::new(),
            script: crate::debug::Script::from_env(),
            snapshot_thread: None,
        };
        app.apply_theme(&cc.egui_ctx);
        let mut opened_any = false;
        let mut goto: Option<usize> = None;
        for a in &args {
            if let Some(n) = a.strip_prefix("-n") {
                goto = n.parse::<usize>().ok();
                continue;
            }
            if a.starts_with('-') {
                continue;
            }
            let p = PathBuf::from(a);
            if p.is_dir() {
                app.add_workspace_root(&p);
                continue;
            }
            app.open_or_create(&p);
            opened_any = true;
        }
        if let Some(n) = goto {
            app.goto_line(n);
        }
        if !opened_any && app.settings.restore_session {
            crate::session::restore(&mut app);
        }
        if app.tabs.is_empty() {
            app.new_doc();
        }
        app
    }

    pub fn theme(&self) -> &Theme {
        self.themes.iter().find(|t| t.name == self.settings.theme).unwrap_or(&self.themes[0])
    }

    pub fn apply_theme(&mut self, ctx: &egui::Context) {
        let t = self.theme().clone();
        let mut v = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = if t.dark { egui::Color32::from_rgb(0x2b, 0x2d, 0x31) } else { egui::Color32::from_rgb(0xf0, 0xf0, 0xf0) };
        v.window_fill = v.panel_fill;
        v.selection.bg_fill = t.selection.c();
        ctx.set_visuals(v);
    }

    // ------------------------------------------------------------ documents

    pub fn doc(&mut self) -> &mut Doc {
        let i = self.active.min(self.tabs.len() - 1);
        &mut self.tabs[i]
    }

    pub fn doc_ref(&self) -> &Doc {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }

    pub fn apply_doc_opts(&self, d: &mut Doc) {
        d.opts = EditOpts {
            tab_width: self.settings.tab_width,
            use_spaces: self.settings.use_spaces,
            auto_indent: self.settings.auto_indent,
            auto_close: self.settings.auto_close,
            auto_close_tags: self.settings.auto_close_tags,
        };
    }

    pub fn default_eol(&self) -> Eol {
        match self.settings.default_eol.as_str() {
            "crlf" => Eol::CrLf,
            "cr" => Eol::Cr,
            _ => Eol::Lf,
        }
    }

    pub fn new_doc(&mut self) -> usize {
        let enc = Enc::from_label(&self.settings.default_new_encoding).unwrap_or(Enc::Utf8);
        let mut d = Doc::from_text("", enc, self.default_eol());
        d.untitled_no = self.next_untitled;
        self.next_untitled += 1;
        self.apply_doc_opts(&mut d);
        self.tabs.push(d);
        self.active = self.tabs.len() - 1;
        self.want_editor_focus = true;
        self.active
    }

    pub fn index_of_path(&self, p: &Path) -> Option<usize> {
        let canon = std::fs::canonicalize(p).ok();
        self.tabs.iter().position(|d| match (&d.path, &canon) {
            (Some(dp), Some(c)) => dp == p || std::fs::canonicalize(dp).ok().as_ref() == Some(c),
            (Some(dp), None) => dp == p,
            _ => false,
        })
    }

    pub fn open_or_create(&mut self, p: &Path) {
        if p.exists() {
            self.open_path(p);
        } else {
            // like N++: opening a non-existing path creates a new buffer bound to that name
            let mut d = Doc::from_text("", Enc::Utf8, self.default_eol());
            d.path = Some(p.to_path_buf());
            d.lang = lang::detect(Some(p), "");
            d.mark_modified();
            self.apply_doc_opts(&mut d);
            self.tabs.push(d);
            self.active = self.tabs.len() - 1;
        }
    }

    pub fn open_path(&mut self, path: &Path) -> bool {
        if let Some(i) = self.index_of_path(path) {
            self.active = i;
            self.want_editor_focus = true;
            return true;
        }
        match self.load_file(path) {
            Ok(mut d) => {
                self.apply_doc_opts(&mut d);
                // replace a pristine untitled buffer
                if self.tabs.len() == 1 && self.tabs[0].path.is_none() && !self.tabs[0].is_modified() && self.tabs[0].len_chars() == 0 {
                    self.tabs.clear();
                }
                self.tabs.push(d);
                self.active = self.tabs.len() - 1;
                self.settings.add_recent(path);
                self.want_editor_focus = true;
                true
            }
            Err(e) => {
                self.pending = Some(Pending::Message(format!("Cannot open {}:\n{e}", path.display())));
                false
            }
        }
    }

    pub fn load_file(&self, path: &Path) -> Result<Doc, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let decoded = encoding::decode_auto(&bytes);
        let eol = Eol::detect(&decoded.text).unwrap_or_else(|| self.default_eol());
        let mut d = Doc::from_text(&decoded.text, decoded.enc, eol);
        let first: String = decoded.text.lines().next().unwrap_or("").chars().take(200).collect();
        d.lang = lang::detect(Some(path), &first);
        d.path = Some(path.to_path_buf());
        d.disk_mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        d.read_only = std::fs::metadata(path).map(|m| m.permissions().readonly()).unwrap_or(false);
        Ok(d)
    }

    pub fn save_doc(&mut self, idx: usize, force_dialog: bool) -> bool {
        let needs_path = self.tabs[idx].path.is_none() || force_dialog;
        if needs_path {
            let mut dlg = rfd::FileDialog::new().set_title("Save As");
            if let Some(p) = &self.tabs[idx].path {
                if let Some(dir) = p.parent() {
                    dlg = dlg.set_directory(dir);
                }
                if let Some(n) = p.file_name() {
                    dlg = dlg.set_file_name(n.to_string_lossy());
                }
            } else {
                dlg = dlg.set_file_name(format!("{}.txt", self.tabs[idx].display_name()));
            }
            let Some(path) = dlg.save_file() else { return false };
            let first = self.tabs[idx].line_text(0);
            let detected = lang::detect(Some(&path), &first);
            let d = &mut self.tabs[idx];
            d.path = Some(path.clone());
            if d.lang == lang::PLAIN || force_dialog {
                d.set_lang(detected);
            }
            self.settings.add_recent(&path);
        }
        let trim = self.settings.trim_on_save;
        let final_nl = self.settings.final_newline_on_save;
        {
            let d = &mut self.tabs[idx];
            if trim {
                d.trim_whitespace(crate::ops::TrimMode::Trailing);
            }
            if final_nl && d.len_chars() > 0 && d.char_at(d.len_chars() - 1) != Some('\n') && d.char_at(d.len_chars() - 1) != Some('\r') {
                let n = d.len_chars();
                let eol = d.eol.as_str();
                d.exec(vec![TextEdit::insert(n, eol)], EditKind::Other);
            }
        }
        let d = &mut self.tabs[idx];
        let path = d.path.clone().unwrap();
        let text = d.text_string();
        let bytes = encoding::encode(&text, d.enc);
        match std::fs::write(&path, bytes) {
            Ok(()) => {
                d.mark_saved();
                d.disk_mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
                d.disk_missing = false;
                self.set_status(format!("Saved {}", path.display()));
                true
            }
            Err(e) => {
                self.pending = Some(Pending::Message(format!("Failed to save {}:\n{e}", path.display())));
                false
            }
        }
    }

    pub fn save_all(&mut self) {
        for i in 0..self.tabs.len() {
            if self.tabs[i].is_modified() && (self.tabs[i].path.is_some()) {
                self.save_doc(i, false);
            }
        }
    }

    /// Close a tab without asking.
    pub fn force_close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let d = self.tabs.remove(idx);
        if let Some(p) = d.path {
            self.closed.push(Closed { path: p });
            if self.closed.len() > 20 {
                self.closed.remove(0);
            }
        }
        if self.tabs.is_empty() {
            self.new_doc();
        } else if self.active > idx || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
        }
        if self.view2 >= self.tabs.len() {
            self.view2 = self.tabs.len() - 1;
        }
        self.want_editor_focus = true;
    }

    /// Request closing the given tabs, asking about unsaved changes.
    pub fn close_tabs(&mut self, ids: Vec<u64>, exit_after: bool) {
        let mut queue = ids;
        // process in order; ConfirmClose handles the first modified one
        queue.retain(|id| self.tabs.iter().any(|d| d.id == *id));
        self.pending = Some(Pending::ConfirmClose { queue, exit_after });
        self.advance_close();
    }

    /// Drive the close queue: silently close clean docs, stop at the first modified one.
    pub fn advance_close(&mut self) {
        let Some(Pending::ConfirmClose { queue, exit_after }) = self.pending.as_mut() else { return };
        let exit_after = *exit_after;
        loop {
            let Some(&id) = queue.first() else {
                self.pending = None;
                if exit_after {
                    self.should_exit = true;
                }
                return;
            };
            let Some(idx) = self.tabs.iter().position(|d| d.id == id) else {
                if let Some(Pending::ConfirmClose { queue, .. }) = self.pending.as_mut() {
                    queue.remove(0);
                }
                return self.advance_close();
            };
            if self.tabs[idx].is_modified() && !(self.tabs[idx].path.is_none() && self.tabs[idx].len_chars() == 0 && false) {
                self.active = idx;
                return; // wait for the dialog
            }
            self.force_close(idx);
            if let Some(Pending::ConfirmClose { queue, .. }) = self.pending.as_mut() {
                queue.remove(0);
            }
            return self.advance_close();
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some((msg.into(), self.time));
    }

    pub fn goto_line(&mut self, line1: usize) {
        let d = self.doc();
        let l = line1.saturating_sub(1).min(d.len_lines() - 1);
        d.reveal_line(l);
        let p = d.line_start(l);
        d.set_caret(p);
        self.ed.scroll_to_caret = true;
        // center the target
        let rows = self.ed.view_rows.1 as f32;
        let d = self.doc();
        let row = d.rowmap.first_row_of_line(l) as f32;
        d.scroll_request = Some((row - rows / 2.0).max(0.0));
    }

    pub fn reveal(&mut self) {
        self.ed.scroll_to_caret = true;
    }

    pub fn add_workspace_root(&mut self, p: &Path) {
        let s = p.to_string_lossy().into_owned();
        if !self.settings.workspace_roots.contains(&s) {
            self.settings.workspace_roots.push(s);
        }
        self.settings.workspace = true;
    }

    // -------------------------------------------------------------- clipboard

    pub fn clipboard_text(&mut self) -> Option<String> {
        self.clipboard.as_mut().and_then(|c| c.get_text().ok())
    }

    // ----------------------------------------------------------- command exec

    pub fn exec(&mut self, ctx: &egui::Context, cmd: Cmd) {
        if self.recording && !self.playing && cmd.recordable() {
            self.macro_buf.push(cmd.clone());
        }
        let metrics = editor::view_metrics(&self.tabs[self.active.min(self.tabs.len() - 1)], &self.ed, &self.settings);
        match cmd {
            // ---------------- file
            Cmd::New => {
                self.new_doc();
            }
            Cmd::Open => {
                let dir = self.doc_ref().path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
                let mut dlg = rfd::FileDialog::new().set_title("Open");
                if let Some(d) = dir {
                    dlg = dlg.set_directory(d);
                }
                if let Some(files) = dlg.pick_files() {
                    for f in files {
                        self.open_path(&f);
                    }
                }
            }
            Cmd::OpenRecent(i) => {
                if let Some(p) = self.settings.recent_files.get(i).cloned() {
                    let path = PathBuf::from(&p);
                    if path.exists() {
                        self.open_path(&path);
                    } else {
                        self.settings.recent_files.remove(i);
                        self.set_status(format!("File no longer exists: {p}"));
                    }
                }
            }
            Cmd::ClearRecent => self.settings.recent_files.clear(),
            Cmd::Save => {
                let i = self.active;
                self.save_doc(i, false);
            }
            Cmd::SaveAs => {
                let i = self.active;
                self.save_doc(i, true);
            }
            Cmd::SaveAll => self.save_all(),
            Cmd::Close => {
                let id = self.doc_ref().id;
                self.close_tabs(vec![id], false);
            }
            Cmd::CloseAll => {
                let ids = self.tabs.iter().map(|d| d.id).collect();
                self.close_tabs(ids, false);
            }
            Cmd::CloseOthers => {
                let keep = self.doc_ref().id;
                let ids = self.tabs.iter().map(|d| d.id).filter(|i| *i != keep).collect();
                self.close_tabs(ids, false);
            }
            Cmd::CloseRight => {
                let ids = self.tabs.iter().skip(self.active + 1).map(|d| d.id).collect();
                self.close_tabs(ids, false);
            }
            Cmd::CloseLeft => {
                let ids = self.tabs.iter().take(self.active).map(|d| d.id).collect();
                self.close_tabs(ids, false);
            }
            Cmd::ReopenClosed => {
                if let Some(c) = self.closed.pop() {
                    self.open_path(&c.path);
                }
            }
            Cmd::Reload => {
                let d = self.doc_ref();
                if d.path.is_some() {
                    if d.is_modified() {
                        self.pending = Some(Pending::ConfirmReload(d.id));
                    } else {
                        self.reload_active();
                    }
                }
            }
            Cmd::Rename => {
                let d = self.doc_ref();
                let name = d.display_name();
                self.pending = Some(Pending::Rename { doc: d.id, name });
            }
            Cmd::MoveToTrash => {
                if self.doc_ref().path.is_some() {
                    self.pending = Some(Pending::ConfirmTrash(self.doc_ref().id));
                }
            }
            Cmd::OpenContainingFolder => {
                if let Some(p) = self.doc_ref().path.clone() {
                    reveal_in_file_manager(&p);
                }
            }
            Cmd::CopyPath => {
                if let Some(p) = &self.doc_ref().path {
                    ctx.copy_text(p.to_string_lossy().into_owned());
                }
            }
            Cmd::CopyFilename => {
                let n = self.doc_ref().display_name();
                ctx.copy_text(n);
            }
            Cmd::CopyDirectory => {
                if let Some(p) = self.doc_ref().path.as_ref().and_then(|p| p.parent()) {
                    ctx.copy_text(p.to_string_lossy().into_owned());
                }
            }
            Cmd::Exit => {
                if self.settings.restore_session {
                    self.should_exit = true;
                } else {
                    let ids = self.tabs.iter().map(|d| d.id).collect();
                    self.close_tabs(ids, true);
                }
            }
            Cmd::NextTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + 1) % self.tabs.len();
                    self.want_editor_focus = true;
                }
            }
            Cmd::PrevTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
                    self.want_editor_focus = true;
                }
            }
            Cmd::MoveTabLeft => {
                if self.active > 0 {
                    self.tabs.swap(self.active, self.active - 1);
                    self.active -= 1;
                }
            }
            Cmd::MoveTabRight => {
                if self.active + 1 < self.tabs.len() {
                    self.tabs.swap(self.active, self.active + 1);
                    self.active += 1;
                }
            }
            Cmd::OpenFolderAsWorkspace => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Open folder as workspace").pick_folder() {
                    self.add_workspace_root(&dir);
                }
            }

            // ---------------- edit
            Cmd::Type(s) => {
                self.doc().type_text(&s);
                self.after_type(&s);
            }
            Cmd::Newline => {
                self.ed.ac = None;
                self.doc().newline();
            }
            Cmd::Backspace => {
                self.doc().backspace();
                self.refresh_completion();
            }
            Cmd::DeleteFwd => self.doc().delete_forward(),
            Cmd::DeleteWordLeft => self.doc().delete_word_left(),
            Cmd::DeleteWordRight => self.doc().delete_word_right(),
            Cmd::Tab => {
                self.ed.ac = None;
                self.doc().tab();
            }
            Cmd::Unindent => self.doc().indent_lines(false),
            Cmd::Move(m, extend) => {
                self.ed.ac = None;
                if matches!(m, Move::Left | Move::Right | Move::Up | Move::Down) {
                    self.doc().move_sels(m, extend, metrics);
                } else {
                    self.doc().move_sels(m, extend, metrics);
                }
            }
            Cmd::RectMove(m) => {
                let d = self.doc();
                if d.rect_anchor.is_none() {
                    let h = d.caret();
                    let (l, _) = d.linecol(h);
                    let col = d.display_col_of(h);
                    d.rect_anchor = Some((l, col));
                    d.rect_head = Some((l, col));
                }
                let (al, ac) = d.rect_anchor.unwrap();
                let (mut hl, mut hc) = d.rect_head.unwrap_or((al, ac));
                match m {
                    Move::Left => hc = hc.saturating_sub(1),
                    Move::Right => hc += 1,
                    Move::Up => hl = hl.saturating_sub(1),
                    Move::Down => hl = (hl + 1).min(d.len_lines() - 1),
                    Move::Home | Move::LineStart => hc = 0,
                    Move::End | Move::LineEnd => hc = (0..d.len_lines()).map(|l| crate::util::display_width(&d.line_text(l), d.opts.tab_width)).max().unwrap_or(0),
                    _ => {}
                }
                d.rect_head = Some((hl, hc));
                editor::set_rect_selection(d, al, ac, hl, hc);
                d.rect_head = Some((hl, hc));
            }
            Cmd::Hash(kind) => {
                let d = self.doc_ref();
                let text = if d.has_selection() { d.selected_text() } else { (*d.text_string()).clone() };
                let hex = kind.digest_hex(text.as_bytes());
                self.pending = Some(Pending::Hash { title: format!("{} of {}", kind.name(), if self.doc_ref().has_selection() { "selection" } else { "document" }), text: hex });
            }
            Cmd::HashFiles(kind) => {
                if let Some(files) = rfd::FileDialog::new().set_title("Generate hash from files").pick_files() {
                    let mut out = String::new();
                    for f in files {
                        match std::fs::read(&f) {
                            Ok(bytes) => out.push_str(&format!("{}  {}\n", kind.digest_hex(&bytes), f.display())),
                            Err(e) => out.push_str(&format!("{}: {e}\n", f.display())),
                        }
                    }
                    self.pending = Some(Pending::Hash { title: format!("{} of files", kind.name()), text: out });
                }
            }
            Cmd::SelectAll => self.doc().select_all(),
            Cmd::Undo => {
                self.doc().undo();
            }
            Cmd::Redo => {
                self.doc().redo();
            }
            Cmd::Cut => {
                let d = self.doc();
                if !d.has_selection() {
                    let t = d.lines_text();
                    d.delete_lines();
                    ctx.copy_text(t);
                } else {
                    let t = d.selected_text();
                    d.delete_selection();
                    ctx.copy_text(t);
                }
            }
            Cmd::Copy => {
                let d = self.doc();
                let t = if d.has_selection() { d.selected_text() } else { d.lines_text() };
                if !t.is_empty() {
                    ctx.copy_text(t);
                }
            }
            Cmd::Paste => {
                if let Some(t) = self.clipboard_text() {
                    self.doc().paste(&t);
                    self.ed.ac = None;
                }
            }
            Cmd::PasteText(t) => {
                self.doc().paste(&t);
                self.ed.ac = None;
            }
            Cmd::DeleteSel => self.doc().delete_selection(),
            Cmd::ToggleOverwrite => {
                let d = self.doc();
                d.overwrite = !d.overwrite;
            }
            Cmd::DuplicateLine => self.doc().duplicate(),
            Cmd::DeleteLine => self.doc().delete_lines(),
            Cmd::CutLine => {
                let d = self.doc();
                let t = d.lines_text();
                d.delete_lines();
                ctx.copy_text(t);
            }
            Cmd::CopyLine => {
                let t = self.doc_ref().lines_text();
                ctx.copy_text(t);
            }
            Cmd::MoveLineUp => self.doc().move_lines(true),
            Cmd::MoveLineDown => self.doc().move_lines(false),
            Cmd::JoinLines => self.doc().join_lines(),
            Cmd::SplitLines => {
                let w = if self.settings.show_edge { self.settings.edge_column } else { 80 };
                self.doc().split_lines_at(w);
            }
            Cmd::TransposeLine => self.doc().transpose_lines(),
            Cmd::BlankAbove => self.doc().insert_blank_line(true),
            Cmd::BlankBelow => self.doc().insert_blank_line(false),
            Cmd::Sort(m, desc) => self.doc().sort_lines(m, desc),
            Cmd::DedupLines(c) => self.doc().remove_duplicate_lines(c),
            Cmd::RemoveEmpty(b) => self.doc().remove_empty_lines(b),
            Cmd::Trim(m) => self.doc().trim_whitespace(m),
            Cmd::TabsToSpaces => self.doc().tabs_to_spaces(),
            Cmd::SpacesToTabs(all) => self.doc().spaces_to_tabs(all),
            Cmd::Case(m) => self.doc().change_case(m),
            Cmd::ToggleComment => self.doc().toggle_line_comment(),
            Cmd::BlockComment => self.doc().toggle_block_comment(),
            Cmd::ConvertEol(e) => self.doc().convert_eol(e),
            Cmd::GotoBrace => {
                let d = self.doc();
                if let Some((a, b)) = d.find_matching_bracket(d.caret()) {
                    let target = if d.char_at(d.caret()).is_some_and(|c| "()[]{}".contains(c)) && d.caret() == a { b } else { b };
                    d.set_caret(target);
                }
            }
            Cmd::SelectBrace => {
                let d = self.doc();
                if let Some((a, b)) = d.find_matching_bracket(d.caret()) {
                    d.set_selection(a.min(b), a.max(b) + 1);
                }
            }
            Cmd::SelectLines => self.doc().select_lines(),
            Cmd::InsertDateTime(long) => {
                let now = chrono::Local::now();
                let s = if long { now.format("%A, %B %d, %Y %H:%M:%S").to_string() } else { now.format("%H:%M %Y-%m-%d").to_string() };
                self.doc().type_text(&s);
            }
            Cmd::ColumnEditor => self.dialogs.column_open = true,
            Cmd::AutoComplete => {
                self.ed.ac = editor::compute_completion(self.doc_ref());
                if self.ed.ac.is_none() {
                    self.set_status("No completions");
                }
            }
            Cmd::MultiSelectNext => self.multi_select_next(),
            Cmd::MultiSelectAll => self.multi_select_all(),
            Cmd::CopyHtmlLikeText => {}

            // ---------------- search
            Cmd::Find => self.open_find(crate::ui::dialogs::FindTab::Find),
            Cmd::Replace => self.open_find(crate::ui::dialogs::FindTab::Replace),
            Cmd::FindInFiles => self.open_find(crate::ui::dialogs::FindTab::Files),
            Cmd::Mark => self.open_find(crate::ui::dialogs::FindTab::Mark),
            Cmd::FindNext => {
                let q = self.settings.find.clone();
                self.search_step(q, false);
            }
            Cmd::FindPrev => {
                let q = self.settings.find.clone();
                self.search_step(q, true);
            }
            Cmd::FindSelNext | Cmd::FindSelPrev => {
                let back = cmd == Cmd::FindSelPrev;
                let d = self.doc_ref();
                let word = if d.has_selection() {
                    d.selected_text()
                } else {
                    let (s, e) = d.word_range_at(d.caret());
                    d.slice(s, e)
                };
                if !word.is_empty() && !word.contains('\n') {
                    self.settings.find.pattern = word;
                    self.settings.find.mode = if self.settings.find.mode == Mode::Regex { Mode::Normal } else { self.settings.find.mode };
                    let mut q = self.settings.find.clone();
                    q.mode = if q.mode == Mode::Regex { Mode::Normal } else { q.mode };
                    // select the word first so repeated invocation moves on
                    let d = self.doc();
                    if !d.has_selection() {
                        let (s, e) = d.word_range_at(d.caret());
                        d.set_selection(s, e);
                    }
                    self.search_step(q, back);
                }
            }
            Cmd::IncrementalSearch => {
                self.inc_search = Some(String::new());
                self.inc_focus = true;
            }
            Cmd::GotoLine => self.dialogs.goto_open = true,
            Cmd::Search(op, q) => self.search_op(ctx, op, q),
            Cmd::ToggleBookmark => {
                let d = self.doc();
                let l = d.line_of(d.caret());
                d.toggle_bookmark(l);
            }
            Cmd::NextBookmark => {
                let d = self.doc();
                let l = d.line_of(d.caret());
                if let Some(t) = d.next_bookmark(l) {
                    d.reveal_line(t);
                    let p = d.line_start(t);
                    d.set_caret(p);
                }
            }
            Cmd::PrevBookmark => {
                let d = self.doc();
                let l = d.line_of(d.caret());
                if let Some(t) = d.prev_bookmark(l) {
                    d.reveal_line(t);
                    let p = d.line_start(t);
                    d.set_caret(p);
                }
            }
            Cmd::ClearBookmarks => self.doc().bookmarks.clear(),
            Cmd::CutBookmarked => {
                let t = self.doc_ref().bookmarked_text();
                ctx.copy_text(t);
                self.doc().remove_bookmarked_lines();
            }
            Cmd::CopyBookmarked => {
                let t = self.doc_ref().bookmarked_text();
                ctx.copy_text(t);
            }
            Cmd::RemoveBookmarked => self.doc().remove_bookmarked_lines(),
            Cmd::InverseBookmarks => self.doc().inverse_bookmarks(),
            Cmd::ClearMarks => self.doc().marks.clear(),

            // ---------------- view
            Cmd::ToggleWrap => self.settings.word_wrap = !self.settings.word_wrap,
            Cmd::ToggleWhitespace => self.settings.show_whitespace = !self.settings.show_whitespace,
            Cmd::ToggleEol => self.settings.show_eol = !self.settings.show_eol,
            Cmd::ToggleAllChars => {
                let on = !(self.settings.show_whitespace && self.settings.show_eol);
                self.settings.show_whitespace = on;
                self.settings.show_eol = on;
            }
            Cmd::ToggleIndentGuides => self.settings.indent_guides = !self.settings.indent_guides,
            Cmd::ToggleLineNumbers => self.settings.line_numbers = !self.settings.line_numbers,
            Cmd::ToggleBookmarkMargin => self.settings.bookmark_margin = !self.settings.bookmark_margin,
            Cmd::ToggleFoldMargin => self.settings.fold_margin = !self.settings.fold_margin,
            Cmd::ToggleStatusBar => self.settings.status_bar = !self.settings.status_bar,
            Cmd::ToggleToolBar => self.settings.tool_bar = !self.settings.tool_bar,
            Cmd::ToggleTabBar => self.settings.tab_bar = !self.settings.tab_bar,
            Cmd::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
            Cmd::TogglePostIt => {
                self.post_it = !self.post_it;
                if self.post_it {
                    self.saved_window_state = Some((self.settings.tool_bar, self.settings.tab_bar));
                    self.fullscreen = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                } else {
                    self.fullscreen = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                }
            }
            Cmd::ToggleAlwaysOnTop => {
                self.always_on_top = !self.always_on_top;
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                }));
            }
            Cmd::ZoomIn => self.settings.zoom = (self.settings.zoom + 1.0).min(40.0),
            Cmd::ZoomOut => self.settings.zoom = (self.settings.zoom - 1.0).max(-self.settings.font_size + 6.0),
            Cmd::ZoomReset => self.settings.zoom = 0.0,
            Cmd::FoldAll => self.doc().fold_all(),
            Cmd::UnfoldAll => self.doc().unfold_all(),
            Cmd::FoldLevel(n) => self.doc().fold_level(n, true),
            Cmd::UnfoldLevel(n) => self.doc().fold_level(n, false),
            Cmd::ToggleFold => {
                let d = self.doc();
                let l = d.line_of(d.caret());
                // fold the nearest enclosing/current range
                let target = d.fold_starts.iter().rev().find(|f| f.0 <= l && l <= f.1).map(|f| f.0);
                if let Some(t) = target {
                    d.toggle_fold(t);
                }
            }
            Cmd::ToggleDocMap => self.settings.doc_map = !self.settings.doc_map,
            Cmd::ToggleFunctionList => self.settings.function_list = !self.settings.function_list,
            Cmd::ToggleWorkspace => self.settings.workspace = !self.settings.workspace,
            Cmd::ToggleResults => self.show_results = !self.show_results,
            Cmd::ToggleSplitOrientation => {
                self.split_vertical = !self.split_vertical;
                self.split = true;
            }
            Cmd::ToggleSplit => {
                self.split = !self.split;
                if self.split {
                    self.view2 = self.active;
                }
            }

            // ---------------- language / encoding
            Cmd::SetLang(id) => self.doc().set_lang(id),
            Cmd::SetEncoding(label) => {
                if let Some(e) = Enc::from_label(&label) {
                    let d = self.doc();
                    d.enc = e;
                    d.mark_modified();
                }
            }
            Cmd::ReloadAsEncoding(label) => {
                if let Some(e) = Enc::from_label(&label) {
                    self.reload_with(e);
                }
            }
            Cmd::ToggleReadOnly => {
                let d = self.doc();
                d.read_only = !d.read_only;
            }
            Cmd::SetTheme(name) => {
                self.settings.theme = name;
                self.apply_theme(ctx);
            }

            // ---------------- macros
            Cmd::MacroRecord => {
                if self.recording {
                    self.exec(ctx, Cmd::MacroStop);
                } else {
                    self.recording = true;
                    self.macro_buf.clear();
                    self.set_status("Recording macro...");
                }
            }
            Cmd::MacroStop => {
                if self.recording {
                    self.recording = false;
                    self.last_macro = std::mem::take(&mut self.macro_buf);
                    self.set_status(format!("Macro recorded ({} steps)", self.last_macro.len()));
                }
            }
            Cmd::MacroPlay => {
                let steps = self.last_macro.clone();
                self.play_macro(ctx, &steps, 1);
            }
            Cmd::MacroRunMultiple => self.dialogs.macro_run_open = true,
            Cmd::MacroSave => {
                if !self.last_macro.is_empty() {
                    self.dialogs.macro_name = format!("Macro {}", self.settings.macros.len() + 1);
                    self.dialogs.macro_save_open = true;
                }
            }
            Cmd::MacroPlayNamed(i) => {
                if let Some(m) = self.settings.macros.get(i).cloned() {
                    self.play_macro(ctx, &m.steps, 1);
                }
            }

            // ---------------- misc
            Cmd::Preferences => self.dialogs.prefs_open = true,
            Cmd::About => self.dialogs.about_open = true,
            Cmd::Run => self.dialogs.run_open = true,
            Cmd::RunNamed(i) => {
                if let Some(r) = self.settings.run_commands.get(i).cloned() {
                    self.run_command(&r.command);
                }
            }
            Cmd::LaunchInBrowser => {
                if let Some(p) = self.doc_ref().path.clone() {
                    open_with_default(&p.to_string_lossy());
                }
            }
            Cmd::Shortcuts => self.dialogs.shortcuts_open = true,
            Cmd::Nop => {}
        }
    }

    pub fn play_macro(&mut self, ctx: &egui::Context, steps: &[Cmd], times: usize) {
        let was = self.playing;
        self.playing = true;
        for _ in 0..times {
            for s in steps {
                self.exec(ctx, s.clone());
            }
        }
        self.playing = was;
        self.reveal();
    }

    fn after_type(&mut self, s: &str) {
        // keep an open completion list in sync with what was typed
        if self.ed.ac.is_some() {
            if s.chars().all(crate::util::is_word_char) {
                self.refresh_completion();
            } else {
                self.ed.ac = None;
            }
        }
    }

    fn refresh_completion(&mut self) {
        if self.ed.ac.is_some() {
            self.ed.ac = editor::compute_completion(self.doc_ref());
        }
    }

    pub fn reload_active(&mut self) {
        let enc = self.doc_ref().enc;
        self.reload_with_opt(None, enc);
    }

    pub fn reload_with(&mut self, enc: Enc) {
        self.reload_with_opt(Some(enc), enc);
    }

    fn reload_with_opt(&mut self, force: Option<Enc>, _cur: Enc) {
        let Some(path) = self.doc_ref().path.clone() else { return };
        match std::fs::read(&path) {
            Ok(bytes) => {
                let (text, enc) = match force {
                    Some(e) => (encoding::decode_with(&bytes, e), e),
                    None => {
                        let d = encoding::decode_auto(&bytes);
                        (d.text, d.enc)
                    }
                };
                let mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
                let eol = Eol::detect(&text);
                let d = self.doc();
                let caret = d.caret();
                d.reset_text(&text);
                d.enc = enc;
                if let Some(e) = eol {
                    d.eol = e;
                }
                d.disk_mtime = mtime;
                d.disk_missing = false;
                let p = caret.min(d.len_chars());
                d.set_caret(p);
                d.mark_saved();
            }
            Err(e) => self.pending = Some(Pending::Message(format!("Cannot reload: {e}"))),
        }
    }

    fn multi_select_next(&mut self) {
        let d = self.doc();
        let last = *d.sels.last().unwrap();
        if last.is_empty() {
            let (s, e) = d.word_range_at(last.head);
            if s < e {
                d.set_selection(s, e);
            }
            return;
        }
        let needle = d.slice(last.start(), last.end());
        let q = Query { pattern: needle, match_case: true, ..Default::default() };
        if let Ok(Some(f)) = search::find(d, &q, last.end(), false, None) {
            if !d.sels.iter().any(|s| s.start() == f.m.start) {
                d.add_selection(Sel::new(f.m.start, f.m.end));
            }
        }
    }

    fn multi_select_all(&mut self) {
        let d = self.doc();
        let last = d.primary_sel();
        let needle = if last.is_empty() {
            let (s, e) = d.word_range_at(last.head);
            d.slice(s, e)
        } else {
            d.slice(last.start(), last.end())
        };
        if needle.is_empty() {
            return;
        }
        let q = Query { pattern: needle, match_case: true, ..Default::default() };
        if let Ok(all) = search::find_all(d, &q, None) {
            let sels: Vec<Sel> = all.iter().map(|m| Sel::new(m.start, m.end)).collect();
            if !sels.is_empty() {
                d.sels = sels;
                d.primary = d.sels.len() - 1;
                d.normalize();
            }
        }
    }

    // ----------------------------------------------------------------- search

    pub fn open_find(&mut self, tab: crate::ui::dialogs::FindTab) {
        let d = self.doc_ref();
        // prefill with the selection (single line) or word under caret
        let sel = if d.has_selection() {
            let t = d.selected_text();
            (!t.contains(['\n', '\r']) && t.chars().count() < 200).then_some(t)
        } else {
            None
        };
        let in_sel = d.has_selection() && d.selected_text().contains(['\n', '\r']);
        let sel_range = if in_sel { Some((d.primary_sel().start(), d.primary_sel().end())) } else { None };
        let word = {
            let (s, e) = d.word_range_at(d.caret());
            d.slice(s, e)
        };
        self.dialogs.find_sel_range = sel_range;
        if let Some(t) = sel {
            self.settings.find.pattern = t;
        } else if self.settings.find.pattern.is_empty() && !word.is_empty() {
            self.settings.find.pattern = word;
        }
        self.dialogs.find_open = true;
        self.dialogs.find_tab = tab;
        self.dialogs.find_focus = true;
        if self.settings.find_dir.is_empty() {
            if let Some(p) = self.doc_ref().path.as_ref().and_then(|p| p.parent()) {
                self.settings.find_dir = p.to_string_lossy().into_owned();
            }
        }
    }

    /// Find next/previous using `q` from the current selection, selecting the result.
    pub fn search_step(&mut self, mut q: Query, backward: bool) -> bool {
        if q.pattern.is_empty() {
            self.open_find(crate::ui::dialogs::FindTab::Find);
            return false;
        }
        q.backward = backward;
        let range = if q.in_selection { self.dialogs.find_sel_range } else { None };
        let d = self.doc();
        let sel = d.primary_sel();
        let from = if backward { sel.start() } else { sel.end() };
        match search::find(d, &q, from, backward, range) {
            Ok(Some(mut f)) => {
                // avoid getting stuck on an empty match at the caret
                if f.m.start == f.m.end && sel.is_empty() && f.m.start == sel.head && !f.wrapped {
                    let step = if backward { from.saturating_sub(1) } else { (from + 1).min(d.len_chars()) };
                    if let Ok(Some(f2)) = search::find(d, &q, step, backward, range) {
                        f = f2;
                    }
                }
                d.reveal_line(d.line_of(f.m.start));
                if backward {
                    d.set_selection(f.m.end, f.m.start);
                } else {
                    d.set_selection(f.m.start, f.m.end);
                }
                let wrapped = f.wrapped;
                self.reveal();
                if wrapped {
                    self.set_status("Search wrapped around the document");
                    self.dialogs.find_status = "Search wrapped".into();
                } else {
                    self.dialogs.find_status.clear();
                }
                self.remember_query(&q);
                true
            }
            Ok(None) => {
                let msg = format!("Can't find the text: \"{}\"", q.pattern);
                self.dialogs.find_status = msg.clone();
                self.set_status(msg);
                false
            }
            Err(e) => {
                self.dialogs.find_status = format!("Invalid search: {e}");
                false
            }
        }
    }

    fn remember_query(&mut self, q: &Query) {
        Settings::push_history(&mut self.settings.find_history, &q.pattern);
        if !q.replace.is_empty() {
            Settings::push_history(&mut self.settings.replace_history, &q.replace);
        }
    }

    pub fn search_op(&mut self, ctx: &egui::Context, op: SearchOp, mut q: Query) {
        let _ = ctx;
        if q.pattern.is_empty() {
            self.dialogs.find_status = "Enter a search pattern".into();
            return;
        }
        let range = if q.in_selection { self.dialogs.find_sel_range } else { None };
        match op {
            SearchOp::FindNext => {
                self.search_step(q, false);
            }
            SearchOp::FindPrev => {
                self.search_step(q, true);
            }
            SearchOp::Replace => {
                q.backward = false;
                // if the selection is a match, replace it; then advance
                let d = self.doc();
                let sel = d.primary_sel();
                let mut replaced = false;
                if !sel.is_empty() || true {
                    if let Ok(Some(rep)) = search::expand_at(d, &q, search::Match { start: sel.start(), end: sel.end() }) {
                        if !sel.is_empty() || matches!(q.mode, Mode::Regex) {
                            let n = rep.chars().count();
                            d.exec(vec![TextEdit::replace(sel.start(), sel.end(), rep).after(crate::doc::After::Select(n, n))], EditKind::Other);
                            replaced = true;
                        }
                    }
                }
                self.remember_query(&q);
                let found = self.search_step(q, false);
                if replaced && !found {
                    self.dialogs.find_status = "Replaced; no more matches".into();
                }
            }
            SearchOp::ReplaceAll => {
                let d = self.doc();
                match search::replace_all_edits(d, &q, range) {
                    Ok(edits) => {
                        let n = edits.len();
                        if n > 0 {
                            d.exec(edits, EditKind::Other);
                        }
                        self.remember_query(&q);
                        let msg = format!("Replace All: {n} occurrence(s) were replaced");
                        self.dialogs.find_status = msg.clone();
                        self.set_status(msg);
                    }
                    Err(e) => self.dialogs.find_status = format!("Invalid search: {e}"),
                }
            }
            SearchOp::Count => {
                let d = self.doc();
                match search::find_all(d, &q, range) {
                    Ok(v) => {
                        let msg = format!("Count: {} match(es)", v.len());
                        self.dialogs.find_status = msg.clone();
                        self.set_status(msg);
                    }
                    Err(e) => self.dialogs.find_status = format!("Invalid search: {e}"),
                }
            }
            SearchOp::MarkAll => {
                let style = self.dialogs.mark_style;
                let bookmark = self.dialogs.mark_bookmark;
                let d = self.doc();
                match search::find_all(d, &q, range) {
                    Ok(v) => {
                        let n = v.len();
                        let mut lines = Vec::new();
                        for m in &v {
                            d.marks.push(Mark { start: m.start, end: m.end, style });
                            lines.push(d.line_of(m.start));
                        }
                        if bookmark {
                            for l in lines {
                                d.bookmarks.insert(l);
                            }
                        }
                        self.dialogs.find_status = format!("Marked {n} match(es)");
                    }
                    Err(e) => self.dialogs.find_status = format!("Invalid search: {e}"),
                }
            }
            SearchOp::FindAll => {
                self.find_all_current(q);
            }
        }
    }

    pub fn find_all_current(&mut self, q: Query) {
        let d = self.doc_ref();
        let hits = match search::build_regex(&q) {
            Ok(re) => {
                let text = d.text_string();
                search::search_text(&re, &text, q.whole_word)
            }
            Err(e) => {
                self.dialogs.find_status = format!("Invalid search: {e}");
                return;
            }
        };
        let name = d.display_name();
        let path = d.path.clone();
        let total: usize = hits.iter().map(|h| h.ranges.len()).sum();
        let title = format!("Search \"{}\" ({} hit{} in 1 file)", q.pattern, total, if total == 1 { "" } else { "s" });
        self.push_results(ResultSet { title, files: vec![ResultFile { path, name, hits, }], collapsed: false });
        self.remember_query(&q);
    }

    pub fn find_all_open_docs(&mut self, q: Query) {
        let re = match search::build_regex(&q) {
            Ok(r) => r,
            Err(e) => {
                self.dialogs.find_status = format!("Invalid search: {e}");
                return;
            }
        };
        let mut files = Vec::new();
        let mut total = 0;
        for d in &self.tabs {
            let text = d.text_string();
            let hits = search::search_text(&re, &text, q.whole_word);
            if !hits.is_empty() {
                total += hits.iter().map(|h| h.ranges.len()).sum::<usize>();
                files.push(ResultFile { path: d.path.clone(), name: d.display_name(), hits });
            }
        }
        let title = format!("Search \"{}\" ({} hits in {} files)", q.pattern, total, files.len());
        self.push_results(ResultSet { title, files, collapsed: false });
        self.remember_query(&q);
    }

    pub fn replace_all_open_docs(&mut self, q: Query) {
        let mut n = 0;
        for d in &mut self.tabs {
            if let Ok(edits) = search::replace_all_edits(d, &q, None) {
                n += edits.len();
                d.exec(edits, EditKind::Other);
            }
        }
        self.dialogs.find_status = format!("Replace All in All Opened Documents: {n} occurrence(s)");
        self.remember_query(&q);
    }

    pub fn push_results(&mut self, rs: ResultSet) {
        for r in &mut self.results {
            r.collapsed = true;
        }
        self.results.insert(0, rs);
        self.results.truncate(10);
        self.show_results = true;
    }

    pub fn start_find_in_files(&mut self, q: Query) {
        if self.fif_running {
            return;
        }
        let dir = PathBuf::from(self.settings.find_dir.clone());
        if !dir.is_dir() {
            self.dialogs.find_status = "Directory does not exist".into();
            return;
        }
        let f = FifQuery {
            dir,
            filters: self.settings.find_filters.clone(),
            recursive: self.settings.find_recursive,
            hidden: self.settings.find_hidden,
        };
        let (tx, rx) = mpsc::channel();
        self.fif_rx = Some(rx);
        self.fif_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.fif_cancel.clone();
        self.fif_running = true;
        self.fif_progress = 0;
        self.remember_query(&q);
        Settings::push_history(&mut self.dialogs.dir_history, &self.settings.find_dir.clone());
        let title = format!("Search \"{}\" in {}", q.pattern, f.dir.display());
        self.push_results(ResultSet { title, files: Vec::new(), collapsed: false });
        self.fif_current = Some(0);
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let tx3 = tx.clone();
            let r = search::find_in_files(&q, &f, &cancel, move |fh| {
                let _ = tx2.send(FifMsg::File(fh));
            }, move |n| {
                let _ = tx3.send(FifMsg::Progress(n));
            });
            let _ = tx.send(FifMsg::Done(r));
        });
    }

    pub fn start_replace_in_files(&mut self, q: Query) {
        if self.fif_running {
            return;
        }
        let dir = PathBuf::from(self.settings.find_dir.clone());
        if !dir.is_dir() {
            self.dialogs.find_status = "Directory does not exist".into();
            return;
        }
        let f = FifQuery {
            dir,
            filters: self.settings.find_filters.clone(),
            recursive: self.settings.find_recursive,
            hidden: self.settings.find_hidden,
        };
        let (tx, rx) = mpsc::channel();
        self.fif_rx = Some(rx);
        self.fif_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.fif_cancel.clone();
        self.fif_running = true;
        self.remember_query(&q);
        std::thread::spawn(move || {
            let r = search::replace_in_files(&q, &f, &cancel);
            let _ = tx.send(FifMsg::Replaced(r));
        });
    }

    pub fn poll_find_in_files(&mut self) {
        let Some(rx) = &self.fif_rx else { return };
        let mut done = false;
        let mut incoming = Vec::new();
        while let Ok(m) = rx.try_recv() {
            incoming.push(m);
        }
        for m in incoming {
            match m {
                FifMsg::File(fh) => {
                    if let Some(rs) = self.results.first_mut() {
                        let name = fh.path.to_string_lossy().into_owned();
                        rs.files.push(ResultFile { path: Some(fh.path), name, hits: fh.hits });
                    }
                }
                FifMsg::Progress(n) => self.fif_progress = n,
                FifMsg::Done(r) => {
                    done = true;
                    match r {
                        Ok(n) => {
                            if let Some(rs) = self.results.first_mut() {
                                let hits: usize = rs.files.iter().map(|f| f.hits.iter().map(|h| h.ranges.len()).sum::<usize>()).sum();
                                rs.title = format!("{} — {} hits in {} files ({} files searched)", rs.title, hits, rs.files.len(), n);
                            }
                            self.dialogs.find_status = "Find in Files finished".into();
                        }
                        Err(e) => self.dialogs.find_status = format!("Invalid search: {e}"),
                    }
                }
                FifMsg::Replaced(r) => {
                    done = true;
                    self.dialogs.find_status = match r {
                        Ok((files, n)) => format!("Replace in Files: {n} occurrence(s) in {files} file(s)"),
                        Err(e) => format!("Replace failed: {e}"),
                    };
                    // refresh any open docs that changed on disk
                    self.last_mtime_check = 0.0;
                }
            }
        }
        if done {
            self.fif_running = false;
            self.fif_rx = None;
        }
    }

    /// Jump to a search result.
    pub fn goto_result(&mut self, path: Option<&Path>, name: &str, line: usize, cols: (usize, usize)) {
        let idx = match path {
            Some(p) => {
                if self.index_of_path(p).is_none() && !self.open_path(p) {
                    return;
                }
                self.index_of_path(p)
            }
            None => self.tabs.iter().position(|d| d.path.is_none() && d.display_name() == name),
        };
        if let Some(i) = idx {
            self.active = i;
            let d = self.doc();
            let l = line.min(d.len_lines() - 1);
            d.reveal_line(l);
            let s = d.line_start(l);
            let a = (s + cols.0).min(d.line_end(l));
            let b = (s + cols.1).min(d.line_end(l));
            d.set_selection(a, b);
            self.ed.scroll_to_caret = true;
            let rows = self.ed.view_rows.1 as f32;
            let d = self.doc();
            let row = d.rowmap.first_row_of_line(l) as f32;
            if row < d.scroll_row || row > d.scroll_row + rows {
                d.scroll_request = Some((row - rows / 2.0).max(0.0));
            }
            self.want_editor_focus = true;
        }
    }

    // ------------------------------------------------------------------ misc

    pub fn run_command(&mut self, template: &str) {
        let (path, dir, name, stem, word) = {
            let d = self.doc_ref();
            let p = d.path.clone().unwrap_or_default();
            let (s, e) = d.word_range_at(d.caret());
            (
                p.to_string_lossy().into_owned(),
                p.parent().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(),
                p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(),
                p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(),
                d.slice(s, e),
            )
        };
        let cmdline = template
            .replace("$(FULL_CURRENT_PATH)", &path)
            .replace("$(CURRENT_DIRECTORY)", &dir)
            .replace("$(FILE_NAME)", &name)
            .replace("$(NAME_PART)", &stem)
            .replace("$(CURRENT_WORD)", &word);
        let res = if cfg!(windows) {
            std::process::Command::new("cmd").args(["/C", &cmdline]).current_dir(if dir.is_empty() { ".".into() } else { dir }).spawn()
        } else {
            std::process::Command::new("sh").args(["-c", &cmdline]).current_dir(if dir.is_empty() { ".".into() } else { dir }).spawn()
        };
        if let Err(e) = res {
            self.pending = Some(Pending::Message(format!("Failed to run command:\n{e}")));
        }
    }

    /// Per-frame housekeeping: file watching, session snapshots, background results.
    pub fn tick(&mut self, ctx: &egui::Context) {
        self.frame_count += 1;
        self.poll_find_in_files();
        if self.fif_running {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }
        // external modification check
        if self.settings.watch_files && self.pending.is_none() && self.time - self.last_mtime_check > 1.5 {
            self.last_mtime_check = self.time;
            let mut changed: Option<(u64, bool)> = None;
            for d in &mut self.tabs {
                let Some(p) = &d.path else { continue };
                match std::fs::metadata(p) {
                    Ok(m) => {
                        let mt = m.modified().ok();
                        if d.disk_missing {
                            d.disk_missing = false;
                        }
                        if mt.is_some() && d.disk_mtime.is_some() && mt != d.disk_mtime {
                            d.disk_mtime = mt;
                            changed = Some((d.id, false));
                            break;
                        }
                    }
                    Err(_) => {
                        if !d.disk_missing && d.disk_mtime.is_some() {
                            d.disk_missing = true;
                            changed = Some((d.id, true));
                            break;
                        }
                    }
                }
            }
            if let Some((id, deleted)) = changed {
                self.pending = Some(if deleted { Pending::ExternalDeleted(id) } else { Pending::ExternalChange(id) });
            }
        }
        // periodic session snapshot
        if self.settings.restore_session && self.settings.autosave_secs > 0 && self.time - self.last_session_save > self.settings.autosave_secs as f64 {
            self.last_session_save = self.time;
            if self.tabs.iter().any(|d| d.is_modified()) || self.frame_count % 40 == 0 {
                crate::session::save(self, false);
            }
        }
        // status message expiry
        if let Some((_, t)) = &self.status_msg {
            if self.time - t > 6.0 {
                self.status_msg = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
        }
    }
}

pub fn reveal_in_file_manager(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = path.parent().unwrap_or(path);
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

pub fn open_with_default(target: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", target]).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(target).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

impl SavedMacro {
    pub fn new(name: String, steps: Vec<Cmd>) -> Self {
        SavedMacro { name, steps }
    }
}
