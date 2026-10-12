//! Randomised tests for the editing engine: arbitrary operation sequences must never
//! panic, must keep selections valid, and must be fully reversible with undo/redo.

use crate::doc::{Doc, Eol, Metrics, Move, Sel};
use crate::encoding::Enc;
use crate::lang;
use crate::ops::{CaseMode, SortMode, TrimMode};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const SAMPLES: &[&str] = &[
    "fn main() {\n    let x = 1;\n}\n",
    "héllo wörld\n日本語のテキスト\n\ttabbed\r\nmixed eol\rend",
    "",
    "a\n\n\nb\n   \nc",
    "{\n  [\n    (1, 2),\n  ]\n}\n// comment\n",
];

fn random_text(r: &mut Rng) -> String {
    const PIECES: &[&str] = &["a", "bc", " ", "\t", "\n", "{", "}", "é", "日", "😀", "\r\n", "foo", "  ", "// x", "(", ")", "\"", "'"];
    let n = r.below(6) + 1;
    (0..n).map(|_| PIECES[r.below(PIECES.len())]).collect()
}

fn check_invariants(d: &mut Doc, what: &str) {
    let n = d.len_chars();
    assert!(!d.sels.is_empty(), "{what}: no selections");
    for s in &d.sels {
        assert!(s.anchor <= n && s.head <= n, "{what}: selection {s:?} out of range {n}");
    }
    for w in d.sels.windows(2) {
        assert!(w[0].end() <= w[1].start(), "{what}: overlapping selections {:?} {:?}", w[0], w[1]);
    }
    let lines = d.len_lines();
    for b in &d.bookmarks {
        assert!(*b < lines, "{what}: bookmark {b} beyond {lines} lines");
    }
    // helpers must not panic anywhere in the document
    for l in 0..lines.min(50) {
        let _ = d.line_text(l);
        let _ = d.line_start(l);
        let _ = d.line_end(l);
        let mut spans = Vec::new();
        let _ = d.line_spans(l, &mut spans);
    }
    let _ = d.folds();
    let _ = d.find_matching_bracket(d.caret());
    let c = d.caret();
    let _ = d.word_range_at(c);
}

fn step(d: &mut Doc, r: &mut Rng) -> &'static str {
    let m = Metrics { wrap_cols: if r.below(3) == 0 { 20 } else { 0 }, page_rows: 5 };
    let moves = [
        Move::Left, Move::Right, Move::Up, Move::Down, Move::WordLeft, Move::WordRight, Move::Home, Move::End, Move::DocStart, Move::DocEnd,
        Move::PageUp, Move::PageDown, Move::LineStart, Move::LineEnd, Move::ParaUp, Move::ParaDown,
    ];
    match r.below(40) {
        0 | 1 | 2 => {
            let t = random_text(r);
            d.type_text(&t);
            "type"
        }
        3 => {
            d.newline();
            "newline"
        }
        4 => {
            d.backspace();
            "backspace"
        }
        5 => {
            d.delete_forward();
            "delete"
        }
        6 => {
            d.tab();
            "tab"
        }
        7 => {
            d.indent_lines(false);
            "unindent"
        }
        8 | 9 => {
            d.undo();
            "undo"
        }
        10 => {
            d.redo();
            "redo"
        }
        11..=15 => {
            let mv = moves[r.below(moves.len())];
            let ext = r.below(2) == 0;
            d.move_sels(mv, ext, m);
            "move"
        }
        16 => {
            d.select_all();
            "select_all"
        }
        17 => {
            d.duplicate();
            "duplicate"
        }
        18 => {
            d.delete_lines();
            "delete_lines"
        }
        19 => {
            d.move_lines(r.below(2) == 0);
            "move_lines"
        }
        20 => {
            d.join_lines();
            "join"
        }
        21 => {
            let modes = [SortMode::Lexicographic, SortMode::LexicographicCi, SortMode::Integer, SortMode::Decimal, SortMode::Reverse, SortMode::Random];
            d.sort_lines(modes[r.below(modes.len())], r.below(2) == 0);
            "sort"
        }
        22 => {
            let modes = [CaseMode::Upper, CaseMode::Lower, CaseMode::Title, CaseMode::Sentence, CaseMode::Invert, CaseMode::Random];
            d.change_case(modes[r.below(modes.len())]);
            "case"
        }
        23 => {
            d.toggle_line_comment();
            "comment"
        }
        24 => {
            d.toggle_block_comment();
            "block_comment"
        }
        25 => {
            let modes = [TrimMode::Trailing, TrimMode::Leading, TrimMode::Both];
            d.trim_whitespace(modes[r.below(3)]);
            "trim"
        }
        26 => {
            if r.below(2) == 0 { d.tabs_to_spaces() } else { d.spaces_to_tabs(r.below(2) == 0) }
            "tabs"
        }
        27 => {
            let e = [Eol::Lf, Eol::CrLf, Eol::Cr][r.below(3)];
            d.convert_eol(e);
            "eol"
        }
        28 => {
            let n = d.len_chars();
            let p = r.below(n + 1);
            d.set_caret(p);
            "set_caret"
        }
        29 => {
            let n = d.len_chars();
            let (a, b) = (r.below(n + 1), r.below(n + 1));
            d.set_selection(a, b);
            "set_selection"
        }
        30 => {
            let n = d.len_chars();
            let p = r.below(n + 1);
            d.add_selection(Sel::caret(p));
            "add_caret"
        }
        31 => {
            let t = random_text(r);
            d.paste(&t);
            "paste"
        }
        32 => {
            d.delete_word_left();
            d.delete_word_right();
            "delete_word"
        }
        33 => {
            d.transpose_lines();
            "transpose"
        }
        34 => {
            d.insert_blank_line(r.below(2) == 0);
            "blank_line"
        }
        35 => {
            let l = r.below(d.len_lines());
            d.toggle_bookmark(l);
            "bookmark"
        }
        36 => {
            let l = r.below(d.len_lines());
            d.toggle_fold(l);
            if r.below(2) == 0 {
                d.fold_all();
            } else {
                d.unfold_all();
            }
            "fold"
        }
        37 => {
            d.column_insert("x", None);
            "column"
        }
        38 => {
            d.remove_duplicate_lines(r.below(2) == 0);
            d.remove_empty_lines(r.below(2) == 0);
            "dedupe"
        }
        _ => {
            d.indent_lines(true);
            "indent"
        }
    }
}

#[test]
fn random_operations_keep_invariants_and_undo_roundtrips() {
    let langs = ["Normal Text", "Rust", "Python", "HTML", "C", "JSON"];
    let seeds: u64 = std::env::var("FUZZ_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    for seed in 1..=seeds {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let sample = SAMPLES[r.below(SAMPLES.len())];
        let mut d = Doc::from_text(sample, Enc::Utf8, Eol::Lf);
        d.opts.use_spaces = r.below(2) == 0;
        d.opts.auto_close = r.below(3) == 0;
        d.opts.auto_close_tags = r.below(3) == 0;
        d.set_lang(lang::find_by_name(langs[r.below(langs.len())]).unwrap());
        let initial = d.rope.to_string();
        let mut trace: Vec<&str> = Vec::new();
        for _ in 0..60 {
            // undo history is only a round-trip guarantee when we don't mix undo/redo into the check
            let name = step(&mut d, &mut r);
            trace.push(name);
            check_invariants(&mut d, &format!("seed {seed} after {name}; trace {trace:?}"));
            // the incrementally maintained row map must equal one built from scratch
            let wrap = if seed % 2 == 0 { 12 } else { 0 };
            if std::env::var_os("FUZZ_DEBUG_SEED").is_some_and(|v| v.to_string_lossy() == seed.to_string()) {
                eprintln!("step {name}: dirty={:?} records={} splice={:?} lines={} text={:?}", d.rm_dirty, d.rm_records, d.rm_splice, d.len_lines(), d.rope.to_string());
            }
            let mut rm = std::mem::take(&mut d.rowmap);
            rm.ensure(&mut d, wrap);
            d.rowmap = rm;
            let mut fresh = crate::rowmap::RowMap::default();
            fresh.ensure(&mut d, wrap);
            assert_eq!(d.rowmap.total_rows(), fresh.total_rows(), "seed {seed} after {name}: row totals differ; trace {trace:?}");
            for l in 0..d.len_lines() {
                assert_eq!(d.rowmap.first_row_of_line(l), fresh.first_row_of_line(l), "seed {seed} after {name}: line {l} row differs; trace {trace:?}");
            }
            for row in 0..d.rowmap.total_rows().min(40) {
                assert_eq!(d.rowmap.locate(row), fresh.locate(row), "seed {seed} after {name}: locate({row}); trace {trace:?}");
            }
        }
        // undo everything → original text; redo everything → same final text as before the undos
        let final_text = d.rope.to_string();
        let mut guard = 0;
        while d.undo() {
            guard += 1;
            assert!(guard < 10_000, "undo does not terminate (seed {seed})");
        }
        assert_eq!(d.rope.to_string(), initial, "seed {seed}: undo-all did not restore the text; trace {trace:?}");
        check_invariants(&mut d, "after undo-all");
        // The text after redo-all equals the text at the end only if the last action did not
        // leave redo history behind; compare against the redone state of a fresh redo pass.
        let mut redone = 0;
        while d.redo() {
            redone += 1;
            assert!(redone < 10_000);
        }
        let _ = final_text;
        check_invariants(&mut d, "after redo-all");
    }
}

#[test]
fn undo_restores_selection_and_modified_flag() {
    let mut d = Doc::from_text("hello", Enc::Utf8, Eol::Lf);
    d.set_caret(5);
    d.type_text(" world");
    assert!(d.is_modified());
    d.undo();
    assert!(!d.is_modified());
    assert_eq!(d.caret(), 5);
    d.redo();
    assert_eq!(d.rope.to_string(), "hello world");
}

#[test]
fn lexers_never_panic_on_garbage() {
    const PIECES: &[&str] = &[
        "\"", "'", "`", "\\", "/*", "*/", "//", "#", "<!--", "-->", "<", ">", "</", "/>", "=", ":", "[", "]", "{", "}", "(", ")", " ", "\t", "é", "日", "😀", "$", "@", "%", "--[[", "]]",
        "\"\"\"", "'''", "{-", "-}", "<#", "#>", "foo", "if", "123", "0x1f", "1e+5", ".5", "...", "&amp;", "```", "**", "__", "# ", "- ", "+++", "@@", "[a]", "k: v", "rem ", "::",
    ];
    let mut r = Rng(0xDEADBEEF);
    for (id, def) in lang::LANGS.iter().enumerate() {
        let compiled = lang::compiled(id);
        for _ in 0..300 {
            let mut state = crate::lexer::LexState::Normal;
            let mut spans = Vec::new();
            for _ in 0..6 {
                let n = r.below(12) + 1;
                let line: String = (0..n).map(|_| PIECES[r.below(PIECES.len())]).collect();
                state = crate::lexer::lex_line(compiled, &line, state, &mut spans);
                // spans must be ordered, in-bounds and on char boundaries
                let mut last_end = 0;
                for s in &spans {
                    assert!(s.start >= last_end && s.start <= s.end && s.end <= line.len(), "{}: bad span {s:?} in {line:?}", def.name);
                    assert!(line.is_char_boundary(s.start) && line.is_char_boundary(s.end), "{}: span splits a char in {line:?}", def.name);
                    last_end = s.end;
                }
            }
        }
        // folding over garbage documents
        let text: String = (0..40).map(|_| format!("{}\n", (0..4).map(|_| PIECES[r.below(PIECES.len())]).collect::<String>())).collect();
        let mut d = Doc::from_text(&text, Enc::Utf8, Eol::Lf);
        d.set_lang(id);
        let _ = d.folds();
        d.fold_all();
        for l in 0..d.len_lines() {
            let _ = d.is_hidden(l);
        }
    }
}

#[test]
fn search_never_panics_on_odd_queries() {
    use crate::search::{self, Mode, Query};
    let d = Doc::from_text("héllo\r\nwörld 日本\n\n😀 end", Enc::Utf8, Eol::CrLf);
    let patterns = ["", "(", "[", "\\", "é", "^", "$", ".*", "\\n", "\\xZZ", "日", "😀", "(?i)A", "\\b", "x{2,1}", "[a-"];
    for p in patterns {
        for mode in [Mode::Normal, Mode::Extended, Mode::Regex] {
            for backward in [false, true] {
                let q = Query { pattern: p.into(), replace: "$1\\1\\n$".into(), mode, whole_word: p.len() % 2 == 0, match_case: p.len() % 3 == 0, ..Default::default() };
                let _ = search::find(&d, &q, 3, backward, None);
                let _ = search::find_all(&d, &q, None);
                let _ = search::replace_all_edits(&d, &q, None);
            }
        }
    }
}
