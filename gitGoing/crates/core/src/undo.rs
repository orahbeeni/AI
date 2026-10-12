//! Undo/redo of ref-changing operations (commit, checkout, branch create and
//! delete, pull) using before/after snapshots of the local branches and HEAD.
//!
//! An undo only runs if the repository is still exactly in the state the
//! operation left it, and it never overwrites uncommitted work: moving the
//! working tree uses a safe checkout, and undoing a commit is a soft reset that
//! keeps the committed changes staged.

use git2::{build::CheckoutBuilder, BranchType, Oid, Repository};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeadState {
    Branch(String),
    Detached(Oid),
    Unborn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub head: HeadState,
    pub branches: BTreeMap<String, Oid>,
}

/// One undoable operation.
#[derive(Clone, Debug)]
pub struct UndoEntry {
    pub label: &'static str,
    /// Keep the index and working tree as they are (used for commits).
    pub soft: bool,
    pub before: Snapshot,
    pub after: Snapshot,
}

pub fn snapshot(repo_path: &Path) -> Result<Snapshot, git2::Error> {
    snapshot_of(&Repository::discover(repo_path)?)
}

fn snapshot_of(repo: &Repository) -> Result<Snapshot, git2::Error> {
    // Mid-merge/rebase states are transient and not safe to restore.
    if repo.state() != git2::RepositoryState::Clean {
        return Err(git2::Error::from_str("repository is busy with another operation"));
    }
    let mut branches = BTreeMap::new();
    for b in repo.branches(Some(BranchType::Local))? {
        let (b, _) = b?;
        if let (Ok(Some(name)), Some(oid)) = (b.name(), b.get().target()) {
            branches.insert(name.to_owned(), oid);
        }
    }
    let head = if repo.head_detached()? {
        match repo.head().ok().and_then(|h| h.target()) {
            Some(o) => HeadState::Detached(o),
            None => HeadState::Unborn,
        }
    } else {
        let target = repo
            .find_reference("HEAD")?
            .symbolic_target()
            .and_then(|t| t.strip_prefix("refs/heads/"))
            .map(str::to_owned);
        match target {
            Some(n) if branches.contains_key(&n) => HeadState::Branch(n),
            _ => HeadState::Unborn,
        }
    };
    Ok(Snapshot { head, branches })
}

fn err(msg: &str) -> git2::Error {
    git2::Error::from_str(msg)
}

fn head_oid(s: &Snapshot) -> Option<Oid> {
    match &s.head {
        HeadState::Branch(b) => s.branches.get(b).copied(),
        HeadState::Detached(o) => Some(*o),
        HeadState::Unborn => None,
    }
}

/// Move the repository from `expected` to `target`.
///
/// `require_index_match` (for soft redo) refuses if the index no longer equals
/// the tree of the commit being restored, so staged work is never discarded.
pub fn restore(
    repo_path: &Path,
    expected: &Snapshot,
    target: &Snapshot,
    soft: bool,
    require_index_match: bool,
) -> Result<(), git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let current = snapshot_of(&repo)?;
    if current != *expected {
        return Err(err("the repository has changed since this action, so it can no longer be undone or redone safely"));
    }
    let (Some(cur_head), Some(new_head)) = (head_oid(&current), head_oid(target)) else {
        return Err(err("cannot undo across a repository with no commits"));
    };

    if cur_head != new_head {
        let commit = repo.find_commit(new_head)?;
        if soft {
            if require_index_match && repo.index()?.write_tree()? != commit.tree_id() {
                return Err(err("staged changes differ from the commit being restored; commit or unstage them first"));
            }
        } else {
            // Safe mode refuses to overwrite local modifications.
            repo.checkout_tree(commit.as_object(), Some(CheckoutBuilder::new().safe()))?;
        }
    }

    for (name, oid) in &target.branches {
        if current.branches.get(name) != Some(oid) {
            repo.reference(&format!("refs/heads/{name}"), *oid, true, "gitgoing: undo/redo")?;
        }
    }
    match &target.head {
        HeadState::Branch(b) => repo.set_head(&format!("refs/heads/{b}"))?,
        HeadState::Detached(o) => repo.set_head_detached(*o)?,
        HeadState::Unborn => {}
    }
    for name in current.branches.keys().filter(|n| !target.branches.contains_key(*n)) {
        repo.find_branch(name, BranchType::Local)?.delete()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{branches, status};

    #[test]
    fn undo_redo_branch_and_commit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let repo = Repository::init(p).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        std::fs::write(p.join("a"), "1").unwrap();
        status::stage(p, &["a".into()]).unwrap();
        status::commit(p, "c1").unwrap();

        let s0 = snapshot(p).unwrap();
        branches::create_and_checkout(p, "feature").unwrap();
        let s1 = snapshot(p).unwrap();
        assert_ne!(s0, s1);

        std::fs::write(p.join("b"), "2").unwrap();
        status::stage(p, &["b".into()]).unwrap();
        status::commit(p, "c2").unwrap();
        let s2 = snapshot(p).unwrap();

        // Undo the commit: branch moves back, changes stay staged.
        restore(p, &s2, &s1, true, false).unwrap();
        assert_eq!(snapshot(p).unwrap(), s1);
        assert_eq!(status::status(p).unwrap().staged.len(), 1);
        assert!(p.join("b").exists());

        // Redo it.
        restore(p, &s1, &s2, true, true).unwrap();
        assert_eq!(snapshot(p).unwrap(), s2);
        assert_eq!(status::status(p).unwrap().changed_paths(), 0);

        // Stale snapshots are refused.
        assert!(restore(p, &s1, &s0, false, false).is_err());

        // Undo commit again, then the redo guard refuses if the index changed.
        restore(p, &s2, &s1, true, false).unwrap();
        status::unstage(p, &["b".into()]).unwrap();
        assert!(restore(p, &s1, &s2, true, true).is_err());
        status::stage(p, &["b".into()]).unwrap();
        restore(p, &s1, &s2, true, true).unwrap();

        // Undo branch creation: back on the original branch, feature deleted.
        restore(p, &s2, &s1, true, false).unwrap();
        status::commit(p, "c2 again").unwrap();
        let s3 = snapshot(p).unwrap();
        restore(p, &s3, &s0, false, false).unwrap();
        assert_eq!(snapshot(p).unwrap(), s0);
        assert!(!p.join("b").exists(), "working tree follows the checkout");
        // Redo creating the branch.
        restore(p, &s0, &s3, false, false).unwrap();
        assert_eq!(snapshot(p).unwrap(), s3);
        assert!(p.join("b").exists());
    }
}
