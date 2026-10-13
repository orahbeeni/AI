//! Operations on a chosen commit: cherry-pick, revert, reset, tag, detached checkout.

use git2::{
    build::CheckoutBuilder, CherrypickOptions, Commit, ObjectType, Oid, Repository, ResetType,
    RevertOptions, StatusOptions,
};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// Applied and committed.
    Committed,
    /// Stopped with this many conflicted files; resolve them, then commit.
    Conflicts(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    /// Move the branch only; keep index and working tree.
    Soft,
    /// Move the branch and reset the index; keep working-tree files.
    Mixed,
    /// Move the branch and discard all uncommitted changes.
    Hard,
}

fn err(msg: &str) -> git2::Error {
    git2::Error::from_str(msg)
}

/// Starting these operations on top of uncommitted tracked changes would make
/// aborting them destructive, so require a clean tracked working tree.
fn ensure_clean_tree(repo: &Repository) -> Result<(), git2::Error> {
    crate::merge::ensure_clean_state(repo)?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(false).include_ignored(false);
    if !repo.statuses(Some(&mut opts))?.is_empty() {
        return Err(err("commit or stash your changes first"));
    }
    Ok(())
}

fn finish(repo: &Repository, author_of: &Commit, message: &str) -> Result<Applied, git2::Error> {
    let mut index = repo.index()?;
    if index.has_conflicts() {
        return Ok(Applied::Conflicts(index.conflicts()?.count()));
    }
    let head = repo.head()?.peel_to_commit()?;
    let tree_id = index.write_tree()?;
    if tree_id == head.tree_id() {
        repo.cleanup_state()?;
        return Err(err("nothing to commit: these changes are already applied"));
    }
    let tree = repo.find_tree(tree_id)?;
    let committer = repo.signature()?;
    repo.commit(Some("HEAD"), &author_of.author(), &committer, message, &tree, &[&head])?;
    repo.cleanup_state()?;
    Ok(Applied::Committed)
}

pub fn cherry_pick(repo_path: &Path, id: Oid) -> Result<Applied, git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git cherry-pick {}", id));
    let repo = Repository::discover(repo_path)?;
    ensure_clean_tree(&repo)?;
    let commit = repo.find_commit(id)?;
    let mut opts = CherrypickOptions::new();
    if commit.parent_count() > 1 {
        opts.mainline(1);
    }
    let mut checkout = CheckoutBuilder::new();
    checkout.safe().allow_conflicts(true).conflict_style_merge(true);
    opts.checkout_builder(checkout);
    repo.cherrypick(&commit, Some(&mut opts))?;
    finish(&repo, &commit, commit.message().unwrap_or(""))
}

pub fn revert(repo_path: &Path, id: Oid) -> Result<Applied, git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git revert --no-edit {}", id));
    let repo = Repository::discover(repo_path)?;
    ensure_clean_tree(&repo)?;
    let commit = repo.find_commit(id)?;
    let mut opts = RevertOptions::new();
    if commit.parent_count() > 1 {
        opts.mainline(1);
    }
    let mut checkout = CheckoutBuilder::new();
    checkout.safe().allow_conflicts(true).conflict_style_merge(true);
    opts.checkout_builder(checkout);
    repo.revert(&commit, Some(&mut opts))?;
    let msg = format!(
        "Revert \"{}\"\n\nThis reverts commit {}.\n",
        commit.summary().unwrap_or(""),
        commit.id()
    );
    // The reverting commit is authored by the current user.
    let me = repo.signature()?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
        return Ok(Applied::Conflicts(index.conflicts()?.count()));
    }
    let head = repo.head()?.peel_to_commit()?;
    let tree_id = index.write_tree()?;
    if tree_id == head.tree_id() {
        repo.cleanup_state()?;
        return Err(err("nothing to commit: this commit is already reverted"));
    }
    let tree = repo.find_tree(tree_id)?;
    repo.commit(Some("HEAD"), &me, &me, &msg, &tree, &[&head])?;
    repo.cleanup_state()?;
    Ok(Applied::Committed)
}

pub fn reset(repo_path: &Path, id: Oid, mode: ResetMode) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git reset --{} {}", format!("{mode:?}").to_lowercase(), id));
    let repo = Repository::discover(repo_path)?;
    crate::merge::ensure_clean_state(&repo)?;
    let obj = repo.find_object(id, Some(ObjectType::Commit))?;
    let (kind, mut checkout) = match mode {
        ResetMode::Soft => (ResetType::Soft, None),
        ResetMode::Mixed => (ResetType::Mixed, None),
        ResetMode::Hard => {
            let mut c = CheckoutBuilder::new();
            c.force();
            (ResetType::Hard, Some(c))
        }
    };
    repo.reset(&obj, kind, checkout.as_mut())
}

/// Detach HEAD at a commit (safe checkout: refuses to overwrite local changes).
pub fn checkout_detached(repo_path: &Path, id: Oid) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git checkout --detach {}", id));
    let repo = Repository::discover(repo_path)?;
    crate::merge::ensure_clean_state(&repo)?;
    let commit = repo.find_commit(id)?;
    crate::branches::checkout_safely(&repo, commit.as_object())?;
    repo.set_head_detached(id)
}

/// Create a tag at a commit: annotated if `message` is non-empty, else lightweight.
pub fn create_tag(repo_path: &Path, name: &str, id: Oid, message: &str) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git tag {}{} {}", if message.trim().is_empty() { "" } else { "-a -m <message> " }, crate::gitlog::quote(name.trim()), id));
    let name = name.trim();
    if !git2::Reference::is_valid_name(&format!("refs/tags/{name}")) {
        return Err(err(&format!("'{name}' is not a valid tag name")));
    }
    let repo = Repository::discover(repo_path)?;
    let obj = repo.find_object(id, Some(ObjectType::Commit))?;
    if message.trim().is_empty() {
        repo.tag_lightweight(name, &obj, false)?;
    } else {
        repo.tag(name, &obj, &repo.signature()?, message.trim(), false)?;
    }
    Ok(())
}

pub fn delete_tag(repo_path: &Path, name: &str) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git tag -d {}", crate::gitlog::quote(name)));
    Repository::discover(repo_path)?.tag_delete(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{branches, status};

    fn write_commit(p: &Path, file: &str, content: &str, msg: &str) -> Oid {
        std::fs::write(p.join(file), content).unwrap();
        status::stage(p, &[file.into()]).unwrap();
        status::commit(p, msg).unwrap()
    }

    fn setup() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        write_commit(dir.path(), "a", "base\n", "base");
        let main = repo.head().unwrap().shorthand().unwrap().to_owned();
        (dir, main)
    }

    #[test]
    fn cherry_pick_revert_reset_tag() {
        let (dir, main) = setup();
        let p = dir.path();
        branches::create_and_checkout(p, "feature").unwrap();
        let picked = write_commit(p, "b", "b\n", "add b");
        branches::checkout(p, &main).unwrap();
        assert!(!p.join("b").exists());

        assert_eq!(cherry_pick(p, picked).unwrap(), Applied::Committed);
        assert!(p.join("b").exists());
        let repo = Repository::open(p).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.summary(), Some("add b"));
        assert_eq!(head.parent_count(), 1);

        assert_eq!(revert(p, head.id()).unwrap(), Applied::Committed);
        assert!(!p.join("b").exists());
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert!(head.summary().unwrap().starts_with("Revert"));

        // Tags.
        create_tag(p, "v1", head.id(), "").unwrap();
        create_tag(p, "v2", head.id(), "release").unwrap();
        assert!(create_tag(p, "bad name", head.id(), "").is_err());
        assert!(repo.find_reference("refs/tags/v2").is_ok());
        delete_tag(p, "v1").unwrap();
        assert!(repo.find_reference("refs/tags/v1").is_err());

        // Reset modes.
        let base = head.parent(0).unwrap().parent(0).unwrap().id();
        reset(p, base, ResetMode::Soft).unwrap();
        assert_eq!(repo.head().unwrap().target(), Some(base));
        reset(p, head.id(), ResetMode::Hard).unwrap();
        std::fs::write(p.join("a"), "dirty\n").unwrap();
        reset(p, head.id(), ResetMode::Hard).unwrap();
        assert_eq!(std::fs::read_to_string(p.join("a")).unwrap(), "base\n");

        checkout_detached(p, base).unwrap();
        assert!(repo.head_detached().unwrap());
    }

    #[test]
    fn cherry_pick_conflict_then_commit() {
        let (dir, main) = setup();
        let p = dir.path();
        branches::create_and_checkout(p, "feature").unwrap();
        let picked = write_commit(p, "a", "feature\n", "feature edit");
        branches::checkout(p, &main).unwrap();
        write_commit(p, "a", "main\n", "main edit");

        assert_eq!(cherry_pick(p, picked).unwrap(), Applied::Conflicts(1));
        let s = status::status(p).unwrap();
        assert_eq!(s.operation, Some("Cherry-pick"));
        crate::merge::resolve(p, "a", crate::merge::Side::Theirs).unwrap();
        status::commit(p, "feature edit").unwrap();
        let repo = Repository::open(p).unwrap();
        assert_eq!(repo.state(), git2::RepositoryState::Clean);
        assert_eq!(repo.head().unwrap().peel_to_commit().unwrap().parent_count(), 1);
    }
}
