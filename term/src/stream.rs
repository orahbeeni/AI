//! Reads a terminal's raw output for what the emulator ignores: shell-integration marks
//! (OSC 133), desktop-notification requests (OSC 9 / OSC 777) and the plain text of each line,
//! which feeds alerts and output recording.

/// A shell-integration mark (OSC 133).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    /// `A`: the prompt starts.
    PromptStart,
    /// `B`: the prompt ends and the user's input starts.
    PromptEnd,
    /// `C`: the command was accepted and its output starts.
    OutputStart,
    /// `D;code`: the command finished.
    OutputEnd(Option<i32>),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Item {
    Mark(Mark),
    /// A program asked for a desktop notification (title, body).
    Notify(String, String),
    /// One line of output with escape sequences removed.
    Line(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum State {
    #[default]
    Ground,
    Esc,
    /// ESC followed by an intermediate byte such as `(`: one more byte to skip.
    EscSkip,
    Csi,
    Osc,
    OscEsc,
    Str,
    StrEsc,
}

const MAX_LINE: usize = 16 * 1024;
const MAX_OSC: usize = 4096;

/// Incremental scanner; keeps its state between chunks, so sequences may be split anywhere.
#[derive(Default)]
pub struct Scanner {
    state: State,
    osc: Vec<u8>,
    line: Vec<u8>,
}

impl Scanner {
    /// Scan `bytes`, calling `out(offset, item)` for each item. `offset` is the index just past
    /// the byte that completed the item, so the emulator can be advanced exactly that far first.
    pub fn feed(&mut self, bytes: &[u8], mut out: impl FnMut(usize, Item)) {
        for (i, &b) in bytes.iter().enumerate() {
            self.step(b, i + 1, &mut out);
        }
    }

    fn step(&mut self, b: u8, end: usize, out: &mut impl FnMut(usize, Item)) {
        match self.state {
            State::Ground => match b {
                0x1b => self.state = State::Esc,
                b'\n' => self.flush(end, out),
                b'\r' => self.flush(end, out),
                0x08 => self.backspace(),
                b'\t' => self.push(b' ', end, out),
                0..=0x1f | 0x7f => {}
                _ => self.push(b, end, out),
            },
            State::Esc => match b {
                b']' => {
                    self.osc.clear();
                    self.state = State::Osc;
                }
                b'[' => self.state = State::Csi,
                b'P' | b'X' | b'^' | b'_' => self.state = State::Str,
                b'(' | b')' | b'*' | b'+' | b'-' | b'.' | b'/' | b' ' | b'#' | b'%' => self.state = State::EscSkip,
                0x1b => {}
                _ => self.state = State::Ground,
            },
            State::EscSkip => self.state = State::Ground,
            State::Csi => match b {
                0x40..=0x7e => self.state = State::Ground,
                0x1b => self.state = State::Esc,
                _ => {}
            },
            State::Osc => match b {
                0x07 => self.end_osc(end, out),
                0x1b => self.state = State::OscEsc,
                _ => {
                    if self.osc.len() < MAX_OSC {
                        self.osc.push(b);
                    }
                }
            },
            State::OscEsc => {
                if b == b'\\' {
                    self.end_osc(end, out);
                } else {
                    // Not a terminator: the OSC was abandoned and a new escape sequence starts.
                    self.osc.clear();
                    self.state = State::Esc;
                    self.step(b, end, out);
                }
            }
            State::Str => match b {
                0x07 => self.state = State::Ground,
                0x1b => self.state = State::StrEsc,
                _ => {}
            },
            State::StrEsc => self.state = if b == b'\\' { State::Ground } else { State::Str },
        }
    }

    fn push(&mut self, b: u8, end: usize, out: &mut impl FnMut(usize, Item)) {
        // A runaway line is handed over in pieces rather than growing without bound.
        if self.line.len() >= MAX_LINE {
            self.flush(end, out);
        }
        self.line.push(b);
    }

    fn backspace(&mut self) {
        while let Some(&last) = self.line.last() {
            self.line.pop();
            if last & 0xc0 != 0x80 {
                break;
            }
        }
    }

    fn flush(&mut self, end: usize, out: &mut impl FnMut(usize, Item)) {
        if self.line.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(&self.line).trim_end().to_string();
        self.line.clear();
        if !text.is_empty() {
            out(end, Item::Line(text));
        }
    }

    fn end_osc(&mut self, end: usize, out: &mut impl FnMut(usize, Item)) {
        self.state = State::Ground;
        let osc = std::mem::take(&mut self.osc);
        if let Some(item) = parse_osc(&osc) {
            out(end, item);
        }
    }
}

fn parse_osc(osc: &[u8]) -> Option<Item> {
    let text = String::from_utf8_lossy(osc);
    if let Some(rest) = text.strip_prefix("133;") {
        let mut parts = rest.split(';');
        return Some(Item::Mark(match parts.next()? {
            "A" => Mark::PromptStart,
            "B" => Mark::PromptEnd,
            "C" => Mark::OutputStart,
            "D" => Mark::OutputEnd(parts.next().and_then(|c| c.trim().parse().ok())),
            _ => return None,
        }));
    }
    // iTerm2: OSC 9 ; message  (OSC 9;4;… is a progress report, not a message).
    if let Some(rest) = text.strip_prefix("9;") {
        if rest.starts_with("4;") || rest.is_empty() {
            return None;
        }
        return Some(Item::Notify(String::new(), rest.to_string()));
    }
    // VTE / urxvt: OSC 777 ; notify ; title ; body
    if let Some(rest) = text.strip_prefix("777;notify;") {
        let (title, body) = rest.split_once(';').unwrap_or((rest, ""));
        return Some(Item::Notify(title.to_string(), body.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<(usize, Item)> {
        let mut s = Scanner::default();
        let mut out = Vec::new();
        for c in chunks {
            s.feed(c, |o, i| out.push((o, i)));
        }
        out
    }

    #[test]
    fn strips_colours_and_splits_lines() {
        let out = scan(&[b"\x1b[32mINFO\x1b[0m started\r\n\x1b[1;31mERROR\x1b[0m boom\n"]);
        let lines: Vec<_> = out.iter().map(|(_, i)| i.clone()).collect();
        assert_eq!(lines, vec![Item::Line("INFO started".into()), Item::Line("ERROR boom".into())]);
    }

    #[test]
    fn progress_bars_and_backspace() {
        let out = scan(&[b"10%\r20%\r30%\rdone\nab\x08c\n"]);
        let lines: Vec<_> = out.into_iter().map(|(_, i)| i).collect();
        assert_eq!(
            lines,
            vec![Item::Line("10%".into()), Item::Line("20%".into()), Item::Line("30%".into()), Item::Line("done".into()), Item::Line("ac".into())]
        );
    }

    #[test]
    fn marks_report_the_offset_after_the_terminator() {
        let data = b"abc\x1b]133;A\x07$ \x1b]133;B\x1b\\ls\r\n\x1b]133;C\x07out\n\x1b]133;D;3\x07";
        let marks: Vec<_> = scan(&[data]).into_iter().filter_map(|(o, i)| if let Item::Mark(m) = i { Some((o, m)) } else { None }).collect();
        assert_eq!(
            marks,
            vec![
                (data.iter().position(|b| *b == 0x07).unwrap() + 1, Mark::PromptStart),
                (data.windows(2).position(|w| w == b"\x1b\\").unwrap() + 2, Mark::PromptEnd),
                (data.windows(8).position(|w| w == b"133;C\x07ou").unwrap() + 6, Mark::OutputStart),
                (data.len(), Mark::OutputEnd(Some(3))),
            ]
        );
    }

    #[test]
    fn sequences_may_be_split_between_chunks() {
        let out = scan(&[b"x\x1b]13", b"3;D;", b"0\x07y\x1b[3", b"1mz\n"]);
        // The mark completes two bytes into the third chunk.
        assert!(out.contains(&(2, Item::Mark(Mark::OutputEnd(Some(0))))), "{out:?}");
        assert!(out.iter().any(|(_, i)| *i == Item::Line("xyz".into())), "{out:?}");
    }

    #[test]
    fn exit_code_is_optional() {
        let out = scan(&[b"\x1b]133;D\x07"]);
        assert_eq!(out[0].1, Item::Mark(Mark::OutputEnd(None)));
    }

    #[test]
    fn notification_requests() {
        let out = scan(&[b"\x1b]9;Build finished\x07\x1b]777;notify;CI;all green\x07\x1b]9;4;1;50\x07"]);
        let items: Vec<_> = out.into_iter().map(|(_, i)| i).collect();
        assert_eq!(
            items,
            vec![Item::Notify(String::new(), "Build finished".into()), Item::Notify("CI".into(), "all green".into())]
        );
    }

    #[test]
    fn charset_and_string_sequences_are_skipped() {
        let out = scan(&[b"\x1b(Bhi\x1bP1$r0m\x1b\\ there\x1b_Gdata\x1b\\!\n"]);
        assert!(out.iter().any(|(_, i)| *i == Item::Line("hi there!".into())), "{out:?}");
    }

    #[test]
    fn utf8_survives() {
        let out = scan(&[b"caf\xc3", b"\xa9 \xe2\x9c\x93\n"]);
        assert!(out.iter().any(|(_, i)| *i == Item::Line("café ✓".into())), "{out:?}");
    }
}
