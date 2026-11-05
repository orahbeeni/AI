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

/// Bring local branch `name` up to its upstream without checking it out: fetch the
/// upstream branch, then fast-forward `name` to it. Only a fast-forward is ever done:
/// a branch that has diverged is refused and left untouched, and one that is merely
/// ahead of (or level with) its upstream has nothing to pull and is left as it is.
pub fn pull_other_branch(repo_path: &Path, name: &str) -> Result<(), String> {
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    let b = repo.find_branch(name, BranchType::Local).map_err(|e| e.message().to_owned())?;
    let up = b.upstream().map_err(|_| format!("'{name}' has no upstream branch. Set one first."))?;
    let up_name = up.name().ok().flatten().unwrap_or_default().to_owned();
    let (remote, branch) = split_remote_branch(&repo, &up_name)?;
    gitcmd::plain_name(&remote)?;
    gitcmd::plain_name(&branch)?;
    gitcmd::plain_name(name)?;
    drop((b, up));

    // Fetch first so the upstream is what the remote has now, not what it had at the last fetch.
    let tracking = format!("refs/remotes/{remote}/{branch}");
    gitcmd::run(Some(repo_path), &["fetch", &remote, &format!("+refs/heads/{branch}:{tracking}")])?;

    let local = repo.find_branch(name, BranchType::Local).map_err(|e| e.message().to_owned())?;
    let local_id = local.get().peel_to_commit().map_err(|e| e.message().to_owned())?.id();
    let up_id = repo
        .find_reference(&tracking)
        .and_then(|r| r.peel_to_commit())
        .map_err(|e| e.message().to_owned())?
        .id();
    let (ahead, behind) = repo.graph_ahead_behind(local_id, up_id).map_err(|e| e.message().to_owned())?;
    if behind == 0 {
        return Ok(()); // level with, or ahead of, the upstream: nothing to bring in
    }
    if ahead > 0 {
        return Err(format!(
            "'{name}' has diverged from {up_name} ({ahead} ahead, {behind} behind), so it can't be fast-forwarded. Check it out and pull to merge or rebase."
        ));
    }
    // A fetch from this very repository moves the branch the way `git pull --ff-only` would,
    // and git itself refuses if the branch is checked out (here or in another worktree).
    gitcmd::run(Some(repo_path), &["fetch", ".", &format!("{tracking}:refs/heads/{name}")]).map(|_| ())
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
}
