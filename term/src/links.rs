//! Finding URLs and file paths (with `:line` positions) in terminal output, and opening them.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Target {
    Url(String),
    File { path: String, line: Option<u32>, col: Option<u32> },
}

/// A link in a piece of text; `start..end` are character (not byte) positions.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hit {
    pub start: usize,
    pub end: usize,
    pub target: Target,
}

static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:(?:https?|ftps?|sftp|file|ssh|git)://|www\.)[^\s<>"'`|\\^{}\[\]]+"#).unwrap());

static PYTHON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"File "(?P<path>[^"]+)", line (?P<line>\d+)"#).unwrap());

static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?P<path>
            (?:[A-Za-z]:[\\/])?
            (?:[~.]{0,2}[\\/])?
            (?:[\w@%+=,~.\-]+[\\/])*
            [\w@%+=\-]+(?:\.[\w\-]+)*\.[A-Za-z][A-Za-z0-9]{0,9}
          |
            [~.]{0,2}[\\/](?:[\w@%+=,~.\-]+[\\/])*[\w@%+=,~.\-]+
        )
        (?:
            :(?P<line>\d+)(?::(?P<col>\d+))?
          |
            \((?P<line2>\d+)(?:,\s*(?P<col2>\d+))?\)
        )?",
    )
    .unwrap()
});

/// Drop punctuation that ends a sentence rather than the URL, keeping balanced brackets.
fn trim_url(mut s: &str) -> &str {
    loop {
        let Some(last) = s.chars().next_back() else { return s };
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => true,
            ')' => s.matches(')').count() > s.matches('(').count(),
            '>' => true,
            _ => false,
        };
        if !drop {
            return s;
        }
        s = &s[..s.len() - last.len_utf8()];
    }
}

/// Every link in `text`, in order.
pub fn find_links(text: &str) -> Vec<Hit> {
    let char_pos = |byte: usize| text[..byte].chars().count();
    let mut hits: Vec<Hit> = Vec::new();
    let overlaps = |hits: &[Hit], a: usize, b: usize| hits.iter().any(|h| a < h.end && h.start < b);

    for m in URL.find_iter(text) {
        let url = trim_url(m.as_str());
        if url.len() > 6 {
            let (a, b) = (char_pos(m.start()), char_pos(m.start()) + url.chars().count());
            hits.push(Hit { start: a, end: b, target: Target::Url(url.to_string()) });
        }
    }
    for c in PYTHON.captures_iter(text) {
        let whole = c.get(0).unwrap();
        let (a, b) = (char_pos(whole.start()), char_pos(whole.end()));
        if !overlaps(&hits, a, b) {
            let line = c["line"].parse().ok();
            hits.push(Hit { start: a, end: b, target: Target::File { path: c["path"].to_string(), line, col: None } });
        }
    }
    for c in PATH.captures_iter(text) {
        let whole = c.get(0).unwrap();
        let (a, b) = (char_pos(whole.start()), char_pos(whole.end()));
        // The path must start at a boundary, not in the middle of a longer word.
        let before = text[..whole.start()].chars().next_back();
        if before.is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '/' | '\\' | '.' | '-' | '_' | '~' | '@' | '%' | '=')) {
            continue;
        }
        if overlaps(&hits, a, b) {
            continue;
        }
        let num = |name: &str| c.name(name).and_then(|m| m.as_str().parse::<u32>().ok());
        let line = num("line").or(num("line2"));
        let col = num("col").or(num("col2"));
        hits.push(Hit { start: a, end: b, target: Target::File { path: c["path"].to_string(), line, col } });
    }
    hits.sort_by_key(|h| h.start);
    hits
}

/// The link covering character position `idx`.
pub fn link_at(text: &str, idx: usize) -> Option<Hit> {
    find_links(text).into_iter().find(|h| (h.start..h.end).contains(&idx))
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
        && let Some(h) = dirs::home_dir()
    {
        return h.join(rest);
    }
    if path == "~"
        && let Some(h) = dirs::home_dir()
    {
        return h;
    }
    PathBuf::from(path)
}

/// The file `path` refers to: as given, relative to `cwd`, or (for a bare file name or a partial
/// path from a stack trace) somewhere below `cwd`.
pub fn resolve(path: &str, cwd: &Path) -> Option<PathBuf> {
    let p = expand_home(path);
    if p.is_absolute() {
        return p.exists().then_some(p);
    }
    let joined = cwd.join(&p);
    if joined.exists() {
        return Some(joined);
    }
    let rel: PathBuf = p.components().filter(|c| matches!(c, std::path::Component::Normal(_))).collect();
    if rel.as_os_str().is_empty() {
        return None;
    }
    find_below(cwd, &rel)
}

/// Breadth-first search below `root` for a file whose path ends with `rel`, within sensible limits.
fn find_below(root: &Path, rel: &Path) -> Option<PathBuf> {
    let name = rel.file_name()?.to_owned();
    let started = Instant::now();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::from([(root.to_path_buf(), 0)]);
    let mut seen = 0usize;
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            seen += 1;
            if seen > 40_000 || started.elapsed() > Duration::from_millis(80) {
                return None;
            }
            let path = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                let n = e.file_name();
                let n = n.to_string_lossy();
                if depth < 8 && !matches!(n.as_ref(), ".git" | "node_modules" | "target" | ".cache" | ".gradle" | ".idea") {
                    queue.push_back((path, depth + 1));
                }
            } else if e.file_name() == name && path.ends_with(rel) {
                return Some(path);
            }
        }
    }
    None
}

fn detach(mut cmd: Command) {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Ok(mut child) = cmd.spawn() {
        // Reap it when it exits, so it does not linger as a zombie.
        let _ = std::thread::Builder::new().name("opener".into()).spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Open a URL or file with the system's default application.
fn open_with_system(target: &str) {
    #[cfg(target_os = "linux")]
    let cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(target);
        c
    };
    #[cfg(target_os = "macos")]
    let cmd = {
        let mut c = Command::new("open");
        c.arg(target);
        c
    };
    #[cfg(windows)]
    let cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", target]).creation_flags(0x0800_0000);
        c
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let cmd = Command::new("true");
    detach(cmd);
}

pub fn open_url(url: &str) {
    if url.starts_with("www.") {
        open_with_system(&format!("https://{url}"));
    } else {
        open_with_system(url);
    }
}

fn shell_quote(s: &str) -> String {
    if cfg!(windows) { format!("\"{}\"", s.replace('"', "\"\"")) } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

/// The command line for opening `file` in the user's editor: `{file}`, `{line}` and `{col}` are replaced.
pub fn editor_line(template: &str, file: &Path, line: Option<u32>, col: Option<u32>) -> String {
    template
        .replace("{file}", &shell_quote(&file.display().to_string()))
        .replace("{line}", &line.unwrap_or(1).to_string())
        .replace("{col}", &col.unwrap_or(1).to_string())
}

/// Open a file at a position, in the configured editor or else the system's default application.
pub fn open_file(file: &Path, line: Option<u32>, col: Option<u32>, editor: &str) {
    if editor.trim().is_empty() {
        open_with_system(&file.display().to_string());
        return;
    }
    let line_text = editor_line(editor, file, line, col);
    #[cfg(windows)]
    let cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        c.args(["/C", &line_text]).creation_flags(0x0800_0000);
        c
    };
    #[cfg(not(windows))]
    let cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", &line_text]);
        c
    };
    detach(cmd);
}

/// Editors worth offering in Preferences: (name, command).
pub const EDITOR_PRESETS: &[(&str, &str)] = &[
    ("System default (no line numbers)", ""),
    ("VS Code", "code -g {file}:{line}:{col}"),
    ("IntelliJ IDEA", "idea --line {line} {file}"),
    ("Sublime Text", "subl {file}:{line}:{col}"),
    ("Kate", "kate -l {line} -c {col} {file}"),
    ("gedit", "gedit +{line}:{col} {file}"),
    ("Vim in a new terminal tab…", "x-terminal-emulator -e vim +{line} {file}"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &str) -> Vec<String> {
        find_links(s)
            .into_iter()
            .map(|h| match h.target {
                Target::Url(u) => format!("url {u}"),
                Target::File { path, line, col } => format!("file {path} {line:?} {col:?}"),
            })
            .collect()
    }

    #[test]
    fn urls_lose_sentence_punctuation() {
        assert_eq!(texts("see https://example.com/a_(b)). and (http://x.org/y)."), vec!["url https://example.com/a_(b)", "url http://x.org/y"]);
        assert_eq!(texts("go to www.example.org, ok"), vec!["url www.example.org"]);
    }

    #[test]
    fn paths_with_positions() {
        assert_eq!(texts("src/main.rs:45:10: error"), vec!["file src/main.rs Some(45) Some(10)"]);
        // (`com.acme.Foo.run` also looks like a file name; it is dropped later because no such file exists.)
        assert!(texts("at com.acme.Foo.run(Foo.java:123)").contains(&"file Foo.java Some(123) None".to_string()));
        assert_eq!(texts("/var/log/hybris/console.log:7"), vec!["file /var/log/hybris/console.log Some(7) None"]);
        assert_eq!(texts("./gradlew and ../up/x.ts(12,5)"), vec!["file ./gradlew None None", "file ../up/x.ts Some(12) Some(5)"]);
        assert_eq!(texts(r#"  File "/tmp/app.py", line 12, in main"#), vec!["file /tmp/app.py Some(12) None"]);
        assert_eq!(texts(r"C:\proj\Main.cs:9"), vec![r"file C:\proj\Main.cs Some(9) None"]);
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        assert!(texts("and/or whatever, version 1.5 is out").is_empty());
        assert!(texts("hello-world").is_empty());
    }

    #[test]
    fn link_at_uses_character_positions() {
        let s = "é see src/a.rs:3 now";
        let h = link_at(s, 7).unwrap();
        assert_eq!(s.chars().skip(h.start).take(h.end - h.start).collect::<String>(), "src/a.rs:3");
        assert!(link_at(s, 0).is_none());
    }

    #[test]
    fn resolving_files() {
        let dir = std::env::temp_dir().join(format!("term-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("core/src/com/acme")).unwrap();
        std::fs::write(dir.join("core/src/com/acme/Foo.java"), "x").unwrap();
        assert_eq!(resolve("Foo.java", &dir), Some(dir.join("core/src/com/acme/Foo.java")));
        assert_eq!(resolve("acme/Foo.java", &dir), Some(dir.join("core/src/com/acme/Foo.java")));
        assert_eq!(resolve("core/src", &dir), Some(dir.join("core/src")));
        assert_eq!(resolve("Nope.java", &dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn editor_command_line() {
        let l = editor_line("code -g {file}:{line}:{col}", Path::new("/a b/c.rs"), Some(4), None);
        assert_eq!(l, if cfg!(windows) { "code -g \"/a b/c.rs\":4:1" } else { "code -g '/a b/c.rs':4:1" });
    }
}
