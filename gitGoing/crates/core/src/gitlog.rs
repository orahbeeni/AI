//! A running log of the git commands the app uses, for the console window.
//!
//! Operations that run the system `git` are logged exactly as executed, with
//! their output, result and duration. Operations done in-process through
//! libgit2 are logged as the equivalent command line, marked as such.
//!
//! The log is process-wide and bounded; the UI polls it with [`since`].

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A command executed through the system `git`.
    Cmd,
    /// The equivalent command for something done in-process (libgit2).
    Equiv,
    /// Like `Equiv`, but only reads state.
    Read,
    /// Standard output of a command.
    Out,
    /// Standard error, or a failure.
    Err,
    /// The result line of a command (exit status and time).
    Done,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub id: u64,
    /// Seconds since the app started.
    pub at: f64,
    /// Short name of the repository the command ran in (may be empty).
    pub repo: String,
    pub kind: Kind,
    pub text: String,
}

const CAPACITY: usize = 5000;
/// Output lines kept per command, so one noisy command can't flood the log.
const MAX_OUTPUT_LINES: usize = 40;

struct Log {
    lines: VecDeque<Line>,
    next_id: u64,
}

static LOG: Mutex<Log> = Mutex::new(Log { lines: VecDeque::new(), next_id: 1 });
static START: OnceLock<Instant> = OnceLock::new();
static LIVE: AtomicBool = AtomicBool::new(false);
static NOTIFY: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

fn now() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Call once at startup so timestamps count from the app's start.
pub fn init() {
    let _ = now();
}

/// Register a callback used to wake the UI when new lines arrive.
pub fn set_notifier(f: Box<dyn Fn() + Send + Sync>) {
    let _ = NOTIFY.set(f);
}

/// Tell the log whether a console window is open (only then is the UI woken).
pub fn set_live(live: bool) {
    LIVE.store(live, Ordering::Relaxed);
}

/// Hide `user:password@` in URLs so credentials never reach the log.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        out.push_str(&rest[..i + 3]);
        rest = &rest[i + 3..];
        let end = rest.find(|c: char| c == '/' || c.is_whitespace()).unwrap_or(rest.len());
        let authority = &rest[..end];
        match authority.rfind('@') {
            Some(at) if authority[..at].contains(':') => {
                out.push_str("***@");
                out.push_str(&authority[at + 1..]);
            }
            _ => out.push_str(authority),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Quote an argument for display, as a shell would need it.
pub fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(|c: char| c.is_whitespace() || "\"'\\$`&|;<>()*?[]{}!#".contains(c)) {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// Quote and join paths for display, abbreviating long lists.
pub fn paths(list: &[String]) -> String {
    const SHOWN: usize = 5;
    let mut out: Vec<String> = list.iter().take(SHOWN).map(|p| quote(p)).collect();
    if list.len() > SHOWN {
        out.push(format!("… (+{} more)", list.len() - SHOWN));
    }
    out.join(" ")
}

pub fn repo_name(path: &Path) -> String {
    let p = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn push(repo: &str, kind: Kind, text: impl Into<String>) {
    let text = redact(&text.into());
    let at = now();
    {
        let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
        let id = log.next_id;
        log.next_id += 1;
        if log.lines.len() == CAPACITY {
            log.lines.pop_front();
        }
        log.lines.push_back(Line { id, at, repo: repo.to_owned(), kind, text });
    }
    if LIVE.load(Ordering::Relaxed) {
        if let Some(f) = NOTIFY.get() {
            f();
        }
    }
}

/// Log the command equivalent to an in-process (libgit2) write operation.
pub fn equiv(repo_path: &Path, command: impl Into<String>) {
    push(&repo_name(repo_path), Kind::Equiv, command);
}

/// Log the command equivalent to an in-process read.
pub fn read(repo_path: &Path, command: impl Into<String>) {
    push(&repo_name(repo_path), Kind::Read, command);
}

/// Log a finished command's output (bounded) and result.
pub fn finished(repo: &str, stdout: &str, stderr: &str, ok: bool, status: &str, ms: u128) {
    for (kind, text) in [(Kind::Out, stdout), (Kind::Err, stderr)] {
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        for l in lines.iter().take(MAX_OUTPUT_LINES) {
            push(repo, kind, l.trim_end());
        }
        if lines.len() > MAX_OUTPUT_LINES {
            push(repo, kind, format!("… {} more line(s)", lines.len() - MAX_OUTPUT_LINES));
        }
    }
    push(repo, if ok { Kind::Done } else { Kind::Err }, format!("{status} · {ms} ms"));
}

/// All lines with an id greater than `after` (pass 0 for everything).
pub fn since(after: u64) -> Vec<Line> {
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    log.lines.iter().filter(|l| l.id > after).cloned().collect()
}

pub fn clear() {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).lines.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials_in_urls() {
        assert_eq!(redact("git clone https://user:s3cret@host.com/o/r.git"), "git clone https://***@host.com/o/r.git");
        assert_eq!(redact("https://host.com/o/r.git"), "https://host.com/o/r.git");
        assert_eq!(redact("git@github.com:o/r.git"), "git@github.com:o/r.git");
        assert_eq!(redact("a https://u:p@h/x and https://u2:p2@h2/y"), "a https://***@h/x and https://***@h2/y");
        // A user name alone (no password) is not a secret.
        assert_eq!(redact("https://user@host/r"), "https://user@host/r");
    }

    #[test]
    fn quotes_arguments_for_display() {
        assert_eq!(quote("main"), "main");
        assert_eq!(quote("two words"), "'two words'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn push_since_and_ids_increase() {
        let marker = "unique-marker-for-push-test";
        push("repo", Kind::Equiv, marker);
        let all = since(0);
        let mine = all.iter().find(|l| l.text == marker).expect("line was logged");
        assert_eq!((mine.kind, mine.repo.as_str()), (Kind::Equiv, "repo"));
        assert!(since(mine.id).iter().all(|l| l.id > mine.id));
    }

    #[test]
    fn finished_bounds_output() {
        let repo = "bounded-output-test";
        let big: String = (0..100).map(|i| format!("line {i}\n")).collect();
        finished(repo, &big, "", true, "exit 0", 5);
        let mine: Vec<_> = since(0).into_iter().filter(|l| l.repo == repo).collect();
        assert!(mine.iter().filter(|l| l.kind == Kind::Out).count() <= MAX_OUTPUT_LINES + 1);
        assert!(mine.iter().any(|l| l.text.starts_with("… 60 more")));
        assert_eq!(mine.last().unwrap().kind, Kind::Done);
    }
}
