//! Fetch, pull and push.
//!
//! Network operations shell out to the system `git` so that the user's
//! credential helpers, SSH config and agent all work exactly as they do on the
//! command line (and so we don't have to link OpenSSL/libssh2).

use git2::{BranchType, Repository};
use std::path::Path;

fn run(repo_path: &Path, args: &[&str]) -> Result<(), String> {
    crate::gitcmd::run(Some(repo_path), args).map(|_| ())
}

pub fn fetch(repo_path: &Path) -> Result<(), String> {
    run(repo_path, &["fetch", "--all", "--prune"])
}

/// How `pull` integrates the upstream branch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PullMode {
    /// Fetch, then fast-forward if possible, otherwise merge.
    #[default]
    Merge,
    /// Fetch, then fast-forward; fail if the branches have diverged.
    FastForwardOnly,
    /// Fetch, then rebase local commits onto the upstream (stashing local changes).
    Rebase,
}

impl PullMode {
    pub const ALL: [PullMode; 3] = [PullMode::Merge, PullMode::FastForwardOnly, PullMode::Rebase];

    pub fn label(self) -> &'static str {
        match self {
            PullMode::Merge => "Pull (fast-forward if possible, else merge)",
            PullMode::FastForwardOnly => "Pull (fast-forward only)",
            PullMode::Rebase => "Pull (rebase)",
        }
    }
}

pub fn pull_with(repo_path: &Path, mode: PullMode) -> Result<(), String> {
    match mode {
        PullMode::Merge => pull(repo_path),
        PullMode::FastForwardOnly => run(repo_path, &["pull", "--ff-only"]),
        PullMode::Rebase => run(repo_path, &["pull", "--rebase", "--autostash"]).map_err(|e| {
            let busy = Repository::discover(repo_path)
                .map(|r| r.state() != git2::RepositoryState::Clean)
                .unwrap_or(false);
            if busy {
                "Pull stopped: the rebase hit conflicts. Resolve them in Changes, then continue (or abort) the rebase."
                    .to_owned()
            } else {
                e
            }
        }),
    }
}

/// Fetch, then merge the branch's upstream (fast-forwarding when possible).
/// Stops with the repo in a merge state if there are conflicts to resolve.
pub fn pull(repo_path: &Path) -> Result<(), String> {
    let e = |e: git2::Error| e.message().to_owned();
    let repo = Repository::discover(repo_path).map_err(e)?;
    crate::merge::ensure_clean_state(&repo).map_err(e)?;
    run(repo_path, &["fetch"])?;
    let head = repo.head().map_err(e)?;
    if !head.is_branch() {
        return Err("HEAD is detached; check out a branch to pull".into());
    }
    let name = head.shorthand().unwrap_or_default().to_owned();
    let upstream = repo
        .find_branch(&name, BranchType::Local)
        .and_then(|b| b.upstream())
        .map_err(|_| format!("'{name}' has no upstream branch; push it first"))?;
    match crate::merge::merge_reference(&repo, upstream.get()).map_err(e)? {
        crate::merge::MergeOutcome::Conflicts(n) => Err(format!(
            "Pull stopped: {n} conflicted file(s). Resolve them in Changes, then commit (or abort the merge)."
        )),
        _ => Ok(()),
    }
}

/// Push the current branch, publishing it to a remote first if it has no
/// upstream yet.
pub fn push(repo_path: &Path) -> Result<(), String> {
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    let head = repo.head().map_err(|e| e.message().to_owned())?;
    if !head.is_branch() {
        return Err("HEAD is detached; check out a branch to push".into());
    }
    let branch = head.shorthand().unwrap_or_default().to_owned();
    let has_upstream = repo
        .find_branch(&branch, BranchType::Local)
        .map(|b| b.upstream().is_ok())
        .unwrap_or(false);
    if has_upstream {
        return run(repo_path, &["push"]);
    }
    let remotes = repo.remotes().map_err(|e| e.message().to_owned())?;
    let remote = if remotes.iter().flatten().any(|r| r == "origin") {
        "origin"
    } else {
        remotes.iter().flatten().next().ok_or("no remote configured")?
    };
    run(repo_path, &["push", "--set-upstream", remote, &branch])
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;

    fn commit(repo: &Repository, name: &str) {
        let wd = repo.workdir().unwrap();
        std::fs::write(wd.join(name), name).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new(name)).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = Signature::now("T", "t@example.com").unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<_> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, name, &tree, &parents).unwrap();
    }

    #[test]
    fn push_fetch_pull_roundtrip() {
        if crate::gitcmd::run(None, &["--version"]).is_err() {
            return; // git not installed; nothing to test against
        }
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("origin.git");
        Repository::init_bare(&bare).unwrap();

        let a_path = tmp.path().join("a");
        let a = Repository::init(&a_path).unwrap();
        a.remote("origin", bare.to_str().unwrap()).unwrap();
        commit(&a, "one");
        push(&a_path).unwrap(); // publishes the branch, setting upstream

        let b_path = tmp.path().join("b");
        let b = Repository::clone(bare.to_str().unwrap(), &b_path).unwrap();

        commit(&a, "two");
        push(&a_path).unwrap(); // plain push now that upstream exists

        fetch(&b_path).unwrap();
        pull(&b_path).unwrap();
        assert_eq!(b.head().unwrap().peel_to_commit().unwrap().summary(), Some("two"));

        // Fast-forward only refuses diverged history; rebase puts local commits on top.
        commit(&a, "ff-test");
        push(&a_path).unwrap();
        // (Fresh handles: a long-lived one keeps a stale cached index.)
        commit(&Repository::open(&b_path).unwrap(), "local-first");
        assert!(pull_with(&b_path, PullMode::FastForwardOnly).is_err());
        let mut cfg = b.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        pull_with(&b_path, PullMode::Rebase).unwrap();
        let b = Repository::open(&b_path).unwrap();
        let head = b.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.summary(), Some("local-first"));
        assert_eq!(head.parent_count(), 1, "rebased, not merged");
        assert_eq!(head.parent(0).unwrap().summary(), Some("ff-test"));

        // Diverged (non-conflicting) history is merged with a merge commit.
        commit(&a, "three");
        push(&a_path).unwrap();
        commit(&b, "local");
        pull(&b_path).unwrap();
        assert_eq!(b.head().unwrap().peel_to_commit().unwrap().parent_count(), 2);
    }
}
