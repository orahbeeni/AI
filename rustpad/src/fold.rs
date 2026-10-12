//! Code-folding range computation. A range hides lines `start+1 ..= end`
//! when collapsed; the line after (a closing brace / tag) stays visible.

use ropey::Rope;

use crate::doc::{Doc, FoldRange};
use crate::lang::{self, FoldMode, Kind, Style};
use crate::lexer::{self, LexState};

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr", "!doctype", "?xml",
];

pub fn compute(doc: &Doc) -> Vec<FoldRange> {
    compute_from(&doc.rope, doc.lang, doc.opts.tab_width)
}

fn line_string(rope: &Rope, i: usize) -> String {
    let mut s = rope.line(i).to_string();
    while s.ends_with(['\n', '\r']) {
        s.pop();
    }
    s
}

/// Fold ranges for a text snapshot; has no dependency on `Doc`, so it can run on a worker thread.
pub fn compute_from(rope: &Rope, lang_id: usize, tab: usize) -> Vec<FoldRange> {
    let compiled = lang::compiled(lang_id);
    match compiled.def.fold {
        FoldMode::None => Vec::new(),
        FoldMode::Braces => by_braces(rope, lang_id),
        FoldMode::Indent => by_indent(rope, tab),
        FoldMode::Tags => by_tags(rope, lang_id),
    }
}

fn by_braces(rope: &Rope, lang_id: usize) -> Vec<FoldRange> {
    let compiled = lang::compiled(lang_id);
    let brackets = compiled.def.name == "JSON";
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut spans = Vec::new();
    let mut state = LexState::Normal;
    let mut comment_start: Option<usize> = None;
    for line in 0..rope.len_lines() {
        let text = line_string(rope, line);
        let was_block = matches!(state, LexState::Block(_));
        state = lexer::lex_line(compiled, &text, state, &mut spans);
        let now_block = matches!(state, LexState::Block(_));
        if !was_block && now_block {
            comment_start = Some(line);
        } else if was_block && !now_block {
            if let Some(cs) = comment_start.take() {
                if line > cs {
                    out.push(FoldRange { start: cs, end: line });
                }
            }
        }
        for sp in &spans {
            if sp.style != Style::Operator {
                continue;
            }
            for c in text[sp.start..sp.end].chars() {
                match c {
                    '{' => stack.push(line),
                    '[' if brackets => stack.push(line),
                    '}' | ']' if c == '}' || brackets => {
                        if let Some(open) = stack.pop() {
                            if line > open + 1 {
                                out.push(FoldRange { start: open, end: line - 1 });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

fn by_indent(rope: &Rope, tab: usize) -> Vec<FoldRange> {
    let tab = tab.max(1);
    let n = rope.len_lines();
    let mut out = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new(); // (indent, line)
    let mut last_nonblank: Option<usize> = None;
    let close = |stack: &mut Vec<(usize, usize)>, out: &mut Vec<FoldRange>, below: usize, end: usize| {
        while let Some(&(ind, h)) = stack.last() {
            if ind >= below {
                stack.pop();
                if end > h {
                    out.push(FoldRange { start: h, end });
                }
            } else {
                break;
            }
        }
    };
    for line in 0..n {
        let text = line_string(rope, line);
        if text.trim().is_empty() {
            continue;
        }
        let mut ind = 0;
        for ch in text.chars() {
            match ch {
                ' ' => ind += 1,
                '\t' => ind += tab - ind % tab,
                _ => break,
            }
        }
        if let Some(prev) = last_nonblank {
            close(&mut stack, &mut out, ind, prev);
        }
        stack.push((ind, line));
        last_nonblank = Some(line);
    }
    if let Some(prev) = last_nonblank {
        close(&mut stack, &mut out, 0, prev);
    }
    out
}

fn by_tags(rope: &Rope, lang_id: usize) -> Vec<FoldRange> {
    let compiled = lang::compiled(lang_id);
    debug_assert!(compiled.def.kind == Kind::Markup);
    let mut out = Vec::new();
    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut pending: Option<(String, usize)> = None;
    let mut spans = Vec::new();
    let mut state = LexState::Normal;
    let mut comment_start: Option<usize> = None;
    for line in 0..rope.len_lines() {
        let text = line_string(rope, line);
        let was = state == LexState::MarkupComment;
        state = lexer::lex_line(compiled, &text, state, &mut spans);
        let now = state == LexState::MarkupComment;
        if !was && now {
            comment_start = Some(line);
        } else if was && !now {
            if let Some(cs) = comment_start.take() {
                if line > cs {
                    out.push(FoldRange { start: cs, end: line });
                }
            }
        }
        for sp in &spans {
            if sp.style != Style::Tag {
                continue;
            }
            let t = &text[sp.start..sp.end];
            if let Some(name) = t.strip_prefix("</") {
                let name = name.to_lowercase();
                if let Some(pos) = stack.iter().rposition(|(n, _)| *n == name) {
                    let (_, open) = stack[pos].clone();
                    stack.truncate(pos);
                    if line > open + 1 {
                        out.push(FoldRange { start: open, end: line - 1 });
                    }
                }
            } else if t.starts_with('<') && t.len() > 1 {
                let name = t[1..].to_lowercase();
                if t.starts_with("<!") || t.starts_with("<?") || VOID_TAGS.contains(&name.as_str()) {
                    pending = None;
                } else {
                    pending = Some((name, line));
                }
            } else if t.ends_with('>') {
                if t.starts_with('/') || t.starts_with('?') {
                    pending = None;
                } else if let Some(p) = pending.take() {
                    stack.push(p);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Eol;
    use crate::encoding::Enc;

    fn folds(lang_name: &str, text: &str) -> Vec<(usize, usize)> {
        let mut d = Doc::from_text(text, Enc::Utf8, Eol::Lf);
        d.set_lang(lang::find_by_name(lang_name).unwrap());
        d.folds().iter().map(|f| (f.start, f.end)).collect::<Vec<_>>()
    }

    #[test]
    fn braces_ignore_strings_and_comments() {
        let f = folds("C", "int f() {\n  s = \"}\";\n  // }\n  x;\n}\n");
        assert_eq!(f, vec![(0, 3)]);
    }

    #[test]
    fn nested_braces() {
        let mut f = folds("C", "a {\n b {\n  c;\n }\n d;\n}\n");
        f.sort();
        assert_eq!(f, vec![(0, 4), (1, 2)]);
    }

    #[test]
    fn python_indent() {
        let mut f = folds("Python", "def f():\n    x = 1\n    if x:\n        y\n\nz = 2\n");
        f.sort();
        assert_eq!(f, vec![(0, 3), (2, 3)]);
    }

    #[test]
    fn html_tags() {
        let f = folds("HTML", "<div>\n<br>\n<p>\ntext\n</p>\n</div>\n");
        let mut f = f;
        f.sort();
        assert_eq!(f, vec![(0, 4), (2, 3)]);
    }

    #[test]
    fn collapse_hides_lines() {
        let mut d = Doc::from_text("a {\n b\n c\n}\nz", Enc::Utf8, Eol::Lf);
        d.set_lang(lang::find_by_name("C").unwrap());
        d.toggle_fold(0);
        assert!(!d.is_hidden(0));
        assert!(d.is_hidden(1) && d.is_hidden(2));
        assert!(!d.is_hidden(3));
        assert_eq!(d.next_visible_line(0), Some(3));
        assert_eq!(d.prev_visible_line(3), Some(0));
    }
}
