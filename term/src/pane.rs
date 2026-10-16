//! One terminal: a shell on a PTY, the emulator state, and the threads that move bytes between them.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, Term};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Processor, Rgb as VteRgb, StdSyncHandler};
use eframe::egui;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

use crate::config::{CursorKind, Settings, Theme};

/// A command saved with a terminal, typed into its shell on demand or when its layout opens.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct SavedCommand {
    pub command: String,
    /// Run automatically when the layout is opened.
    pub auto_run: bool,
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
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
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
                    let started = Instant::now();
                    loop {
                        let msg = match parser.sync_timeout().sync_timeout() {
                            Some(t) => rrx.recv_timeout(t.saturating_duration_since(Instant::now())),
                            None => rrx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                        };
                        match msg {
                            Ok(first) => {
                                let mut t = term.lock();
                                let before = HoldMark::take(&t);
                                parser.advance(&mut *t, &first);
                                // Drain whatever else is queued under the same lock.
                                while let Ok(more) = rrx.try_recv() {
                                    parser.advance(&mut *t, &more);
                                }
                                if shared.pinned.load(Ordering::Relaxed) || shared.locked.load(Ordering::Relaxed) {
                                    before.restore(&mut t, &shared.held_lines);
                                }
                            }
                            Err(RecvTimeoutError::Timeout) => parser.stop_sync(&mut *term.lock()),
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                        shared.activity.store(true, Ordering::Relaxed);
                        shared.got_output.store(true, Ordering::Relaxed);
                        shared.last_output_ms.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
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
            pending: Vec::new(),
            spawned: Instant::now(),
            selecting: false,
            click_count: 0,
            last_click: None,
            mouse_reported_button: None,
            last_mouse_cell: None,
            scroll_accum: 0.0,
            bell_until: None,
        })
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.writer.send(bytes.into());
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
        self.size = size;
        *self.shared.size.lock().unwrap() = window_size(size, cell);
        self.term.lock().resize(size);
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
