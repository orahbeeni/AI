//! Stash list, save, apply, pop and drop.

use git2::{Oid, Repository, StashFlags};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    /// The stash commit; it can be inspected like any other commit.
    pub id: Oid,
}

pub fn list(repo: &mut Repository) -> Vec<StashEntry> {
    let mut out = Vec::new();
    let _ = repo.stash_foreach(|index, message, id| {
        out.push(StashEntry { index, message: message.to_owned(), id: *id });
        true
    });
    out
}

/// Stash tracked and untracked changes, leaving a clean working tree.
pub fn save(repo_path: &Path) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, "git stash push --include-untracked");
    let mut repo = Repository::discover(repo_path)?;
    let sig = repo.signature()?;
    repo.stash_save2(&sig, None, Some(StashFlags::INCLUDE_UNTRACKED))?;
    Ok(())
}

/// What to put in a stash. The default stashes everything tracked, with git's own message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveOptions {
    /// Description shown in the stash list; empty for git's "WIP on <branch>".
    pub message: String,
    pub include_untracked: bool,
    /// Keep staged changes in the index and working tree, stashing only a copy.
    pub keep_index: bool,
    /// Stash only what is staged, leaving unstaged changes in place.
    pub staged_only: bool,
    /// Limit the stash to these paths (all changes when empty).
    pub paths: Vec<String>,
}

/// Stash with options, using the system `git stash push`.
pub fn push(repo_path: &Path, opts: &SaveOptions) -> Result<(), String> {
    let mut cmd = crate::gitcmd::command(Some(repo_path));
    cmd.args(["stash", "push"]);
    // `--staged` can't be combined with these.
    if opts.include_untracked && !opts.staged_only {
        cmd.arg("--include-untracked");
    }
    if opts.keep_index && !opts.staged_only {
        cmd.arg("--keep-index");
    }
    if opts.staged_only {
        cmd.arg("--staged");
    }
    if !opts.message.trim().is_empty() {
        cmd.arg(format!("--message={}", opts.message.trim()));
    }
    if !opts.paths.is_empty() {
        cmd.arg("--").args(&opts.paths);
    }
    let out = crate::gitcmd::run_cmd(&mut cmd, "stash push")?;
    // With nothing to stash git still exits successfully.
    match out.lines().next() {
        Some(l) if l.starts_with("No ") && l.contains("to save") => Err("There is nothing to stash.".to_owned()),
        _ => Ok(()),
    }
}

/// Apply and remove a stash. On conflict the stash is kept.
pub fn pop(repo_path: &Path, index: usize) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git stash pop stash@{{{}}}", index));
    Repository::discover(repo_path)?.stash_pop(index, None)
}

pub fn apply(repo_path: &Path, index: usize) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git stash apply stash@{{{}}}", index));
    Repository::discover(repo_path)?.stash_apply(index, None)
}

pub fn drop(repo_path: &Path, index: usize) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git stash drop stash@{{{}}}", index));
    Repository::discover(repo_path)?.stash_drop(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::status;

    #[test]
    fn save_pop_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let mut repo = Repository::init(p).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        std::fs::write(p.join("a"), "1\n").unwrap();
        crate::status::stage(p, &["a".into()]).unwrap();
        crate::status::commit(p, "init").unwrap();

        assert!(save(p).is_err(), "nothing to stash");

        std::fs::write(p.join("a"), "2\n").unwrap();
        std::fs::write(p.join("new"), "n\n").unwrap();
        save(p).unwrap();
        assert_eq!(status(p).unwrap().changed_paths(), 0);
        let entries = list(&mut repo);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].index, 0);

        pop(p, 0).unwrap();
        assert_eq!(status(p).unwrap().changed_paths(), 2);
        assert!(list(&mut repo).is_empty());

        save(p).unwrap();
        drop(p, 0).unwrap();
        assert!(list(&mut repo).is_empty());
        assert_eq!(status(p).unwrap().changed_paths(), 0);
    }
    fn repo_with_commit() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        std::fs::write(dir.path().join("a"), "1\n").unwrap();
        std::fs::write(dir.path().join("b"), "1\n").unwrap();
        crate::status::stage(dir.path(), &["a".into(), "b".into()]).unwrap();
        crate::status::commit(dir.path(), "init").unwrap();
        (dir, repo)
    }

    #[test]
    fn push_with_a_message_and_without_untracked_files() {
        let (dir, mut repo) = repo_with_commit();
        let p = dir.path();
        assert!(push(p, &SaveOptions::default()).is_err(), "nothing to stash");
        std::fs::write(p.join("a"), "2\n").unwrap();
        std::fs::write(p.join("new"), "n\n").unwrap();
        push(p, &SaveOptions { message: "half-done login".into(), ..Default::default() }).unwrap();
        assert!(list(&mut repo)[0].message.contains("half-done login"));
        let s = status(p).unwrap();
        assert_eq!(s.unstaged.len(), 1, "the untracked file stayed: {:?}", s.unstaged);
        assert_eq!(s.unstaged[0].path, "new");
    }

    #[test]
    fn keep_index_leaves_staged_changes_in_place() {
        let (dir, mut repo) = repo_with_commit();
        let p = dir.path();
        std::fs::write(p.join("a"), "staged\n").unwrap();
        crate::status::stage(p, &["a".into()]).unwrap();
        std::fs::write(p.join("b"), "unstaged\n").unwrap();
        push(p, &SaveOptions { keep_index: true, ..Default::default() }).unwrap();
        let s = status(p).unwrap();
        assert_eq!((s.staged.len(), s.unstaged.len()), (1, 0));
        assert_eq!(list(&mut repo).len(), 1);
    }

    #[test]
    fn staged_only_leaves_unstaged_changes_in_place() {
        let (dir, mut repo) = repo_with_commit();
        let p = dir.path();
        std::fs::write(p.join("a"), "staged\n").unwrap();
        crate::status::stage(p, &["a".into()]).unwrap();
        std::fs::write(p.join("b"), "unstaged\n").unwrap();
        push(p, &SaveOptions { staged_only: true, include_untracked: true, ..Default::default() }).unwrap();
        let s = status(p).unwrap();
        assert!(s.staged.is_empty());
        assert_eq!(s.unstaged.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(list(&mut repo).len(), 1);
    }

    #[test]
    fn only_the_chosen_paths_are_stashed() {
        let (dir, mut repo) = repo_with_commit();
        let p = dir.path();
        std::fs::write(p.join("a"), "2\n").unwrap();
        std::fs::write(p.join("b"), "2\n").unwrap();
        push(p, &SaveOptions { paths: vec!["a".into()], ..Default::default() }).unwrap();
        let s = status(p).unwrap();
        assert_eq!(s.unstaged.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(list(&mut repo).len(), 1);
        pop(p, 0).unwrap();
        assert_eq!(status(p).unwrap().unstaged.len(), 2);
    }

}
