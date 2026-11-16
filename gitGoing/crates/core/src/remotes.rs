//! Managing remotes, upstream tracking, and remote branches/tags.

use crate::gitcmd;
use git2::{Branch, BranchType, Repository};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteInfo {
    pub name: String,
    pub url: String,
}

fn e(err: git2::Error) -> String {
    err.message().to_owned()
}

pub fn list(repo: &Repository) -> Vec<RemoteInfo> {
    let Ok(names) = repo.remotes() else { return Vec::new() };
    names
        .iter()
        .flatten()
        .map(|n| RemoteInfo {
            name: n.to_owned(),
            url: repo.find_remote(n).ok().and_then(|r| r.url().map(str::to_owned)).unwrap_or_default(),
        })
        .collect()
}

pub fn add(repo_path: &Path, name: &str, url: &str) -> Result<(), String> {
    crate::gitlog::equiv(repo_path, format!("git remote add {} {}", crate::gitlog::quote(name.trim()), crate::gitlog::quote(url.trim())));
    let (name, url) = (name.trim(), url.trim());
    if url.is_empty() {
        return Err("enter a URL".into());
    }
    if !git2::Remote::is_valid_name(name) {
        return Err(format!("'{name}' is not a valid remote name"));
    }
    Repository::discover(repo_path).map_err(e)?.remote(name, url).map(|_| ()).map_err(e)
}

pub fn remove(repo_path: &Path, name: &str) -> Result<(), String> {
    crate::gitlog::equiv(repo_path, format!("git remote remove {}", crate::gitlog::quote(name)));
    Repository::discover(repo_path).map_err(e)?.remote_delete(name).map_err(e)
}

pub fn rename(repo_path: &Path, old: &str, new: &str) -> Result<(), String> {
    crate::gitlog::equiv(repo_path, format!("git remote rename {} {}", crate::gitlog::quote(old), crate::gitlog::quote(new.trim())));
    let new = new.trim();
    if !git2::Remote::is_valid_name(new) {
        return Err(format!("'{new}' is not a valid remote name"));
    }
    Repository::discover(repo_path).map_err(e)?.remote_rename(old, new).map(|_| ()).map_err(e)
}

pub fn set_url(repo_path: &Path, name: &str, url: &str) -> Result<(), String> {
    crate::gitlog::equiv(repo_path, format!("git remote set-url {} {}", crate::gitlog::quote(name), crate::gitlog::quote(url.trim())));
    let url = url.trim();
    if url.is_empty() {
        return Err("enter a URL".into());
    }
    Repository::discover(repo_path).map_err(e)?.remote_set_url(name, url).map_err(e)
}

/// `origin` if there is one, else the first remote.
pub fn default_remote(repo_path: &Path) -> Result<String, String> {
    let repo = Repository::discover(repo_path).map_err(e)?;
    let remotes = repo.remotes().map_err(e)?;
    if remotes.iter().flatten().any(|r| r == "origin") {
        return Ok("origin".into());
    }
    remotes.iter().flatten().next().map(str::to_owned).ok_or_else(|| "no remote configured".to_owned())
}

pub fn fetch_remote(repo_path: &Path, remote: &str) -> Result<(), String> {
    gitcmd::plain_name(remote)?;
    gitcmd::run(Some(repo_path), &["fetch", "--prune", remote]).map(|_| ())
}

/// Fetch the latest commits of one branch from its remote. `name` is a remote
/// branch (`origin/main`) or, with `local`, a local branch whose upstream is used.
pub fn fetch_branch(repo_path: &Path, name: &str, local: bool) -> Result<(), String> {
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    let (remote, branch) = if local {
        let b = repo.find_branch(name, BranchType::Local).map_err(|e| e.message().to_owned())?;
        let up = b.upstream().map_err(|_| format!("'{name}' has no upstream branch. Set one first."))?;
        let up_name = up.name().ok().flatten().unwrap_or_default().to_owned();
        split_remote_branch(&repo, &up_name)?
    } else {
        split_remote_branch(&repo, name)?
    };
    gitcmd::plain_name(&remote)?;
    gitcmd::plain_name(&branch)?;
    let spec = format!("+refs/heads/{branch}:refs/remotes/{remote}/{branch}");
    gitcmd::run(Some(repo_path), &["fetch", &remote, &spec]).map(|_| ())
}

/// How a local branch stands against its upstream after fetching.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchSync {
    /// Nothing to pull. `ahead` commits are local only (not pushed).
    UpToDate { ahead: usize },
    /// The branch was fast-forwarded by this many commits.
    FastForwarded { commits: usize },
    /// Both sides have new commits; merging or rebasing is needed.
    Diverged { ahead: usize, behind: usize },
    /// The branch is checked out and behind: only a pull can update it.
    BehindCurrent { behind: usize },
}

/// The result of [`fetch_and_update_branch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchFetch {
    /// The upstream, e.g. `origin/main`.
    pub upstream: String,
    /// The fetch brought something new (the remote-tracking branch moved).
    pub tracking_moved: bool,
    pub sync: BranchSync,
}

impl BranchFetch {
    /// A title and a sentence for the user saying what happened to `name`.
    pub fn message(&self, name: &str) -> (&'static str, String) {
        let up = &self.upstream;
        let pushed = |ahead: usize| if ahead > 0 { format!(" It has {ahead} local commit(s) not pushed yet.") } else { String::new() };
        match self.sync {
            BranchSync::FastForwarded { commits } => (
                "Fetched and Updated",
                format!("{name} moved forward {commits} commit(s) to match {up}."),
            ),
            BranchSync::UpToDate { ahead } if self.tracking_moved => {
                ("Fetched", format!("{up} has new commits, but {name} is not behind it.{}", pushed(ahead)))
            }
            BranchSync::UpToDate { ahead } => {
                ("Already Up-to-Date", format!("{name} has everything {up} has.{}", pushed(ahead)))
            }
            BranchSync::Diverged { ahead, behind } => (
                "Fetched, but Not Updated",
                format!(
                    "{name} has diverged from {up}: {ahead} commit(s) of its own and {behind} new one(s) on the remote, so it can't be fast-forwarded. Check it out and pull to merge or rebase."
                ),
            ),
            BranchSync::BehindCurrent { behind } => (
                "Fetched, but Not Updated",
                format!("{up} is {behind} commit(s) ahead of the checked-out branch {name}. Use Pull to bring them in."),
            ),
        }
    }
}

/// Fetch a local branch's upstream, then fast-forward the branch to it when that is
/// possible without touching the working tree. A branch that has diverged, or is checked
/// out, is left alone; the result says which, so the caller can explain.
pub fn fetch_and_update_branch(repo_path: &Path, name: &str) -> Result<BranchFetch, String> {
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    let b = repo.find_branch(name, BranchType::Local).map_err(|e| e.message().to_owned())?;
    let up = b.upstream().map_err(|_| format!("'{name}' has no upstream branch. Set one first."))?;
    let up_name = up.name().ok().flatten().unwrap_or_default().to_owned();
    let (remote, branch) = split_remote_branch(&repo, &up_name)?;
    gitcmd::plain_name(&remote)?;
    gitcmd::plain_name(&branch)?;
    gitcmd::plain_name(name)?;
    let is_head = b.is_head();
    drop((b, up));

    // Fetch first so the upstream is what the remote has now, not what it had at the last fetch.
    let tracking = format!("refs/remotes/{remote}/{branch}");
    let tracking_id = |repo: &Repository| repo.find_reference(&tracking).ok().and_then(|r| r.target());
    let before = tracking_id(&repo);
    gitcmd::run(Some(repo_path), &["fetch", &remote, &format!("+refs/heads/{branch}:{tracking}")])?;
    let tracking_moved = tracking_id(&repo) != before;

    let local = repo.find_branch(name, BranchType::Local).map_err(|e| e.message().to_owned())?;
    let local_id = local.get().peel_to_commit().map_err(|e| e.message().to_owned())?.id();
    let up_id = repo
        .find_reference(&tracking)
        .and_then(|r| r.peel_to_commit())
        .map_err(|e| e.message().to_owned())?
        .id();
    let (ahead, behind) = repo.graph_ahead_behind(local_id, up_id).map_err(|e| e.message().to_owned())?;
    let done = |sync| Ok(BranchFetch { upstream: up_name.clone(), tracking_moved, sync });
    if behind == 0 {
        return done(BranchSync::UpToDate { ahead }); // level with, or ahead of, the upstream
    }
    if ahead > 0 {
        return done(BranchSync::Diverged { ahead, behind });
    }
    if is_head {
        return done(BranchSync::BehindCurrent { behind });
    }
    // A fetch from this very repository moves the branch the way `git pull --ff-only` would,
    // and git itself refuses if the branch is checked out in another worktree.
    gitcmd::run(Some(repo_path), &["fetch", ".", &format!("{tracking}:refs/heads/{name}")])?;
    done(BranchSync::FastForwarded { commits: behind })
}

/// Bring local branch `name` up to its upstream without checking it out. Only a
/// fast-forward is ever done: a branch that has diverged is refused and left untouched,
/// and one that is merely ahead of (or level with) its upstream has nothing to pull.
pub fn pull_other_branch(repo_path: &Path, name: &str) -> Result<(), String> {
    let r = fetch_and_update_branch(repo_path, name)?;
    match r.sync {
        BranchSync::Diverged { ahead, behind } => Err(format!(
            "'{name}' has diverged from {} ({ahead} ahead, {behind} behind), so it can't be fast-forwarded. Check it out and pull to merge or rebase.",
            r.upstream
        )),
        BranchSync::BehindCurrent { .. } => Err(format!("'{name}' is checked out; use Pull to update it.")),
        BranchSync::UpToDate { .. } | BranchSync::FastForwarded { .. } => Ok(()),
    }
}

/// Split `origin/feature/x` into (`origin`, `feature/x`), allowing remote names with slashes.
fn split_remote_branch(repo: &Repository, full: &str) -> Result<(String, String), String> {
    let remotes = repo.remotes().map_err(|e| e.message().to_owned())?;
    remotes
        .iter()
        .flatten()
        .filter_map(|r| full.strip_prefix(r).and_then(|rest| rest.strip_prefix('/')).map(|b| (r.to_owned(), b.to_owned())))
        .max_by_key(|(r, _)| r.len())
        .ok_or_else(|| format!("'{full}' doesn't belong to a known remote"))
}

/// Delete `branch` on `remote` (as `git push <remote> --delete <branch>`).
pub fn delete_remote_branch(repo_path: &Path, remote: &str, branch: &str) -> Result<(), String> {
    gitcmd::plain_name(remote)?;
    gitcmd::plain_name(branch)?;
    gitcmd::run(Some(repo_path), &["push", remote, "--delete", branch]).map(|_| ())
}

/// Split `origin/feature/x` into (`origin`, `feature/x`) using the repository's remote names.
pub fn parse_remote_branch(repo_path: &Path, full: &str) -> Option<(String, String)> {
    split_remote_branch(&Repository::discover(repo_path).ok()?, full).ok()
}

/// The remote branch a local branch tracks, as (remote, branch name on the remote).
pub fn tracked_remote_branch(repo_path: &Path, local: &str) -> Option<(String, String)> {
    let repo = Repository::discover(repo_path).ok()?;
    let up = repo.find_branch(local, BranchType::Local).ok()?.upstream().ok()?;
    let full = up.name().ok().flatten()?.to_owned();
    split_remote_branch(&repo, &full).ok()
}

/// Rename `old` to `new` on `remote`: publish the commit under the new name, then delete
/// the old one (git has no rename for remote branches). Local branches that tracked
/// `remote/old` are pointed at `remote/new`. Nothing is forced: if `new` already exists
/// on the remote with different history, git refuses and the old branch is left alone.
pub fn rename_remote_branch(repo_path: &Path, remote: &str, old: &str, new: &str) -> Result<String, String> {
    gitcmd::plain_name(remote)?;
    gitcmd::plain_name(old)?;
    let new = crate::branches::sanitize_name(new);
    gitcmd::plain_name(&new)?;
    if !Branch::name_is_valid(&new).unwrap_or(false) {
        return Err(format!("'{new}' is not a valid branch name"));
    }
    if new == old {
        return Err("that is already the branch's name".to_owned());
    }
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    if repo.find_reference(&format!("refs/remotes/{remote}/{old}")).is_err() {
        return Err(format!("{remote}/{old} is not known here; fetch first"));
    }
    // Local branches that follow the old name (found now: deleting it removes the tracking ref).
    let old_tracking = format!("{remote}/{old}");
    let followers: Vec<String> = repo
        .branches(Some(BranchType::Local))
        .map_err(|e| e.message().to_owned())?
        .flatten()
        .filter(|(b, _)| b.upstream().ok().and_then(|u| u.name().ok().flatten().map(str::to_owned)).as_deref() == Some(old_tracking.as_str()))
        .filter_map(|(b, _)| b.name().ok().flatten().map(str::to_owned))
        .collect();
    drop(repo);
    gitcmd::run(Some(repo_path), &["push", remote, &format!("refs/remotes/{remote}/{old}:refs/heads/{new}")])?;
    if let Err(e) = gitcmd::run(Some(repo_path), &["push", remote, "--delete", old]) {
        return Err(format!(
            "'{new}' was created on {remote}, but the old branch '{old}' could not be deleted there: {e}"
        ));
    }
    // They follow the new one.
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    for name in followers {
        if let Ok(mut b) = repo.find_branch(&name, BranchType::Local) {
            let _ = b.set_upstream(Some(&format!("{remote}/{new}")));
        }
    }
    Ok(new)
}

pub fn push_tag(repo_path: &Path, tag: &str) -> Result<(), String> {
    let remote = default_remote(repo_path)?;
    gitcmd::run(Some(repo_path), &["push", &remote, &format!("refs/tags/{tag}")]).map(|_| ())
}

pub fn delete_remote_tag(repo_path: &Path, tag: &str) -> Result<(), String> {
    let remote = default_remote(repo_path)?;
    gitcmd::run(Some(repo_path), &["push", &remote, "--delete", &format!("refs/tags/{tag}")]).map(|_| ())
}

/// Make `local` track `upstream` (`remote/branch`), or stop tracking with `None`.
pub fn set_upstream(repo_path: &Path, local: &str, upstream: Option<&str>) -> Result<(), String> {
    crate::gitlog::equiv(repo_path, match upstream { Some(u) => format!("git branch --set-upstream-to={} {}", crate::gitlog::quote(u), crate::gitlog::quote(local)), None => format!("git branch --unset-upstream {}", crate::gitlog::quote(local)) });
    let repo = Repository::discover(repo_path).map_err(e)?;
    let mut b: Branch = repo.find_branch(local, BranchType::Local).map_err(e)?;
    b.set_upstream(upstream).map_err(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;

    #[test]
    fn manage_remotes_and_upstream() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("o.git");
        Repository::init_bare(&bare).unwrap();
        let p = tmp.path().join("w");
        let repo = Repository::init(&p).unwrap();
        let sig = Signature::now("T", "t@example.com").unwrap();
        let tree = repo.find_tree(repo.treebuilder(None).unwrap().write().unwrap()).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &[]).unwrap();

        assert!(add(&p, "bad name", "x").is_err());
        assert!(add(&p, "origin", "").is_err());
        add(&p, "origin", bare.to_str().unwrap()).unwrap();
        assert_eq!(default_remote(&p).unwrap(), "origin");
        assert_eq!(list(&Repository::open(&p).unwrap())[0].url, bare.to_str().unwrap());

        rename(&p, "origin", "up").unwrap();
        assert_eq!(default_remote(&p).unwrap(), "up");
        set_url(&p, "up", "https://example.com/r.git").unwrap();
        assert_eq!(list(&Repository::open(&p).unwrap())[0].url, "https://example.com/r.git");
        set_url(&p, "up", bare.to_str().unwrap()).unwrap();

        if gitcmd::run(None, &["--version"]).is_ok() {
            let branch = repo.head().unwrap().shorthand().unwrap().to_owned();
            gitcmd::run(Some(&p), &["push", "up", &branch]).unwrap();
            gitcmd::run(Some(&p), &["push", "up", &format!("{branch}:topic")]).unwrap();
            fetch_remote(&p, "up").unwrap();

            set_upstream(&p, &branch, Some("up/topic")).unwrap();
            let r = Repository::open(&p).unwrap();
            let up = r.find_branch(&branch, BranchType::Local).unwrap().upstream().unwrap();
            assert_eq!(up.name().unwrap(), Some("up/topic"));
            set_upstream(&p, &branch, None).unwrap();
            assert!(Repository::open(&p).unwrap().find_branch(&branch, BranchType::Local).unwrap().upstream().is_err());

            delete_remote_branch(&p, "up", "topic").unwrap();
            let bare_repo = Repository::open_bare(&bare).unwrap();
            assert!(bare_repo.find_branch("topic", BranchType::Local).is_err());

            // Tags.
            let head = Repository::open(&p).unwrap().head().unwrap().peel_to_commit().unwrap().id();
            crate::commit_ops::create_tag(&p, "v1", head, "").unwrap();
            push_tag(&p, "v1").unwrap();
            assert!(bare_repo.find_reference("refs/tags/v1").is_ok());
            delete_remote_tag(&p, "v1").unwrap();
            assert!(bare_repo.find_reference("refs/tags/v1").is_err());
        }

        remove(&p, "up").unwrap();
        assert!(list(&Repository::open(&p).unwrap()).is_empty());
    }

    #[test]
    fn fetching_a_branch_updates_it_when_it_can_and_says_why_when_it_cannot() {
        if gitcmd::run(None, &["--version"]).is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("o.git");
        let g = |dir: &Path, args: &[&str]| gitcmd::run(Some(dir), args).unwrap();
        gitcmd::run(None, &["init", "-q", "--bare", bare.to_str().unwrap()]).unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        gitcmd::run(None, &["clone", "-q", bare.to_str().unwrap(), a.to_str().unwrap()]).unwrap();
        for d in [&a] {
            g(d, &["config", "user.email", "t@example.com"]);
            g(d, &["config", "user.name", "T"]);
            g(d, &["config", "commit.gpgsign", "false"]);
        }
        g(&a, &["checkout", "-q", "-b", "main"]);
        let commit = |d: &Path, f: &str| {
            std::fs::write(d.join(f), f).unwrap();
            g(d, &["add", "."]);
            g(d, &["commit", "-q", "-m", f]);
        };
        commit(&a, "one");
        g(&a, &["push", "-q", "-u", "origin", "main"]);
        g(&a, &["branch", "other"]);
        g(&a, &["push", "-q", "-u", "origin", "other"]);
        gitcmd::run(None, &["clone", "-q", bare.to_str().unwrap(), b.to_str().unwrap()]).unwrap();
        g(&b, &["config", "user.email", "t@example.com"]);
        g(&b, &["config", "user.name", "T"]);
        g(&b, &["config", "commit.gpgsign", "false"]);
        g(&b, &["checkout", "-q", "other"]);
        let tip = |d: &Path, n: &str| g(d, &["rev-parse", n]).trim().to_owned();

        // Nothing new.
        let r = fetch_and_update_branch(&a, "other").unwrap();
        assert_eq!(r.sync, BranchSync::UpToDate { ahead: 0 });
        assert_eq!(r.message("other").0, "Already Up-to-Date");

        // Someone pushes: the branch is fast-forwarded by the fetch itself.
        commit(&b, "two");
        g(&b, &["push", "-q", "origin", "other"]);
        let r = fetch_and_update_branch(&a, "other").unwrap();
        assert_eq!(r.sync, BranchSync::FastForwarded { commits: 1 });
        assert!(r.tracking_moved);
        assert_eq!(tip(&a, "other"), tip(&b, "other"));

        // Diverged: refreshed remote-side, branch untouched, and the message says why.
        g(&a, &["checkout", "-q", "other"]);
        commit(&a, "mine");
        g(&a, &["checkout", "-q", "main"]);
        commit(&b, "three");
        g(&b, &["push", "-q", "origin", "other"]);
        let mine = tip(&a, "other");
        let r = fetch_and_update_branch(&a, "other").unwrap();
        assert_eq!(r.sync, BranchSync::Diverged { ahead: 1, behind: 1 });
        assert_eq!(tip(&a, "other"), mine);
        let (title, text) = r.message("other");
        assert_eq!(title, "Fetched, but Not Updated");
        assert!(text.contains("diverged") && text.contains("origin/other"), "{text}");
        // Fetching again finds nothing new, but the explanation stays true.
        let again = fetch_and_update_branch(&a, "other").unwrap();
        assert!(!again.tracking_moved && matches!(again.sync, BranchSync::Diverged { .. }));

        // The checked-out branch is never moved by a fetch.
        g(&b, &["checkout", "-q", "main"]);
        commit(&b, "four");
        g(&b, &["push", "-q", "origin", "main"]);
        let r = fetch_and_update_branch(&a, "main").unwrap();
        assert_eq!(r.sync, BranchSync::BehindCurrent { behind: 1 });
        assert!(r.message("main").1.contains("Pull"));
    }

    #[test]
    fn pulling_another_branch_only_fast_forwards() {
        if gitcmd::run(None, &["--version"]).is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("o.git");
        let g = |dir: &Path, args: &[&str]| gitcmd::run(Some(dir), args).unwrap();
        gitcmd::run(None, &["init", "-q", "--bare", bare.to_str().unwrap()]).unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        gitcmd::run(None, &["clone", "-q", bare.to_str().unwrap(), a.to_str().unwrap()]).unwrap();
        for d in [&a] {
            g(d, &["config", "user.email", "t@example.com"]);
            g(d, &["config", "user.name", "T"]);
        }
        g(&a, &["checkout", "-q", "-b", "main"]);
        let commit = |d: &Path, f: &str| {
            std::fs::write(d.join(f), f).unwrap();
            g(d, &["add", "."]);
            g(d, &["commit", "-q", "-m", f]);
        };
        commit(&a, "one");
        g(&a, &["push", "-q", "-u", "origin", "main"]);
        g(&a, &["branch", "other"]);
        g(&a, &["push", "-q", "-u", "origin", "other"]);
        gitcmd::run(None, &["clone", "-q", bare.to_str().unwrap(), b.to_str().unwrap()]).unwrap();
        g(&b, &["config", "user.email", "t@example.com"]);
        g(&b, &["config", "user.name", "T"]);
        g(&b, &["checkout", "-q", "other"]);
        let tip = |d: &Path, n: &str| g(d, &["rev-parse", n]).trim().to_owned();

        // Someone else pushes to `other`; in `a`, `other` is behind and `main` is checked out.
        commit(&b, "two");
        g(&b, &["push", "-q", "origin", "other"]);
        pull_other_branch(&a, "other").unwrap();
        assert_eq!(tip(&a, "other"), tip(&b, "other"));
        assert_eq!(g(&a, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main");

        // Nothing new: fine, and nothing moves.
        let before = tip(&a, "other");
        pull_other_branch(&a, "other").unwrap();
        assert_eq!(tip(&a, "other"), before);

        // Local commits only (ahead): nothing to pull, and it must not be called diverged.
        g(&a, &["checkout", "-q", "other"]);
        commit(&a, "three");
        g(&a, &["checkout", "-q", "main"]);
        let ahead = tip(&a, "other");
        pull_other_branch(&a, "other").unwrap();
        assert_eq!(tip(&a, "other"), ahead);

        // Both sides moved: refused, left as it was.
        commit(&b, "four");
        g(&b, &["push", "-q", "origin", "other"]);
        let err = pull_other_branch(&a, "other").unwrap_err();
        assert!(err.contains("diverged"), "{err}");
        assert_eq!(tip(&a, "other"), ahead);

        // The checked-out branch can't be fast-forwarded this way, git refuses it.
        assert!(pull_other_branch(&a, "main").is_ok());
        g(&a, &["checkout", "-q", "other"]);
        assert!(pull_other_branch(&a, "other").is_err());
    }
    #[test]
    fn rename_a_branch_on_the_remote_and_follow_it_locally() {
        let remote_dir = tempfile::tempdir().unwrap();
        Repository::init_bare(remote_dir.path()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let repo = Repository::init(p).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        cfg.set_bool("commit.gpgsign", false).unwrap();
        std::fs::write(p.join("a"), "1").unwrap();
        crate::status::stage(p, &["a".into()]).unwrap();
        crate::status::commit(p, "c").unwrap();
        add(p, "origin", remote_dir.path().to_str().unwrap()).unwrap();
        crate::branches::create_and_checkout(p, "feature").unwrap();
        crate::remote::push(p).unwrap(); // publishes feature and sets its upstream
        assert_eq!(tracked_remote_branch(p, "feature"), Some(("origin".into(), "feature".into())));

        crate::branches::rename(p, "feature", "feature-2").unwrap();
        // The upstream still names the old remote branch.
        assert_eq!(tracked_remote_branch(p, "feature-2"), Some(("origin".into(), "feature".into())));
        assert!(rename_remote_branch(p, "origin", "feature", "bad..name").is_err());
        rename_remote_branch(p, "origin", "feature", "feature-2").unwrap();

        let remote = Repository::open_bare(remote_dir.path()).unwrap();
        assert!(remote.find_reference("refs/heads/feature-2").is_ok());
        assert!(remote.find_reference("refs/heads/feature").is_err(), "old name is gone on the remote");
        assert!(repo.find_reference("refs/remotes/origin/feature").is_err());
        assert_eq!(tracked_remote_branch(p, "feature-2"), Some(("origin".into(), "feature-2".into())));
    }

}
