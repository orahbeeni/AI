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
    Repository::discover(repo_path).map_err(e)?.remote_delete(name).map_err(e)
}

pub fn rename(repo_path: &Path, old: &str, new: &str) -> Result<(), String> {
    let new = new.trim();
    if !git2::Remote::is_valid_name(new) {
        return Err(format!("'{new}' is not a valid remote name"));
    }
    Repository::discover(repo_path).map_err(e)?.remote_rename(old, new).map(|_| ()).map_err(e)
}

pub fn set_url(repo_path: &Path, name: &str, url: &str) -> Result<(), String> {
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
    gitcmd::run(Some(repo_path), &["fetch", "--prune", remote]).map(|_| ())
}

/// Delete `branch` on `remote` (as `git push <remote> --delete <branch>`).
pub fn delete_remote_branch(repo_path: &Path, remote: &str, branch: &str) -> Result<(), String> {
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
