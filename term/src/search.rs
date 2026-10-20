//! Searching a terminal's whole scrollback and selecting what was found.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Column, Direction, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{Match, RegexSearch};

/// Escape `s` so a regular expression matches it literally.
pub fn escape_regex(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// How well `query` matches `text`: every word of the query must appear, in order, as scattered
/// letters. Higher is better; `None` when something does not match.
pub fn fuzzy(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut total = 0;
    for word in query.split_whitespace() {
        let mut from = 0;
        let mut prev: Option<usize> = None;
        for qc in word.to_lowercase().chars() {
            let at = (from..text.len()).find(|&i| text[i] == qc)?;
            total += 10 - (at - from).min(8) as i32;
            if at > 0 && prev == Some(at - 1) {
                total += 12;
            }
            if at == 0 || !text[at - 1].is_alphanumeric() {
                total += 8;
            }
            prev = Some(at);
            from = at + 1;
        }
    }
    Some(total)
}

/// The next match of `pattern` (a regular expression) after `from`, or the first one from the
/// top (`backwards`: the last one from the bottom). The search wraps around.
/// `Err` means the pattern is not valid.
pub fn find<T: EventListener>(t: &Term<T>, pattern: &str, from: Option<&Match>, backwards: bool) -> Result<Option<Match>, ()> {
    let mut re = RegexSearch::new(pattern).map_err(|_| ())?;
    let origin = match from {
        Some(m) if backwards => m.start().sub(t, Boundary::None, 1),
        Some(m) => m.end().add(t, Boundary::None, 1),
        None if backwards => Point::new(t.bottommost_line(), t.last_column()),
        None => Point::new(t.topmost_line(), Column(0)),
    };
    let dir = if backwards { Direction::Left } else { Direction::Right };
    let side = if backwards { Side::Right } else { Side::Left };
    Ok(t.search_next(&mut re, origin, dir, side, None))
}

/// Select a match and scroll it into view.
pub fn select<T: EventListener>(t: &mut Term<T>, m: &Match) {
    let mut sel = Selection::new(SelectionType::Simple, *m.start(), Side::Left);
    sel.update(*m.end(), Side::Right);
    t.selection = Some(sel);
    t.scroll_to_point(*m.start());
}

/// Whether `m` is exactly what is selected now.
pub fn is_selected<T: EventListener>(t: &Term<T>, m: &Match) -> bool {
    t.selection.as_ref().and_then(|s| s.to_range(t)).is_some_and(|r| r.start == *m.start() && r.end == *m.end())
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::pane::TermSize;

    fn term_with(text: &str) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &TermSize { cols: 40, rows: 5 }, VoidListener);
        let mut p: Processor<StdSyncHandler> = Processor::new();
        p.advance(&mut t, text.replace('\n', "\r\n").as_bytes());
        t
    }

    #[test]
    fn fuzzy_matching() {
        assert!(fuzzy("", "anything").is_some());
        assert!(fuzzy("hyb core", "hybris · core-customize").is_some());
        assert!(fuzzy("xyz", "hybris").is_none());
        // Contiguous, word-start matches beat scattered ones.
        assert!(fuzzy("core", "hybris core").unwrap() > fuzzy("core", "c-o-r-e stuff").unwrap());
        assert!(fuzzy("ab", "xaxb").is_some() && fuzzy("ba", "ab").is_none());
    }

    #[test]
    fn finds_newest_first_and_steps_back() {
        let mut t = term_with("one ERROR a\ntwo\nthree error b\nfour\nfive\nsix\nseven");
        let pat = format!("(?i){}", escape_regex("error"));
        let newest = find(&t, &pat, None, true).unwrap().unwrap();
        assert_eq!(t.bounds_to_string(*newest.start(), *newest.end()), "error");
        select(&mut t, &newest);
        assert!(is_selected(&t, &newest));
        let older = find(&t, &pat, Some(&newest), true).unwrap().unwrap();
        assert!(older.start() < newest.start());
        assert_eq!(t.bounds_to_string(*older.start(), *older.end()), "ERROR");
        assert!(find(&t, "(", None, false).is_err());
        assert!(find(&t, "nothing here", None, false).unwrap().is_none());
    }
}
