//! The document model: text (rope), multiple selections, undo history,
//! bookmarks, folding state and lexer caches. UI-free so it can be tested.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

use ropey::Rope;

use crate::encoding::Enc;
use crate::lang::{self, LangId};
use crate::lexer::{self, LexState, Span};
use crate::util::{self, char_class};

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Eol {
    Lf,
    CrLf,
    Cr,
}

impl Eol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
            Eol::Cr => "\r",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Eol::Lf => "Unix (LF)",
            Eol::CrLf => "Windows (CR LF)",
            Eol::Cr => "Macintosh (CR)",
        }
    }
    pub fn native() -> Eol {
        if cfg!(windows) { Eol::CrLf } else { Eol::Lf }
    }
    /// Dominant line ending in a text, or None if there are no line breaks.
    pub fn detect(text: &str) -> Option<Eol> {
        let b = text.as_bytes();
        let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\n' => lf += 1,
                b'\r' => {
                    if b.get(i + 1) == Some(&b'\n') {
                        crlf += 1;
                        i += 1;
                    } else {
                        cr += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if lf + crlf + cr == 0 {
            None
        } else if crlf >= lf && crlf >= cr {
            Some(Eol::CrLf)
        } else if lf >= cr {
            Some(Eol::Lf)
        } else {
            Some(Eol::Cr)
        }
    }
}

/// A selection: `anchor` is where it started, `head` is the caret.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sel {
    pub anchor: usize,
    pub head: usize,
    /// Remembered display column for vertical movement.
    pub want_col: Option<usize>,
}

impl Sel {
    pub fn caret(p: usize) -> Sel {
        Sel { anchor: p, head: p, want_col: None }
    }
    pub fn new(anchor: usize, head: usize) -> Sel {
        Sel { anchor, head, want_col: None }
    }
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum After {
    End,
    Start,
    /// Select relative offsets (anchor, head) inside the new text.
    Select(usize, usize),
}

#[derive(Clone, Debug)]
pub struct TextEdit {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub after: After,
}

impl TextEdit {
    pub fn replace(start: usize, end: usize, text: impl Into<String>) -> TextEdit {
        TextEdit { start, end, text: text.into(), after: After::End }
    }
    pub fn insert(at: usize, text: impl Into<String>) -> TextEdit {
        TextEdit::replace(at, at, text)
    }
    pub fn delete(start: usize, end: usize) -> TextEdit {
        TextEdit::replace(start, end, "")
    }
    pub fn after(mut self, a: After) -> TextEdit {
        self.after = a;
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKind {
    Typing,
    Deleting,
    Other,
}

#[derive(Clone, Debug)]
struct Rec {
    pos: usize,
    removed: String,
    inserted: String,
}

#[derive(Clone, Debug)]
struct Step {
    recs: Vec<Rec>,
    /// Number of edits (carets) per typed unit, to know when steps are alike.
    width: usize,
    before: Vec<Sel>,
    after: Vec<Sel>,
    kind: EditKind,
}

#[derive(Clone, Copy, Debug)]
pub struct EditOpts {
    pub tab_width: usize,
    pub use_spaces: bool,
    pub auto_indent: bool,
    pub auto_close: bool,
    pub auto_close_tags: bool,
}

impl Default for EditOpts {
    fn default() -> Self {
        EditOpts { tab_width: 4, use_spaces: false, auto_indent: true, auto_close: false, auto_close_tags: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldRange {
    pub start: usize,
    pub end: usize,
}

/// A highlighted range from "Mark" / "Find all" (style 0..5).
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub start: usize,
    pub end: usize,
    pub style: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Metrics {
    /// Wrap width in display columns (0 = no wrapping).
    pub wrap_cols: usize,
    pub page_rows: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    Home,
    End,
    DocStart,
    DocEnd,
    PageUp,
    PageDown,
    LineStart,
    LineEnd,
    ParaUp,
    ParaDown,
}

pub struct Doc {
    pub id: u64,
    pub rope: Rope,
    pub sels: Vec<Sel>,
    pub primary: usize,
    pub path: Option<PathBuf>,
    pub untitled_no: u32,
    pub enc: Enc,
    pub eol: Eol,
    pub lang: LangId,
    pub version: u64,
    pub read_only: bool,
    /// Tab shows the file rendered in an embedded web view instead of the text editor.
    pub html_view: bool,
    /// Hidden-encryption state; see `vault.rs`.
    pub vault: crate::vault::Vault,
    pub overwrite: bool,
    pub opts: EditOpts,
    pub bookmarks: BTreeSet<usize>,
    pub marks: Vec<Mark>,
    pub collapsed: BTreeSet<usize>,
    pub disk_mtime: Option<SystemTime>,
    pub disk_missing: bool,
    /// Scroll offset in visual rows / pixels, kept here so tabs remember their view.
    pub scroll_row: f32,
    pub scroll_x: f32,
    /// Column-selection (rectangular) anchor in (line, display col).
    pub rect_anchor: Option<(usize, usize)>,
    pub rect_head: Option<(usize, usize)>,
    pub rowmap: crate::rowmap::RowMap,
    /// Row-map bookkeeping since its last sync: lines touched, number of edits, and the
    /// (line, removed line breaks, added line breaks) of the last edit that changed the line count.
    pub rm_dirty: Option<(usize, usize)>,
    pub rm_records: u32,
    pub rm_splice: Option<(usize, usize, usize)>,
    pub scroll_request: Option<f32>,
    pub backup_ver: u64,
    /// Longest line seen so far, in chars (drives the horizontal scrollbar).
    pub max_cols: usize,
    pub hidden_gen: u64,
    pub fold_starts: Vec<(usize, usize)>,

    history: Vec<Step>,
    hist_idx: usize,
    saved_idx: Option<usize>,
    force_modified: bool,
    break_coalesce: bool,

    lex_states: Vec<LexState>,
    lex_run: Option<LexRun>,
    pub(crate) folds: Vec<FoldRange>,
    folds_version: u64,
    fold_job: Option<FoldJob>,
    last_edit: std::time::Instant,
    hidden: Vec<(usize, usize)>,
    hidden_dirty: bool,
    text_cache: RefCell<Option<(u64, Rc<String>)>>,
}

/// Approximate lexer states for a window far from the start of a huge document.
struct LexRun {
    version: u64,
    start: usize,
    states: Vec<LexState>,
}

/// A fold computation running on a worker thread over a snapshot of the text.
struct FoldJob {
    version: u64,
    lang: LangId,
    rx: std::sync::mpsc::Receiver<Vec<FoldRange>>,
}

/// Documents up to this size recompute folds synchronously (well under a millisecond or two).
const SYNC_FOLD_CHARS: usize = 40_000;
/// Wait this long after the last edit before starting a background fold pass.
const FOLD_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(100);

/// How many lines may be lexed in one go to reach an exact state.
const EXACT_LEX_BUDGET: usize = 30_000;
/// How far before a far-away target an approximate window starts.
const APPROX_LEX_LOOKBACK: usize = 2_000;

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Doc {
    pub fn new() -> Doc {
        Doc::from_text("", Enc::Utf8, Eol::native())
    }

    pub fn from_text(text: &str, enc: Enc, eol: Eol) -> Doc {
        Doc {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            rope: Rope::from_str(text),
            sels: vec![Sel::caret(0)],
            primary: 0,
            path: None,
            untitled_no: 0,
            enc,
            eol,
            lang: lang::PLAIN,
            version: 1,
            read_only: false,
            html_view: false,
            vault: Default::default(),
            overwrite: false,
            opts: EditOpts::default(),
            bookmarks: BTreeSet::new(),
            marks: Vec::new(),
            collapsed: BTreeSet::new(),
            disk_mtime: None,
            disk_missing: false,
            scroll_row: 0.0,
            scroll_x: 0.0,
            rect_anchor: None,
            rect_head: None,
            rowmap: Default::default(),
            rm_dirty: None,
            rm_records: 0,
            rm_splice: None,
            scroll_request: None,
            backup_ver: 0,
            max_cols: 0,
            hidden_gen: 0,
            fold_starts: Vec::new(),
            history: Vec::new(),
            hist_idx: 0,
            saved_idx: Some(0),
            force_modified: false,
            break_coalesce: false,
            lex_states: vec![LexState::Normal],
            lex_run: None,
            folds: Vec::new(),
            folds_version: 0,
            fold_job: None,
            last_edit: std::time::Instant::now(),
            hidden: Vec::new(),
            hidden_dirty: false,
            text_cache: RefCell::new(None),
        }
    }

    // ------------------------------------------------------------ queries

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }
    /// Forget the cached full-text copy (it may hold text that must not outlive a lock).
    pub fn drop_text_cache(&mut self) {
        *self.text_cache.borrow_mut() = None;
    }

    pub fn is_modified(&self) -> bool {
        self.force_modified || self.saved_idx != Some(self.hist_idx)
    }
    pub fn mark_modified(&mut self) {
        self.force_modified = true;
    }
    pub fn mark_saved(&mut self) {
        self.saved_idx = Some(self.hist_idx);
        self.force_modified = false;
        self.break_coalesce = true;
    }
    pub fn can_undo(&self) -> bool {
        self.hist_idx > 0
    }
    pub fn can_redo(&self) -> bool {
        self.hist_idx < self.history.len()
    }

    pub fn display_name(&self) -> String {
        match &self.path {
            Some(p) => p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            None => format!("new {}", self.untitled_no),
        }
    }

    pub fn char_at(&self, pos: usize) -> Option<char> {
        if pos < self.rope.len_chars() { Some(self.rope.char(pos)) } else { None }
    }

    pub fn slice(&self, a: usize, b: usize) -> String {
        let n = self.len_chars();
        self.rope.slice(a.min(n)..b.min(n)).to_string()
    }

    pub fn text_string(&self) -> Rc<String> {
        let mut c = self.text_cache.borrow_mut();
        if let Some((v, s)) = c.as_ref() {
            if *v == self.version {
                return s.clone();
            }
        }
        let s = Rc::new(self.rope.to_string());
        *c = Some((self.version, s.clone()));
        s
    }

    pub fn line_of(&self, pos: usize) -> usize {
        self.rope.char_to_line(pos.min(self.len_chars()))
    }
    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line.min(self.len_lines()))
    }
    /// Position just before the line terminator.
    pub fn line_end(&self, line: usize) -> usize {
        let line = line.min(self.len_lines() - 1);
        let s = self.rope.line_to_char(line);
        let l = self.rope.line(line);
        let mut n = l.len_chars();
        if n > 0 {
            let last = l.char(n - 1);
            if last == '\n' {
                n -= 1;
                if n > 0 && l.char(n - 1) == '\r' {
                    n -= 1;
                }
            } else if last == '\r' {
                n -= 1;
            }
        }
        s + n
    }
    pub fn line_len(&self, line: usize) -> usize {
        self.line_end(line) - self.line_start(line)
    }
    pub fn line_text(&self, line: usize) -> String {
        if line >= self.len_lines() {
            return String::new();
        }
        self.slice(self.line_start(line), self.line_end(line))
    }
    pub fn linecol(&self, pos: usize) -> (usize, usize) {
        let l = self.line_of(pos);
        (l, pos - self.line_start(l))
    }
    pub fn display_col_of(&self, pos: usize) -> usize {
        let (l, c) = self.linecol(pos);
        util::display_col(&self.line_text(l), c, self.opts.tab_width)
    }

    pub fn primary_sel(&self) -> Sel {
        self.sels[self.primary.min(self.sels.len() - 1)]
    }
    pub fn caret(&self) -> usize {
        self.primary_sel().head
    }

    pub fn selected_text(&self) -> String {
        let mut out = String::new();
        for (i, s) in self.sels.iter().enumerate() {
            if s.is_empty() {
                continue;
            }
            if i > 0 && !out.is_empty() {
                out.push_str(self.eol.as_str());
            }
            out.push_str(&self.slice(s.start(), s.end()));
        }
        out
    }
    pub fn has_selection(&self) -> bool {
        self.sels.iter().any(|s| !s.is_empty())
    }

    pub fn next_pos(&self, p: usize) -> usize {
        let n = self.len_chars();
        if p >= n {
            n
        } else if self.rope.char(p) == '\r' && p + 1 < n && self.rope.char(p + 1) == '\n' {
            p + 2
        } else {
            p + 1
        }
    }
    pub fn prev_pos(&self, p: usize) -> usize {
        if p == 0 {
            0
        } else if p >= 2 && self.rope.char(p - 1) == '\n' && self.rope.char(p - 2) == '\r' {
            p - 2
        } else {
            p - 1
        }
    }

    // ------------------------------------------------------ selections

    pub fn set_caret(&mut self, pos: usize) {
        let p = pos.min(self.len_chars());
        self.sels = vec![Sel::caret(p)];
        self.primary = 0;
        self.break_coalesce = true;
    }
    pub fn set_selection(&mut self, anchor: usize, head: usize) {
        let n = self.len_chars();
        self.sels = vec![Sel::new(anchor.min(n), head.min(n))];
        self.primary = 0;
        self.break_coalesce = true;
    }
    pub fn add_selection(&mut self, s: Sel) {
        self.sels.push(s);
        self.primary = self.sels.len() - 1;
        self.normalize();
    }
    pub fn select_all(&mut self) {
        self.set_selection(0, self.len_chars());
    }

    /// Sort, clamp and merge overlapping selections.
    pub fn normalize(&mut self) {
        let n = self.len_chars();
        let prim = self.sels.get(self.primary).copied();
        for s in &mut self.sels {
            s.anchor = s.anchor.min(n);
            s.head = s.head.min(n);
        }
        if self.sels.is_empty() {
            self.sels.push(Sel::caret(0));
        }
        self.sels.sort_by_key(|s| (s.start(), s.end()));
        let mut out: Vec<Sel> = Vec::with_capacity(self.sels.len());
        for s in self.sels.drain(..) {
            if let Some(last) = out.last_mut() {
                let touch = if last.is_empty() || s.is_empty() { s.start() <= last.end() } else { s.start() < last.end() };
                if touch {
                    // merge, keep direction of the later one
                    let start = last.start();
                    let end = last.end().max(s.end());
                    if s.head >= s.anchor {
                        *last = Sel { anchor: start, head: end, want_col: s.want_col };
                    } else {
                        *last = Sel { anchor: end, head: start, want_col: s.want_col };
                    }
                    continue;
                }
            }
            out.push(s);
        }
        self.sels = out;
        self.primary = match prim {
            Some(p) => self
                .sels
                .iter()
                .position(|s| s.start() <= p.head && p.head <= s.end())
                .unwrap_or(self.sels.len() - 1),
            None => self.sels.len() - 1,
        };
    }

    pub fn collapse_to_primary(&mut self) {
        let p = self.primary_sel();
        self.sels = vec![p];
        self.primary = 0;
    }

    pub fn word_range_at(&self, pos: usize) -> (usize, usize) {
        let n = self.len_chars();
        if n == 0 {
            return (0, 0);
        }
        let pos = pos.min(n);
        let cls_at = |p: usize| self.char_at(p).map(char_class);
        // prefer the char under the caret, else the one before it
        let base = match (cls_at(pos), pos.checked_sub(1).and_then(cls_at)) {
            (Some(c), _) if c != 3 && c != 0 => pos,
            (_, Some(c)) if c != 3 => pos - 1,
            (Some(c), _) if c != 3 => pos,
            _ => return (pos, pos),
        };
        let cls = char_class(self.rope.char(base));
        let mut s = base;
        while s > 0 && char_class(self.rope.char(s - 1)) == cls {
            s -= 1;
        }
        let mut e = base + 1;
        while e < n && char_class(self.rope.char(e)) == cls {
            e += 1;
        }
        (s, e)
    }

    pub fn word_left(&self, mut p: usize) -> usize {
        while p > 0 && char_class(self.rope.char(p - 1)) == 0 {
            p -= 1;
        }
        if p == 0 {
            return 0;
        }
        let cls = char_class(self.rope.char(p - 1));
        if cls == 3 {
            return self.prev_pos(p);
        }
        while p > 0 && char_class(self.rope.char(p - 1)) == cls {
            p -= 1;
        }
        p
    }
    pub fn word_right(&self, mut p: usize) -> usize {
        let n = self.len_chars();
        if p >= n {
            return n;
        }
        let cls = char_class(self.rope.char(p));
        if cls == 3 {
            return self.next_pos(p);
        }
        while p < n && char_class(self.rope.char(p)) == cls {
            p += 1;
        }
        while p < n && char_class(self.rope.char(p)) == 0 {
            p += 1;
        }
        p
    }

    /// Inclusive line ranges touched by the selections (merged). A selection that
    /// ends at column 0 of a line does not include that line.
    pub fn selected_line_blocks(&self) -> Vec<(usize, usize)> {
        let mut blocks: Vec<(usize, usize)> = Vec::new();
        for s in &self.sels {
            let first = self.line_of(s.start());
            let mut last = self.line_of(s.end());
            if !s.is_empty() && last > first && s.end() == self.line_start(last) {
                last -= 1;
            }
            match blocks.last_mut() {
                Some(b) if first <= b.1 => b.1 = b.1.max(last),
                _ => blocks.push((first, last)),
            }
        }
        blocks
    }

    // ------------------------------------------------------- movement

    pub fn move_sels(&mut self, mv: Move, extend: bool, m: Metrics) {
        self.rect_anchor = None;
        let sels = std::mem::take(&mut self.sels);
        let mut out = Vec::with_capacity(sels.len());
        for s in sels {
            out.push(self.moved(s, mv, extend, m));
        }
        self.sels = out;
        self.normalize();
        self.break_coalesce = true;
    }

    fn moved(&mut self, s: Sel, mv: Move, extend: bool, m: Metrics) -> Sel {
        let n = self.len_chars();
        let mut want = None;
        let head = match mv {
            Move::Left => {
                if !extend && !s.is_empty() { s.start() } else { self.prev_pos(s.head) }
            }
            Move::Right => {
                if !extend && !s.is_empty() { s.end() } else { self.next_pos(s.head) }
            }
            Move::WordLeft => self.word_left(s.head),
            Move::WordRight => self.word_right(s.head),
            Move::DocStart => 0,
            Move::DocEnd => n,
            Move::LineStart => self.line_start(self.line_of(s.head)),
            Move::LineEnd => self.line_end(self.line_of(s.head)),
            Move::Home => self.smart_home(s.head, m),
            Move::End => self.row_end(s.head, m),
            Move::Up | Move::Down | Move::PageUp | Move::PageDown => {
                let dir: isize = if matches!(mv, Move::Up | Move::PageUp) { -1 } else { 1 };
                let count = if matches!(mv, Move::PageUp | Move::PageDown) { m.page_rows.max(1) } else { 1 };
                let mut head = s.head;
                let mut wc = s.want_col;
                for _ in 0..count {
                    let (h, w) = self.vertical(head, dir, wc, m);
                    head = h;
                    wc = Some(w);
                }
                want = wc;
                head
            }
            Move::ParaUp | Move::ParaDown => {
                let mut l = self.line_of(s.head);
                let last = self.len_lines() - 1;
                if mv == Move::ParaDown {
                    while l < last && self.line_len(l) == 0 {
                        l += 1;
                    }
                    while l < last && self.line_len(l) != 0 {
                        l += 1;
                    }
                    self.line_start(l)
                } else {
                    while l > 0 && self.line_len(l) == 0 {
                        l -= 1;
                    }
                    while l > 0 && self.line_len(l) != 0 {
                        l -= 1;
                    }
                    self.line_start(l)
                }
            }
        };
        if extend {
            Sel { anchor: s.anchor, head, want_col: want }
        } else {
            Sel { anchor: head, head, want_col: want }
        }
    }

    fn row_bounds(&self, pos: usize, m: Metrics) -> (usize, usize, usize) {
        // (row_start_abs, row_end_abs, line)
        let l = self.line_of(pos);
        let ls = self.line_start(l);
        if m.wrap_cols == 0 {
            return (ls, self.line_end(l), l);
        }
        let text = self.line_text(l);
        let rows = util::wrap_rows(&text, m.wrap_cols, self.opts.tab_width);
        let c = pos - ls;
        let ri = rows.iter().rposition(|&r| r <= c).unwrap_or(0);
        let start = rows[ri];
        let end = rows.get(ri + 1).copied().unwrap_or(text.chars().count());
        (ls + start, ls + end, l)
    }

    fn row_end(&self, pos: usize, m: Metrics) -> usize {
        let (_, e, _) = self.row_bounds(pos, m);
        e
    }

    fn smart_home(&self, pos: usize, m: Metrics) -> usize {
        let (rs, _, l) = self.row_bounds(pos, m);
        let ls = self.line_start(l);
        if rs > ls {
            return rs;
        }
        let text = self.line_text(l);
        let ws = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        let first = ls + ws;
        if pos == first { ls } else { first }
    }

    fn vertical(&mut self, pos: usize, dir: isize, want: Option<usize>, m: Metrics) -> (usize, usize) {
        let tab = self.opts.tab_width;
        let l = self.line_of(pos);
        let ls = self.line_start(l);
        let text = self.line_text(l);
        let rows = if m.wrap_cols == 0 { vec![0] } else { util::wrap_rows(&text, m.wrap_cols, tab) };
        let nchars = text.chars().count();
        let c = pos - ls;
        let ri = rows.iter().rposition(|&r| r <= c).unwrap_or(0);
        let row_text = |text: &str, rows: &[usize], i: usize| -> (usize, usize, String) {
            let s = rows[i];
            let e = rows.get(i + 1).copied().unwrap_or(nchars);
            (s, e, text.chars().skip(s).take(e - s).collect())
        };
        let (rs, _, rtext) = row_text(&text, &rows, ri);
        let cur_col = util::display_col(&rtext, c - rs, tab);
        let wc = want.unwrap_or(cur_col);

        let place = |line: usize, ri: usize, text: &str, rows: &[usize]| -> usize {
            let (s, e, rt) = row_text(text, rows, ri);
            let mut idx = util::index_at_display_col(&rt, wc, tab);
            let max = if ri + 1 < rows.len() { e - s - 1 } else { e - s };
            idx = idx.min(max);
            self.line_start(line) + s + idx
        };

        let target = ri as isize + dir;
        if target >= 0 && (target as usize) < rows.len() {
            return (place(l, target as usize, &text, &rows), wc);
        }
        // cross to another visible line
        let nl = if dir > 0 { self.next_visible_line(l) } else { self.prev_visible_line(l) };
        match nl {
            None => {
                if dir > 0 { (self.len_chars(), wc) } else { (0, wc) }
            }
            Some(nl) => {
                let t2 = self.line_text(nl);
                let r2 = if m.wrap_cols == 0 { vec![0] } else { util::wrap_rows(&t2, m.wrap_cols, tab) };
                let ri2 = if dir > 0 { 0 } else { r2.len() - 1 };
                let (s, e, rt) = {
                    let n2 = t2.chars().count();
                    let s = r2[ri2];
                    let e = r2.get(ri2 + 1).copied().unwrap_or(n2);
                    (s, e, t2.chars().skip(s).take(e - s).collect::<String>())
                };
                let mut idx = util::index_at_display_col(&rt, wc, tab);
                let max = if ri2 + 1 < r2.len() { e - s - 1 } else { e - s };
                idx = idx.min(max);
                (self.line_start(nl) + s + idx, wc)
            }
        }
    }

    // ---------------------------------------------------------- editing

    /// Sort edits, clamp them to the document and drop overlaps.
    pub fn sanitize_edits(&self, mut edits: Vec<TextEdit>) -> Vec<TextEdit> {
        edits.sort_by_key(|e| (e.start, e.end));
        let n = self.len_chars();
        let mut clean: Vec<TextEdit> = Vec::with_capacity(edits.len());
        for mut e in edits {
            e.start = e.start.min(n);
            e.end = e.end.clamp(e.start, n);
            if let Some(last) = clean.last() {
                if e.start < last.end {
                    continue;
                }
            }
            clean.push(e);
        }
        clean
    }

    /// Apply edits (non-overlapping) as one undo step; selections follow `TextEdit::after`.
    pub fn exec(&mut self, edits: Vec<TextEdit>, kind: EditKind) {
        if self.read_only || edits.is_empty() {
            return;
        }
        let clean = self.sanitize_edits(edits);
        // compute resulting selections
        let mut delta: isize = 0;
        let mut new_sels = Vec::with_capacity(clean.len());
        for e in &clean {
            let ns = (e.start as isize + delta) as usize;
            let len = e.text.chars().count();
            let sel = match e.after {
                After::End => Sel::caret(ns + len),
                After::Start => Sel::caret(ns),
                After::Select(a, h) => Sel::new(ns + a, ns + h),
            };
            new_sels.push(sel);
            delta += len as isize - (e.end - e.start) as isize;
        }
        self.commit(clean, new_sels, kind);
    }

    /// Map a position through a sorted list of edits.
    pub fn map_pos(edits: &[TextEdit], pos: usize) -> usize {
        let mut delta: isize = 0;
        for e in edits {
            let new_len = e.text.chars().count();
            if pos >= e.end {
                delta += new_len as isize - (e.end - e.start) as isize;
            } else if pos > e.start {
                return (e.start as isize + delta) as usize + new_len;
            } else {
                break;
            }
        }
        (pos as isize + delta) as usize
    }

    /// Apply edits while keeping the current selections mapped through them.
    pub fn exec_keep(&mut self, edits: Vec<TextEdit>, kind: EditKind) {
        if self.read_only || edits.is_empty() {
            return;
        }
        let clean = self.sanitize_edits(edits);
        let same_len = clean.iter().all(|e| e.text.chars().count() == e.end - e.start);
        let sels = if same_len {
            self.sels.clone()
        } else {
            self.sels
                .iter()
                .map(|s| Sel::new(Doc::map_pos(&clean, s.anchor), Doc::map_pos(&clean, s.head)))
                .collect()
        };
        self.commit(clean, sels, kind);
    }

    /// Apply edits and set explicit selections (in post-edit coordinates).
    pub fn commit(&mut self, edits: Vec<TextEdit>, new_sels: Vec<Sel>, kind: EditKind) {
        if self.read_only {
            return;
        }
        let before = self.sels.clone();
        let mut recs = Vec::with_capacity(edits.len());
        for e in edits.iter().rev() {
            let removed = self.slice(e.start, e.end);
            if e.end > e.start || !e.text.is_empty() {
                self.apply_raw(e.start, e.end, &e.text);
                recs.push(Rec { pos: e.start, removed, inserted: e.text.clone() });
            }
        }
        if recs.is_empty() {
            return;
        }
        self.sels = if new_sels.is_empty() { before.clone() } else { new_sels };
        self.primary = self.sels.len() - 1;
        self.normalize();
        let width = recs.len();
        self.push_step(Step { recs, width, before, after: self.sels.clone(), kind });
        self.rect_anchor = None;
    }

    fn push_step(&mut self, step: Step) {
        // drop redo tail
        if self.hist_idx < self.history.len() {
            self.history.truncate(self.hist_idx);
            if self.saved_idx.is_some_and(|s| s > self.hist_idx) {
                self.saved_idx = None;
            }
        }
        let coalesce = !self.break_coalesce
            && step.kind != EditKind::Other
            && self.hist_idx > 0
            && self.history.last().is_some_and(|last| {
                last.kind == step.kind
                    && last.after == step.before
                    && last.width == step.width
                    && self.saved_idx != Some(self.hist_idx)
                    && step.recs.iter().all(|r| !r.inserted.contains(['\n', '\r']))
                    && last.recs.iter().all(|r| !r.inserted.contains(['\n', '\r']))
                    && step.recs.iter().all(|r| r.inserted.chars().count() <= 1 && r.removed.chars().count() <= 1)
            });
        self.break_coalesce = false;
        if coalesce {
            let last = self.history.last_mut().unwrap();
            last.recs.extend(step.recs);
            last.after = step.after;
        } else {
            self.history.push(step);
            self.hist_idx = self.history.len();
        }
        // keep the idx consistent after coalescing
        self.hist_idx = self.history.len();
    }

    /// Apply a replacement to the rope and update derived caches.
    fn apply_raw(&mut self, start: usize, end: usize, text: &str) {
        let line = self.rope.char_to_line(start);
        let last_line = if end > start { self.rope.char_to_line(end) } else { line };
        let lines_before = self.rope.len_lines();
        if end > start {
            self.rope.remove(start..end);
        }
        let lines_mid = self.rope.len_lines();
        if !text.is_empty() {
            self.rope.insert(start, text);
        }
        let lines_after = self.rope.len_lines();
        // derived from the rope so that edits splitting / joining a CR LF pair stay consistent
        self.rm_dirty = Some(match self.rm_dirty {
            Some((a, b)) => (a.min(line), b.max(last_line)),
            None => (line, last_line),
        });
        let removed_breaks = lines_before.saturating_sub(lines_mid);
        let added_breaks = lines_after.saturating_sub(lines_mid);
        self.rm_records += 1;
        // Edits that split or join a CR LF pair move line boundaries unexpectedly; the row map
        // cannot patch those incrementally, so force it to rebuild.
        let text_breaks = if text.is_empty() { 0 } else { util::split_lines(text).len() - 1 };
        if (last_line - line) != removed_breaks || text_breaks != added_breaks {
            self.rm_records += 2;
        }
        if lines_before != lines_after {
            self.rm_splice = Some((line, removed_breaks, added_breaks));
        }
        self.version += 1;
        self.last_edit = std::time::Instant::now();
        self.marks.clear();
        self.lex_states.truncate(line + 1);
        if removed_breaks != added_breaks {
            self.shift_lines(line, removed_breaks, added_breaks);
        }
        let n = self.rope.len_lines();
        let _ = self.bookmarks.split_off(&n);
        let _ = self.collapsed.split_off(&n);
        self.hidden_dirty = true;
    }

    fn shift_lines(&mut self, at: usize, removed: usize, added: usize) {
        let fix = |set: &mut BTreeSet<usize>| {
            let old = std::mem::take(set);
            for l in old {
                if l <= at {
                    set.insert(l);
                } else if l <= at + removed {
                    // line merged away
                    if removed > added {
                        continue;
                    }
                    set.insert(l + added - removed);
                } else {
                    set.insert(l + added - removed);
                }
            }
        };
        fix(&mut self.bookmarks);
        fix(&mut self.collapsed);
    }

    pub fn undo(&mut self) -> bool {
        if self.read_only || self.hist_idx == 0 {
            return false;
        }
        self.hist_idx -= 1;
        let step = self.history[self.hist_idx].clone();
        for r in step.recs.iter().rev() {
            let ins_len = r.inserted.chars().count();
            self.apply_raw(r.pos, r.pos + ins_len, &r.removed);
        }
        self.sels = step.before;
        self.primary = self.sels.len() - 1;
        self.normalize();
        self.break_coalesce = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.read_only || self.hist_idx >= self.history.len() {
            return false;
        }
        let step = self.history[self.hist_idx].clone();
        self.hist_idx += 1;
        for r in &step.recs {
            let rem_len = r.removed.chars().count();
            self.apply_raw(r.pos, r.pos + rem_len, &r.inserted);
        }
        self.sels = step.after;
        self.primary = self.sels.len() - 1;
        self.normalize();
        self.break_coalesce = true;
        true
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.hist_idx = 0;
        self.saved_idx = Some(0);
    }

    /// Load fresh text, resetting history (open / reload).
    pub fn reset_text(&mut self, text: &str) {
        self.rope = Rope::from_str(text);
        self.version += 1;
        self.sels = vec![Sel::caret(0)];
        self.primary = 0;
        self.clear_history();
        self.lex_states.truncate(1);
        self.marks.clear();
        self.bookmarks.clear();
        self.collapsed.clear();
        self.hidden_dirty = true;
        self.force_modified = false;
    }

    // ------------------------------------------------- typing commands

    pub fn type_text(&mut self, s: &str) {
        if s.is_empty() || self.read_only {
            return;
        }
        let single = s.chars().count() == 1;
        if single {
            let c = s.chars().next().unwrap();
            if c == '\n' || c == '\r' {
                return self.newline();
            }
            if c == '>' && self.opts.auto_close_tags && self.try_close_tag() {
                return;
            }
            if self.opts.auto_close && self.try_auto_close(c) {
                return;
            }
        }
        let kind = if single { EditKind::Typing } else { EditKind::Other };
        let overwrite = self.overwrite && single;
        let edits = self
            .sels
            .iter()
            .map(|sel| {
                let mut end = sel.end();
                if overwrite && sel.is_empty() {
                    if let Some(ch) = self.char_at(end) {
                        if ch != '\n' && ch != '\r' {
                            end += 1;
                        }
                    }
                }
                TextEdit::replace(sel.start(), end, s)
            })
            .collect();
        self.exec(edits, kind);
    }

    /// Typing `>` after `<name attr...` inserts the matching `</name>` (markup languages).
    fn try_close_tag(&mut self) -> bool {
        if lang::compiled(self.lang).def.kind != lang::Kind::Markup || self.sels.iter().any(|s| !s.is_empty()) {
            return false;
        }
        const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
        let mut edits = Vec::new();
        for s in &self.sels {
            let (l, c) = self.linecol(s.head);
            let before: String = self.line_text(l).chars().take(c).collect();
            let Some(lt) = before.rfind('<') else { return false };
            let inner = &before[lt + 1..];
            if inner.contains('>') || inner.starts_with('/') || inner.starts_with('!') || inner.starts_with('?') || inner.ends_with('/') {
                return false;
            }
            let name: String = inner.chars().take_while(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.')).collect();
            if name.is_empty() || VOID.contains(&name.to_lowercase().as_str()) {
                return false;
            }
            edits.push(TextEdit::insert(s.head, format!("></{name}>")).after(After::Select(1, 1)));
        }
        self.exec(edits, EditKind::Other);
        true
    }

    fn try_auto_close(&mut self, c: char) -> bool {
        let pairs = [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];
        if self.sels.iter().any(|s| !s.is_empty()) {
            return false;
        }
        // type-over a closing char
        if pairs.iter().any(|&(_, close)| close == c) && self.sels.iter().all(|s| self.char_at(s.head) == Some(c)) {
            let is_opener = pairs.iter().any(|&(o, _)| o == c);
            let prev_word = self.sels.iter().all(|s| s.head > 0 && util::is_word_char(self.rope.char(s.head - 1)));
            if !is_opener || !prev_word {
                for s in &mut self.sels {
                    *s = Sel::caret(s.head + 1);
                }
                self.break_coalesce = true;
                return true;
            }
        }
        if let Some(&(_, close)) = pairs.iter().find(|&&(o, _)| o == c) {
            let quote = c == '"' || c == '\'';
            let ok = self.sels.iter().all(|s| {
                let next_ok = self.char_at(s.head).is_none_or(|n| n.is_whitespace() || ")]};,".contains(n));
                let prev_ok = !quote || s.head == 0 || !util::is_word_char(self.rope.char(s.head - 1));
                next_ok && prev_ok
            });
            if ok {
                let text = format!("{c}{close}");
                let edits = self
                    .sels
                    .iter()
                    .map(|s| TextEdit::insert(s.head, text.clone()).after(After::Select(1, 1)))
                    .collect();
                self.exec(edits, EditKind::Other);
                return true;
            }
        }
        false
    }

    pub fn newline(&mut self) {
        if self.read_only {
            return;
        }
        let eol = self.eol.as_str();
        let unit = self.indent_unit();
        let edits = self
            .sels
            .iter()
            .map(|s| {
                let l = self.line_of(s.start());
                let upto: String = self.slice(self.line_start(l), s.start());
                let mut text = String::from(eol);
                if self.opts.auto_indent {
                    let ws = util::leading_ws(&upto).to_string();
                    text.push_str(&ws);
                    let trimmed = upto.trim_end();
                    let opens = trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') || (trimmed.ends_with(':') && self.lang_is_indent());
                    let next = self.char_at(s.end());
                    if opens {
                        text.push_str(&unit);
                        let closes = matches!((trimmed.chars().last(), next), (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']')));
                        if closes {
                            let caret = text.chars().count();
                            text.push_str(eol);
                            text.push_str(&ws);
                            return TextEdit::replace(s.start(), s.end(), text).after(After::Select(caret, caret));
                        }
                    }
                }
                TextEdit::replace(s.start(), s.end(), text)
            })
            .collect();
        self.exec(edits, EditKind::Other);
    }

    fn lang_is_indent(&self) -> bool {
        lang::compiled(self.lang).def.fold == lang::FoldMode::Indent
    }

    pub fn indent_unit(&self) -> String {
        if self.opts.use_spaces { " ".repeat(self.opts.tab_width.max(1)) } else { "\t".to_string() }
    }

    pub fn backspace(&mut self) {
        let tab = self.opts.tab_width.max(1);
        let edits = self
            .sels
            .iter()
            .filter_map(|s| {
                if !s.is_empty() {
                    return Some(TextEdit::delete(s.start(), s.end()));
                }
                if s.head == 0 {
                    return None;
                }
                // smart un-indent within leading spaces
                if self.opts.use_spaces {
                    let (l, c) = self.linecol(s.head);
                    let text = self.line_text(l);
                    let before: String = text.chars().take(c).collect();
                    if c > 0 && before.chars().all(|ch| ch == ' ') {
                        let back = if c % tab == 0 { tab } else { c % tab };
                        return Some(TextEdit::delete(s.head - back, s.head));
                    }
                }
                Some(TextEdit::delete(self.prev_pos(s.head), s.head))
            })
            .collect();
        self.exec(edits, EditKind::Deleting);
    }

    pub fn delete_forward(&mut self) {
        let edits = self
            .sels
            .iter()
            .filter_map(|s| {
                if !s.is_empty() {
                    Some(TextEdit::delete(s.start(), s.end()))
                } else if s.head < self.len_chars() {
                    Some(TextEdit::delete(s.head, self.next_pos(s.head)))
                } else {
                    None
                }
            })
            .collect();
        self.exec(edits, EditKind::Deleting);
    }

    pub fn delete_word_left(&mut self) {
        let edits = self
            .sels
            .iter()
            .filter_map(|s| {
                if !s.is_empty() {
                    Some(TextEdit::delete(s.start(), s.end()))
                } else {
                    let w = self.word_left(s.head);
                    (w < s.head).then(|| TextEdit::delete(w, s.head))
                }
            })
            .collect();
        self.exec(edits, EditKind::Other);
    }

    pub fn delete_word_right(&mut self) {
        let edits = self
            .sels
            .iter()
            .filter_map(|s| {
                if !s.is_empty() {
                    Some(TextEdit::delete(s.start(), s.end()))
                } else {
                    let w = self.word_right(s.head);
                    (w > s.head).then(|| TextEdit::delete(s.head, w))
                }
            })
            .collect();
        self.exec(edits, EditKind::Other);
    }

    pub fn delete_selection(&mut self) {
        let edits = self.sels.iter().filter(|s| !s.is_empty()).map(|s| TextEdit::delete(s.start(), s.end())).collect();
        self.exec(edits, EditKind::Other);
    }

    /// Paste text; if there are as many pasted lines as carets, distribute them.
    pub fn paste(&mut self, text: &str) {
        let lines = util::split_lines(text);
        let eol = self.eol.as_str();
        if self.sels.len() > 1 && lines.len() == self.sels.len() {
            let edits = self.sels.iter().zip(lines).map(|(s, l)| TextEdit::replace(s.start(), s.end(), l)).collect();
            self.exec(edits, EditKind::Other);
        } else {
            let norm = lines.join(eol);
            let edits = self.sels.iter().map(|s| TextEdit::replace(s.start(), s.end(), norm.clone())).collect();
            self.exec(edits, EditKind::Other);
        }
    }

    /// Tab key: insert indentation or indent selected lines.
    pub fn tab(&mut self) {
        let multiline = self.sels.iter().any(|s| !s.is_empty() && self.line_of(s.start()) != self.line_of(s.end()));
        if multiline {
            return self.indent_lines(true);
        }
        let tab = self.opts.tab_width.max(1);
        let use_spaces = self.opts.use_spaces;
        let edits = self
            .sels
            .iter()
            .map(|s| {
                let text = if use_spaces {
                    let col = self.display_col_of(s.start());
                    " ".repeat(tab - col % tab)
                } else {
                    "\t".to_string()
                };
                TextEdit::replace(s.start(), s.end(), text)
            })
            .collect();
        self.exec(edits, EditKind::Other);
    }

    // ------------------------------------------------- lexing / folding

    pub fn lex_state_at(&mut self, line: usize) -> LexState {
        let compiled = lang::compiled(self.lang);
        if compiled.def.kind == lang::Kind::Plain {
            return LexState::Normal;
        }
        let line = line.min(self.len_lines() - 1);
        let mut spans = Vec::new();
        if line < self.lex_states.len() || line - self.lex_states.len() <= EXACT_LEX_BUDGET {
            while self.lex_states.len() <= line {
                let i = self.lex_states.len() - 1;
                let st = self.lex_states[i];
                let text = self.line_text(i);
                let next = lexer::lex_line(compiled, &text, st, &mut spans);
                self.lex_states.push(next);
            }
            return self.lex_states[line];
        }
        // Far away in a huge file: lex a short window from a clean state instead of
        // everything before it (colors may be off if the window starts inside a comment).
        let reusable = matches!(&self.lex_run, Some(r) if r.version == self.version
            && line >= r.start && line - r.start <= r.states.len() + EXACT_LEX_BUDGET / 4);
        if !reusable {
            self.lex_run = Some(LexRun { version: self.version, start: line.saturating_sub(APPROX_LEX_LOOKBACK), states: vec![LexState::Normal] });
        }
        let mut run = self.lex_run.take().unwrap();
        while run.states.len() <= line - run.start {
            let i = run.start + run.states.len() - 1;
            let st = run.states[run.states.len() - 1];
            let text = self.line_text(i);
            let next = lexer::lex_line(compiled, &text, st, &mut spans);
            run.states.push(next);
        }
        let st = run.states[line - run.start];
        self.lex_run = Some(run);
        st
    }

    /// Lex a single line, returning its text; spans are written to `out`.
    pub fn line_spans(&mut self, line: usize, out: &mut Vec<Span>) -> String {
        let st = self.lex_state_at(line);
        let text = self.line_text(line);
        lexer::lex_line(lang::compiled(self.lang), &text, st, out);
        text
    }

    pub fn set_lang(&mut self, id: LangId) {
        self.lang = id;
        self.lex_states.truncate(1);
        self.lex_run = None;
        self.fold_job = None;
        self.folds_version = 0;
        self.collapsed.clear();
        self.hidden_dirty = true;
    }

    pub fn folds(&mut self) -> &[FoldRange] {
        self.refresh_folds();
        &self.folds
    }

    /// True while fold ranges lag behind the text (a background pass is pending).
    pub fn folds_pending(&self) -> bool {
        self.folds_version != self.version && self.len_chars() > SYNC_FOLD_CHARS && self.len_chars() <= 8_000_000
    }

    fn refresh_folds(&mut self) {
        if self.folds_version == self.version {
            return;
        }
        let n = self.len_chars();
        if n > 8_000_000 {
            self.apply_folds(Vec::new(), self.version);
            return;
        }
        if n <= SYNC_FOLD_CHARS {
            self.fold_job = None;
            let folds = crate::fold::compute(self);
            self.apply_folds(folds, self.version);
            return;
        }
        // large document: collect a finished background result, then (re)start one when idle
        if let Some(job) = &self.fold_job {
            match job.rx.try_recv() {
                Ok(folds) => {
                    let (v, l) = (job.version, job.lang);
                    self.fold_job = None;
                    if l == self.lang {
                        self.apply_folds(folds, v);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.fold_job = None,
            }
            if self.folds_version == self.version {
                return;
            }
        }
        if self.fold_job.is_none() && self.last_edit.elapsed() >= FOLD_DEBOUNCE {
            let (tx, rx) = std::sync::mpsc::channel();
            let (rope, lang, tab) = (self.rope.clone(), self.lang, self.opts.tab_width);
            self.fold_job = Some(FoldJob { version: self.version, lang, rx });
            std::thread::spawn(move || {
                let _ = tx.send(crate::fold::compute_from(&rope, lang, tab));
            });
        }
    }

    fn apply_folds(&mut self, folds: Vec<FoldRange>, version: u64) {
        self.folds = folds;
        self.folds_version = version;
        if version == self.version {
            let starts: BTreeSet<usize> = self.folds.iter().map(|f| f.start).collect();
            self.collapsed.retain(|l| starts.contains(l));
        }
        let mut fs: Vec<(usize, usize)> = self.folds.iter().map(|f| (f.start, f.end)).collect();
        fs.sort();
        fs.dedup_by(|b, a| {
            if a.0 == b.0 {
                a.1 = a.1.max(b.1);
                true
            } else {
                false
            }
        });
        self.fold_starts = fs;
        self.hidden_dirty = true;
    }

    fn refresh_hidden(&mut self) {
        if self.collapsed.is_empty() {
            if !self.hidden.is_empty() {
                self.hidden_gen += 1;
            }
            self.hidden.clear();
            self.hidden_dirty = false;
            return;
        }
        self.refresh_folds();
        if !self.hidden_dirty {
            return;
        }
        self.hidden_dirty = false;
        let mut v: Vec<(usize, usize)> = self
            .folds
            .iter()
            .filter(|f| self.collapsed.contains(&f.start))
            .map(|f| (f.start + 1, f.end))
            .collect();
        v.sort();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (a, b) in v {
            match merged.last_mut() {
                Some(l) if a <= l.1 + 1 => l.1 = l.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        self.hidden = merged;
        self.hidden_gen += 1;
    }

    pub fn is_hidden(&mut self, line: usize) -> bool {
        self.refresh_hidden();
        match self.hidden.binary_search_by(|&(a, b)| {
            if line < a {
                std::cmp::Ordering::Greater
            } else if line > b {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        }) {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    pub fn hidden_ranges(&mut self) -> &[(usize, usize)] {
        self.refresh_hidden();
        &self.hidden
    }

    pub fn fold_at(&mut self, line: usize) -> Option<FoldRange> {
        self.refresh_folds();
        // the largest range starting at this line
        self.folds.iter().filter(|f| f.start == line).max_by_key(|f| f.end).copied()
    }

    pub fn toggle_fold(&mut self, line: usize) {
        if self.fold_at(line).is_none() {
            return;
        }
        if !self.collapsed.remove(&line) {
            self.collapsed.insert(line);
        }
        self.hidden_dirty = true;
    }

    pub fn fold_all(&mut self) {
        self.refresh_folds();
        self.collapsed = self.folds.iter().map(|f| f.start).collect();
        self.hidden_dirty = true;
    }
    pub fn unfold_all(&mut self) {
        self.collapsed.clear();
        self.hidden_dirty = true;
    }
    /// Fold ranges nested exactly `depth` levels deep (1 = top level).
    pub fn fold_level(&mut self, depth: usize, fold: bool) {
        self.refresh_folds();
        let folds = self.folds.clone();
        for f in &folds {
            let d = 1 + folds.iter().filter(|o| o.start < f.start && o.end >= f.end).count();
            if d == depth {
                if fold {
                    self.collapsed.insert(f.start);
                } else {
                    self.collapsed.remove(&f.start);
                }
            }
        }
        self.hidden_dirty = true;
    }

    /// Make a line visible by unfolding whatever hides it.
    pub fn reveal_line(&mut self, line: usize) {
        self.refresh_hidden();
        if !self.is_hidden(line) {
            return;
        }
        let starts: Vec<usize> = self.folds.iter().filter(|f| f.start < line && line <= f.end).map(|f| f.start).collect();
        for s in starts {
            self.collapsed.remove(&s);
        }
        self.hidden_dirty = true;
    }

    pub fn next_visible_line(&mut self, line: usize) -> Option<usize> {
        let mut l = line + 1;
        if l >= self.len_lines() {
            return None;
        }
        self.refresh_hidden();
        if let Some(&(_, b)) = self.hidden.iter().find(|&&(a, b)| a <= l && l <= b) {
            l = b + 1;
        }
        (l < self.len_lines()).then_some(l)
    }

    pub fn prev_visible_line(&mut self, line: usize) -> Option<usize> {
        if line == 0 {
            return None;
        }
        let mut l = line - 1;
        self.refresh_hidden();
        if let Some(&(a, _)) = self.hidden.iter().find(|&&(a, b)| a <= l && l <= b) {
            l = a - 1;
        }
        Some(l)
    }

    // -------------------------------------------------------- bookmarks

    pub fn toggle_bookmark(&mut self, line: usize) {
        if !self.bookmarks.remove(&line) {
            self.bookmarks.insert(line);
        }
    }
    pub fn next_bookmark(&self, from_line: usize) -> Option<usize> {
        self.bookmarks
            .range(from_line + 1..)
            .next()
            .or_else(|| self.bookmarks.iter().next())
            .copied()
    }
    pub fn prev_bookmark(&self, from_line: usize) -> Option<usize> {
        self.bookmarks
            .range(..from_line)
            .next_back()
            .or_else(|| self.bookmarks.iter().next_back())
            .copied()
    }
}

impl Default for Doc {
    fn default() -> Self {
        Doc::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> Doc {
        Doc::from_text(s, Enc::Utf8, Eol::Lf)
    }

    #[test]
    fn typing_and_undo_coalesce() {
        let mut d = doc("");
        for c in "hello".chars() {
            d.type_text(&c.to_string());
        }
        assert_eq!(d.rope.to_string(), "hello");
        assert!(d.is_modified());
        d.undo();
        assert_eq!(d.rope.to_string(), "");
        assert!(!d.is_modified());
        d.redo();
        assert_eq!(d.rope.to_string(), "hello");
    }

    #[test]
    fn save_point_tracks_modified() {
        let mut d = doc("abc");
        d.set_caret(3);
        d.type_text("d");
        d.mark_saved();
        assert!(!d.is_modified());
        d.type_text("e");
        assert!(d.is_modified());
        d.undo();
        assert!(!d.is_modified());
    }

    #[test]
    fn multi_caret_typing() {
        let mut d = doc("a\nb\nc");
        d.sels = vec![Sel::caret(0), Sel::caret(2), Sel::caret(4)];
        d.type_text("x");
        assert_eq!(d.rope.to_string(), "xa\nxb\nxc");
        assert_eq!(d.sels.iter().map(|s| s.head).collect::<Vec<_>>(), vec![1, 4, 7]);
        d.undo();
        assert_eq!(d.rope.to_string(), "a\nb\nc");
    }

    #[test]
    fn crlf_is_one_unit() {
        let mut d = Doc::from_text("a\r\nb", Enc::Utf8, Eol::CrLf);
        assert_eq!(d.len_lines(), 2);
        assert_eq!(d.line_text(0), "a");
        d.set_caret(3);
        d.backspace();
        assert_eq!(d.rope.to_string(), "ab");
    }

    #[test]
    fn newline_autoindent() {
        let mut d = doc("    foo {");
        d.opts.use_spaces = true;
        d.set_caret(9);
        d.newline();
        assert_eq!(d.rope.to_string(), "    foo {\n        ");
        let mut d = doc("fn x() {}");
        d.opts.use_spaces = true;
        d.set_caret(8);
        d.newline();
        assert_eq!(d.rope.to_string(), "fn x() {\n    \n}");
        assert_eq!(d.caret(), 13);
    }

    #[test]
    fn vertical_movement_keeps_column() {
        let mut d = doc("abcdef\nab\nabcdef");
        let m = Metrics::default();
        d.set_caret(5);
        d.move_sels(Move::Down, false, m);
        assert_eq!(d.caret(), 9);
        d.move_sels(Move::Down, false, m);
        assert_eq!(d.caret(), 15);
    }

    #[test]
    fn word_motion() {
        let d = doc("foo bar  baz");
        assert_eq!(d.word_right(0), 4);
        assert_eq!(d.word_right(4), 9);
        assert_eq!(d.word_left(12), 9);
        assert_eq!(d.word_left(9), 4);
        assert_eq!(d.word_range_at(5), (4, 7));
    }

    #[test]
    fn bookmarks_shift_with_edits() {
        let mut d = doc("a\nb\nc\nd");
        d.toggle_bookmark(2);
        d.set_caret(0);
        d.newline();
        assert!(d.bookmarks.contains(&3));
        d.undo();
        assert!(d.bookmarks.contains(&2));
    }

    #[test]
    fn selection_merge() {
        let mut d = doc("hello world");
        d.sels = vec![Sel::new(0, 5), Sel::new(3, 8)];
        d.normalize();
        assert_eq!(d.sels.len(), 1);
        assert_eq!((d.sels[0].start(), d.sels[0].end()), (0, 8));
    }

    #[test]
    fn auto_close_and_typeover() {
        let mut d = doc("");
        d.opts.auto_close = true;
        d.type_text("(");
        assert_eq!(d.rope.to_string(), "()");
        d.type_text(")");
        assert_eq!(d.rope.to_string(), "()");
        assert_eq!(d.caret(), 2);
    }
}
