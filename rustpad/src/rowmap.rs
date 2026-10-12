//! Mapping between logical lines and visual rows (word wrap + folded lines).

use crate::doc::Doc;
use crate::util;

#[derive(Default)]
pub struct RowMap {
    key: Option<(u64, usize, u64, usize, u64)>,
    identity: bool,
    rows: Vec<u32>,
    /// prefix[i] = first visual row of line i; last entry = total rows.
    prefix: Vec<u64>,
    total: u64,
}

/// Number of visual rows of one (visible) line.
fn rows_of(doc: &Doc, l: usize, wrap_cols: usize, tab: usize) -> u32 {
    if wrap_cols == 0 {
        return 1;
    }
    // cheap upper bound first: no need to materialise most lines
    let len = doc.rope.line(l).len_chars();
    if len * tab.max(2) <= wrap_cols { 1 } else { util::wrap_rows(&doc.line_text(l), wrap_cols, tab).len() as u32 }
}

impl RowMap {
    fn reset_sync(doc: &mut Doc) {
        doc.rm_dirty = None;
        doc.rm_records = 0;
        doc.rm_splice = None;
    }

    fn rebuild_prefix_from(&mut self, from: usize) {
        let n = self.rows.len();
        self.prefix.resize(n + 1, 0);
        let from = from.min(n);
        let mut acc = if from == 0 { 0 } else { self.prefix[from] };
        for l in from..n {
            self.prefix[l] = acc;
            acc += self.rows[l] as u64;
        }
        self.prefix[n] = acc;
        self.total = acc;
    }

    /// Bring the map up to date. A single edit (same line count, or one splice of lines) only
    /// touches the affected lines; anything else (wrap width, folds, many edits) rebuilds.
    pub fn ensure(&mut self, doc: &mut Doc, wrap_cols: usize) {
        let hidden_any = !doc.hidden_ranges().is_empty();
        let n = doc.len_lines();
        let tab = doc.opts.tab_width;
        let key = (doc.version, wrap_cols, doc.hidden_gen, tab, n as u64);
        if self.key == Some(key) {
            return;
        }
        let same_layout = matches!(self.key, Some((_, w, h, t, _)) if w == wrap_cols && h == doc.hidden_gen && t == tab);
        self.key = Some(key);
        if wrap_cols == 0 && !hidden_any {
            self.identity = true;
            self.rows.clear();
            self.prefix.clear();
            self.total = n as u64;
            Self::reset_sync(doc);
            return;
        }
        if !self.identity && same_layout && n > 0 {
            if doc.rm_splice.is_none() && self.rows.len() == n {
                // same line count: recompute the touched lines
                if let Some((lo, hi)) = doc.rm_dirty {
                    let (lo, hi) = (lo.min(n - 1), hi.min(n - 1));
                    for l in lo..=hi {
                        if self.rows[l] != 0 {
                            self.rows[l] = rows_of(doc, l, wrap_cols, tab);
                        }
                    }
                    self.rebuild_prefix_from(lo);
                }
                Self::reset_sync(doc);
                return;
            }
            if doc.rm_records == 1 {
                if let Some((line, removed, added)) = doc.rm_splice {
                    let a = (line + 1).min(self.rows.len());
                    let b = (line + 1 + removed).min(self.rows.len());
                    self.rows.drain(a..b);
                    self.rows.splice(a..a, std::iter::repeat_n(1u32, added));
                    if self.rows.len() == n {
                        for l in line.min(n - 1)..=(line + added).min(n - 1) {
                            if self.rows[l] != 0 {
                                self.rows[l] = rows_of(doc, l, wrap_cols, tab);
                            }
                        }
                        self.rebuild_prefix_from(line.min(n - 1));
                        Self::reset_sync(doc);
                        return;
                    }
                }
            }
        }
        self.identity = false;
        let hidden: Vec<(usize, usize)> = doc.hidden_ranges().to_vec();
        let mut hi = 0;
        self.rows.clear();
        self.rows.reserve(n);
        for l in 0..n {
            while hi < hidden.len() && hidden[hi].1 < l {
                hi += 1;
            }
            let is_hidden = hi < hidden.len() && hidden[hi].0 <= l && l <= hidden[hi].1;
            self.rows.push(if is_hidden { 0 } else { rows_of(doc, l, wrap_cols, tab) });
        }
        self.rebuild_prefix_from(0);
        Self::reset_sync(doc);
    }

    pub fn total_rows(&self) -> u64 {
        self.total.max(1)
    }

    pub fn first_row_of_line(&self, line: usize) -> u64 {
        if self.identity {
            line as u64
        } else {
            self.prefix.get(line).copied().unwrap_or(self.total)
        }
    }

    /// (line, row within line) for a visual row, clamped to the document.
    pub fn locate(&self, row: u64) -> (usize, u32) {
        if self.identity {
            return (row.min(self.total.saturating_sub(1)) as usize, 0);
        }
        if self.prefix.len() < 2 {
            return (0, 0);
        }
        let row = row.min(self.total.saturating_sub(1));
        // last line whose first row <= row and that has rows
        let idx = self.prefix.partition_point(|&p| p <= row).saturating_sub(1);
        let mut l = idx.min(self.rows.len() - 1);
        while l > 0 && self.rows[l] == 0 {
            l -= 1;
        }
        (l, (row - self.prefix[l]) as u32)
    }
}
