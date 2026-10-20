//! Where each command's prompt, input and output sit in the scrollback, from shell-integration marks.
//!
//! Lines are stored as a *key*: the line number plus the number of lines scrolled off the top of the
//! screen so far. A key never changes while the text stays in the buffer, however much output arrives.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

const MAX_BLOCKS: usize = 4000;

/// One command: its prompt, what was typed, its output and how it ended.
#[derive(Clone, Debug)]
pub struct Block {
    /// Key of the line the prompt starts on.
    pub prompt: i64,
    /// Where the input starts (end of the prompt): key and column.
    pub input: Option<(i64, usize)>,
    /// Key of the first output line, once the command started.
    pub output: Option<i64>,
    /// Where the output ended: key and column of the cursor when the command finished.
    pub end: Option<(i64, usize)>,
    pub exit: Option<i32>,
    pub command: String,
    pub started: Option<Instant>,
    pub took: Option<Duration>,
}

impl Block {
    pub fn running(&self) -> bool {
        self.output.is_some() && self.end.is_none()
    }

    /// Key of the last line of output (the line before the cursor when it was at column 0).
    pub fn last_output_line(&self) -> Option<i64> {
        let (key, col) = self.end?;
        let first = self.output?;
        let last = if col == 0 { key - 1 } else { key };
        (last >= first).then_some(last)
    }
}

/// What finished, for alerts.
pub struct Finished {
    pub command: String,
    pub exit: Option<i32>,
    pub took: Duration,
}

#[derive(Default)]
pub struct MarkLog {
    /// Lines scrolled off the top of the screen since tracking began.
    pub scrolled: i64,
    pub blocks: VecDeque<Block>,
    /// The shell has sent at least one mark, so its integration is installed.
    pub seen: bool,
}

impl MarkLog {
    pub fn key(&self, line: i32) -> i64 {
        line as i64 + self.scrolled
    }

    pub fn line(&self, key: i64) -> i32 {
        (key - self.scrolled).clamp(i32::MIN as i64, i32::MAX as i64) as i32
    }

    /// Forget every position (after a reflow, or when scrolling could not be measured).
    pub fn invalidate(&mut self) {
        self.blocks.clear();
    }

    pub fn prompt_start(&mut self, key: i64) {
        self.seen = true;
        // A prompt nothing was run at (just Enter) is replaced by the next one.
        if self.blocks.back().is_some_and(|b| b.output.is_none()) {
            self.blocks.pop_back();
        }
        self.blocks.push_back(Block { prompt: key, input: None, output: None, end: None, exit: None, command: String::new(), started: None, took: None });
        while self.blocks.len() > MAX_BLOCKS {
            self.blocks.pop_front();
        }
    }

    pub fn prompt_end(&mut self, key: i64, col: usize) {
        self.seen = true;
        if let Some(b) = self.blocks.back_mut().filter(|b| b.output.is_none()) {
            b.input = Some((key, col));
        }
    }

    /// The command was accepted. `col` is the cursor column; at column 0 the output starts on this
    /// line, otherwise on the next one (some shells mark before echoing the newline).
    pub fn output_start(&mut self, key: i64, col: usize, command: String) {
        self.seen = true;
        if self.blocks.is_empty() {
            self.prompt_start(key);
        }
        if let Some(b) = self.blocks.back_mut().filter(|b| b.output.is_none()) {
            b.output = Some(if col == 0 { key } else { key + 1 });
            b.command = command;
            b.started = Some(Instant::now());
        }
    }

    pub fn output_end(&mut self, key: i64, col: usize, exit: Option<i32>) -> Option<Finished> {
        self.seen = true;
        let b = self.blocks.back_mut().filter(|b| b.running())?;
        b.end = Some((key, col));
        b.exit = exit;
        let took = b.started.map(|s| s.elapsed()).unwrap_or_default();
        b.took = Some(took);
        Some(Finished { command: b.command.clone(), exit, took })
    }

    /// Drop blocks whose prompt has scrolled out of the history.
    pub fn prune(&mut self, topmost_line: i32) {
        let top = topmost_line as i64 + self.scrolled;
        while self.blocks.front().is_some_and(|b| b.prompt < top) {
            self.blocks.pop_front();
        }
    }

    /// The last block that has output (finished or still running).
    pub fn last_with_output(&self) -> Option<&Block> {
        self.blocks.iter().rev().find(|b| b.output.is_some())
    }

    /// Key of the nearest prompt strictly above (`up`) or below `key`.
    pub fn neighbour_prompt(&self, key: i64, up: bool) -> Option<i64> {
        if up {
            self.blocks.iter().rev().map(|b| b.prompt).find(|p| *p < key)
        } else {
            self.blocks.iter().map(|b| b.prompt).find(|p| *p > key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_survive_scrolling() {
        let mut m = MarkLog::default();
        let key = m.key(5);
        m.scrolled += 12;
        assert_eq!(m.line(key), -7);
    }

    #[test]
    fn a_command_lifecycle() {
        let mut m = MarkLog::default();
        m.prompt_start(m.key(0));
        m.prompt_end(m.key(0), 2);
        m.output_start(m.key(1), 0, "ls".into());
        assert!(m.blocks[0].running());
        m.scrolled += 3;
        let done = m.output_end(m.key(-1), 0, Some(2)).unwrap();
        assert_eq!((done.command.as_str(), done.exit), ("ls", Some(2)));
        let b = &m.blocks[0];
        assert!(!b.running());
        assert_eq!(b.output, Some(1));
        // Output ended at column 0 of key 2, so its last line is key 1.
        assert_eq!(b.last_output_line(), Some(1));
    }

    #[test]
    fn empty_prompts_are_replaced_and_old_ones_pruned() {
        let mut m = MarkLog::default();
        m.prompt_start(0);
        m.prompt_start(1);
        assert_eq!(m.blocks.len(), 1);
        m.output_start(1, 5, "x".into());
        assert_eq!(m.blocks[0].output, Some(2), "marked before the newline: output is on the next line");
        m.prompt_start(10);
        m.prompt_start(20);
        m.output_start(20, 0, "y".into());
        m.prompt_start(30);
        assert_eq!(m.neighbour_prompt(20, true), Some(1));
        assert_eq!(m.neighbour_prompt(1, false), Some(20));
        m.prune(15);
        assert_eq!(m.blocks.front().map(|b| b.prompt), Some(20));
    }

    #[test]
    fn finishing_without_a_running_command_is_ignored() {
        let mut m = MarkLog::default();
        m.prompt_start(0);
        assert!(m.output_end(1, 0, Some(0)).is_none());
    }
}
