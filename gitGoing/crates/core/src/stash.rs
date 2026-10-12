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
    let mut repo = Repository::discover(repo_path)?;
    let sig = repo.signature()?;
    repo.stash_save2(&sig, None, Some(StashFlags::INCLUDE_UNTRACKED))?;
    Ok(())
}

/// Apply and remove a stash. On conflict the stash is kept.
pub fn pop(repo_path: &Path, index: usize) -> Result<(), git2::Error> {
    Repository::discover(repo_path)?.stash_pop(index, None)
}

pub fn apply(repo_path: &Path, index: usize) -> Result<(), git2::Error> {
    Repository::discover(repo_path)?.stash_apply(index, None)
}

pub fn drop(repo_path: &Path, index: usize) -> Result<(), git2::Error> {
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
}
