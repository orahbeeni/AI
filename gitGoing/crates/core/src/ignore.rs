//! Telling git to ignore files: patterns in `.gitignore` (shared) or in
//! `.git/info/exclude` (this clone only), and stopping tracking of files that were committed.

use git2::{Repository, Status, StatusOptions};
use std::path::Path;

/// One way to ignore a file: what to show and the line to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub pattern: String,
}

/// Escape characters that mean something in an ignore pattern.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for (i, c) in s.chars().enumerate() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']') || (i == 0 && matches!(c, '#' | '!')) {
            out.push('\\');
        }
        out.push(c);
    }
    if out.ends_with(' ') {
        out.pop();
        out.push_str("\\ ");
    }
    out
}

/// The pattern that ignores exactly this file, wherever the `.gitignore` is read from.
pub fn exact_pattern(path: &str) -> String {
    format!("/{}", escape(path))
}

/// The sensible ways to ignore `path` (relative to the repository root, `/`-separated).
pub fn suggestions(path: &str) -> Vec<Suggestion> {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    let mut out = vec![Suggestion { label: format!("This file ({name})"), pattern: exact_pattern(path) }];
    if let Some((stem, ext)) = name.rsplit_once('.') {
        if !stem.is_empty() && !ext.is_empty() {
            out.push(Suggestion { label: format!("All .{ext} files"), pattern: format!("*.{}", escape(ext)) });
        }
    }
    if !dir.is_empty() {
        out.push(Suggestion { label: format!("The folder {dir}/"), pattern: format!("/{}/", escape(dir)) });
    }
    out
}

/// Add `patterns` to `.gitignore` in the working directory, or with `local` to
/// `.git/info/exclude`. Patterns that are already there are skipped.
pub fn add_patterns(repo_path: &Path, patterns: &[String], local: bool) -> Result<(), String> {
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    let file = if local {
        repo.path().join("info").join("exclude")
    } else {
        repo.workdir().ok_or("this repository has no working directory")?.join(".gitignore")
    };
    let target = if local { ".git/info/exclude" } else { ".gitignore" };
    crate::gitlog::equiv(repo_path, format!("echo {} >> {target}", crate::gitlog::paths(patterns)));
    let mut text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("could not read {target}: {e}")),
    };
    let fresh: Vec<&String> = patterns
        .iter()
        .filter(|p| !p.trim().is_empty() && !text.lines().any(|l| l.trim_end() == p.as_str()))
        .collect();
    if fresh.is_empty() {
        return Ok(());
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for p in fresh {
        text.push_str(p);
        text.push('\n');
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    std::fs::write(&file, text).map_err(|e| format!("could not write {target}: {e}"))
}

/// Stop tracking files (or folders) without deleting them (`git rm --cached`).
/// The removal is staged; ignore the files too, or they come back as untracked.
pub fn untrack(repo_path: &Path, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut cmd = crate::gitcmd::command(Some(repo_path));
    cmd.args(["rm", "--cached", "-r", "-q", "--"]).args(paths);
    crate::gitcmd::run_cmd(&mut cmd, "rm --cached").map(|_| ())
}

/// Files and folders git is ignoring (a wholly ignored folder is one entry ending in `/`).
pub fn ignored(repo_path: &Path) -> Result<Vec<String>, git2::Error> {
    crate::gitlog::read(repo_path, "git status --ignored --porcelain");
    let repo = Repository::discover(repo_path)?;
    let mut opts = StatusOptions::new();
    opts.include_ignored(true).include_untracked(false).recurse_ignored_dirs(false).include_unmodified(false);
    let mut out: Vec<String> = repo
        .statuses(Some(&mut opts))?
        .iter()
        .filter(|e| e.status().contains(Status::IGNORED))
        .filter_map(|e| e.path().map(str::to_owned))
        .collect();
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_cover_file_extension_and_folder() {
        let s = suggestions("src/gen/out.min.js");
        let pats: Vec<&str> = s.iter().map(|s| s.pattern.as_str()).collect();
        assert_eq!(pats, ["/src/gen/out.min.js", "*.js", "/src/gen/"]);
        let s = suggestions(".env");
        assert_eq!(s.iter().map(|s| s.pattern.as_str()).collect::<Vec<_>>(), ["/.env"], "a dotfile has no extension");
        assert_eq!(suggestions("Makefile").len(), 1);
    }

    #[test]
    fn special_characters_are_escaped() {
        assert_eq!(exact_pattern("a b*[1].txt"), "/a b\\*\\[1\\].txt");
        assert_eq!(exact_pattern("#notes"), "/\\#notes");
        assert_eq!(escape("#x"), "\\#x");
        assert_eq!(escape("!x"), "\\!x");
        assert_eq!(escape("x "), "x\\ ");
    }

    fn repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        cfg.set_bool("commit.gpgsign", false).unwrap();
        (dir, repo)
    }

    #[test]
    fn patterns_are_appended_once_and_take_effect() {
        let (dir, _repo) = repo();
        let p = dir.path();
        std::fs::write(p.join(".gitignore"), "existing").unwrap(); // no trailing newline
        std::fs::write(p.join("a.log"), "x").unwrap();
        std::fs::create_dir(p.join("build")).unwrap();
        std::fs::write(p.join("build/o"), "x").unwrap();
        std::fs::write(p.join("keep.txt"), "x").unwrap();

        add_patterns(p, &["*.log".into(), "/build/".into(), "existing".into()], false).unwrap();
        add_patterns(p, &["*.log".into()], false).unwrap();
        assert_eq!(std::fs::read_to_string(p.join(".gitignore")).unwrap(), "existing\n*.log\n/build/\n");

        let untracked: Vec<String> =
            crate::status::status(p).unwrap().unstaged.into_iter().map(|e| e.path).collect();
        assert_eq!(untracked, [".gitignore", "keep.txt"]);
        assert_eq!(ignored(p).unwrap(), ["a.log", "build/"]);
    }

    #[test]
    fn local_ignores_go_to_the_exclude_file() {
        let (dir, repo) = repo();
        let p = dir.path();
        std::fs::write(p.join("scratch.txt"), "x").unwrap();
        add_patterns(p, &["/scratch.txt".into()], true).unwrap();
        assert!(!p.join(".gitignore").exists());
        assert_eq!(std::fs::read_to_string(repo.path().join("info/exclude")).unwrap().lines().last(), Some("/scratch.txt"));
        assert!(crate::status::status(p).unwrap().unstaged.is_empty());
    }

    #[test]
    fn untrack_keeps_the_file_and_stages_its_removal() {
        let (dir, _repo) = repo();
        let p = dir.path();
        std::fs::write(p.join("secret.env"), "k=v").unwrap();
        crate::status::stage(p, &["secret.env".into()]).unwrap();
        crate::status::commit(p, "oops").unwrap();
        untrack(p, &["secret.env".into()]).unwrap();
        assert!(p.join("secret.env").exists());
        add_patterns(p, &[exact_pattern("secret.env")], false).unwrap();
        let s = crate::status::status(p).unwrap();
        assert_eq!(s.staged.len(), 1);
        assert_eq!(s.staged[0].kind, crate::diff::ChangeKind::Deleted);
        assert!(s.unstaged.iter().all(|e| e.path != "secret.env"), "ignored, so not offered as untracked");
    }
}
