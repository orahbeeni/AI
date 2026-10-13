//! Branch operations: checkout, create, delete.

use git2::{
    build::CheckoutBuilder,
    Branch, BranchType, Error, ErrorCode, Repository,
};
use std::path::Path;

fn err(msg: impl AsRef<str>) -> git2::Error {
    git2::Error::from_str(msg.as_ref())
}

/// Check out `obj`'s tree, refusing to overwrite local modifications. When that
/// is what stops it, the error says which files are in the way (and has
/// [`ErrorCode::Conflict`], see [`blocked_by_local_changes`]) instead of just
/// "1 conflict prevents checkout".
pub(crate) fn checkout_safely(repo: &Repository, obj: &git2::Object) -> Result<(), git2::Error> {
    let mut in_the_way: Vec<String> = Vec::new();
    let result = {
        let mut co = CheckoutBuilder::new();
        co.safe().notify_on(git2::CheckoutNotificationType::CONFLICT).notify(|_, path, _, _, _| {
            if let Some(p) = path {
                in_the_way.push(p.display().to_string());
            }
            true
        });
        repo.checkout_tree(obj, Some(&mut co))
    };
    match result {
        Err(e) if e.code() == ErrorCode::Conflict => {
            in_the_way.sort();
            in_the_way.dedup();
            let list = match in_the_way.len() {
                0 => "some files".to_owned(),
                1..=6 => in_the_way.join(", "),
                n => format!("{}, and {} more", in_the_way[..6].join(", "), n - 6),
            };
            Err(Error::new(
                ErrorCode::Conflict,
                e.class(),
                format!(
                    "Your uncommitted changes to {list} would be overwritten by this checkout. \
                     Commit, stash or discard them first."
                ),
            ))
        }
        other => other,
    }
}

/// Did a checkout fail only because uncommitted changes are in the way?
pub fn blocked_by_local_changes(e: &git2::Error) -> bool {
    e.code() == ErrorCode::Conflict
}

fn checkout_local(repo: &Repository, name: &str) -> Result<(), git2::Error> {
    crate::merge::ensure_clean_state(repo)?;
    let refname = format!("refs/heads/{name}");
    let obj = repo.revparse_single(&refname)?;
    checkout_safely(repo, &obj)?;
    repo.set_head(&refname)
}

pub fn checkout(repo_path: &Path, name: &str) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git checkout {}", crate::gitlog::quote(name)));
    checkout_local(&Repository::discover(repo_path)?, name)
}

/// Check out the branch we just created; if that fails (for example local
/// changes would be overwritten) remove it again, so a failed attempt leaves
/// nothing behind.
fn checkout_new(repo: &Repository, name: &str) -> Result<(), git2::Error> {
    checkout_local(repo, name).inspect_err(|_| {
        if let Ok(mut b) = repo.find_branch(name, BranchType::Local) {
            let _ = b.delete();
        }
    })
}

/// Check out `remote/branch` as a local branch, creating it (tracking the
/// remote) if it doesn't exist yet.
pub fn checkout_remote(repo_path: &Path, full_name: &str) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git checkout --track {}", crate::gitlog::quote(full_name)));
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
        return checkout_new(&repo, local);
    }
    checkout_local(&repo, local)
}

/// Create a branch at HEAD and check it out.
pub fn create_and_checkout(repo_path: &Path, name: &str) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git checkout -b {}", crate::gitlog::quote(name.trim())));
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
    checkout_new(&repo, name)
}

/// Create a branch at an arbitrary commit, optionally checking it out.
pub fn create_at(repo_path: &Path, name: &str, id: git2::Oid, checkout: bool) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git branch {} {}{}", crate::gitlog::quote(name.trim()), id, if checkout { " && git checkout " } else { "" }));
    let name = name.trim();
    if !Branch::name_is_valid(name)? {
        return Err(err(format!("'{name}' is not a valid branch name")));
    }
    let repo = Repository::discover(repo_path)?;
    crate::merge::ensure_clean_state(&repo)?;
    repo.branch(name, &repo.find_commit(id)?, false)?;
    if checkout {
        checkout_new(&repo, name)?;
    }
    Ok(())
}

pub fn delete(repo_path: &Path, name: &str) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git branch -D {}", crate::gitlog::quote(name)));
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

    #[test]
    fn failed_checkout_leaves_no_new_branch_behind() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let repo = Repository::init(p).unwrap();
        let sig = git2::Signature::now("T", "t@example.com").unwrap();
        let commit = |content: &str, parents: &[&git2::Commit], update: Option<&str>| {
            std::fs::write(p.join("a"), content).unwrap();
            let mut idx = repo.index().unwrap();
            idx.add_path(Path::new("a")).unwrap();
            idx.write().unwrap();
            let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
            repo.commit(update, &sig, &sig, content, &tree, parents).unwrap()
        };
        let first = commit("1", &[], Some("HEAD"));
        let first = repo.find_commit(first).unwrap();
        // A commit that changes `a`, not on any branch yet.
        let second = commit("2", &[&first], None);
        std::fs::write(p.join("a"), "1").unwrap();
        // Uncommitted edit to the same file: checking out `second` would clobber it.
        std::fs::write(p.join("a"), "my local edit").unwrap();

        let err = create_at(p, "new-branch", second, true).unwrap_err();
        assert!(blocked_by_local_changes(&err));
        assert!(err.message().contains('a') && err.message().contains("uncommitted changes"), "{}", err.message());
        assert!(repo.find_branch("new-branch", BranchType::Local).is_err(), "failed attempt cleaned up");
        assert_eq!(std::fs::read_to_string(p.join("a")).unwrap(), "my local edit", "work untouched");
    }
}
