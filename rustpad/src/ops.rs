//! Text transformation commands on a `Doc`: line operations, case, whitespace,
//! comments, sorting, column editing.

use std::collections::HashSet;

use crate::doc::{After, Doc, EditKind, Eol, Sel, TextEdit};
use crate::lang;
use crate::util;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum SortMode {
    Lexicographic,
    LexicographicCi,
    Integer,
    Decimal,
    Random,
    Reverse,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum CaseMode {
    Upper,
    Lower,
    Title,
    Sentence,
    Invert,
    Random,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum TrimMode {
    Trailing,
    Leading,
    Both,
}

#[derive(Clone, Debug)]
pub struct ColumnNumbers {
    pub start: i64,
    pub step: i64,
    pub repeat: usize,
    pub width: usize,
    pub radix: u32,
    pub zero_pad: bool,
}

fn fmt_radix(mut v: i64, radix: u32, upper: bool) -> String {
    let neg = v < 0;
    if neg {
        v = -v;
    }
    let mut digits = Vec::new();
    let mut n = v as u64;
    if n == 0 {
        digits.push('0');
    }
    while n > 0 {
        let d = (n % radix as u64) as u32;
        let c = std::char::from_digit(d, radix).unwrap();
        digits.push(if upper { c.to_ascii_uppercase() } else { c });
        n /= radix as u64;
    }
    if neg {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

impl Doc {
    fn block_text(&self, a: usize, b: usize) -> String {
        self.slice(self.line_start(a), self.line_end(b))
    }

    /// Text of the lines touched by the selections, each with a line terminator.
    pub fn lines_text(&self) -> String {
        let eol = self.eol.as_str();
        let mut out = String::new();
        for (a, b) in self.selected_line_blocks() {
            for l in a..=b {
                out.push_str(&self.line_text(l));
                out.push_str(eol);
            }
        }
        out
    }

    pub fn delete_lines(&mut self) {
        let n = self.len_lines();
        let edits = self
            .selected_line_blocks()
            .into_iter()
            .map(|(a, b)| {
                if b + 1 < n {
                    TextEdit::delete(self.line_start(a), self.line_start(b + 1)).after(After::Start)
                } else if a > 0 {
                    TextEdit::delete(self.line_end(a - 1), self.line_end(b)).after(After::Start)
                } else {
                    TextEdit::delete(0, self.line_end(b)).after(After::Start)
                }
            })
            .collect();
        self.exec(edits, EditKind::Other);
    }

    pub fn duplicate(&mut self) {
        let eol = self.eol.as_str();
        // selections with content: duplicate the selected text; carets: duplicate the line
        if self.sels.iter().any(|s| !s.is_empty()) {
            let mut edits = Vec::new();
            for s in &self.sels {
                if !s.is_empty() {
                    edits.push(TextEdit::insert(s.end(), self.slice(s.start(), s.end())));
                }
            }
            // select the new copy
            let mut sels2 = Vec::new();
            let mut sh = 0usize;
            for s in &self.sels {
                if s.is_empty() {
                    sels2.push(Sel::caret(s.head + sh));
                } else {
                    let len = s.end() - s.start();
                    sels2.push(Sel::new(s.end() + sh, s.end() + sh + len));
                    sh += len;
                }
            }
            self.commit(edits, sels2, EditKind::Other);
            return;
        }
        let mut edits = Vec::new();
        let mut sels = Vec::new();
        let mut shift = 0usize;
        let blocks = self.selected_line_blocks();
        for s in &self.sels {
            let l = self.line_of(s.head);
            let bi = blocks.iter().position(|&(a, b)| a <= l && l <= b).unwrap_or(0);
            let (a, b) = blocks[bi];
            let t = self.block_text(a, b);
            let ins = format!("{eol}{t}");
            let len = ins.chars().count();
            // caret moves to the same column on the copy
            let block_shift = shift + (self.line_end(b) - self.line_start(a)) + eol.chars().count();
            sels.push(Sel::caret(s.head + block_shift));
            if !edits.iter().any(|e: &TextEdit| e.start == self.line_end(b)) {
                edits.push(TextEdit::insert(self.line_end(b), ins));
                shift += len;
            }
        }
        self.commit(edits, sels, EditKind::Other);
    }

    pub fn move_lines(&mut self, up: bool) {
        let eol = self.eol.as_str();
        let eol_len = eol.chars().count();
        let n = self.len_lines();
        let mut blocks: Vec<(usize, usize)> = Vec::new();
        for (a, b) in self.selected_line_blocks() {
            match blocks.last_mut() {
                Some(l) if a <= l.1 + 1 => l.1 = l.1.max(b),
                _ => blocks.push((a, b)),
            }
        }
        // only the first/last block can be blocked by the document edge; keep it simple
        let mut edits = Vec::new();
        let mut moves: Vec<(usize, usize, isize)> = Vec::new(); // (old start, old end, shift)
        for (a, b) in blocks {
            if up {
                if a == 0 {
                    continue;
                }
                let prev = self.line_text(a - 1);
                let body = self.block_text(a, b);
                let start = self.line_start(a - 1);
                let end = self.line_end(b);
                edits.push(TextEdit::replace(start, end, format!("{body}{eol}{prev}")));
                let shift = -((prev.chars().count() + eol_len) as isize);
                moves.push((self.line_start(a), self.line_end(b), shift));
            } else {
                if b + 1 >= n {
                    continue;
                }
                let next = self.line_text(b + 1);
                let body = self.block_text(a, b);
                let start = self.line_start(a);
                let end = self.line_end(b + 1);
                edits.push(TextEdit::replace(start, end, format!("{next}{eol}{body}")));
                let shift = (next.chars().count() + eol_len) as isize;
                moves.push((self.line_start(a), self.line_end(b), shift));
            }
        }
        if edits.is_empty() {
            return;
        }
        let sels: Vec<Sel> = self
            .sels
            .iter()
            .map(|s| {
                let f = |p: usize| -> usize {
                    for &(a, b, sh) in &moves {
                        if p >= a && p <= b {
                            return (p as isize + sh) as usize;
                        }
                    }
                    p
                };
                Sel::new(f(s.anchor), f(s.head))
            })
            .collect();
        self.commit(edits, sels, EditKind::Other);
    }

    pub fn join_lines(&mut self) {
        let n = self.len_lines();
        let mut edits = Vec::new();
        for (a, b) in self.selected_line_blocks() {
            let b = if a == b { (a + 1).min(n - 1) } else { b };
            if b == a {
                continue;
            }
            let mut text = self.line_text(a);
            for l in a + 1..=b {
                let t = self.line_text(l);
                let t = t.trim_start();
                if !text.is_empty() && !text.ends_with(' ') && !t.is_empty() {
                    text.push(' ');
                }
                text.push_str(t);
            }
            let len = text.chars().count();
            edits.push(TextEdit::replace(self.line_start(a), self.line_end(b), text).after(After::Select(len, len)));
        }
        self.exec(edits, EditKind::Other);
    }

    /// Re-wrap the selected lines to `width` columns.
    pub fn split_lines_at(&mut self, width: usize) {
        let eol = self.eol.as_str().to_string();
        let width = width.max(1);
        self.transform_lines(|lines| {
            let words: Vec<&str> = lines.iter().flat_map(|l| l.split_whitespace()).collect();
            let mut out = Vec::new();
            let mut cur = String::new();
            for w in words {
                if !cur.is_empty() && cur.chars().count() + 1 + w.chars().count() > width {
                    out.push(std::mem::take(&mut cur));
                }
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(w);
            }
            if !cur.is_empty() {
                out.push(cur);
            }
            let _ = &eol;
            out
        });
    }

    pub fn transpose_lines(&mut self) {
        let l = self.line_of(self.caret());
        if l == 0 {
            return;
        }
        let eol = self.eol.as_str();
        let a = self.line_text(l - 1);
        let b = self.line_text(l);
        let text = format!("{b}{eol}{a}");
        let start = self.line_start(l - 1);
        let end = self.line_end(l);
        let len = text.chars().count();
        let nl = if l + 1 < self.len_lines() { Some(self.line_start(l + 1)) } else { None };
        let _ = nl;
        self.commit(vec![TextEdit::replace(start, end, text)], vec![Sel::caret(start + len.min(len))], EditKind::Other);
        // place the caret at the start of the following line when possible
        let target = (l + 1).min(self.len_lines() - 1);
        self.set_caret(self.line_start(target));
    }

    pub fn insert_blank_line(&mut self, above: bool) {
        let eol = self.eol.as_str();
        let mut edits = Vec::new();
        for (a, b) in self.selected_line_blocks() {
            let indent = util::leading_ws(&self.line_text(if above { a } else { b })).to_string();
            let ilen = indent.chars().count();
            if above {
                edits.push(TextEdit::insert(self.line_start(a), format!("{indent}{eol}")).after(After::Select(ilen, ilen)));
            } else {
                edits.push(TextEdit::insert(self.line_end(b), format!("{eol}{indent}")).after(After::End));
            }
        }
        self.exec(edits, EditKind::Other);
    }

    /// Apply `f` to the lines of the selected blocks (or the whole document when nothing is selected).
    pub fn transform_lines(&mut self, f: impl Fn(Vec<String>) -> Vec<String>) {
        let eol = self.eol.as_str();
        let blocks = if self.has_selection() { self.selected_line_blocks() } else { vec![(0, self.len_lines() - 1)] };
        let mut edits = Vec::new();
        for (a, b) in blocks {
            let lines: Vec<String> = (a..=b).map(|l| self.line_text(l)).collect();
            let out = f(lines).join(eol);
            let len = out.chars().count();
            edits.push(TextEdit::replace(self.line_start(a), self.line_end(b), out).after(After::Select(0, len)));
        }
        self.exec(edits, EditKind::Other);
        if self.sels.iter().all(|s| s.is_empty() || s.end() - s.start() == 0) {
            return;
        }
    }

    pub fn sort_lines(&mut self, mode: SortMode, descending: bool) {
        self.transform_lines(|mut lines| {
            match mode {
                SortMode::Lexicographic => lines.sort(),
                SortMode::LexicographicCi => lines.sort_by_key(|a| a.to_lowercase()),
                SortMode::Integer | SortMode::Decimal => {
                    let key = |s: &String| util::leading_number(s).map(|v| if mode == SortMode::Integer { v.trunc() } else { v });
                    lines.sort_by(|a, b| match (key(a), key(b)) {
                        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                        (None, Some(_)) => std::cmp::Ordering::Less,
                        (Some(_), None) => std::cmp::Ordering::Greater,
                        (None, None) => a.cmp(b),
                    });
                }
                SortMode::Reverse => {
                    lines.reverse();
                    return lines;
                }
                SortMode::Random => {
                    let mut seed = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(12345)
                        | 1;
                    let mut next = || {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        seed
                    };
                    for i in (1..lines.len()).rev() {
                        let j = (next() % (i as u64 + 1)) as usize;
                        lines.swap(i, j);
                    }
                    return lines;
                }
            }
            if descending {
                lines.reverse();
            }
            lines
        });
    }

    pub fn remove_duplicate_lines(&mut self, consecutive_only: bool) {
        self.transform_lines(|lines| {
            let mut seen = HashSet::new();
            let mut out: Vec<String> = Vec::new();
            for l in lines {
                if consecutive_only {
                    if out.last() != Some(&l) {
                        out.push(l);
                    }
                } else if seen.insert(l.clone()) {
                    out.push(l);
                }
            }
            out
        });
    }

    pub fn remove_empty_lines(&mut self, blank_too: bool) {
        self.transform_lines(|lines| {
            lines.into_iter().filter(|l| if blank_too { !l.trim().is_empty() } else { !l.is_empty() }).collect()
        });
    }

    pub fn trim_whitespace(&mut self, mode: TrimMode) {
        let n = self.len_lines();
        let mut edits = Vec::new();
        let blocks = if self.has_selection() { self.selected_line_blocks() } else { vec![(0, n - 1)] };
        for (a, b) in blocks {
            for l in a..=b {
                let t = self.line_text(l);
                let new = match mode {
                    TrimMode::Trailing => t.trim_end_matches([' ', '\t']).to_string(),
                    TrimMode::Leading => t.trim_start_matches([' ', '\t']).to_string(),
                    TrimMode::Both => t.trim_matches([' ', '\t']).to_string(),
                };
                if new != t {
                    edits.push(TextEdit::replace(self.line_start(l), self.line_end(l), new));
                }
            }
        }
        self.exec_keep(edits, EditKind::Other);
    }

    pub fn tabs_to_spaces(&mut self) {
        let tab = self.opts.tab_width.max(1);
        let n = self.len_lines();
        let mut edits = Vec::new();
        for l in 0..n {
            let t = self.line_text(l);
            if !t.contains('\t') {
                continue;
            }
            let mut out = String::new();
            let mut col = 0;
            for c in t.chars() {
                if c == '\t' {
                    let w = tab - col % tab;
                    out.extend(std::iter::repeat_n(' ', w));
                    col += w;
                } else {
                    out.push(c);
                    col += util::char_cells(c, col, tab);
                }
            }
            edits.push(TextEdit::replace(self.line_start(l), self.line_end(l), out));
        }
        self.exec_keep(edits, EditKind::Other);
    }

    /// Convert runs of leading spaces into tabs (`all` = also inside the line).
    pub fn spaces_to_tabs(&mut self, all: bool) {
        let tab = self.opts.tab_width.max(1);
        let n = self.len_lines();
        let mut edits = Vec::new();
        for l in 0..n {
            let t = self.line_text(l);
            let new = if all {
                let mut out = String::new();
                let mut col = 0usize;
                let mut run = 0usize;
                let flush = |out: &mut String, col: &mut usize, run: &mut usize| {
                    // emit `run` spaces ending at column `col`
                    let mut start = *col - *run;
                    let mut left = *run;
                    while left > 0 {
                        let next_stop = (start / tab + 1) * tab;
                        let span = next_stop - start;
                        if span <= left && span > 1 {
                            out.push('\t');
                            left -= span;
                            start = next_stop;
                        } else {
                            out.push(' ');
                            left -= 1;
                            start += 1;
                        }
                    }
                    *run = 0;
                };
                for c in t.chars() {
                    if c == ' ' {
                        run += 1;
                        col += 1;
                    } else {
                        flush(&mut out, &mut col, &mut run);
                        out.push(c);
                        col += util::char_cells(c, col, tab);
                    }
                }
                flush(&mut out, &mut col, &mut run);
                out
            } else {
                let lead = util::leading_ws(&t);
                let width = util::display_width(lead, tab);
                let rest = &t[lead.len()..];
                format!("{}{}{}", "\t".repeat(width / tab), " ".repeat(width % tab), rest)
            };
            if new != t {
                edits.push(TextEdit::replace(self.line_start(l), self.line_end(l), new));
            }
        }
        self.exec_keep(edits, EditKind::Other);
    }

    pub fn change_case(&mut self, mode: CaseMode) {
        let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
        let edits: Vec<TextEdit> = self
            .sels
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| {
                let t = self.slice(s.start(), s.end());
                let new: String = match mode {
                    CaseMode::Upper => t.to_uppercase(),
                    CaseMode::Lower => t.to_lowercase(),
                    CaseMode::Title => {
                        let mut out = String::new();
                        let mut start = true;
                        for c in t.chars() {
                            if c.is_alphanumeric() || c == '\'' && !start {
                                if start {
                                    out.extend(c.to_uppercase());
                                } else {
                                    out.extend(c.to_lowercase());
                                }
                                start = false;
                            } else {
                                out.push(c);
                                start = true;
                            }
                        }
                        out
                    }
                    CaseMode::Sentence => {
                        let mut out = String::new();
                        let mut cap = true;
                        for c in t.chars() {
                            if c.is_alphabetic() {
                                if cap {
                                    out.extend(c.to_uppercase());
                                } else {
                                    out.extend(c.to_lowercase());
                                }
                                cap = false;
                            } else {
                                out.push(c);
                                if matches!(c, '.' | '!' | '?' | '\n') {
                                    cap = true;
                                }
                            }
                        }
                        out
                    }
                    CaseMode::Invert => t
                        .chars()
                        .flat_map(|c| -> Vec<char> {
                            if c.is_uppercase() {
                                c.to_lowercase().collect()
                            } else if c.is_lowercase() {
                                c.to_uppercase().collect()
                            } else {
                                vec![c]
                            }
                        })
                        .collect(),
                    CaseMode::Random => t
                        .chars()
                        .flat_map(|c| -> Vec<char> {
                            seed ^= seed << 13;
                            seed ^= seed >> 7;
                            seed ^= seed << 17;
                            if seed & 1 == 0 { c.to_uppercase().collect() } else { c.to_lowercase().collect() }
                        })
                        .collect(),
                };
                let len = new.chars().count();
                TextEdit::replace(s.start(), s.end(), new).after(if s.head >= s.anchor { After::Select(0, len) } else { After::Select(len, 0) })
            })
            .collect();
        self.exec(edits, EditKind::Other);
    }

    pub fn indent_lines(&mut self, indent: bool) {
        let unit = self.indent_unit();
        let tab = self.opts.tab_width.max(1);
        let mut edits = Vec::new();
        for (a, b) in self.selected_line_blocks() {
            for l in a..=b {
                let t = self.line_text(l);
                let ls = self.line_start(l);
                if indent {
                    if !t.is_empty() {
                        edits.push(TextEdit::insert(ls, unit.clone()));
                    }
                } else if t.starts_with('\t') {
                    edits.push(TextEdit::delete(ls, ls + 1));
                } else {
                    let sp = t.chars().take(tab).take_while(|c| *c == ' ').count();
                    if sp > 0 {
                        edits.push(TextEdit::delete(ls, ls + sp));
                    }
                }
            }
        }
        // keep selections spanning the same text: insertions at a selection start shift it too
        self.exec_keep(edits, EditKind::Other);
    }

    pub fn toggle_line_comment(&mut self) {
        let compiled = lang::compiled(self.lang);
        let Some(&tok) = compiled.line_comments.first() else {
            return self.toggle_block_comment();
        };
        let tok = tok.to_string();
        let blocks = self.selected_line_blocks();
        let mut all_commented = true;
        let mut any = false;
        let mut min_indent = usize::MAX;
        for &(a, b) in &blocks {
            for l in a..=b {
                let t = self.line_text(l);
                if t.trim().is_empty() {
                    continue;
                }
                any = true;
                let ind = util::leading_ws(&t);
                min_indent = min_indent.min(ind.chars().count());
                if !t[ind.len()..].starts_with(&tok) {
                    all_commented = false;
                }
            }
        }
        if !any {
            return;
        }
        let mut edits = Vec::new();
        for (a, b) in blocks {
            for l in a..=b {
                let t = self.line_text(l);
                if t.trim().is_empty() {
                    continue;
                }
                let ls = self.line_start(l);
                if all_commented {
                    let ind = util::leading_ws(&t).chars().count();
                    let after = &t[util::leading_ws(&t).len() + tok.len()..];
                    let extra = usize::from(after.starts_with(' '));
                    edits.push(TextEdit::delete(ls + ind, ls + ind + tok.chars().count() + extra));
                } else {
                    edits.push(TextEdit::insert(ls + min_indent, format!("{tok} ")));
                }
            }
        }
        self.exec_keep(edits, EditKind::Other);
    }

    pub fn toggle_block_comment(&mut self) {
        let def = lang::compiled(self.lang).def;
        let (open, close) = if def.block_open.is_empty() {
            // fall back to a line comment per line
            if let Some(tok) = def.line_comment.split_whitespace().next() {
                let _ = tok;
            }
            return;
        } else {
            (def.block_open, def.block_close)
        };
        let mut edits = Vec::new();
        for s in &self.sels {
            let t = self.slice(s.start(), s.end());
            let (ol, cl) = (open.chars().count(), close.chars().count());
            if t.starts_with(open) && t.ends_with(close) && t.chars().count() >= ol + cl {
                edits.push(TextEdit::delete(s.end() - cl, s.end()));
                edits.push(TextEdit::delete(s.start(), s.start() + ol));
            } else {
                edits.push(TextEdit::insert(s.start(), format!("{open} ")));
                edits.push(TextEdit::insert(s.end(), format!(" {close}")));
            }
        }
        self.exec_keep(edits, EditKind::Other);
    }

    pub fn convert_eol(&mut self, eol: Eol) {
        let old = self.eol;
        self.eol = eol;
        if old == eol && Eol::detect(&self.text_string()) == Some(eol) {
            return;
        }
        let text = self.text_string();
        let converted = util::split_lines(&text).join(eol.as_str());
        if converted != *text {
            let caret = self.caret().min(converted.chars().count());
            let n = self.len_chars();
            self.commit(vec![TextEdit::replace(0, n, converted)], vec![Sel::caret(caret)], EditKind::Other);
        } else {
            self.mark_modified();
        }
    }

    pub fn column_insert(&mut self, text: &str, numbers: Option<&ColumnNumbers>) {
        let mut edits = Vec::new();
        let mut counter = 0usize;
        for s in self.sels.clone() {
            let ins = match numbers {
                Some(n) => {
                    let idx = (counter / n.repeat.max(1)) as i64;
                    let v = n.start + idx * n.step;
                    counter += 1;
                    let mut digits = fmt_radix(v, n.radix, false);
                    if n.zero_pad && digits.chars().count() < n.width {
                        let neg = digits.starts_with('-');
                        let body = digits.trim_start_matches('-').to_string();
                        let pad = n.width - digits.chars().count();
                        digits = format!("{}{}{}", if neg { "-" } else { "" }, "0".repeat(pad), body);
                    }
                    digits
                }
                None => text.to_string(),
            };
            edits.push(TextEdit::replace(s.start(), s.end(), ins));
        }
        self.exec(edits, EditKind::Other);
    }

    /// Expand every caret/selection to full lines, N++ "select line".
    pub fn select_lines(&mut self) {
        let mut sels = Vec::new();
        for (a, b) in self.selected_line_blocks() {
            let end = if b + 1 < self.len_lines() { self.line_start(b + 1) } else { self.line_end(b) };
            sels.push(Sel::new(self.line_start(a), end));
        }
        self.sels = sels;
        self.primary = self.sels.len() - 1;
        self.normalize();
    }

    pub fn find_matching_bracket(&self, pos: usize) -> Option<(usize, usize)> {
        // check char at pos, then char before pos
        for p in [Some(pos), pos.checked_sub(1)].into_iter().flatten() {
            let Some(c) = self.char_at(p) else { continue };
            let Some((partner, forward)) = util::matching_bracket(c) else { continue };
            if c == '<' || c == '>' {
                continue;
            }
            let mut depth = 0i32;
            if forward {
                let n = self.len_chars();
                let mut i = p;
                while i < n {
                    let ch = self.rope.char(i);
                    if ch == c {
                        depth += 1;
                    } else if ch == partner {
                        depth -= 1;
                        if depth == 0 {
                            return Some((p, i));
                        }
                    }
                    i += 1;
                    if i - p > 400_000 {
                        break;
                    }
                }
            } else {
                let mut i = p as isize;
                while i >= 0 {
                    let ch = self.rope.char(i as usize);
                    if ch == c {
                        depth += 1;
                    } else if ch == partner {
                        depth -= 1;
                        if depth == 0 {
                            return Some((p, i as usize));
                        }
                    }
                    i -= 1;
                    if p as isize - i > 400_000 {
                        break;
                    }
                }
            }
        }
        None
    }

    pub fn remove_bookmarked_lines(&mut self) {
        let eol_less = self.len_lines();
        let marks: Vec<usize> = self.bookmarks.iter().copied().collect();
        let mut edits = Vec::new();
        for l in marks {
            if l + 1 < eol_less {
                edits.push(TextEdit::delete(self.line_start(l), self.line_start(l + 1)));
            } else if l > 0 {
                edits.push(TextEdit::delete(self.line_end(l - 1), self.line_end(l)));
            } else {
                edits.push(TextEdit::delete(0, self.line_end(l)));
            }
        }
        self.exec_keep(edits, EditKind::Other);
        self.bookmarks.clear();
    }

    pub fn bookmarked_text(&self) -> String {
        let eol = self.eol.as_str();
        self.bookmarks.iter().map(|&l| format!("{}{eol}", self.line_text(l))).collect()
    }

    pub fn inverse_bookmarks(&mut self) {
        let all: std::collections::BTreeSet<usize> = (0..self.len_lines()).collect();
        self.bookmarks = all.difference(&self.bookmarks).copied().collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::Enc;

    fn doc(s: &str) -> Doc {
        let mut d = Doc::from_text(s, Enc::Utf8, Eol::Lf);
        d.opts.use_spaces = true;
        d
    }

    #[test]
    fn delete_and_duplicate_lines() {
        let mut d = doc("a\nb\nc");
        d.set_caret(2);
        d.delete_lines();
        assert_eq!(d.rope.to_string(), "a\nc");
        d.set_caret(0);
        d.duplicate();
        assert_eq!(d.rope.to_string(), "a\na\nc");
        assert_eq!(d.caret(), 2);
        let mut d = doc("a\nb");
        d.set_caret(2);
        d.delete_lines();
        assert_eq!(d.rope.to_string(), "a");
    }

    #[test]
    fn duplicate_selection() {
        let mut d = doc("abc def");
        d.set_selection(0, 3);
        d.duplicate();
        assert_eq!(d.rope.to_string(), "abcabc def");
        assert_eq!((d.sels[0].start(), d.sels[0].end()), (3, 6));
    }

    #[test]
    fn move_lines_up_down() {
        let mut d = doc("a\nb\nc");
        d.set_caret(2);
        d.move_lines(true);
        assert_eq!(d.rope.to_string(), "b\na\nc");
        assert_eq!(d.caret(), 0);
        d.move_lines(false);
        assert_eq!(d.rope.to_string(), "a\nb\nc");
        d.set_caret(4);
        d.move_lines(false);
        assert_eq!(d.rope.to_string(), "a\nb\nc");
        d.move_lines(true);
        assert_eq!(d.rope.to_string(), "a\nc\nb");
    }

    #[test]
    fn sorting() {
        let mut d = doc("b\nA\nc");
        d.select_all();
        d.sort_lines(SortMode::Lexicographic, false);
        assert_eq!(d.rope.to_string(), "A\nb\nc");
        d.sort_lines(SortMode::LexicographicCi, true);
        assert_eq!(d.rope.to_string(), "c\nb\nA");
        let mut d = doc("10\n9\n100");
        d.select_all();
        d.sort_lines(SortMode::Integer, false);
        assert_eq!(d.rope.to_string(), "9\n10\n100");
    }

    #[test]
    fn dedupe_and_blank_removal() {
        let mut d = doc("a\na\n\nb\na");
        d.remove_duplicate_lines(true);
        assert_eq!(d.rope.to_string(), "a\n\nb\na");
        d.remove_empty_lines(true);
        assert_eq!(d.rope.to_string(), "a\nb\na");
        d.remove_duplicate_lines(false);
        assert_eq!(d.rope.to_string(), "a\nb");
    }

    #[test]
    fn case_conversions() {
        let mut d = doc("hello wORLD");
        d.select_all();
        d.change_case(CaseMode::Upper);
        assert_eq!(d.rope.to_string(), "HELLO WORLD");
        d.change_case(CaseMode::Title);
        assert_eq!(d.rope.to_string(), "Hello World");
        d.change_case(CaseMode::Invert);
        assert_eq!(d.rope.to_string(), "hELLO wORLD");
        assert_eq!(d.sels[0].end(), 11);
    }

    #[test]
    fn indent_unindent() {
        let mut d = doc("a\n\nb");
        d.select_all();
        d.indent_lines(true);
        assert_eq!(d.rope.to_string(), "    a\n\n    b");
        d.indent_lines(false);
        assert_eq!(d.rope.to_string(), "a\n\nb");
    }

    #[test]
    fn comments() {
        let mut d = doc("    foo\n    bar");
        d.set_lang(lang::find_by_name("Rust").unwrap());
        d.select_all();
        d.toggle_line_comment();
        assert_eq!(d.rope.to_string(), "    // foo\n    // bar");
        d.toggle_line_comment();
        assert_eq!(d.rope.to_string(), "    foo\n    bar");
    }

    #[test]
    fn trim_and_tabs() {
        let mut d = doc("a  \n\tb \n");
        d.trim_whitespace(TrimMode::Trailing);
        assert_eq!(d.rope.to_string(), "a\n\tb\n");
        d.tabs_to_spaces();
        assert_eq!(d.rope.to_string(), "a\n    b\n");
        d.spaces_to_tabs(false);
        assert_eq!(d.rope.to_string(), "a\n\tb\n");
    }

    #[test]
    fn eol_conversion() {
        let mut d = doc("a\nb\nc");
        d.convert_eol(Eol::CrLf);
        assert_eq!(d.rope.to_string(), "a\r\nb\r\nc");
        assert_eq!(d.len_lines(), 3);
        d.convert_eol(Eol::Cr);
        assert_eq!(d.rope.to_string(), "a\rb\rc");
        d.undo();
        assert_eq!(d.rope.to_string(), "a\r\nb\r\nc");
    }

    #[test]
    fn column_numbers() {
        let mut d = doc("a\nb\nc");
        d.sels = vec![Sel::caret(0), Sel::caret(2), Sel::caret(4)];
        d.column_insert("", Some(&ColumnNumbers { start: 8, step: 1, repeat: 1, width: 2, radix: 10, zero_pad: true }));
        assert_eq!(d.rope.to_string(), "08a\n09b\n10c");
    }

    #[test]
    fn matching_bracket() {
        let d = doc("f(a[1], {x})");
        assert_eq!(d.find_matching_bracket(1), Some((1, 11)));
        assert_eq!(d.find_matching_bracket(12), Some((11, 1)));
        assert_eq!(d.find_matching_bracket(0), None);
    }

    #[test]
    fn join() {
        let mut d = doc("a\n  b\nc");
        d.select_all();
        d.join_lines();
        assert_eq!(d.rope.to_string(), "a b c");
    }

    #[test]
    fn insert_blank() {
        let mut d = doc("  foo");
        d.set_caret(3);
        d.insert_blank_line(false);
        assert_eq!(d.rope.to_string(), "  foo\n  ");
    }
}
