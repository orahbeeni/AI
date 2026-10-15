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
/// upstream branch straight into it. Git only allows this as a fast-forward, so a
/// branch that has diverged is refused and left untouched.
pub fn pull_other_branch(repo_path: &Path, name: &str) -> Result<(), String> {
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    let b = repo.find_branch(name, BranchType::Local).map_err(|e| e.message().to_owned())?;
    let up = b.upstream().map_err(|_| format!("'{name}' has no upstream branch. Set one first."))?;
    let up_name = up.name().ok().flatten().unwrap_or_default().to_owned();
    let (remote, branch) = split_remote_branch(&repo, &up_name)?;
    gitcmd::plain_name(&remote)?;
    gitcmd::plain_name(&branch)?;
    gitcmd::plain_name(name)?;
    let spec = format!("refs/heads/{branch}:refs/heads/{name}");
    gitcmd::run(Some(repo_path), &["fetch", &remote, &spec]).map(|_| ()).map_err(|e| {
        if e.to_lowercase().contains("non-fast-forward") || e.contains("rejected") {
            format!("'{name}' has diverged from {up_name}, so it can't be fast-forwarded. Check it out and pull to merge or rebase.")
        } else {
            e
        }
    })
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
}
