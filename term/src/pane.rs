//! One terminal: a shell on a PTY, the emulator state, and the threads that move bytes between them.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::search::Match;
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Processor, Rgb as VteRgb, StdSyncHandler};
use eframe::egui;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

use crate::config::{CursorKind, Settings, Theme};
use crate::marks::MarkLog;
use crate::record::Recorder;
use crate::stream::{Item, Mark, Scanner};

/// A command saved with a terminal, typed into its shell on demand or when its layout opens.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct SavedCommand {
    pub command: String,
    /// Run automatically when the layout is opened.
    pub auto_run: bool,
    /// Keep it running: when it ends with an error (an ssh connection that dropped, say) run it again.
    pub reconnect: bool,
}

/// Text saved with a terminal to find again later, and optionally to be told about when it appears.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct SavedSearch {
    pub pattern: String,
    /// A regular expression; otherwise plain text, ignoring case.
    pub regex: bool,
    /// Raise an alert when this appears in new output.
    pub alert: bool,
}

impl SavedSearch {
    /// The pattern as a regular expression (plain text is escaped and made case-insensitive).
    pub fn regex_source(&self) -> String {
        if self.regex { self.pattern.clone() } else { format!("(?i){}", regex::escape(&self.pattern)) }
    }
}

/// Something that happened in a terminal's output, for the window to react to.
#[derive(Clone, Debug)]
pub enum PaneEvent {
    CommandDone { command: String, exit: Option<i32>, took: Duration },
    Watch { pattern: String, line: String },
    Notify { title: String, body: String },
}

struct Watch {
    pattern: String,
    re: regex::Regex,
    last: Option<Instant>,
}

/// State shared between the UI and a terminal's background threads.
pub struct Shared {
    pub title: Mutex<Option<String>>,
    pub exited: AtomicBool,
    /// Keep the visible text still while output arrives (selection in progress, or scroll lock).
    pub pinned: AtomicBool,
    /// The user turned scroll lock on explicitly; survives typing.
    pub locked: AtomicBool,
    pub bell: AtomicBool,
    pub activity: AtomicBool,
    pub got_output: AtomicBool,
    /// When output last arrived, in milliseconds since the terminal started.
    pub last_output_ms: AtomicU64,
    pub clipboard_store: Mutex<Option<String>>,
    pub size: Mutex<WindowSize>,
    pub theme: Mutex<Theme>,
    /// Lines that scrolled into history while the view was held still.
    pub held_lines: AtomicU64,
    /// Prompt and output positions from shell integration.
    /// Lock order: the terminal first, then this.
    pub marks: Mutex<MarkLog>,
    pub events: Mutex<Vec<PaneEvent>>,
    watches: Mutex<Vec<Watch>>,
    recorder: Mutex<Option<Recorder>>,
    /// Whether programs may read the clipboard (OSC 52).
    pub osc52_read: AtomicBool,
    /// The most lines the scrollback holds (see `track_scroll`).
    history_cap: AtomicUsize,
}

impl Shared {
    fn push_event(&self, e: PaneEvent) {
        let mut v = self.events.lock().unwrap();
        if v.len() < 200 {
            v.push(e);
        }
    }

    /// Text lines that arrived: record them and look for watched patterns.
    fn lines_arrived(&self, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        if let Some(r) = self.recorder.lock().unwrap().as_mut() {
            r.write_lines(lines);
        }
        let mut watches = self.watches.lock().unwrap();
        if watches.is_empty() {
            return;
        }
        let now = Instant::now();
        for line in lines {
            for w in watches.iter_mut() {
                // One alert per pattern every few seconds, however fast the log scrolls.
                if w.last.is_some_and(|t| now.duration_since(t) < Duration::from_secs(3)) || !w.re.is_match(line) {
                    continue;
                }
                w.last = Some(now);
                self.push_event(PaneEvent::Watch { pattern: w.pattern.clone(), line: line.chars().take(300).collect() });
            }
        }
    }
}

#[derive(Clone)]
pub struct Listener {
    shared: Arc<Shared>,
    writer: Sender<Vec<u8>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(s) => {
                let _ = self.writer.send(s.into_bytes());
            }
            Event::Title(t) => *self.shared.title.lock().unwrap() = Some(t),
            Event::ResetTitle => *self.shared.title.lock().unwrap() = None,
            Event::Bell => self.shared.bell.store(true, Ordering::Relaxed),
            Event::ClipboardStore(_, text) => *self.shared.clipboard_store.lock().unwrap() = Some(text),
            Event::ClipboardLoad(_, format) => {
                // A program (possibly on a remote host) asking for the clipboard: only when allowed.
                let text = if self.shared.osc52_read.load(Ordering::Relaxed) {
                    arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default()
                } else {
                    String::new()
                };
                let _ = self.writer.send(format(&text).into_bytes());
            }
            Event::ColorRequest(index, format) => {
                let theme = self.shared.theme.lock().unwrap();
                let c = match index {
                    256 => theme.foreground,
                    257 => theme.background,
                    258 => theme.cursor,
                    i => theme.indexed(i),
                };
                let _ = self.writer.send(format(VteRgb { r: c.0, g: c.1, b: c.2 }).into_bytes());
            }
            Event::TextAreaSizeRequest(format) => {
                let size = *self.shared.size.lock().unwrap();
                let _ = self.writer.send(format(size).into_bytes());
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
pub struct TermSize {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// The link under the pointer while the link key is held: the cells to underline and where it leads.
#[derive(Clone, Debug)]
pub struct LinkHover {
    /// (line, first column, one past the last column)
    pub segments: Vec<(i32, usize, usize)>,
    pub target: crate::links::Target,
}

pub struct Pane {
    pub id: u64,
    pub term: Arc<FairMutex<Term<Listener>>>,
    pub shared: Arc<Shared>,
    writer: Sender<Vec<u8>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    pub size: TermSize,
    /// Last known working directory of the shell.
    pub cwd: PathBuf,
    pub cwd_checked: Instant,
    /// User-chosen name shown in the title bar.
    pub name: Option<String>,
    pub commands: Vec<SavedCommand>,
    pub searches: Vec<SavedSearch>,
    /// Identifies this terminal's saved scrollback between runs.
    pub uid: String,
    /// The searches the background watches were last built from.
    watch_sig: Vec<SavedSearch>,
    /// The saved search last selected with a click, and the match it found.
    pub search_hit: Option<(usize, Match)>,
    /// Something happened here that the user has not looked at yet.
    pub alert: bool,
    /// When the foreground job started, for finish alerts on shells without integration.
    pub job_since: Option<Instant>,
    /// The command to run again when it fails (see `SavedCommand::reconnect`) and the state of retrying.
    pub keep_alive: Option<String>,
    pub reconnect_at: Option<Instant>,
    pub reconnect_tries: u32,
    /// The keep-alive command ended; offer to run it again.
    pub reconnect_offer: bool,
    pub recording: Option<std::path::PathBuf>,
    /// Output time (see `Shared::last_output_ms`) when the scrollback was last written to disk.
    saved_ms: Option<u64>,
    /// A full-screen program ran during the current job, so its end is not worth an alert.
    pub job_fullscreen: bool,
    /// Saved commands waiting for the shell's first prompt.
    pub pending: Vec<String>,
    pub spawned: Instant,
    // Mouse state.
    pub selecting: bool,
    pub click_count: u32,
    pub last_click: Option<(Instant, egui::Pos2)>,
    pub mouse_reported_button: Option<u8>,
    pub last_mouse_cell: Option<(usize, usize)>,
    pub scroll_accum: f32,
    pub bell_until: Option<Instant>,
    pub link_hover: Option<LinkHover>,
    /// Where file names seen in the output lead: (key, when looked up, the file).
    pub link_cache: std::collections::HashMap<String, (Instant, Option<PathBuf>)>,
}

pub fn term_config(s: &Settings) -> term::Config {
    let shape = match s.cursor_shape {
        CursorKind::Block => CursorShape::Block,
        CursorKind::Beam => CursorShape::Beam,
        CursorKind::Underline => CursorShape::Underline,
    };
    term::Config {
        scrolling_history: s.scrollback_lines,
        default_cursor_style: CursorStyle { shape, blinking: s.cursor_blink },
        semantic_escape_chars: s.word_separators.clone(),
        ..Default::default()
    }
}

fn default_shell() -> CommandBuilder {
    #[cfg(windows)]
    {
        let ps = ["pwsh.exe", "powershell.exe"]
            .into_iter()
            .find(|exe| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(exe).exists())));
        CommandBuilder::new(ps.unwrap_or("cmd.exe"))
    }
    #[cfg(not(windows))]
    {
        // Runs $SHELL (or the passwd shell) as a login shell, like a native terminal.
        CommandBuilder::new_default_prog()
    }
}

impl Pane {
    pub fn spawn(
        id: u64,
        settings: &Settings,
        dir: Option<PathBuf>,
        size: TermSize,
        cell: (f32, f32),
        ctx: &egui::Context,
        restore: Option<&str>,
    ) -> Result<Pane, String> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows: size.rows as u16, cols: size.cols as u16, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("could not open a pty: {e}"))?;

        let mut cmd = if settings.shell.trim().is_empty() {
            default_shell()
        } else {
            let mut c = CommandBuilder::new(settings.shell.trim());
            c.args(&settings.shell_args);
            c
        };
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let dir = dir.filter(|d| d.is_dir()).unwrap_or(home);
        cmd.cwd(&dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "term");
        cmd.env("TERM_INTEGRATION_DIR", crate::integration::dir());
        // A login shell started from Finder has no locale; Java apps (e.g. SAP Commerce) then mangle UTF-8.
        if std::env::var_os("LANG").is_none() {
            cmd.env("LANG", "en_US.UTF-8");
        }
        let child = pair.slave.spawn_command(cmd).map_err(|e| format!("could not start the shell: {e}"))?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;

        let shared = Arc::new(Shared {
            title: Mutex::new(None),
            exited: AtomicBool::new(false),
            pinned: AtomicBool::new(false),
            locked: AtomicBool::new(false),
            bell: AtomicBool::new(false),
            activity: AtomicBool::new(false),
            got_output: AtomicBool::new(false),
            last_output_ms: AtomicU64::new(0),
            clipboard_store: Mutex::new(None),
            size: Mutex::new(window_size(size, cell)),
            theme: Mutex::new(settings.theme.clone()),
            held_lines: AtomicU64::new(0),
            marks: Mutex::new(MarkLog::default()),
            events: Mutex::new(Vec::new()),
            watches: Mutex::new(Vec::new()),
            recorder: Mutex::new(None),
            osc52_read: AtomicBool::new(settings.osc52_read),
            history_cap: AtomicUsize::new(settings.scrollback_lines),
        });

        // Writer thread: the UI never blocks on a full PTY buffer.
        let (wtx, wrx) = mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new()
            .name(format!("pty-writer-{id}"))
            .spawn(move || {
                while let Ok(bytes) = wrx.recv() {
                    if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        let listener = Listener { shared: shared.clone(), writer: wtx.clone() };
        let term = Arc::new(FairMutex::new(Term::new(term_config(settings), &size, listener)));
        if let Some(text) = restore.filter(|t| !t.is_empty()) {
            restore_scrollback(&mut term.lock(), text);
        }

        // Reader thread: raw bytes from the shell.
        let (rtx, rrx) = mpsc::sync_channel::<Vec<u8>>(64);
        std::thread::Builder::new()
            .name(format!("pty-reader-{id}"))
            .spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if rtx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        // Parser thread: feeds the emulator and keeps a held view still.
        {
            let term = term.clone();
            let shared = shared.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("pty-parser-{id}"))
                .spawn(move || {
                    let mut parser: Processor<StdSyncHandler> = Processor::new();
                    let mut scanner = Scanner::default();
                    let started = Instant::now();
                    loop {
                        let msg = match parser.sync_timeout().sync_timeout() {
                            Some(t) => rrx.recv_timeout(t.saturating_duration_since(Instant::now())),
                            None => rrx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                        };
                        match msg {
                            Ok(mut bytes) => {
                                // Drain whatever else is queued so it is handled under the same lock.
                                while let Ok(more) = rrx.try_recv() {
                                    bytes.extend_from_slice(&more);
                                }
                                let lines = {
                                    let mut t = term.lock();
                                    let before = HoldMark::take(&t);
                                    let lines = ingest(&mut t, &mut parser, &shared, &mut scanner, &bytes);
                                    if shared.pinned.load(Ordering::Relaxed) || shared.locked.load(Ordering::Relaxed) {
                                        before.restore(&mut t, &shared.held_lines);
                                    }
                                    lines
                                };
                                shared.lines_arrived(&lines);
                            }
                            Err(RecvTimeoutError::Timeout) => parser.stop_sync(&mut *term.lock()),
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                        shared.activity.store(true, Ordering::Relaxed);
                        shared.got_output.store(true, Ordering::Relaxed);
                        shared.last_output_ms.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
                        crate::diag::repaint_requested();
                        ctx.request_repaint();
                    }
                    shared.exited.store(true, Ordering::Relaxed);
                    ctx.request_repaint();
                })
                .map_err(|e| e.to_string())?;
        }

        Ok(Pane {
            id,
            term,
            shared,
            writer: wtx,
            master: pair.master,
            child,
            size,
            cwd: dir,
            cwd_checked: Instant::now(),
            name: None,
            commands: Vec::new(),
            searches: Vec::new(),
            uid: new_uid(id),
            watch_sig: Vec::new(),
            search_hit: None,
            alert: false,
            job_since: None,
            keep_alive: None,
            reconnect_at: None,
            reconnect_tries: 0,
            reconnect_offer: false,
            recording: None,
            saved_ms: None,
            job_fullscreen: false,
            pending: Vec::new(),
            spawned: Instant::now(),
            selecting: false,
            click_count: 0,
            last_click: None,
            mouse_reported_button: None,
            last_mouse_cell: None,
            scroll_accum: 0.0,
            bell_until: None,
            link_hover: None,
            link_cache: std::collections::HashMap::new(),
        })
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.writer.send(bytes.into());
    }

    /// Remember that a keep-alive saved command is being run here.
    pub fn note_run(&mut self, cmd: &str) {
        if self.commands.iter().any(|c| c.reconnect && c.command.trim() == cmd.trim()) {
            self.keep_alive = Some(cmd.to_string());
            self.reconnect_at = None;
            self.reconnect_offer = false;
        }
    }

    /// Type a command into the shell and press Enter.
    pub fn run_command(&self, cmd: &str) {
        let mut text = cmd.replace("\r\n", "\r").replace('\n', "\r");
        text.push('\r');
        self.write(text);
    }

    pub fn resize(&mut self, size: TermSize, cell: (f32, f32)) {
        if size.cols == self.size.cols && size.rows == self.size.rows {
            return;
        }
        let reflow = size.cols != self.size.cols;
        self.size = size;
        *self.shared.size.lock().unwrap() = window_size(size, cell);
        {
            let mut t = self.term.lock();
            let mut marks = self.shared.marks.lock().unwrap();
            if reflow {
                // Wrapping lines differently moves text in ways that cannot be followed.
                t.resize(size);
                marks.invalidate();
            } else {
                let cap = self.shared.history_cap.load(Ordering::Relaxed);
                track_scroll(&mut t, &mut marks, cap, true, |t| t.resize(size));
            }
        }
        let _ = self.master.resize(PtySize { rows: size.rows as u16, cols: size.cols as u16, pixel_width: 0, pixel_height: 0 });
    }

    pub fn exited(&self) -> bool {
        self.shared.exited.load(Ordering::Relaxed)
    }

    pub fn title(&self) -> String {
        if let Some(n) = &self.name {
            return n.clone();
        }
        if let Some(t) = self.shared.title.lock().unwrap().clone().filter(|t| !t.trim().is_empty()) {
            return t;
        }
        let home = dirs::home_dir();
        if home.as_deref() == Some(self.cwd.as_path()) {
            return "~".into();
        }
        self.cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.cwd.display().to_string())
    }

    /// Refresh `cwd` from the shell process, at most once a second.
    pub fn poll_cwd(&mut self) {
        if self.cwd_checked.elapsed().as_millis() < 1000 {
            return;
        }
        self.cwd_checked = Instant::now();
        if let Some(pid) = self.child.process_id()
            && let Some(d) = crate::cwd::of_process(pid)
        {
            self.cwd = d;
        }
    }

    /// Current directory, read now rather than from the cache (used when saving layouts).
    pub fn current_dir(&mut self) -> PathBuf {
        self.cwd_checked = Instant::now() - std::time::Duration::from_secs(5);
        self.poll_cwd();
        self.cwd.clone()
    }

    /// Whether something other than the shell itself is running in the foreground.
    pub fn has_running_job(&self) -> bool {
        #[cfg(unix)]
        {
            let leader = self.master.process_group_leader();
            let shell = self.child.process_id().map(|p| p as i32);
            matches!((leader, shell), (Some(l), Some(s)) if l != s)
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    pub fn set_hold(&self, on: bool) {
        self.shared.pinned.store(on, Ordering::Relaxed);
        if !on && !self.shared.locked.load(Ordering::Relaxed) {
            self.shared.held_lines.store(0, Ordering::Relaxed);
        }
    }

    pub fn is_held(&self) -> bool {
        self.shared.pinned.load(Ordering::Relaxed) || self.shared.locked.load(Ordering::Relaxed)
    }

    pub fn set_lock(&self, on: bool) {
        self.shared.locked.store(on, Ordering::Relaxed);
        if !on {
            self.shared.pinned.store(false, Ordering::Relaxed);
            self.shared.held_lines.store(0, Ordering::Relaxed);
        }
    }

    pub fn scroll_to_bottom(&self) {
        self.set_lock(false);
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    pub fn apply_settings(&self, s: &Settings) {
        self.term.lock().set_options(term_config(s));
        *self.shared.theme.lock().unwrap() = s.theme.clone();
        self.shared.osc52_read.store(s.osc52_read, Ordering::Relaxed);
        self.shared.history_cap.store(s.scrollback_lines, Ordering::Relaxed);
    }

    /// Rebuild the background watches when the saved searches changed.
    pub fn sync_watches(&mut self) {
        if self.watch_sig == self.searches {
            return;
        }
        self.watch_sig = self.searches.clone();
        let list = self
            .searches
            .iter()
            .filter(|s| s.alert && !s.pattern.is_empty())
            .filter_map(|s| {
                let re = regex::Regex::new(&s.regex_source()).ok()?;
                Some(Watch { pattern: s.pattern.clone(), re, last: None })
            })
            .collect();
        *self.shared.watches.lock().unwrap() = list;
    }

    /// Start or stop writing this terminal's output to a file in `dir`.
    pub fn set_recording(&mut self, on: bool, dir: &std::path::Path) -> Result<(), String> {
        if !on {
            *self.shared.recorder.lock().unwrap() = None;
            self.recording = None;
            return Ok(());
        }
        if self.recording.is_some() {
            return Ok(());
        }
        let r = Recorder::create(dir, &self.title()).map_err(|e| format!("could not start recording in {}: {e}", dir.display()))?;
        self.recording = Some(r.path.clone());
        *self.shared.recorder.lock().unwrap() = Some(r);
        Ok(())
    }

    /// Write the recent scrollback to disk unless nothing new has arrived since the last time.
    pub fn save_scrollback(&mut self, max_lines: usize) {
        let ms = self.shared.last_output_ms.load(Ordering::Relaxed);
        if self.saved_ms == Some(ms) {
            return;
        }
        self.saved_ms = Some(ms);
        let text = self.scrollback_text(max_lines);
        crate::session::save_scrollback(&self.uid, &text);
    }

    /// The pane's recent scrollback as plain text, newest `max_lines` lines.
    pub fn scrollback_text(&self, max_lines: usize) -> String {
        let t = self.term.lock();
        let top = (t.bottommost_line().0 - max_lines as i32 + 1).max(t.topmost_line().0);
        let from = Point::new(Line(top), Column(0));
        let to = Point::new(t.bottommost_line(), t.last_column());
        let mut s = t.bounds_to_string(from, to);
        while s.ends_with('\n') || s.ends_with(' ') {
            s.pop();
        }
        s
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn window_size(size: TermSize, cell: (f32, f32)) -> WindowSize {
    WindowSize { num_lines: size.rows as u16, num_cols: size.cols as u16, cell_width: cell.0 as u16, cell_height: cell.1 as u16 }
}

/// Where the view was before a batch of output, so it can be put back.
///
/// The emulator already keeps the view still once it is scrolled up by at least one line, but at
/// the very bottom new lines push everything up. When a hold is active and the view was at the
/// bottom, we measure how far the content moved and scroll the view back by the same amount;
/// after that the emulator keeps it in place by itself.
struct HoldMark {
    offset: usize,
    history: usize,
    selection_start: Option<Line>,
}

impl HoldMark {
    fn take<T>(t: &Term<T>) -> HoldMark {
        HoldMark {
            offset: t.grid().display_offset(),
            history: t.grid().history_size(),
            selection_start: t.selection.as_ref().and_then(|s| s.to_range(t)).map(|r| r.start.line),
        }
    }

    fn restore<T: EventListener>(self, t: &mut Term<T>, held: &AtomicU64) {
        if t.mode().contains(term::TermMode::ALT_SCREEN) {
            return;
        }
        let offset = t.grid().display_offset();
        if self.offset > 0 {
            if offset > self.offset {
                held.fetch_add((offset - self.offset) as u64, Ordering::Relaxed);
            }
            return;
        }
        if offset != 0 {
            return;
        }
        // Lines added to history; when history is full it stops growing, so fall back to how
        // far the selection (which the emulator rotates with the text) moved.
        let mut moved = t.grid().history_size().saturating_sub(self.history);
        if moved == 0
            && let (Some(before), Some(after)) =
                (self.selection_start, t.selection.as_ref().and_then(|s| s.to_range(t)).map(|r| r.start.line))
        {
            moved = (before.0 - after.0).max(0) as usize;
        }
        if moved > 0 {
            t.scroll_display(Scroll::Delta(moved as i32));
            held.fetch_add(moved as u64, Ordering::Relaxed);
        }
    }
}

fn new_uid(id: u64) -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{ms:x}-{id:x}-{:x}", std::process::id())
}

/// Print the previous run's output (dimmed) at the top of a fresh terminal.
fn restore_scrollback(t: &mut Term<Listener>, text: &str) {
    let mut body = String::from("\x1b[2m");
    for line in text.lines() {
        body.push_str(&line.replace(|c: char| c.is_control(), " "));
        body.push_str("\r\n");
    }
    body.push_str("── restored from the previous session ──\x1b[0m\r\n");
    let mut parser: Processor<StdSyncHandler> = Processor::new();
    parser.advance(t, body.as_bytes());
}

/// Run `op` on the terminal and add to the marks' scroll counter by however far the text moved.
///
/// The movement is measured with a selection anchored at the top of the screen, because the
/// emulator rotates a selection together with the text (the user's own selection is set aside
/// meanwhile and put back rotated the same way). That stays exact only while the text moves less
/// than the scrollback holds, so `op` must not scroll more than `cap` lines. When the probe is
/// lost (the screen was cleared) the growth of the history gives the answer, as long as the
/// history is not full. If neither works every mark is dropped rather than left wrong.
pub fn track_scroll<T: EventListener>(t: &mut Term<T>, marks: &mut MarkLog, cap: usize, allow_negative: bool, op: impl FnOnce(&mut Term<T>)) {
    if marks.blocks.is_empty() || t.mode().contains(TermMode::ALT_SCREEN) {
        op(t);
        return;
    }
    let history_before = t.grid().history_size();
    let saved = t.selection.take();
    let anchor = Point::new(Line(0), Column(0));
    let mut probe = Selection::new(SelectionType::Lines, anchor, Side::Left);
    probe.update(anchor, Side::Left);
    t.selection = Some(probe);
    op(t);
    let probe_line = t.selection.take().and_then(|s| s.to_range(&*t)).map(|r| r.start.line.0 as i64);
    // Gone to the other screen: this one did not move.
    if t.mode().contains(TermMode::ALT_SCREEN) {
        return;
    }
    let history_after = t.grid().history_size();
    let moved = match probe_line {
        Some(l) => Some(-l),
        None if history_after < cap => Some(history_after as i64 - history_before as i64),
        None => None,
    };
    match moved {
        Some(d) if d >= 0 || allow_negative => {
            marks.scrolled += d;
            // The emulator drops the user's selection whenever it loses the probe (clearing the screen, say).
            t.selection = saved.filter(|_| probe_line.is_some()).and_then(|s| s.rotate(&*t, &(Line(0)..Line(t.screen_lines() as i32)), d as i32));
        }
        _ => marks.invalidate(),
    }
}

/// Advance the emulator through `bytes`, in steps small enough for `track_scroll` to follow.
fn advance_tracked(t: &mut Term<Listener>, parser: &mut Processor<StdSyncHandler>, marks: &mut MarkLog, cap: usize, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    if marks.blocks.is_empty() {
        parser.advance(t, bytes);
        return;
    }
    // Every line that scrolls needs at least a byte of output, so a piece of this size moves the text less than the history holds.
    for piece in bytes.chunks((cap / 2).max(50)) {
        track_scroll(t, marks, cap, false, |t| parser.advance(t, piece));
    }
}

/// Feed output to the emulator, acting on shell-integration marks exactly where they occur.
/// Returns the plain text lines that arrived (none for full-screen programs).
fn ingest(
    t: &mut Term<Listener>,
    parser: &mut Processor<StdSyncHandler>,
    shared: &Shared,
    scanner: &mut Scanner,
    bytes: &[u8],
) -> Vec<String> {
    let mut items: Vec<(usize, Item)> = Vec::new();
    scanner.feed(bytes, |end, item| items.push((end, item)));
    let alt_start = t.mode().contains(TermMode::ALT_SCREEN);
    let cap = shared.history_cap.load(Ordering::Relaxed);
    let mut marks = shared.marks.lock().unwrap();
    let mut lines = Vec::new();
    let mut at = 0;
    for (end, item) in items {
        match item {
            Item::Line(l) => lines.push(l),
            Item::Notify(title, body) => shared.push_event(PaneEvent::Notify { title, body }),
            Item::Mark(mark) => {
                advance_tracked(t, parser, &mut marks, cap, &bytes[at..end]);
                at = end;
                if !t.mode().contains(TermMode::ALT_SCREEN) {
                    handle_mark(t, &mut marks, shared, mark);
                }
            }
        }
    }
    advance_tracked(t, parser, &mut marks, cap, &bytes[at..]);
    marks.prune(t.topmost_line().0);
    if alt_start || t.mode().contains(TermMode::ALT_SCREEN) {
        lines.clear();
    }
    lines
}

fn handle_mark(t: &Term<Listener>, marks: &mut MarkLog, shared: &Shared, mark: Mark) {
    let cursor = t.grid().cursor.point;
    let key = marks.key(cursor.line.0);
    let col = cursor.column.0;
    match mark {
        Mark::PromptStart => marks.prompt_start(key),
        Mark::PromptEnd => marks.prompt_end(key, col),
        Mark::OutputStart => {
            let command = marks
                .blocks
                .back()
                .and_then(|b| b.input)
                .map(|(k, c)| {
                    let start = Point::new(Line(marks.line(k)), Column(c));
                    let end = if col == 0 {
                        Point::new(Line(cursor.line.0 - 1), t.last_column())
                    } else {
                        Point::new(cursor.line, Column(col - 1))
                    };
                    if start.line >= t.topmost_line() && start <= end {
                        t.bounds_to_string(start, end).split_whitespace().collect::<Vec<_>>().join(" ")
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();
            marks.output_start(key, col, command);
        }
        Mark::OutputEnd(exit) => {
            if let Some(f) = marks.output_end(key, col, exit) {
                shared.push_event(PaneEvent::CommandDone { command: f.command, exit: f.exit, took: f.took });
            }
        }
    }
}

/// The text of the whole buffer, history included.
pub fn all_text(t: &Term<Listener>) -> String {
    let top = Point::new(t.grid().topmost_line(), Column(0));
    let bottom = Point::new(t.grid().bottommost_line(), t.grid().last_column());
    let mut s = t.bounds_to_string(top, bottom);
    while s.ends_with('\n') {
        s.pop();
    }
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::Side;
    use alacritty_terminal::selection::{Selection, SelectionType};

    use super::*;

    fn term(rows: usize, history: usize) -> Term<VoidListener> {
        let config = term::Config { scrolling_history: history, ..Default::default() };
        Term::new(config, &TermSize { cols: 40, rows }, VoidListener)
    }

    fn feed(t: &mut Term<VoidListener>, p: &mut Processor<StdSyncHandler>, text: &str) {
        p.advance(t, text.replace('\n', "\r\n").as_bytes());
    }

    /// Feed output the way the parser thread does, keeping the view still when `hold` is set.
    fn feed_held(t: &mut Term<VoidListener>, p: &mut Processor<StdSyncHandler>, text: &str, hold: bool, held: &AtomicU64) {
        let mark = HoldMark::take(t);
        feed(t, p, text);
        if hold {
            mark.restore(t, held);
        }
    }

    /// The text currently shown on screen row `row`.
    fn visible_row(t: &Term<VoidListener>, row: usize) -> String {
        let line = Line(row as i32 - t.grid().display_offset() as i32);
        t.bounds_to_string(Point::new(line, Column(0)), Point::new(line, t.last_column())).trim_end().to_string()
    }

    fn log_lines(from: usize, to: usize) -> String {
        (from..to).map(|i| format!("INFO server log line {i}\n")).collect()
    }

    #[test]
    fn view_follows_output_when_not_held() {
        let (mut t, mut p, held) = (term(10, 1000), Processor::new(), AtomicU64::new(0));
        feed(&mut t, &mut p, &log_lines(0, 30));
        feed_held(&mut t, &mut p, &log_lines(30, 35), false, &held);
        assert_eq!(t.grid().display_offset(), 0);
    }

    #[test]
    fn held_view_and_selection_stay_put_while_log_streams() {
        let (mut t, mut p, held) = (term(10, 1000), Processor::new(), AtomicU64::new(0));
        feed(&mut t, &mut p, &log_lines(0, 30));
        let row3 = visible_row(&t, 3);

        // The user selects screen row 3, at the bottom of the scrollback.
        let line = Line(3);
        let mut sel = Selection::new(SelectionType::Simple, Point::new(line, Column(0)), Side::Left);
        sel.update(Point::new(line, Column(39)), Side::Right);
        t.selection = Some(sel);
        let copied = t.selection_to_string();

        // The server keeps logging in several bursts.
        for burst in 0..5 {
            feed_held(&mut t, &mut p, &log_lines(30 + burst * 7, 37 + burst * 7), true, &held);
            assert_eq!(visible_row(&t, 3), row3, "text under the selection moved after burst {burst}");
            assert_eq!(t.selection_to_string(), copied, "selected text changed after burst {burst}");
        }
        assert_eq!(t.grid().display_offset(), 35);
        assert_eq!(held.load(Ordering::Relaxed), 35);
    }

    #[test]
    fn held_view_stays_put_when_history_is_full() {
        // Small scrollback, already full: history size stops growing, so the hold
        // falls back to following the selection.
        let (mut t, mut p, held) = (term(10, 50), Processor::new(), AtomicU64::new(0));
        feed(&mut t, &mut p, &log_lines(0, 200));
        assert_eq!(t.grid().history_size(), 50);
        let row5 = visible_row(&t, 5);
        let mut sel = Selection::new(SelectionType::Lines, Point::new(Line(5), Column(0)), Side::Left);
        sel.update(Point::new(Line(5), Column(0)), Side::Left);
        t.selection = Some(sel);

        feed_held(&mut t, &mut p, &log_lines(200, 204), true, &held);
        assert_eq!(visible_row(&t, 5), row5);
        feed_held(&mut t, &mut p, &log_lines(204, 210), true, &held);
        assert_eq!(visible_row(&t, 5), row5);
    }

    #[test]
    fn scrolled_back_view_is_not_dragged_by_output() {
        let (mut t, mut p, held) = (term(10, 1000), Processor::new(), AtomicU64::new(0));
        feed(&mut t, &mut p, &log_lines(0, 100));
        t.scroll_display(Scroll::Delta(20));
        let row0 = visible_row(&t, 0);
        feed_held(&mut t, &mut p, &log_lines(100, 140), false, &held);
        assert_eq!(visible_row(&t, 0), row0);
        assert_eq!(t.grid().display_offset(), 60);
    }

    #[test]
    fn selection_survives_line_clears_elsewhere() {
        // Many loggers end each line with "erase to end of line"; that must not drop a
        // selection on an older line.
        let (mut t, mut p, held) = (term(10, 1000), Processor::new(), AtomicU64::new(0));
        feed(&mut t, &mut p, &log_lines(0, 30));
        let mut sel = Selection::new(SelectionType::Simple, Point::new(Line(2), Column(0)), Side::Left);
        sel.update(Point::new(Line(2), Column(10)), Side::Right);
        t.selection = Some(sel);
        let copied = t.selection_to_string();
        let colourful: String = (0..10).map(|i| format!("\x1b[32mINFO\x1b[0m line {i}\x1b[K\n")).collect();
        feed_held(&mut t, &mut p, &colourful, true, &held);
        assert_eq!(t.selection_to_string(), copied);
    }
}
