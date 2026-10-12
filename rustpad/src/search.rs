//! Find / replace engine: normal, extended and regex modes, in documents and files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::doc::{Doc, Eol, TextEdit};
use crate::encoding;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Mode {
    Normal,
    Extended,
    Regex,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Query {
    pub pattern: String,
    pub replace: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub mode: Mode,
    pub dot_newline: bool,
    pub wrap: bool,
    pub backward: bool,
    pub in_selection: bool,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            pattern: String::new(),
            replace: String::new(),
            match_case: false,
            whole_word: false,
            mode: Mode::Normal,
            dot_newline: false,
            wrap: true,
            backward: false,
            in_selection: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

/// Interpret `\n \r \t \0 \\ \xHH \x{H..} \uHHHH` escapes (Extended mode).
pub fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('x') => {
                let mut hex = String::new();
                if it.peek() == Some(&'{') {
                    it.next();
                    while let Some(&d) = it.peek() {
                        it.next();
                        if d == '}' {
                            break;
                        }
                        hex.push(d);
                    }
                } else {
                    for _ in 0..2 {
                        if let Some(&d) = it.peek() {
                            if d.is_ascii_hexdigit() {
                                hex.push(d);
                                it.next();
                            }
                        }
                    }
                }
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => {
                        out.push_str("\\x");
                        out.push_str(&hex);
                    }
                }
            }
            Some('u') => {
                let mut hex = String::new();
                for _ in 0..4 {
                    if let Some(&d) = it.peek() {
                        if d.is_ascii_hexdigit() {
                            hex.push(d);
                            it.next();
                        }
                    }
                }
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => {
                        out.push_str("\\u");
                        out.push_str(&hex);
                    }
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

pub fn build_regex(q: &Query) -> Result<Regex, String> {
    if q.pattern.is_empty() {
        return Err("empty pattern".into());
    }
    let pat = match q.mode {
        Mode::Normal => regex::escape(&q.pattern),
        Mode::Extended => regex::escape(&unescape(&q.pattern)),
        Mode::Regex => q.pattern.clone(),
    };
    let pat = if q.mode == Mode::Regex { format!("(?mR){pat}") } else { pat };
    RegexBuilder::new(&pat)
        .case_insensitive(!q.match_case)
        .dot_matches_new_line(q.dot_newline && q.mode == Mode::Regex)
        .size_limit(50 << 20)
        .build()
        .map_err(|e| {
            let msg = e.to_string();
            msg.lines().last().unwrap_or("invalid regular expression").trim().to_string()
        })
}

/// Turn the user's replacement text into a `regex` crate template.
pub fn replacement_template(q: &Query) -> String {
    let src = match q.mode {
        Mode::Normal => return q.replace.replace('$', "$$"),
        _ => &q.replace,
    };
    let mut out = String::new();
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some(d) if d.is_ascii_digit() => {
                    out.push_str("${");
                    out.push(d);
                    out.push('}');
                }
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some('x') | Some('u') if q.mode == Mode::Extended => {
                    // reuse the extended unescaper for this one sequence
                    let mut seq = String::from("\\");
                    seq.push(if src.contains("\\x") { 'x' } else { 'u' });
                    while let Some(&h) = it.peek() {
                        if h.is_ascii_hexdigit() || h == '{' || h == '}' {
                            seq.push(h);
                            it.next();
                            if h == '}' {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    out.push_str(&unescape(&seq).replace('$', "$$"));
                }
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            },
            '$' => match it.peek() {
                Some(d) if d.is_ascii_digit() || *d == '{' || *d == '$' => {
                    out.push('$');
                    if *d == '$' {
                        out.push('$');
                        it.next();
                    }
                }
                _ => out.push_str("$$"),
            },
            c => out.push(c),
        }
    }
    out
}

fn is_word_byte_boundary(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    let w = crate::util::is_word_char;
    // for a match that itself begins/ends with a non-word char, N++ accepts it as-is
    let first = text[start..end].chars().next();
    let last = text[start..end].chars().next_back();
    let left_ok = match (before, first) {
        (Some(b), Some(f)) if w(f) => !w(b),
        _ => true,
    };
    let right_ok = match (after, last) {
        (Some(a), Some(l)) if w(l) => !w(a),
        _ => true,
    };
    left_ok && right_ok
}

/// First match at or after byte offset `from` honouring whole-word.
fn find_from(re: &Regex, text: &str, mut from: usize, whole_word: bool, limit: usize) -> Option<(usize, usize)> {
    while from <= text.len() {
        let m = re.find_at(text, from)?;
        if m.end() > limit {
            return None;
        }
        if !whole_word || is_word_byte_boundary(text, m.start(), m.end()) {
            return Some((m.start(), m.end()));
        }
        let step = text[m.start()..].chars().next().map_or(1, |c| c.len_utf8());
        from = m.start() + step;
    }
    None
}

pub struct FoundMatch {
    pub m: Match,
    pub wrapped: bool,
}

/// Find the next / previous match relative to `from` (a char position).
/// Forward searches start at `from`; backward searches end at `from`.
pub fn find(doc: &Doc, q: &Query, from: usize, backward: bool, range: Option<(usize, usize)>) -> Result<Option<FoundMatch>, String> {
    let re = build_regex(q)?;
    let text = doc.text_string();
    let to_b = |c: usize| doc.rope.char_to_byte(c.min(doc.len_chars()));
    let to_c = |b: usize| doc.rope.byte_to_char(b);
    let (lo, hi) = match range {
        Some((a, b)) => (to_b(a), to_b(b)),
        None => (0, text.len()),
    };
    let from_b = to_b(from).clamp(lo, hi);
    let mk = |s: usize, e: usize, wrapped: bool| Some(FoundMatch { m: Match { start: to_c(s), end: to_c(e) }, wrapped });
    if !backward {
        if let Some((s, e)) = find_from(&re, &text, from_b, q.whole_word, hi) {
            return Ok(mk(s, e, false));
        }
        if q.wrap || range.is_some() && q.wrap {
            if let Some((s, e)) = find_from(&re, &text, lo, q.whole_word, hi) {
                return Ok(mk(s, e, true));
            }
        }
        Ok(None)
    } else {
        let last_before = |limit: usize, from_lo: usize| -> Option<(usize, usize)> {
            let mut best = None;
            let mut pos = from_lo;
            while let Some((s, e)) = find_from(&re, &text, pos, q.whole_word, limit) {
                best = Some((s, e));
                pos = s + text[s..].chars().next().map_or(1, |c| c.len_utf8());
                if pos > limit {
                    break;
                }
            }
            best
        };
        if let Some((s, e)) = last_before(from_b, lo) {
            return Ok(mk(s, e, false));
        }
        if q.wrap {
            if let Some((s, e)) = last_before(hi, lo) {
                return Ok(mk(s, e, true));
            }
        }
        Ok(None)
    }
}

pub fn find_all(doc: &Doc, q: &Query, range: Option<(usize, usize)>) -> Result<Vec<Match>, String> {
    let re = build_regex(q)?;
    let text = doc.text_string();
    let (lo, hi) = match range {
        Some((a, b)) => (doc.rope.char_to_byte(a), doc.rope.char_to_byte(b)),
        None => (0, text.len()),
    };
    let mut out = Vec::new();
    let mut pos = lo;
    while pos <= hi {
        match find_from(&re, &text, pos, q.whole_word, hi) {
            Some((s, e)) => {
                out.push(Match { start: doc.rope.byte_to_char(s), end: doc.rope.byte_to_char(e) });
                pos = if e > s { e } else { e + text[e..].chars().next().map_or(1, |c| c.len_utf8()) };
            }
            None => break,
        }
    }
    Ok(out)
}

fn normalize_eol(s: &str, eol: Eol) -> String {
    crate::util::split_lines(s).join(eol.as_str())
}

/// Expand the replacement for a match located at char range `m` (must be a real match).
pub fn expand_at(doc: &Doc, q: &Query, m: Match) -> Result<Option<String>, String> {
    let re = build_regex(q)?;
    let text = doc.text_string();
    let sb = doc.rope.char_to_byte(m.start);
    let eb = doc.rope.char_to_byte(m.end);
    if let Some(caps) = re.captures_at(&text, sb) {
        let whole = caps.get(0).unwrap();
        if whole.start() == sb && whole.end() == eb {
            let mut dst = String::new();
            caps.expand(&replacement_template(q), &mut dst);
            return Ok(Some(normalize_eol(&dst, doc.eol)));
        }
    }
    Ok(None)
}

/// Build the edits for "replace all" over `range` (whole document if None).
pub fn replace_all_edits(doc: &Doc, q: &Query, range: Option<(usize, usize)>) -> Result<Vec<TextEdit>, String> {
    let re = build_regex(q)?;
    let text = doc.text_string();
    let (lo, hi) = match range {
        Some((a, b)) => (doc.rope.char_to_byte(a), doc.rope.char_to_byte(b)),
        None => (0, text.len()),
    };
    let tpl = replacement_template(q);
    let mut edits = Vec::new();
    let mut pos = lo;
    while pos <= hi {
        let Some(caps) = re.captures_at(&text, pos) else { break };
        let w = caps.get(0).unwrap();
        if w.end() > hi {
            break;
        }
        if q.whole_word && !is_word_byte_boundary(&text, w.start(), w.end()) {
            pos = w.start() + text[w.start()..].chars().next().map_or(1, |c| c.len_utf8());
            continue;
        }
        let mut dst = String::new();
        caps.expand(&tpl, &mut dst);
        edits.push(TextEdit::replace(
            doc.rope.byte_to_char(w.start()),
            doc.rope.byte_to_char(w.end()),
            normalize_eol(&dst, doc.eol),
        ));
        pos = if w.end() > w.start() { w.end() } else { w.end() + text[w.end()..].chars().next().map_or(1, |c| c.len_utf8()) };
    }
    Ok(edits)
}

// ---------------------------------------------------------------- files

#[derive(Clone, Debug)]
pub struct FifQuery {
    pub dir: PathBuf,
    pub filters: String,
    pub recursive: bool,
    pub hidden: bool,
}

pub struct FileFilter {
    include: Option<globset::GlobSet>,
    exclude: globset::GlobSet,
    exclude_dirs: Vec<String>,
}

impl FileFilter {
    pub fn parse(spec: &str) -> FileFilter {
        let mut inc = globset::GlobSetBuilder::new();
        let mut exc = globset::GlobSetBuilder::new();
        let mut any_inc = false;
        let mut exclude_dirs = Vec::new();
        for tok in spec.split(|c: char| c == ';' || c.is_whitespace()).filter(|t| !t.is_empty()) {
            if let Some(ex) = tok.strip_prefix('!') {
                if let Some(d) = ex.strip_prefix('\\').or_else(|| ex.strip_prefix('+')) {
                    exclude_dirs.push(d.trim_matches(['\\', '/']).to_lowercase());
                } else if let Ok(g) = globset::GlobBuilder::new(ex).case_insensitive(true).build() {
                    exc.add(g);
                }
            } else if tok != "*.*" && tok != "*" {
                if let Ok(g) = globset::GlobBuilder::new(tok).case_insensitive(true).build() {
                    inc.add(g);
                    any_inc = true;
                }
            }
        }
        FileFilter {
            include: if any_inc { inc.build().ok() } else { None },
            exclude: exc.build().unwrap_or_else(|_| globset::GlobSet::empty()),
            exclude_dirs,
        }
    }
    pub fn accepts(&self, name: &str) -> bool {
        self.include.as_ref().is_none_or(|g| g.is_match(name)) && !self.exclude.is_match(name)
    }
    pub fn dir_excluded(&self, name: &str) -> bool {
        let n = name.to_lowercase();
        self.exclude_dirs.iter().any(|d| *d == n)
    }
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub line: usize,
    pub text: String,
    /// Match ranges as char columns within `text`.
    pub ranges: Vec<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub struct FileHits {
    pub path: PathBuf,
    pub hits: Vec<Hit>,
}

fn list_files(f: &FifQuery, cancel: &AtomicBool) -> Vec<PathBuf> {
    let filter = FileFilter::parse(&f.filters);
    let mut out = Vec::new();
    let mut walker = walkdir::WalkDir::new(&f.dir).follow_links(false);
    if !f.recursive {
        walker = walker.max_depth(1);
    }
    let it = walker.into_iter().filter_entry(|e| {
        if e.depth() == 0 {
            return true;
        }
        let name = e.file_name().to_string_lossy();
        if !f.hidden && name.starts_with('.') {
            return false;
        }
        if e.file_type().is_dir() && filter.dir_excluded(&name) {
            return false;
        }
        true
    });
    for e in it.flatten() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if e.file_type().is_file() && filter.accepts(&e.file_name().to_string_lossy()) {
            out.push(e.into_path());
        }
    }
    out
}

pub fn read_text_file(path: &Path) -> Option<(String, encoding::Enc)> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 200 << 20 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let head = &bytes[..bytes.len().min(8192)];
    let utf16_bom = head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]);
    if !utf16_bom && head.contains(&0) {
        let d = encoding::decode_auto(&bytes);
        if !matches!(d.enc, encoding::Enc::Utf16Le | encoding::Enc::Utf16Be) {
            return None;
        }
        return Some((d.text, d.enc));
    }
    let d = encoding::decode_auto(&bytes);
    Some((d.text, d.enc))
}

pub fn search_text(re: &Regex, text: &str, whole_word: bool) -> Vec<Hit> {
    // byte offsets of line starts (split on \n; \r stripped from display text)
    let mut starts = vec![0usize];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    let mut hits: Vec<Hit> = Vec::new();
    let mut pos = 0;
    while pos <= text.len() {
        let Some((s, e)) = find_from(re, text, pos, whole_word, text.len()) else { break };
        let line = match starts.binary_search(&s) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let ls = starts[line];
        let le = text[ls..].find('\n').map_or(text.len(), |p| ls + p);
        let line_text = text[ls..le].trim_end_matches('\r');
        let cs = text[ls..s].chars().count();
        let ce = cs + text[s..e.min(ls + line_text.len())].chars().count();
        match hits.last_mut() {
            Some(h) if h.line == line => h.ranges.push((cs, ce)),
            _ => {
                let shown: String = line_text.chars().take(400).collect();
                hits.push(Hit { line, text: shown, ranges: vec![(cs, ce)] });
            }
        }
        pos = if e > s { e } else { e + text[e..].chars().next().map_or(1, |c| c.len_utf8()) };
    }
    hits
}

/// Search files, calling `on_file` for each file with hits. Returns number of files scanned.
pub fn find_in_files(q: &Query, f: &FifQuery, cancel: &AtomicBool, mut on_file: impl FnMut(FileHits), mut on_progress: impl FnMut(usize)) -> Result<usize, String> {
    let re = build_regex(q)?;
    let files = list_files(f, cancel);
    let mut scanned = 0;
    for path in files {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        scanned += 1;
        if scanned % 25 == 0 {
            on_progress(scanned);
        }
        let Some((text, _)) = read_text_file(&path) else { continue };
        let hits = search_text(&re, &text, q.whole_word);
        if !hits.is_empty() {
            on_file(FileHits { path, hits });
        }
    }
    Ok(scanned)
}

/// Replace in all matching files on disk. Returns (files changed, replacements).
pub fn replace_in_files(q: &Query, f: &FifQuery, cancel: &AtomicBool) -> Result<(usize, usize), String> {
    let re = build_regex(q)?;
    let tpl = replacement_template(q);
    let files = list_files(f, cancel);
    let (mut changed, mut total) = (0, 0);
    for path in files {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Some((text, enc)) = read_text_file(&path) else { continue };
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let mut count = 0;
        let mut pos = 0;
        while pos <= text.len() {
            let Some(caps) = re.captures_at(&text, pos) else { break };
            let w = caps.get(0).unwrap();
            if q.whole_word && !is_word_byte_boundary(&text, w.start(), w.end()) {
                pos = w.start() + text[w.start()..].chars().next().map_or(1, |c| c.len_utf8());
                continue;
            }
            out.push_str(&text[last..w.start()]);
            caps.expand(&tpl, &mut out);
            last = w.end();
            count += 1;
            pos = if w.end() > w.start() { w.end() } else { w.end() + text[w.end()..].chars().next().map_or(1, |c| c.len_utf8()) };
        }
        if count == 0 {
            continue;
        }
        out.push_str(&text[last..]);
        std::fs::write(&path, encoding::encode(&out, enc)).map_err(|e| format!("{}: {e}", path.display()))?;
        changed += 1;
        total += count;
    }
    Ok((changed, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::Enc;

    fn doc(s: &str) -> Doc {
        Doc::from_text(s, Enc::Utf8, Eol::Lf)
    }
    fn q(p: &str) -> Query {
        Query { pattern: p.into(), ..Default::default() }
    }

    #[test]
    fn forward_backward_wrap() {
        let d = doc("foo bar foo baz foo");
        let r = find(&d, &q("foo"), 0, false, None).unwrap().unwrap();
        assert_eq!((r.m.start, r.m.end, r.wrapped), (0, 3, false));
        let r = find(&d, &q("foo"), 3, false, None).unwrap().unwrap();
        assert_eq!(r.m.start, 8);
        let r = find(&d, &q("foo"), 15, false, None).unwrap().unwrap();
        assert_eq!((r.m.start, r.wrapped), (16, false));
        let r = find(&d, &q("foo"), 19, false, None).unwrap().unwrap();
        assert!(r.wrapped && r.m.start == 0);
        let r = find(&d, &q("foo"), 8, true, None).unwrap().unwrap();
        assert_eq!(r.m.start, 0);
        let r = find(&d, &q("foo"), 0, true, None).unwrap().unwrap();
        assert!(r.wrapped && r.m.start == 16);
    }

    #[test]
    fn whole_word_and_case() {
        let d = doc("Foo food foo");
        let mut qq = q("foo");
        qq.whole_word = true;
        assert_eq!(find_all(&d, &qq, None).unwrap().len(), 2);
        qq.match_case = true;
        assert_eq!(find_all(&d, &qq, None).unwrap().len(), 1);
    }

    #[test]
    fn unicode_positions_are_chars() {
        let d = doc("héllo wörld héllo");
        let all = find_all(&d, &q("héllo"), None).unwrap();
        assert_eq!(all, vec![Match { start: 0, end: 5 }, Match { start: 12, end: 17 }]);
    }

    #[test]
    fn extended_mode() {
        let d = doc("a\tb\nc");
        let mut qq = q("\\t");
        qq.mode = Mode::Extended;
        assert_eq!(find_all(&d, &qq, None).unwrap().len(), 1);
        qq.pattern = "b\\nc".into();
        assert_eq!(find_all(&d, &qq, None).unwrap().len(), 1);
    }

    #[test]
    fn regex_replace_groups() {
        let mut d = doc("john smith\njane doe");
        let mut qq = q(r"(\w+) (\w+)");
        qq.mode = Mode::Regex;
        qq.replace = r"\2, $1".into();
        let edits = replace_all_edits(&d, &qq, None).unwrap();
        d.exec(edits, crate::doc::EditKind::Other);
        assert_eq!(d.rope.to_string(), "smith, john\ndoe, jane");
    }

    #[test]
    fn literal_replace_with_dollar() {
        let mut d = doc("price");
        let mut qq = q("price");
        qq.replace = "$5".into();
        let edits = replace_all_edits(&d, &qq, None).unwrap();
        d.exec(edits, crate::doc::EditKind::Other);
        assert_eq!(d.rope.to_string(), "$5");
    }

    #[test]
    fn regex_anchors_work_per_line_with_crlf() {
        let d = Doc::from_text("ab\r\ncd\r\n", Enc::Utf8, Eol::CrLf);
        let mut qq = q("^.");
        qq.mode = Mode::Regex;
        assert_eq!(find_all(&d, &qq, None).unwrap().len(), 2);
        qq.pattern = ".$".into();
        let all = find_all(&d, &qq, None).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn zero_length_matches_terminate() {
        let d = doc("abc");
        let mut qq = q("x*");
        qq.mode = Mode::Regex;
        let all = find_all(&d, &qq, None).unwrap();
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn bad_regex_reports_error() {
        let d = doc("abc");
        let mut qq = q("(");
        qq.mode = Mode::Regex;
        assert!(find(&d, &qq, 0, false, None).is_err());
    }

    #[test]
    fn in_selection_range() {
        let d = doc("foo foo foo foo");
        let all = find_all(&d, &q("foo"), Some((4, 11))).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn filters() {
        let f = FileFilter::parse("*.rs *.toml !*.lock !\\target");
        assert!(f.accepts("main.rs") && f.accepts("Cargo.toml") && !f.accepts("Cargo.lock") && !f.accepts("x.md"));
        assert!(f.dir_excluded("Target"));
        assert!(FileFilter::parse("*.*").accepts("anything.xyz"));
    }

    #[test]
    fn search_text_lines() {
        let re = build_regex(&q("foo")).unwrap();
        let hits = search_text(&re, "a foo\r\nb\nfoo foo", false);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, 0);
        assert_eq!(hits[0].text, "a foo");
        assert_eq!(hits[0].ranges, vec![(2, 5)]);
        assert_eq!(hits[1].ranges.len(), 2);
    }
}
