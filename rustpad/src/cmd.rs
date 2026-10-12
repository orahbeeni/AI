//! Every user-visible action is a `Cmd`. Menus, shortcuts, the editor and macros
//! all funnel through `App::exec(Cmd)`.

use std::sync::{OnceLock, RwLock};

use eframe::egui::{Key, Modifiers};
use serde::{Deserialize, Serialize};

use crate::doc::{Eol, Move};
use crate::hash::HashKind;
use crate::ops::{CaseMode, SortMode, TrimMode};
use crate::search::Query;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum SearchOp {
    FindNext,
    FindPrev,
    Replace,
    ReplaceAll,
    Count,
    MarkAll,
    FindAll,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Cmd {
    // ---- file
    New,
    Open,
    OpenRecent(usize),
    ClearRecent,
    Save,
    SaveAs,
    SaveAll,
    Close,
    CloseAll,
    CloseOthers,
    CloseRight,
    CloseLeft,
    ReopenClosed,
    Reload,
    Rename,
    MoveToTrash,
    OpenContainingFolder,
    CopyPath,
    CopyFilename,
    CopyDirectory,
    Exit,
    NextTab,
    PrevTab,
    MoveTabLeft,
    MoveTabRight,
    OpenFolderAsWorkspace,
    // ---- edit
    Type(String),
    Newline,
    Backspace,
    DeleteFwd,
    DeleteWordLeft,
    DeleteWordRight,
    Tab,
    Unindent,
    Move(Move, bool),
    SelectAll,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    PasteText(String),
    DeleteSel,
    ToggleOverwrite,
    DuplicateLine,
    DeleteLine,
    CutLine,
    CopyLine,
    MoveLineUp,
    MoveLineDown,
    JoinLines,
    SplitLines,
    TransposeLine,
    BlankAbove,
    BlankBelow,
    Sort(SortMode, bool),
    DedupLines(bool),
    RemoveEmpty(bool),
    Trim(TrimMode),
    TabsToSpaces,
    SpacesToTabs(bool),
    Case(CaseMode),
    ToggleComment,
    BlockComment,
    ConvertEol(Eol),
    GotoBrace,
    SelectBrace,
    SelectLines,
    InsertDateTime(bool),
    ColumnEditor,
    RectMove(Move),
    Hash(HashKind),
    HashFiles(HashKind),
    AutoComplete,
    MultiSelectNext,
    MultiSelectAll,
    CopyHtmlLikeText,
    // ---- search
    Find,
    Replace,
    FindInFiles,
    Mark,
    FindNext,
    FindPrev,
    FindSelNext,
    FindSelPrev,
    IncrementalSearch,
    GotoLine,
    Search(SearchOp, Query),
    ToggleBookmark,
    NextBookmark,
    PrevBookmark,
    ClearBookmarks,
    CutBookmarked,
    CopyBookmarked,
    RemoveBookmarked,
    InverseBookmarks,
    ClearMarks,
    // ---- view
    ToggleWrap,
    ToggleWhitespace,
    ToggleEol,
    ToggleAllChars,
    ToggleIndentGuides,
    ToggleLineNumbers,
    ToggleBookmarkMargin,
    ToggleFoldMargin,
    ToggleStatusBar,
    ToggleToolBar,
    ToggleTabBar,
    ToggleFullscreen,
    TogglePostIt,
    ToggleAlwaysOnTop,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    FoldAll,
    UnfoldAll,
    FoldLevel(usize),
    UnfoldLevel(usize),
    ToggleFold,
    ToggleDocMap,
    ToggleFunctionList,
    ToggleWorkspace,
    ToggleResults,
    ToggleSplit,
    ToggleSplitOrientation,
    // ---- language / encoding
    SetLang(usize),
    SetEncoding(String),
    ReloadAsEncoding(String),
    ToggleReadOnly,
    SetTheme(String),
    // ---- macros
    MacroRecord,
    MacroStop,
    MacroPlay,
    MacroRunMultiple,
    MacroSave,
    MacroPlayNamed(usize),
    // ---- misc
    Preferences,
    About,
    Run,
    RunNamed(usize),
    LaunchInBrowser,
    Shortcuts,
    Nop,
}

impl Cmd {
    /// Commands worth replaying in a macro.
    pub fn recordable(&self) -> bool {
        use Cmd::*;
        matches!(
            self,
            Type(_)
                | Newline
                | Backspace
                | DeleteFwd
                | DeleteWordLeft
                | DeleteWordRight
                | Tab
                | Unindent
                | Move(..)
                | RectMove(_)
                | SelectAll
                | Undo
                | Redo
                | Cut
                | Copy
                | Paste
                | PasteText(_)
                | DeleteSel
                | DuplicateLine
                | DeleteLine
                | CutLine
                | CopyLine
                | MoveLineUp
                | MoveLineDown
                | JoinLines
                | SplitLines
                | TransposeLine
                | BlankAbove
                | BlankBelow
                | Sort(..)
                | DedupLines(_)
                | RemoveEmpty(_)
                | Trim(_)
                | TabsToSpaces
                | SpacesToTabs(_)
                | Case(_)
                | ToggleComment
                | BlockComment
                | ConvertEol(_)
                | GotoBrace
                | SelectBrace
                | SelectLines
                | InsertDateTime(_)
                | FindNext
                | FindPrev
                | FindSelNext
                | FindSelPrev
                | Search(..)
                | ToggleBookmark
                | NextBookmark
                | PrevBookmark
                | ClearBookmarks
                | InverseBookmarks
                | RemoveBookmarked
        )
    }
}

impl Cmd {
    /// Friendly name used by the shortcut mapper.
    pub fn label(&self) -> String {
        use Cmd::*;
        let s: &str = match self {
            New => "File: New",
            Open => "File: Open",
            Save => "File: Save",
            SaveAs => "File: Save As",
            SaveAll => "File: Save All",
            Close => "File: Close",
            CloseAll => "File: Close All",
            CloseOthers => "File: Close All BUT Active",
            CloseRight => "File: Close All to the Right",
            CloseLeft => "File: Close All to the Left",
            ReopenClosed => "File: Restore Recent Closed File",
            Reload => "File: Reload from Disk",
            Rename => "File: Rename",
            Exit => "File: Exit",
            NextTab => "Tab: Next",
            PrevTab => "Tab: Previous",
            MoveTabLeft => "Tab: Move Left",
            MoveTabRight => "Tab: Move Right",
            Undo => "Edit: Undo",
            Redo => "Edit: Redo",
            Cut => "Edit: Cut",
            Copy => "Edit: Copy",
            Paste => "Edit: Paste",
            SelectAll => "Edit: Select All",
            DuplicateLine => "Line: Duplicate",
            DeleteLine => "Line: Delete",
            CutLine => "Line: Cut",
            CopyLine => "Line: Copy",
            MoveLineUp => "Line: Move Up",
            MoveLineDown => "Line: Move Down",
            JoinLines => "Line: Join",
            SplitLines => "Line: Split",
            TransposeLine => "Line: Transpose",
            BlankAbove => "Line: Insert Blank Above",
            BlankBelow => "Line: Insert Blank Below",
            Tab => "Edit: Indent",
            Unindent => "Edit: Unindent",
            ToggleComment => "Edit: Toggle Single Line Comment",
            BlockComment => "Edit: Toggle Block Comment",
            GotoBrace => "Search: Go to Matching Brace",
            SelectBrace => "Search: Select to Matching Brace",
            AutoComplete => "Edit: Auto-Completion",
            MultiSelectNext => "Edit: Select Next Occurrence",
            MultiSelectAll => "Edit: Select All Occurrences",
            ColumnEditor => "Edit: Column Editor",
            Find => "Search: Find",
            Replace => "Search: Replace",
            FindInFiles => "Search: Find in Files",
            Mark => "Search: Mark",
            FindNext => "Search: Find Next",
            FindPrev => "Search: Find Previous",
            FindSelNext => "Search: Select and Find Next",
            FindSelPrev => "Search: Select and Find Previous",
            IncrementalSearch => "Search: Incremental Search",
            GotoLine => "Search: Go to Line",
            ToggleBookmark => "Bookmark: Toggle",
            NextBookmark => "Bookmark: Next",
            PrevBookmark => "Bookmark: Previous",
            ClearBookmarks => "Bookmark: Clear All",
            ToggleWrap => "View: Word Wrap",
            ToggleWhitespace => "View: Show Whitespace",
            ToggleEol => "View: Show End of Line",
            ToggleAllChars => "View: Show All Characters",
            ToggleIndentGuides => "View: Indent Guides",
            ToggleFullscreen => "View: Full Screen",
            TogglePostIt => "View: Post-It",
            ToggleAlwaysOnTop => "View: Always on Top",
            ToggleSplit => "View: Split View",
            ToggleDocMap => "View: Document Map",
            ToggleFunctionList => "View: Function List",
            ToggleWorkspace => "View: Folder as Workspace",
            ToggleResults => "View: Search Results Panel",
            ZoomIn => "View: Zoom In",
            ZoomOut => "View: Zoom Out",
            ZoomReset => "View: Restore Default Zoom",
            FoldAll => "Fold: Fold All",
            UnfoldAll => "Fold: Unfold All",
            ToggleFold => "Fold: Toggle Current",
            MacroRecord => "Macro: Start / Stop Recording",
            MacroPlay => "Macro: Playback",
            Run => "Run: Run...",
            About => "Help: About",
            Preferences => "Settings: Preferences",
            ToggleReadOnly => "Edit: Read-Only",
            _ => return match self {
                FoldLevel(n) => format!("Fold: Level {n}"),
                UnfoldLevel(n) => format!("Fold: Unfold Level {n}"),
                Case(m) => format!("Edit: Convert Case ({m:?})"),
                Sort(m, d) => format!("Line: Sort ({m:?}{})", if *d { ", descending" } else { "" }),
                Trim(m) => format!("Edit: Trim ({m:?})"),
                other => format!("{other:?}"),
            },
        };
        s.to_string()
    }

    /// Commands offered in the shortcut mapper (everything with a menu entry worth binding).
    pub fn bindable() -> Vec<Cmd> {
        use Cmd::*;
        let mut v: Vec<Cmd> = vec![
            New, Open, Save, SaveAs, SaveAll, Close, CloseAll, CloseOthers, CloseRight, CloseLeft, ReopenClosed, Reload, Rename, Exit, NextTab, PrevTab, MoveTabLeft,
            MoveTabRight, Undo, Redo, Cut, Copy, Paste, SelectAll, DuplicateLine, DeleteLine, CutLine, CopyLine, MoveLineUp, MoveLineDown, JoinLines, SplitLines,
            TransposeLine, BlankAbove, BlankBelow, Unindent, ToggleComment, BlockComment, GotoBrace, SelectBrace, AutoComplete, MultiSelectNext, MultiSelectAll,
            ColumnEditor, Find, Replace, FindInFiles, Mark, FindNext, FindPrev, FindSelNext, FindSelPrev, IncrementalSearch, GotoLine, ToggleBookmark, NextBookmark,
            PrevBookmark, ClearBookmarks, CutBookmarked, CopyBookmarked, RemoveBookmarked, InverseBookmarks, ToggleWrap, ToggleWhitespace, ToggleEol, ToggleAllChars,
            ToggleIndentGuides, ToggleFullscreen, TogglePostIt, ToggleAlwaysOnTop, ToggleSplit, ToggleDocMap, ToggleFunctionList, ToggleWorkspace, ToggleResults,
            ZoomIn, ZoomOut, ZoomReset, FoldAll, UnfoldAll, ToggleFold, MacroRecord, MacroPlay, Run, About, Preferences, ToggleReadOnly, ToggleStatusBar, ToggleToolBar,
            ToggleTabBar, ToggleLineNumbers, Trim(TrimMode::Trailing), Trim(TrimMode::Leading), Trim(TrimMode::Both), TabsToSpaces, SpacesToTabs(true), SpacesToTabs(false),
            Case(CaseMode::Upper), Case(CaseMode::Lower), Case(CaseMode::Title), Case(CaseMode::Sentence), Case(CaseMode::Invert),
            Sort(SortMode::Lexicographic, false), Sort(SortMode::Lexicographic, true), Sort(SortMode::LexicographicCi, false), Sort(SortMode::Integer, false),
            Sort(SortMode::Reverse, false), DedupLines(false), RemoveEmpty(false), InsertDateTime(false), InsertDateTime(true),
        ];
        for n in 1..=8 {
            v.push(FoldLevel(n));
            v.push(UnfoldLevel(n));
        }
        v
    }
}

pub struct Shortcut {
    pub mods: Modifiers,
    pub key: Key,
    pub cmd: Cmd,
}

const C: Modifiers = Modifiers::COMMAND;
const CS: Modifiers = Modifiers { alt: false, ctrl: false, shift: true, mac_cmd: false, command: true };
const CA: Modifiers = Modifiers { alt: true, ctrl: false, shift: false, mac_cmd: false, command: true };
const CAS: Modifiers = Modifiers { alt: true, ctrl: false, shift: true, mac_cmd: false, command: true };
const S: Modifiers = Modifiers::SHIFT;
const A: Modifiers = Modifiers::ALT;
const AS: Modifiers = Modifiers { alt: true, ctrl: false, shift: true, mac_cmd: false, command: false };
const N: Modifiers = Modifiers::NONE;

fn default_bindings() -> Vec<(Modifiers, Key, Cmd)> {
    use Cmd::*;
    let mut v: Vec<(Modifiers, Key, Cmd)> = vec![
        (C, Key::N, New),
        (C, Key::O, Open),
        (C, Key::S, Save),
        (CA, Key::S, SaveAs),
        (CS, Key::S, SaveAll),
        (C, Key::W, Close),
        (CS, Key::W, CloseAll),
        (CS, Key::T, ReopenClosed),
        (A, Key::F4, Exit),
        (C, Key::Tab, NextTab),
        (CS, Key::Tab, PrevTab),
        (C, Key::PageDown, NextTab),
        (C, Key::PageUp, PrevTab),
        (CS, Key::PageDown, MoveTabRight),
        (CS, Key::PageUp, MoveTabLeft),
        (C, Key::Z, Undo),
        (C, Key::Y, Redo),
        (CS, Key::Z, Redo),
        (C, Key::A, SelectAll),
        (C, Key::D, DuplicateLine),
        (C, Key::L, CutLine),
        (CS, Key::L, DeleteLine),
        (C, Key::I, SplitLines),
        (C, Key::J, JoinLines),
        (CS, Key::ArrowUp, MoveLineUp),
        (CS, Key::ArrowDown, MoveLineDown),
        (CA, Key::Enter, BlankAbove),
        (CAS, Key::Enter, BlankBelow),
        (C, Key::T, TransposeLine),
        (C, Key::U, Case(CaseMode::Lower)),
        (CS, Key::U, Case(CaseMode::Upper)),
        (C, Key::K, ToggleComment),
        (C, Key::Slash, ToggleComment),
        (C, Key::B, GotoBrace),
        (CS, Key::B, SelectBrace),
        (C, Key::Space, AutoComplete),
        (CA, Key::D, MultiSelectNext),
        (A, Key::C, ColumnEditor),
        (S, Key::Tab, Unindent),
        (C, Key::F, Find),
        (C, Key::H, Replace),
        (CS, Key::F, FindInFiles),
        (C, Key::M, Mark),
        (N, Key::F3, FindNext),
        (S, Key::F3, FindPrev),
        (C, Key::F3, FindSelNext),
        (CS, Key::F3, FindSelPrev),
        (CA, Key::I, IncrementalSearch),
        (C, Key::G, GotoLine),
        (C, Key::F2, ToggleBookmark),
        (N, Key::F2, NextBookmark),
        (S, Key::F2, PrevBookmark),
        (CS, Key::F2, ClearBookmarks),
        (N, Key::F11, ToggleFullscreen),
        (N, Key::F12, TogglePostIt),
        (C, Key::Equals, ZoomIn),
        (C, Key::Plus, ZoomIn),
        (C, Key::Minus, ZoomOut),
        (C, Key::Num0, ZoomReset),
        (A, Key::Num0, FoldAll),
        (AS, Key::Num0, UnfoldAll),
        (A, Key::F, ToggleFold),
        (CS, Key::R, MacroRecord),
        (CS, Key::P, MacroPlay),
        (N, Key::F5, Run),
        (N, Key::F1, About),
    ];
    if cfg!(target_os = "macos") {
        // Cmd+Q quits on macOS
        v.push((C, Key::Q, Exit));
    } else {
        v.push((C, Key::Q, ToggleComment));
    }
    v.push((CS, Key::Q, BlockComment));
    for n in 1..=8usize {
        let key = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8][n - 1];
        v.push((A, key, FoldLevel(n)));
        v.push((AS, key, UnfoldLevel(n)));
    }
    v
}

/// User overrides loaded from / saved to `shortcuts.json`: `None` = unbound.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Override {
    cmd: Cmd,
    key: Option<String>,
}

struct Table {
    overrides: Vec<Override>,
    list: Vec<Shortcut>,
}

fn overrides_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("shortcuts.json")
}

fn build_table(overrides: Vec<Override>) -> Table {
    let mut v = default_bindings();
    for o in &overrides {
        v.retain(|(_, _, c)| *c != o.cmd);
    }
    for o in &overrides {
        if let Some((m, k)) = o.key.as_deref().and_then(parse_shortcut) {
            // a combo can only trigger one command
            v.retain(|(m2, k2, _)| !(*m2 == m && *k2 == k));
            v.push((m, k, o.cmd.clone()));
        }
    }
    let weight = |m: &Modifiers| m.alt as u8 + m.shift as u8 + m.command as u8 + m.ctrl as u8;
    v.sort_by_key(|(m, _, _)| std::cmp::Reverse(weight(m)));
    Table { overrides, list: v.into_iter().map(|(mods, key, cmd)| Shortcut { mods, key, cmd }).collect() }
}

fn table() -> &'static RwLock<Table> {
    static CELL: OnceLock<RwLock<Table>> = OnceLock::new();
    CELL.get_or_init(|| {
        let overrides: Vec<Override> = std::fs::read_to_string(overrides_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        RwLock::new(build_table(overrides))
    })
}

/// Run `f` with the active bindings, most specific modifiers first so that
/// `Ctrl+Shift+S` is tested before `Ctrl+S`.
pub fn with_shortcuts<R>(f: impl FnOnce(&[Shortcut]) -> R) -> R {
    let t = table().read().unwrap();
    f(&t.list)
}

/// Rebind (or unbind with `None`) a command and persist the change.
pub fn set_binding(cmd: Cmd, binding: Option<(Modifiers, Key)>) {
    let mut t = table().write().unwrap();
    let mut ov = std::mem::take(&mut t.overrides);
    ov.retain(|o| o.cmd != cmd);
    ov.push(Override { cmd, key: binding.map(|(m, k)| format_shortcut(m, k)) });
    // an unbound-by-conflict command must stay unbound after reload
    *t = build_table(ov);
    save_overrides(&t.overrides);
}

pub fn reset_bindings() {
    let mut t = table().write().unwrap();
    *t = build_table(Vec::new());
    let _ = std::fs::remove_file(overrides_path());
}

fn save_overrides(ov: &[Override]) {
    let _ = std::fs::create_dir_all(crate::settings::config_dir());
    if let Ok(text) = serde_json::to_string_pretty(ov) {
        let _ = std::fs::write(overrides_path(), text);
    }
}

pub fn binding_of(cmd: &Cmd) -> Option<(Modifiers, Key)> {
    with_shortcuts(|l| l.iter().find(|s| &s.cmd == cmd).map(|s| (s.mods, s.key)))
}

/// Human-readable shortcut for a command (first binding), for menu hints.
pub fn shortcut_text(cmd: &Cmd) -> String {
    binding_of(cmd).map(|(m, k)| format_shortcut(m, k)).unwrap_or_default()
}

pub fn format_shortcut(m: Modifiers, key: Key) -> String {
    let mut parts: Vec<String> = Vec::new();
    if m.command || m.ctrl {
        parts.push(if cfg!(target_os = "macos") { "Cmd".into() } else { "Ctrl".into() });
    }
    if m.alt {
        parts.push(if cfg!(target_os = "macos") { "Opt".into() } else { "Alt".into() });
    }
    if m.shift {
        parts.push("Shift".into());
    }
    let k = match key {
        Key::Equals | Key::Plus => "+".to_string(),
        Key::Minus => "-".to_string(),
        Key::Slash => "/".to_string(),
        Key::ArrowUp => "Up".into(),
        Key::ArrowDown => "Down".into(),
        Key::ArrowLeft => "Left".into(),
        Key::ArrowRight => "Right".into(),
        Key::Num0 => "0".into(),
        Key::Num1 => "1".into(),
        Key::Num2 => "2".into(),
        Key::Num3 => "3".into(),
        Key::Num4 => "4".into(),
        Key::Num5 => "5".into(),
        Key::Num6 => "6".into(),
        Key::Num7 => "7".into(),
        Key::Num8 => "8".into(),
        Key::Num9 => "9".into(),
        other => format!("{other:?}"),
    };
    parts.push(k);
    parts.join("+")
}

/// Inverse of `format_shortcut`.
pub fn parse_shortcut(s: &str) -> Option<(Modifiers, Key)> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    // the key part may itself be "+" ("Ctrl++")
    let (head, tail) = match s.strip_suffix("++") {
        Some(h) => (h, Some("+")),
        None => (s, None),
    };
    let mut parts: Vec<&str> = if head.is_empty() { Vec::new() } else { head.split('+').collect() };
    if let Some(t) = tail {
        parts.push(t);
    }
    for part in parts {
        match part.to_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => {
                mods.command = true;
                mods.ctrl = !cfg!(target_os = "macos");
                mods.mac_cmd = cfg!(target_os = "macos");
            }
            "alt" | "opt" | "option" => mods.alt = true,
            "shift" => mods.shift = true,
            "+" | "plus" | "=" => key = Some(Key::Equals),
            "-" | "minus" => key = Some(Key::Minus),
            "/" | "slash" => key = Some(Key::Slash),
            "up" => key = Some(Key::ArrowUp),
            "down" => key = Some(Key::ArrowDown),
            "left" => key = Some(Key::ArrowLeft),
            "right" => key = Some(Key::ArrowRight),
            d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
                key = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9].get((d.as_bytes()[0] - b'0') as usize).copied();
            }
            _ => key = Key::from_name(part).or_else(|| Key::from_name(&part.to_uppercase())),
        }
    }
    key.map(|k| (Modifiers { command: mods.command, ..mods }, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_order_is_specific_first() {
        with_shortcuts(|s| {
            let save = s.iter().position(|x| x.cmd == Cmd::Save).unwrap();
            let save_all = s.iter().position(|x| x.cmd == Cmd::SaveAll).unwrap();
            assert!(save_all < save);
        });
    }

    #[test]
    fn no_duplicate_bindings() {
        let d = default_bindings();
        for (i, a) in d.iter().enumerate() {
            for b in &d[i + 1..] {
                assert!(!(a.0 == b.0 && a.1 == b.1), "duplicate binding {:?} {:?}", a.1, a.2);
            }
        }
    }

    #[test]
    fn shortcut_text_roundtrip() {
        for (m, k, _) in default_bindings() {
            let text = format_shortcut(m, k);
            let (m2, k2) = parse_shortcut(&text).unwrap_or_else(|| panic!("cannot parse {text}"));
            // Plus/Equals are interchangeable spellings of the same key
            let same_key = k2 == k || (matches!(k, Key::Plus | Key::Equals) && matches!(k2, Key::Plus | Key::Equals));
            assert!(same_key && m2.alt == m.alt && m2.shift == m.shift && m2.command == m.command, "{text}");
        }
    }

    #[test]
    fn overrides_replace_and_unbind() {
        let t = build_table(vec![
            Override { cmd: Cmd::Save, key: Some("Ctrl+Alt+K".into()) },
            Override { cmd: Cmd::Open, key: None },
        ]);
        assert!(t.list.iter().any(|s| s.cmd == Cmd::Save && s.key == Key::K && s.mods.alt));
        assert!(!t.list.iter().any(|s| s.cmd == Cmd::Open));
        assert_eq!(t.list.iter().filter(|s| s.cmd == Cmd::Save).count(), 1);
    }

    #[test]
    fn macro_roundtrip() {
        let steps = vec![Cmd::Type("hi".into()), Cmd::Move(Move::Down, false), Cmd::Sort(SortMode::Integer, true)];
        let json = serde_json::to_string(&steps).unwrap();
        let back: Vec<Cmd> = serde_json::from_str(&json).unwrap();
        assert_eq!(steps, back);
    }
}
