//! Branch operations: checkout, create, delete.

use git2::{build::CheckoutBuilder, Branch, BranchType, Repository};
use std::path::Path;

fn err(msg: impl AsRef<str>) -> git2::Error {
    git2::Error::from_str(msg.as_ref())
}

fn checkout_local(repo: &Repository, name: &str) -> Result<(), git2::Error> {
    crate::merge::ensure_clean_state(repo)?;
    let refname = format!("refs/heads/{name}");
    let obj = repo.revparse_single(&refname)?;
    // Safe mode refuses to overwrite local modifications.
    repo.checkout_tree(&obj, Some(CheckoutBuilder::new().safe()))?;
    repo.set_head(&refname)
}

pub fn checkout(repo_path: &Path, name: &str) -> Result<(), git2::Error> {
    checkout_local(&Repository::discover(repo_path)?, name)
}

/// Check out `remote/branch` as a local branch, creating it (tracking the
/// remote) if it doesn't exist yet.
pub fn checkout_remote(repo_path: &Path, full_name: &str) -> Result<(), git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let local = full_name
        .split_once('/')
        .map(|(_, b)| b)
        .ok_or_else(|| err("not a remote branch name"))?;
    if repo.find_branch(local, BranchType::Local).is_err() {
        let remote = repo.find_branch(full_name, BranchType::Remote)?;
        let commit = remote.get().peel_to_commit()?;
        let mut b = repo.branch(local, &commit, false)?;
        b.set_upstream(Some(full_name))?;
    }
    checkout_local(&repo, local)
}

/// Create a branch at HEAD and check it out.
pub fn create_and_checkout(repo_path: &Path, name: &str) -> Result<(), git2::Error> {
    let name = name.trim();
    if !Branch::name_is_valid(name)? {
        return Err(err(format!("'{name}' is not a valid branch name")));
    }
    let repo = Repository::discover(repo_path)?;
    crate::merge::ensure_clean_state(&repo)?;
    let head = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(|_| err("cannot create a branch before the first commit"))?;
    repo.branch(name, &head, false)?;
    checkout_local(&repo, name)
}

/// Create a branch at an arbitrary commit, optionally checking it out.
pub fn create_at(repo_path: &Path, name: &str, id: git2::Oid, checkout: bool) -> Result<(), git2::Error> {
    let name = name.trim();
    if !Branch::name_is_valid(name)? {
        return Err(err(format!("'{name}' is not a valid branch name")));
    }
    let repo = Repository::discover(repo_path)?;
    crate::merge::ensure_clean_state(&repo)?;
    repo.branch(name, &repo.find_commit(id)?, false)?;
    if checkout {
        checkout_local(&repo, name)?;
    }
    Ok(())
}

pub fn delete(repo_path: &Path, name: &str) -> Result<(), git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let mut b = repo.find_branch(name, BranchType::Local)?;
    if b.is_head() {
        return Err(err("cannot delete the checked-out branch"));
    }
    b.delete()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_checkout_delete() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let repo = Repository::init(p).unwrap();
        let sig = git2::Signature::now("T", "t@example.com").unwrap();
        std::fs::write(p.join("a"), "1").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("a")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &[]).unwrap();
        let first = repo.head().unwrap().shorthand().unwrap().to_owned();

        assert!(create_and_checkout(p, "bad name").is_err());
        create_and_checkout(p, "feature").unwrap();
        assert_eq!(repo.head().unwrap().shorthand(), Some("feature"));
        assert!(delete(p, "feature").is_err(), "checked-out branch");
        checkout(p, &first).unwrap();
        delete(p, "feature").unwrap();
        assert!(repo.find_branch("feature", BranchType::Local).is_err());
    }
}
