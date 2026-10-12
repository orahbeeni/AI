//! Small text helpers shared across the editor: character classes, display
//! columns (tabs / wide characters) and a few string utilities.

use unicode_width::UnicodeWidthChar;

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 0 = whitespace, 1 = word, 2 = punctuation, 3 = line break.
pub fn char_class(c: char) -> u8 {
    if c == '\n' || c == '\r' {
        3
    } else if c.is_whitespace() {
        0
    } else if is_word_char(c) {
        1
    } else {
        2
    }
}

/// Number of terminal-style cells a character occupies when it starts at `col`.
pub fn char_cells(c: char, col: usize, tab: usize) -> usize {
    if c == '\t' {
        let tab = tab.max(1);
        tab - (col % tab)
    } else {
        match c.width() {
            Some(w) => w,
            None => 1,
        }
    }
}

/// Display column of the char at index `idx` of `s`.
pub fn display_col(s: &str, idx: usize, tab: usize) -> usize {
    let mut col = 0;
    for (i, c) in s.chars().enumerate() {
        if i >= idx {
            break;
        }
        col += char_cells(c, col, tab);
    }
    col
}

/// Total display width of a string.
pub fn display_width(s: &str, tab: usize) -> usize {
    display_col(s, usize::MAX, tab)
}

/// Char index in `s` whose cell range contains/is nearest display column `col`.
/// `round` makes a click in the right half of a cell land after it.
pub fn index_at_display_col(s: &str, col: usize, tab: usize) -> usize {
    let mut cur = 0;
    let mut n = 0;
    for c in s.chars() {
        let w = char_cells(c, cur, tab);
        if cur + w > col {
            return n;
        }
        cur += w;
        n += 1;
    }
    n
}

/// Start char indices of each visual row when `text` is wrapped at `cols` display
/// columns. Breaks after whitespace when possible. The first entry is always 0.
pub fn wrap_rows(text: &str, cols: usize, tab: usize) -> Vec<usize> {
    let mut rows = vec![0];
    if cols == 0 {
        return rows;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut col = 0;
    let mut row_start = 0;
    let mut last_break: Option<usize> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let w = char_cells(c, col, tab);
        if col + w > cols && i > row_start && c != ' ' {
            let b = match last_break {
                Some(b) if b > row_start => b,
                _ => i,
            };
            rows.push(b);
            row_start = b;
            col = 0;
            for &ch in &chars[b..i] {
                col += char_cells(ch, col, tab);
            }
            last_break = None;
            continue;
        }
        col += w;
        if c == ' ' || c == '\t' {
            last_break = Some(i + 1);
        }
        i += 1;
    }
    rows
}

/// Leading run of spaces / tabs.
pub fn leading_ws(s: &str) -> &str {
    let end = s
        .char_indices()
        .find(|(_, c)| *c != ' ' && *c != '\t')
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    &s[..end]
}

/// Split text on any of the three line-break styles, keeping no terminators.
pub fn split_lines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\n' {
            out.push(&s[start..i]);
            start = i + 1;
        } else if b[i] == b'\r' {
            out.push(&s[start..i]);
            if i + 1 < b.len() && b[i + 1] == b'\n' {
                i += 1;
            }
            start = i + 1;
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

/// Natural-ish numeric prefix parse used by numeric line sorting.
pub fn leading_number(s: &str) -> Option<f64> {
    let t = s.trim_start();
    let mut end = 0;
    let b = t.as_bytes();
    if end < b.len() && (b[end] == b'-' || b[end] == b'+') {
        end += 1;
    }
    let mut seen_digit = false;
    while end < b.len() && (b[end].is_ascii_digit() || b[end] == b'.') {
        if b[end].is_ascii_digit() {
            seen_digit = true;
        }
        end += 1;
    }
    if !seen_digit {
        return None;
    }
    t[..end].parse().ok()
}

pub fn matching_bracket(c: char) -> Option<(char, bool)> {
    // (partner, partner_is_forward)
    match c {
        '(' => Some((')', true)),
        '[' => Some((']', true)),
        '{' => Some(('}', true)),
        '<' => Some(('>', true)),
        ')' => Some(('(', false)),
        ']' => Some(('[', false)),
        '}' => Some(('{', false)),
        '>' => Some(('<', false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_with_tabs() {
        assert_eq!(display_col("a\tb", 2, 4), 4);
        assert_eq!(display_col("\t\tx", 2, 4), 8);
        assert_eq!(index_at_display_col("a\tb", 3, 4), 1);
        assert_eq!(index_at_display_col("a\tb", 4, 4), 2);
    }

    #[test]
    fn wrapping() {
        assert_eq!(wrap_rows("hello world foo", 8, 4), vec![0, 6, 12]);
        assert_eq!(wrap_rows("abcdefghij", 4, 4), vec![0, 4, 8]);
        assert_eq!(wrap_rows("short", 80, 4), vec![0]);
        assert_eq!(wrap_rows("anything", 0, 4), vec![0]);
    }

    #[test]
    fn wide_chars() {
        assert_eq!(display_width("日本", 4), 4);
    }

    #[test]
    fn splitting() {
        assert_eq!(split_lines("a\r\nb\rc\nd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_lines(""), vec![""]);
    }

    #[test]
    fn numbers() {
        assert_eq!(leading_number("  -12.5abc"), Some(-12.5));
        assert_eq!(leading_number("abc"), None);
    }
}
