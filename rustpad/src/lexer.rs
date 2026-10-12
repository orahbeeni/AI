//! Line-oriented syntax lexer. State carried between lines is tiny, so the
//! document can re-lex only what is visible (plus whatever precedes it).

use crate::lang::{flag::*, Compiled, Kind, Style};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LexState {
    #[default]
    Normal,
    /// Inside a block comment, with nesting depth.
    Block(u8),
    /// Inside a string that continues across lines.
    Str { q: u8, triple: bool },
    MarkupComment,
    /// Inside a markup tag; the byte is an open attribute quote (0 = none).
    InTag(u8),
    /// Inside a Markdown code fence.
    Fence,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

fn push(out: &mut Vec<Span>, start: usize, end: usize, style: Style) {
    if start >= end {
        return;
    }
    if let Some(last) = out.last_mut() {
        if last.style == style && last.end == start && style != Style::Function && style != Style::Tag {
            last.end = end;
            return;
        }
    }
    out.push(Span { start, end, style });
}

pub fn lex_line(lang: &Compiled, line: &str, st: LexState, out: &mut Vec<Span>) -> LexState {
    out.clear();
    match lang.def.kind {
        Kind::Plain => LexState::Normal,
        Kind::Generic => lex_generic(lang, line, st, out, 0, false),
        Kind::Markup => lex_markup(line, st, out),
        Kind::Markdown => lex_markdown(line, st, out),
        Kind::Diff => lex_diff(line, out),
        Kind::KeyValue => lex_keyvalue(lang, line, out),
        Kind::Impex => lex_impex(lang, line, st, out),
    }
}

fn starts_at(line: &str, i: usize, pat: &str, ci: bool) -> bool {
    let b = line.as_bytes();
    let p = pat.as_bytes();
    if i + p.len() > b.len() {
        return false;
    }
    if ci {
        b[i..i + p.len()].eq_ignore_ascii_case(p)
    } else {
        &b[i..i + p.len()] == p
    }
}

fn is_ident_char(c: char, lang: &Compiled) -> bool {
    c.is_alphanumeric() || c == '_' || lang.def.ident_extra.contains(c)
}

fn is_ident_start(c: char, lang: &Compiled) -> bool {
    c.is_alphabetic()
        || c == '_'
        || (lang.def.ident_extra.contains(c) && !"-!?*<>=+/:'".contains(c) && !c.is_ascii_digit())
}

/// Scan the remainder of a string whose opening delimiter has been consumed.
/// Returns the index just past the closing delimiter, or None if the line ends first.
fn scan_string(lang: &Compiled, b: &[u8], mut i: usize, q: u8, triple: bool) -> Option<usize> {
    let dq = lang.has(DOUBLE_QUOTE_ESCAPE);
    while i < b.len() {
        let c = b[i];
        if c == b'\\' && !dq {
            i += 2;
            continue;
        }
        if c == q {
            if triple {
                if i + 2 < b.len() + 0 && b[i + 1] == q && b[i + 2] == q {
                    return Some(i + 3);
                }
            } else if dq && i + 1 < b.len() && b[i + 1] == q {
                i += 2;
                continue;
            } else {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

fn lex_generic(
    lang: &Compiled,
    line: &str,
    st: LexState,
    out: &mut Vec<Span>,
    from: usize,
    comment_needs_ws: bool,
) -> LexState {
    let b = line.as_bytes();
    let mut i = from;
    let def = lang.def;
    let ci = lang.case_insensitive;
    let nested = lang.has(NESTED_COMMENTS);

    // Resume multi-line constructs.
    match st {
        LexState::Block(depth) => {
            let (ni, nd) = scan_block(line, i, depth, def.block_open, def.block_close, nested);
            push(out, i, ni, Style::Comment);
            i = ni;
            if nd != 0 {
                return LexState::Block(nd);
            }
        }
        LexState::Str { q, triple } => {
            match scan_string(lang, b, i, q, triple) {
                Some(e) => {
                    push(out, i, e, Style::String);
                    i = e;
                }
                None => {
                    push(out, i, b.len(), Style::String);
                    return st;
                }
            }
        }
        _ => {}
    }

    let ws_before = |i: usize| i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t';
    let brace_items = lang.has(BRACE_ITEM);
    let mut depth: usize = 0;

    while i < b.len() {
        let c = b[i];
        if c == b' ' || c == b'\t' {
            i += 1;
            continue;
        }

        // Block comments (checked before line comments: Lua's --[[ vs --).
        if !def.block_open.is_empty() && starts_at(line, i, def.block_open, false) {
            let start = i;
            let (ni, nd) =
                scan_block(line, i + def.block_open.len(), 1, def.block_open, def.block_close, nested);
            let doc = def.block_open == "/*" && b.get(i + 2) == Some(&b'*') && b.get(i + 3) != Some(&b'/');
            push(out, start, ni, if doc { Style::DocComment } else { Style::Comment });
            i = ni;
            if nd != 0 {
                return LexState::Block(nd);
            }
            continue;
        }

        // Line comments.
        let mut matched_comment = false;
        if !(comment_needs_ws && !ws_before(i)) {
            for lc in &lang.line_comments {
                if starts_at(line, i, lc, ci) {
                    let alpha = lc.as_bytes()[0].is_ascii_alphabetic();
                    if alpha {
                        let before_ok = i == 0 || !b[i - 1].is_ascii_alphanumeric();
                        let after = b.get(i + lc.len());
                        let after_ok = after.is_none_or(|a| !a.is_ascii_alphanumeric());
                        if !(before_ok && after_ok) {
                            continue;
                        }
                    }
                    let doc = *lc == "//" && matches!(b.get(i + 2), Some(b'/') | Some(b'!')) && b.get(i + 3) != Some(&b'/');
                    push(out, i, b.len(), if doc { Style::DocComment } else { Style::Comment });
                    matched_comment = true;
                    break;
                }
            }
        }
        if matched_comment {
            return LexState::Normal;
        }

        // Preprocessor directives / attributes.
        if c == b'#' {
            if lang.has(PREPROC) && line[..i].trim().is_empty() {
                let mut end = b.len();
                if let Some(p) = line[i..].find("//") {
                    end = i + p;
                }
                if let Some(p) = line[i..end].find("/*") {
                    end = i + p;
                }
                push(out, i, end, Style::Preproc);
                i = end;
                continue;
            }
            if def.name == "Rust" && (starts_at(line, i, "#[", false) || starts_at(line, i, "#![", false)) {
                let end = line[i..].find(']').map(|p| i + p + 1).unwrap_or(b.len());
                push(out, i, end, Style::Preproc);
                i = end;
                continue;
            }
        }

        // Strings.
        if c == b'"' || c == b'\'' || (c == b'`' && lang.has(BACKTICK)) {
            let triple = lang.has(TRIPLE_QUOTE) && c != b'`' && starts_at(line, i, if c == b'"' { "\"\"\"" } else { "'''" }, false);
            if c == b'\'' && !triple {
                if lang.has(CHAR_LITERAL) {
                    // 'x' or '\n' → char; otherwise a lifetime / label.
                    if b.get(i + 1) == Some(&b'\\') {
                        if let Some(p) = line[i + 2..].find('\'') {
                            if p <= 10 {
                                push(out, i, i + 2 + p + 1, Style::Char);
                                i += 2 + p + 1;
                                continue;
                            }
                        }
                    } else if let Some(ch) = line[i + 1..].chars().next() {
                        let n = ch.len_utf8();
                        if b.get(i + 1 + n) == Some(&b'\'') {
                            push(out, i, i + 2 + n, Style::Char);
                            i += 2 + n;
                            continue;
                        }
                    }
                    // lifetime
                    let mut j = i + 1;
                    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                        j += 1;
                    }
                    if j > i + 1 {
                        push(out, i, j, Style::Variable);
                        i = j;
                        continue;
                    }
                    push(out, i, i + 1, Style::Operator);
                    i += 1;
                    continue;
                }
                if lang.has(NO_SQUOTE) {
                    push(out, i, i + 1, Style::Operator);
                    i += 1;
                    continue;
                }
            }
            let start = i;
            let q = c;
            let open_len = if triple { 3 } else { 1 };
            match scan_string(lang, b, i + open_len, q, triple) {
                Some(e) => {
                    push(out, start, e, Style::String);
                    i = e;
                }
                None => {
                    let multi = triple || q == b'`' || lang.has(MULTILINE_STR);
                    if def.kind == Kind::KeyValue && q == b'\'' {
                        // apostrophe in prose, not a string
                        i += 1;
                        continue;
                    }
                    push(out, start, b.len(), Style::String);
                    if multi {
                        return LexState::Str { q, triple };
                    }
                    return LexState::Normal;
                }
            }
            continue;
        }

        // Numbers.
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            let hex = c == b'0' && matches!(b.get(i + 1), Some(b'x') | Some(b'X'));
            i += 1;
            while i < b.len() {
                let d = b[i];
                if d.is_ascii_alphanumeric() || d == b'_' {
                    if !hex && (d == b'e' || d == b'E') && matches!(b.get(i + 1), Some(b'+') | Some(b'-')) {
                        i += 1;
                    }
                    i += 1;
                } else if d == b'.' && b.get(i + 1) != Some(&b'.') && !b.get(i + 1).is_some_and(|x| x.is_ascii_alphabetic() && !hex && *x != b'e' && *x != b'E') {
                    i += 1;
                } else {
                    break;
                }
            }
            push(out, start, i, Style::Number);
            continue;
        }

        // `?param` query parameters.
        if c == b'?' && lang.has(QUESTION_VAR) && b.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic() || *n == b'_') {
            let start = i;
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            push(out, start, i, Style::Variable);
            continue;
        }

        // Variables with a sigil.
        if (c == b'$' && lang.has(DOLLAR_VAR)) || (c == b'@' && lang.has(AT_VAR)) || (c == b'%' && lang.has(PERCENT_VAR)) {
            let start = i;
            i += 1;
            if c == b'$' && b.get(i) == Some(&b'{') {
                let end = line[i..].find('}').map(|p| i + p + 1).unwrap_or(b.len());
                push(out, start, end, Style::Variable);
                i = end;
                continue;
            }
            if c == b'$' && b.get(i) == Some(&b'(') {
                push(out, start, i + 1, Style::Variable);
                i += 1;
                continue;
            }
            if let Some(&n) = b.get(i) {
                if n.is_ascii_digit() || (c == b'$' && b"@?$!#*-&".contains(&n)) || (n == b'~' && c == b'%') || (c == b'%' && n == b'*') {
                    i += 1;
                    while c == b'%' && i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'~') {
                        i += 1;
                    }
                    push(out, start, i, Style::Variable);
                    continue;
                }
            }
            let word_start = i;
            while i < b.len() {
                let ch = line[i..].chars().next().unwrap();
                if ch.is_alphanumeric() || ch == '_' || (ch == ':' && b.get(i + 1) == Some(&b':')) && false {
                    i += ch.len_utf8();
                } else {
                    break;
                }
            }
            if i == word_start {
                push(out, start, i, Style::Operator);
                continue;
            }
            if c == b'%' && b.get(i) == Some(&b'%') {
                i += 1; // batch %var%
            }
            let full = &line[start..i];
            let key = if ci { full.to_lowercase() } else { full.to_string() };
            let style = lang.words.get(&key).copied().unwrap_or(Style::Variable);
            push(out, start, i, style);
            continue;
        }

        // Identifiers / keywords.
        let ch = line[i..].chars().next().unwrap();
        if is_ident_start(ch, lang) || (lang.has(HYPHEN_IDENT) && ch == '-' && b.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic())) {
            let start = i;
            i += ch.len_utf8();
            while i < b.len() {
                let ch = line[i..].chars().next().unwrap();
                let hyphen_ok = lang.has(HYPHEN_IDENT) && ch == '-' && b.get(i + 1).is_some_and(|n| n.is_ascii_alphanumeric());
                if is_ident_char(ch, lang) || hyphen_ok {
                    i += ch.len_utf8();
                } else {
                    break;
                }
            }
            let word = &line[start..i];
            let style = if ci {
                lang.words.get(&word.to_lowercase()).copied()
            } else {
                lang.words.get(word).copied()
            };
            let style = match style {
                Some(s) => s,
                None if brace_items && depth > 0 => {
                    let before = line[..start].trim_end().as_bytes().last().copied();
                    let after = line[i..].trim_start().as_bytes().first().copied();
                    if before == Some(b':') {
                        Style::Attr
                    } else if after == Some(b':') {
                        Style::Variable
                    } else if before == Some(b'{') {
                        Style::Type
                    } else {
                        Style::Variable
                    }
                }
                None => {
                    // `name(` → function call / definition
                    let mut j = i;
                    while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                        j += 1;
                    }
                    if b.get(j) == Some(&b'(') && !word.starts_with('\\') {
                        Style::Function
                    } else if word.starts_with('\\') {
                        Style::Keyword
                    } else {
                        Style::Default
                    }
                }
            };
            push(out, start, i, style);
            continue;
        }

        // Operators / punctuation.
        if brace_items {
            if c == b'{' {
                depth += 1;
            } else if c == b'}' {
                depth = depth.saturating_sub(1);
            }
        }
        if c.is_ascii_punctuation() {
            push(out, i, i + 1, Style::Operator);
        }
        i += ch.len_utf8();
    }
    LexState::Normal
}

/// Find the end of a block comment. Returns (index after, remaining depth); depth 0 = closed.
fn scan_block(line: &str, mut i: usize, mut depth: u8, open: &str, close: &str, nested: bool) -> (usize, u8) {
    while i < line.len() {
        if starts_at(line, i, close, false) {
            i += close.len();
            depth -= 1;
            if depth == 0 {
                return (i, 0);
            }
        } else if nested && starts_at(line, i, open, false) {
            i += open.len();
            depth = depth.saturating_add(1);
        } else {
            i += line[i..].chars().next().map_or(1, |c| c.len_utf8());
        }
    }
    (line.len(), depth)
}

// ---------------------------------------------------------------- markup

fn tag_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.')
}

fn lex_markup(line: &str, mut st: LexState, out: &mut Vec<Span>) -> LexState {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match st {
            LexState::MarkupComment => {
                match line[i..].find("-->") {
                    Some(p) => {
                        push(out, i, i + p + 3, Style::Comment);
                        i += p + 3;
                        st = LexState::Normal;
                    }
                    None => {
                        push(out, i, b.len(), Style::Comment);
                        return st;
                    }
                }
            }
            LexState::InTag(q) => {
                if q != 0 {
                    match line[i..].find(q as char) {
                        Some(p) => {
                            push(out, i, i + p + 1, Style::String);
                            i += p + 1;
                            st = LexState::InTag(0);
                        }
                        None => {
                            push(out, i, b.len(), Style::String);
                            return st;
                        }
                    }
                    continue;
                }
                let c = b[i];
                if c == b' ' || c == b'\t' {
                    i += 1;
                } else if c == b'>' {
                    push(out, i, i + 1, Style::Tag);
                    i += 1;
                    st = LexState::Normal;
                } else if (c == b'/' || c == b'?') && b.get(i + 1) == Some(&b'>') {
                    push(out, i, i + 2, Style::Tag);
                    i += 2;
                    st = LexState::Normal;
                } else if c == b'"' || c == b'\'' {
                    st = LexState::InTag(c);
                    push(out, i, i + 1, Style::String);
                    i += 1;
                } else if c == b'=' {
                    push(out, i, i + 1, Style::Operator);
                    i += 1;
                } else if tag_name_char(c) {
                    let s = i;
                    while i < b.len() && tag_name_char(b[i]) {
                        i += 1;
                    }
                    push(out, s, i, Style::Attr);
                } else {
                    i += line[i..].chars().next().map_or(1, |c| c.len_utf8());
                }
            }
            _ => {
                st = LexState::Normal;
                let c = b[i];
                if c == b'<' {
                    if line[i..].starts_with("<!--") {
                        st = LexState::MarkupComment;
                        push(out, i, i + 4, Style::Comment);
                        i += 4;
                        continue;
                    }
                    let n = b.get(i + 1).copied().unwrap_or(0);
                    if n.is_ascii_alphabetic() || n == b'/' || n == b'?' || n == b'!' {
                        let s = i;
                        i += 2;
                        while i < b.len() && tag_name_char(b[i]) {
                            i += 1;
                        }
                        push(out, s, i, Style::Tag);
                        st = LexState::InTag(0);
                        continue;
                    }
                    i += 1;
                } else if c == b'&' {
                    if let Some(p) = line[i..].find(';') {
                        if p > 1 && p < 12 && line[i + 1..i + p].bytes().all(|x| x.is_ascii_alphanumeric() || x == b'#') {
                            push(out, i, i + p + 1, Style::Entity);
                            i += p + 1;
                            continue;
                        }
                    }
                    i += 1;
                } else {
                    i += line[i..].chars().next().map_or(1, |c| c.len_utf8());
                }
            }
        }
    }
    st
}

// -------------------------------------------------------------- markdown

fn lex_markdown(line: &str, st: LexState, out: &mut Vec<Span>) -> LexState {
    let t = line.trim_start();
    let ws = line.len() - t.len();
    let fence = t.starts_with("```") || t.starts_with("~~~");
    if st == LexState::Fence {
        push(out, 0, line.len(), Style::Code);
        return if fence { LexState::Normal } else { LexState::Fence };
    }
    if fence {
        push(out, 0, line.len(), Style::Code);
        return LexState::Fence;
    }
    let hashes = t.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && (t.len() == hashes || t.as_bytes()[hashes] == b' ') {
        push(out, 0, line.len(), Style::Heading);
        return LexState::Normal;
    }
    if t.starts_with('>') {
        push(out, 0, line.len(), Style::Quote);
        return LexState::Normal;
    }
    let tt = t.trim_end();
    if tt.len() >= 3 && (tt.bytes().all(|b| b == b'-') || tt.bytes().all(|b| b == b'*') || tt.bytes().all(|b| b == b'_') || tt.bytes().all(|b| b == b'=')) {
        push(out, 0, line.len(), Style::Operator);
        return LexState::Normal;
    }
    let mut i = ws;
    // list markers
    let tb = t.as_bytes();
    if tb.len() >= 2 && matches!(tb[0], b'-' | b'*' | b'+') && tb[1] == b' ' {
        push(out, i, i + 1, Style::Keyword);
        i += 1;
    } else {
        let digits = tb.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits > 0 && digits < 10 && tb.len() > digits + 1 && (tb[digits] == b'.' || tb[digits] == b')') && tb[digits + 1] == b' ' {
            push(out, i, i + digits + 1, Style::Keyword);
            i += digits + 1;
        }
    }
    let b = line.as_bytes();
    while i < b.len() {
        let c = b[i];
        match c {
            b'`' => {
                let ticks = b[i..].iter().take_while(|x| **x == b'`').count();
                let pat = "`".repeat(ticks);
                if let Some(p) = line[i + ticks..].find(&pat) {
                    let e = i + ticks + p + ticks;
                    push(out, i, e, Style::Code);
                    i = e;
                    continue;
                }
                i += ticks;
            }
            b'*' | b'_' => {
                let double = b.get(i + 1) == Some(&c);
                let n = if double { 2 } else { 1 };
                let inner = i + n;
                let opens = b.get(inner).is_some_and(|x| *x != b' ');
                // underscores inside words are not emphasis
                let word_internal = c == b'_' && i > 0 && b[i - 1].is_ascii_alphanumeric();
                if opens && !word_internal {
                    let pat: String = std::iter::repeat_n(c as char, n).collect();
                    if let Some(p) = line[inner..].find(&pat) {
                        if p > 0 {
                            let e = inner + p + n;
                            push(out, i, e, if double { Style::Bold } else { Style::Italic });
                            i = e;
                            continue;
                        }
                    }
                }
                i += n;
            }
            b'[' => {
                if let Some(p) = line[i..].find("](") {
                    if let Some(q) = line[i + p..].find(')') {
                        let e = i + p + q + 1;
                        push(out, i, e, Style::Link);
                        i = e;
                        continue;
                    }
                }
                i += 1;
            }
            b'<' if line[i..].starts_with("<http") => {
                let e = line[i..].find('>').map(|p| i + p + 1).unwrap_or(b.len());
                push(out, i, e, Style::Link);
                i = e;
            }
            _ => i += line[i..].chars().next().map_or(1, |c| c.len_utf8()),
        }
    }
    LexState::Normal
}

// ------------------------------------------------------------------ diff

fn lex_diff(line: &str, out: &mut Vec<Span>) -> LexState {
    let style = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") || line.starts_with("index ") {
        Style::Section
    } else if line.starts_with("@@") {
        Style::Hunk
    } else if line.starts_with('+') || line.starts_with('>') {
        Style::Added
    } else if line.starts_with('-') || line.starts_with('<') {
        Style::Removed
    } else {
        Style::Default
    };
    push(out, 0, line.len(), style);
    LexState::Normal
}

// -------------------------------------------------------------- key=value

fn lex_keyvalue(lang: &Compiled, line: &str, out: &mut Vec<Span>) -> LexState {
    let b = line.as_bytes();
    let t = line.trim_start();
    let mut i = line.len() - t.len();
    if t.is_empty() {
        return LexState::Normal;
    }
    let is_yaml = lang.def.name == "YAML";
    if lang.line_comments.iter().any(|c| t.starts_with(c)) {
        push(out, i, b.len(), Style::Comment);
        return LexState::Normal;
    }
    if is_yaml && (t == "---" || t == "...") {
        push(out, i, b.len(), Style::Preproc);
        return LexState::Normal;
    }
    if t.starts_with('[') && !is_yaml {
        if let Some(p) = t.rfind(']') {
            let e = i + p + 1;
            push(out, i, e, Style::Section);
            lex_generic(lang, line, LexState::Normal, out, e, true);
            return LexState::Normal;
        }
    }
    // yaml list markers
    while is_yaml && b.get(i) == Some(&b'-') && matches!(b.get(i + 1), Some(b' ') | None) {
        push(out, i, i + 1, Style::Operator);
        i += 1;
        while b.get(i) == Some(&b' ') {
            i += 1;
        }
    }
    // find key / separator
    let mut j = i;
    let mut quote = 0u8;
    let mut sep = None;
    while j < b.len() {
        let c = b[j];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if (c == b'"' || c == b'\'') && j == i {
            quote = c;
        } else if c == b'#' && j > i && b[j - 1] == b' ' {
            break;
        } else if is_yaml {
            if c == b':' && matches!(b.get(j + 1), Some(b' ') | None) {
                sep = Some(j);
                break;
            }
        } else if c == b'=' || (c == b':' && lang.def.name == "INI") {
            sep = Some(j);
            break;
        }
        j += 1;
    }
    match sep {
        Some(s) => {
            let key_end = line[i..s].trim_end().len() + i;
            push(out, i, key_end, Style::Key);
            push(out, s, s + 1, Style::Operator);
            lex_generic(lang, line, LexState::Normal, out, s + 1, true);
        }
        None => {
            lex_generic(lang, line, LexState::Normal, out, i, true);
        }
    }
    LexState::Normal
}

// ----------------------------------------------------------------- impex

fn is_impex_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'@' | b'&')
}

/// SAP Commerce ImpEx: `# comments`, `#% beanshell`, `$macros`, header lines
/// (`INSERT_UPDATE Type;attr[mod=val];...`) and `;`-separated data lines.
fn lex_impex(lang: &Compiled, line: &str, st: LexState, out: &mut Vec<Span>) -> LexState {
    if matches!(st, LexState::Str { .. }) {
        return lex_generic(lang, line, st, out, 0, true);
    }
    let b = line.as_bytes();
    let t = line.trim_start();
    let ws = line.len() - t.len();
    if t.starts_with("#%") {
        push(out, ws, b.len(), Style::Preproc);
        return LexState::Normal;
    }
    if t.starts_with('#') {
        push(out, ws, b.len(), Style::Comment);
        return LexState::Normal;
    }
    if t.starts_with('$') {
        // macro definition: $name=value
        let mut i = ws + 1;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'-' | b'(' | b')' | b'.')) {
            i += 1;
        }
        push(out, ws, i, Style::Variable);
        return lex_generic(lang, line, LexState::Normal, out, i, true);
    }
    let word_end = t.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(t.len());
    let word = t[..word_end].to_ascii_lowercase();
    if !matches!(word.as_str(), "insert_update" | "insert" | "update" | "remove") {
        return lex_generic(lang, line, LexState::Normal, out, 0, true);
    }
    // header line
    let mut i = ws;
    push(out, i, ws + word_end, Style::Keyword);
    i += word_end;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let ts = i;
    while i < b.len() && b[i] != b';' && b[i] != b' ' && b[i] != b'\t' {
        i += 1;
    }
    push(out, ts, i, Style::Type);
    let mut paren = 0usize;
    let mut bracket = false;
    let mut after_eq = false;
    while i < b.len() {
        let c = b[i];
        match c {
            b';' => {
                push(out, i, i + 1, Style::Operator);
                paren = 0;
                bracket = false;
                after_eq = false;
                i += 1;
            }
            b'[' => {
                bracket = true;
                after_eq = false;
                push(out, i, i + 1, Style::Operator);
                i += 1;
            }
            b']' => {
                bracket = false;
                push(out, i, i + 1, Style::Operator);
                i += 1;
            }
            b'(' => {
                paren += 1;
                push(out, i, i + 1, Style::Operator);
                i += 1;
            }
            b')' => {
                paren = paren.saturating_sub(1);
                push(out, i, i + 1, Style::Operator);
                i += 1;
            }
            b'=' if bracket => {
                after_eq = true;
                push(out, i, i + 1, Style::Operator);
                i += 1;
            }
            b',' => {
                after_eq = false;
                push(out, i, i + 1, Style::Operator);
                i += 1;
            }
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += 1;
                }
                i = (i + 1).min(b.len());
                push(out, start, i, Style::String);
            }
            b'$' => {
                let start = i;
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'-')) {
                    i += 1;
                }
                push(out, start, i, Style::Variable);
            }
            c if is_impex_ident(c) => {
                let start = i;
                while i < b.len() && is_impex_ident(b[i]) {
                    i += 1;
                }
                let style = if bracket {
                    if after_eq {
                        let w = line[start..i].to_ascii_lowercase();
                        if matches!(w.as_str(), "true" | "false") { Style::Literal } else { Style::String }
                    } else {
                        Style::Key
                    }
                } else if paren > 0 {
                    Style::Builtin
                } else {
                    Style::Attr
                };
                push(out, start, i, style);
            }
            _ => i += line[i..].chars().next().map_or(1, |c| c.len_utf8()),
        }
    }
    LexState::Normal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::{self, compiled};

    fn styles(lang_name: &str, line: &str, st: LexState) -> (Vec<(String, Style)>, LexState) {
        let l = compiled(lang::find_by_name(lang_name).unwrap());
        let mut out = Vec::new();
        let ns = lex_line(l, line, st, &mut out);
        (out.iter().map(|s| (line[s.start..s.end].to_string(), s.style)).collect(), ns)
    }

    #[test]
    fn rust_basic() {
        let (s, st) = styles("Rust", "fn main() { let x = 'a'; // hi", LexState::Normal);
        assert_eq!(st, LexState::Normal);
        assert!(s.contains(&("fn".into(), Style::Keyword)));
        assert!(s.contains(&("main".into(), Style::Function)));
        assert!(s.contains(&("'a'".into(), Style::Char)));
        assert!(s.contains(&("// hi".into(), Style::Comment)));
    }

    #[test]
    fn block_comment_across_lines() {
        let (s, st) = styles("C", "int a; /* start", LexState::Normal);
        assert_eq!(st, LexState::Block(1));
        assert_eq!(s.last().unwrap(), &("/* start".into(), Style::Comment));
        let (s, st) = styles("C", "end */ int b;", st);
        assert_eq!(st, LexState::Normal);
        assert_eq!(s[0], ("end */".into(), Style::Comment));
        assert!(s.contains(&("int".into(), Style::Type)));
    }

    #[test]
    fn nested_rust_comments() {
        let (_, st) = styles("Rust", "/* a /* b */", LexState::Normal);
        assert_eq!(st, LexState::Block(1));
    }

    #[test]
    fn python_triple() {
        let (_, st) = styles("Python", "x = \"\"\"abc", LexState::Normal);
        assert!(matches!(st, LexState::Str { triple: true, .. }));
        let (s, st) = styles("Python", "def\"\"\" + 1", st);
        assert_eq!(st, LexState::Normal);
        assert_eq!(s[0].1, Style::String);
    }

    #[test]
    fn preproc() {
        let (s, _) = styles("C", "#include <stdio.h> // x", LexState::Normal);
        assert_eq!(s[0], ("#include <stdio.h> ".into(), Style::Preproc));
    }

    #[test]
    fn html() {
        let (s, st) = styles("HTML", "<a href=\"x\">t &amp; u</a>", LexState::Normal);
        assert_eq!(st, LexState::Normal);
        assert!(s.contains(&("href".into(), Style::Attr)));
        assert!(s.contains(&("\"x\"".into(), Style::String)));
        assert!(s.contains(&("&amp;".into(), Style::Entity)));
    }

    #[test]
    fn yaml_key() {
        let (s, _) = styles("YAML", "name: it's fine # c", LexState::Normal);
        assert_eq!(s[0], ("name".into(), Style::Key));
        assert!(s.iter().any(|(t, st)| t.starts_with('#') && *st == Style::Comment));
    }

    #[test]
    fn markdown() {
        let (s, _) = styles("Markdown", "# Title", LexState::Normal);
        assert_eq!(s[0].1, Style::Heading);
        let (s, _) = styles("Markdown", "a **b** `c`", LexState::Normal);
        assert!(s.contains(&("**b**".into(), Style::Bold)));
        assert!(s.contains(&("`c`".into(), Style::Code)));
    }

    #[test]
    fn impex_header_data_and_comments() {
        let (s, st) = styles("ImpEx", "INSERT_UPDATE Product;code[unique=true];name[lang=en];catalogVersion(catalog(id),version)", LexState::Normal);
        assert_eq!(st, LexState::Normal);
        assert_eq!(s[0], ("INSERT_UPDATE".into(), Style::Keyword));
        assert!(s.contains(&("Product".into(), Style::Type)));
        assert!(s.contains(&("code".into(), Style::Attr)));
        assert!(s.contains(&("unique".into(), Style::Key)));
        assert!(s.contains(&("true".into(), Style::Literal)));
        assert!(s.contains(&("catalog".into(), Style::Builtin)));
        let (s, _) = styles("ImpEx", "# a comment", LexState::Normal);
        assert_eq!(s[0].1, Style::Comment);
        let (s, _) = styles("ImpEx", "#% impex.setLocale(Locale.ENGLISH);", LexState::Normal);
        assert_eq!(s[0].1, Style::Preproc);
        let (s, _) = styles("ImpEx", "$catalog=electronicsProductCatalog", LexState::Normal);
        assert_eq!(s[0], ("$catalog".into(), Style::Variable));
        let (s, _) = styles("ImpEx", ";P-100;\"Phone\";$catalog:Staged;#fff", LexState::Normal);
        assert!(s.contains(&("\"Phone\"".into(), Style::String)));
        assert!(!s.iter().any(|(t, st)| t.starts_with("#fff") && *st == Style::Comment));
        // multi-line quoted value
        let (_, st) = styles("ImpEx", ";P-1;\"line one", LexState::Normal);
        assert!(matches!(st, LexState::Str { .. }));
        let (_, st) = styles("ImpEx", "line two\";x", st);
        assert_eq!(st, LexState::Normal);
    }

    #[test]
    fn flexible_search_items_and_params() {
        let (s, _) = styles("FlexibleSearch", "SELECT {p:pk} FROM {Product AS p} WHERE {p:code} = ?code AND {p:name[en]} LIKE '%x%'", LexState::Normal);
        assert!(s.contains(&("SELECT".into(), Style::Keyword)));
        assert!(s.contains(&("p".into(), Style::Variable)));
        assert!(s.contains(&("pk".into(), Style::Attr)));
        assert!(s.contains(&("Product".into(), Style::Type)));
        assert!(s.contains(&("?code".into(), Style::Variable)));
        assert!(s.contains(&("'%x%'".into(), Style::String)));
        let (s, _) = styles("FlexibleSearch", "-- comment", LexState::Normal);
        assert_eq!(s[0].1, Style::Comment);
    }

    #[test]
    fn utf8_safe() {
        // must not panic on multibyte content anywhere
        for name in ["Rust", "HTML", "Markdown", "YAML", "Python", "Batch", "Bash"] {
            styles(name, "héllo ✓ \"日本\" 'é' $é %é% <é>", LexState::Normal);
        }
    }
}
