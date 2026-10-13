//! Fetch, pull and push.
//!
//! Network operations shell out to the system `git` so that the user's
//! credential helpers, SSH config and agent all work exactly as they do on the
//! command line (and so we don't have to link OpenSSL/libssh2).

use git2::{BranchType, Oid, Repository};
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
    crate::gitlog::equiv(repo_path, format!("git merge {}", upstream.get().shorthand().unwrap_or("@{u}")));
    match crate::merge::merge_reference(&repo, upstream.get()).map_err(e)? {
        crate::merge::MergeOutcome::Conflicts(n) => Err(format!(
            "Pull stopped: {n} conflicted file(s). Resolve them in Changes, then commit (or abort the merge)."
        )),
        _ => Ok(()),
    }
}

/// Push the current branch, replacing what is on the remote even if it has
/// diverged (after amend, squash or rebase). `--force-with-lease` refuses if
/// the remote branch has moved since it was last fetched, so a teammate's
/// newer commits are never silently overwritten.
pub fn force_push(repo_path: &Path) -> Result<(), String> {
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
    if !has_upstream {
        return Err(format!("'{branch}' has no upstream branch yet; do a normal push first"));
    }
    run(repo_path, &["push", "--force-with-lease"])
}

/// Tip of the current branch's upstream, if it has one.
fn upstream_tip(repo: &Repository) -> Option<Oid> {
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    let b = repo.find_branch(head.shorthand()?, BranchType::Local).ok()?;
    b.upstream().ok()?.get().target()
}

/// Is `id` already on the upstream branch (so rewriting it means a force push)?
pub fn is_pushed(repo: &Repository, id: Oid) -> bool {
    upstream_tip(repo).is_some_and(|up| up == id || repo.graph_descendant_of(up, id).unwrap_or(false))
}

/// How many commits from `from` (inclusive) up to HEAD are already on the
/// upstream branch. Rewriting history from `from` replaces all of them.
pub fn pushed_count(repo_path: &Path, from: Oid) -> usize {
    let Ok(repo) = Repository::discover(repo_path) else { return 0 };
    if upstream_tip(&repo).is_none() {
        return 0;
    }
    let Ok(mut walk) = repo.revwalk() else { return 0 };
    let Ok(head) = repo.head().and_then(|h| h.peel_to_commit()) else { return 0 };
    if walk.push(head.id()).is_err() {
        return 0;
    }
    if let Ok(parent) = repo.find_commit(from).and_then(|c| c.parent_id(0)) {
        let _ = walk.hide(parent);
    }
    walk.flatten().filter(|id| is_pushed(&repo, *id)).count()
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

    fn gitcmd_run(path: &Path, args: &[&str]) {
        crate::gitcmd::run(Some(path), args).unwrap_or_else(|e| panic!("git {args:?}: {e}"));
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

        // Rewriting pushed history: a plain push is refused, a force push works,
        // and the lease stops it overwriting commits we haven't fetched.
        let b = Repository::open(&b_path).unwrap();
        let tip = b.head().unwrap().peel_to_commit().unwrap();
        assert!(is_pushed(&b, tip.id()), "b's tip is on the remote after pulling");
        assert_eq!(pushed_count(&b_path, tip.id()), 1);
        {
            // Rewrite b's last commit (amend) so it diverges from origin.
            let amended = tip.amend(Some("HEAD"), None, None, None, Some("rewritten"), None).unwrap();
            assert_ne!(amended, tip.id());
        }
        let b = Repository::open(&b_path).unwrap();
        assert!(!is_pushed(&b, b.head().unwrap().target().unwrap()));
        let plain = push(&b_path);
        assert!(plain.is_err(), "non-fast-forward is rejected: {plain:?}");
        force_push(&b_path).unwrap();
        let remote = Repository::open_bare(&bare).unwrap();
        let branch = b.head().unwrap().shorthand().unwrap().to_owned();
        assert_eq!(
            remote.find_branch(&branch, BranchType::Local).unwrap().get().peel_to_commit().unwrap().summary(),
            Some("rewritten")
        );
        // Someone else pushes; we have not fetched, so the lease refuses.
        commit(&a, "teammate");
        let a_branch = a.head().unwrap().shorthand().unwrap().to_owned();
        gitcmd_run(&a_path, &["pull", "--rebase"]);
        gitcmd_run(&a_path, &["push"]);
        let b = Repository::open(&b_path).unwrap();
        b.head().unwrap().peel_to_commit().unwrap().amend(Some("HEAD"), None, None, None, Some("again"), None).unwrap();
        let stale = force_push(&b_path);
        assert!(stale.is_err(), "lease must refuse when the remote moved: {stale:?}");
        let _ = a_branch;

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
