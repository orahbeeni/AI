//! Merging, conflict resolution and aborting.

use git2::{
    build::CheckoutBuilder, AnnotatedCommit, Reference, Repository, RepositoryState, ResetType,
    StatusOptions,
};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    UpToDate,
    FastForward,
    /// Merged cleanly and a merge commit was created.
    Merged,
    /// Stopped with this many conflicted files; resolve them, then commit.
    Conflicts(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

fn err(msg: impl AsRef<str>) -> git2::Error {
    git2::Error::from_str(msg.as_ref())
}

/// Refuse to start branch-moving operations in the middle of a merge.
pub fn ensure_clean_state(repo: &Repository) -> Result<(), git2::Error> {
    match repo.state() {
        RepositoryState::Clean => Ok(()),
        RepositoryState::Merge | RepositoryState::CherryPick | RepositoryState::Revert | RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge => Err(err("an operation is in progress; commit the resolution or abort it first")),
        other => Err(err(format!("repository is busy ({other:?}); finish or abort that operation first"))),
    }
}

/// Merge the branch named `name` (local, or `remote/branch`) into HEAD.
pub fn merge(repo_path: &Path, name: &str) -> Result<MergeOutcome, git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git merge {}", crate::gitlog::quote(name)));
    let repo = Repository::discover(repo_path)?;
    let reference = repo.resolve_reference_from_short_name(name)?;
    merge_reference(&repo, &reference)
}

pub(crate) fn merge_reference(repo: &Repository, reference: &Reference) -> Result<MergeOutcome, git2::Error> {
    ensure_clean_state(repo)?;
    let head = repo.head().map_err(|_| err("cannot merge before the first commit"))?;
    head.peel_to_commit()?;

    // Aborting a merge resets the working tree, so never start one on top of
    // uncommitted work to tracked files.
    let mut opts = StatusOptions::new();
    opts.include_untracked(false).include_ignored(false);
    if !repo.statuses(Some(&mut opts))?.is_empty() {
        return Err(err("commit or stash your changes before merging"));
    }

    let theirs: AnnotatedCommit = repo.reference_to_annotated_commit(reference)?;
    let (analysis, _) = repo.merge_analysis(&[&theirs])?;
    if analysis.is_up_to_date() {
        return Ok(MergeOutcome::UpToDate);
    }

    if analysis.is_fast_forward() {
        let target = repo.find_commit(theirs.id())?;
        crate::branches::checkout_safely(repo, target.as_object())?;
        match head.name().filter(|_| head.is_branch()) {
            Some(refname) => {
                repo.reference(refname, target.id(), true, "gitgoing: merge (fast-forward)")?;
            }
            None => repo.set_head_detached(target.id())?,
        }
        return Ok(MergeOutcome::FastForward);
    }
    if !analysis.is_normal() {
        return Err(err("these branches cannot be merged"));
    }

    let their_name = reference.shorthand().unwrap_or("branch");
    let kind = if reference.is_remote() { "remote-tracking branch" } else { "branch" };
    let into = head.shorthand().filter(|_| head.is_branch());
    let msg = match into {
        Some(b) => format!("Merge {kind} '{their_name}' into {b}"),
        None => format!("Merge {kind} '{their_name}'"),
    };

    let mut checkout = CheckoutBuilder::new();
    checkout.safe().allow_conflicts(true).conflict_style_merge(true);
    repo.merge(&[&theirs], None, Some(&mut checkout))?;

    let index = repo.index()?;
    if index.has_conflicts() {
        let n = index.conflicts()?.count();
        // Pre-fill the message the user will commit with.
        std::fs::write(repo.path().join("MERGE_MSG"), format!("{msg}\n")).map_err(|e| err(e.to_string()))?;
        return Ok(MergeOutcome::Conflicts(n));
    }

    // Conclude with the system git so commit hooks and signing apply. If a hook
    // refuses, the merge stays in progress with this message ready to commit.
    std::fs::write(repo.path().join("MERGE_MSG"), format!("{msg}\n")).map_err(|e| err(e.to_string()))?;
    repo.signature()?;
    crate::status::git_commit(repo.workdir().unwrap_or_else(|| repo.path()), &msg, false, true)?;
    Ok(MergeOutcome::Merged)
}

/// Abandon an in-progress merge, restoring HEAD's tree. Safe because merges
/// only start on a clean tracked working tree.
pub fn abort(repo_path: &Path) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, "git merge --abort   # or cherry-pick/revert --abort");
    let repo = Repository::discover(repo_path)?;
    if repo.state() == RepositoryState::Clean {
        return Err(err("no merge, cherry-pick, revert or rebase in progress"));
    }
    if crate::rebase::in_progress(&repo) {
        return crate::rebase::abort(repo_path).map_err(|e| err(&e));
    }
    let head = repo.head()?.peel_to_commit()?;
    repo.reset(head.as_object(), ResetType::Hard, Some(CheckoutBuilder::new().force()))?;
    repo.cleanup_state()
}

/// Resolve a conflicted file by taking one side wholesale.
pub fn resolve(repo_path: &Path, path: &str, side: Side) -> Result<(), git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git checkout --{} -- {} && git add -- {}", if side == Side::Ours { "ours" } else { "theirs" }, crate::gitlog::quote(path), crate::gitlog::quote(path)));
    let repo = Repository::discover(repo_path)?;
    let workdir = repo.workdir().ok_or_else(|| err("repository has no working directory"))?.to_path_buf();
    let mut index = repo.index()?;
    let conflict = index
        .conflicts()?
        .filter_map(Result::ok)
        .find(|c| {
            [&c.our, &c.their, &c.ancestor]
                .into_iter()
                .flatten()
                .any(|e| e.path == path.as_bytes())
        })
        .ok_or_else(|| err(format!("'{path}' is not conflicted")))?;
    let chosen = match side {
        Side::Ours => conflict.our,
        Side::Theirs => conflict.their,
    };
    let file = workdir.join(path);
    match chosen {
        Some(entry) => {
            std::fs::write(&file, repo.find_blob(entry.id)?.content()).map_err(|e| err(e.to_string()))?;
            #[cfg(unix)]
            {
                // Keep a script executable if that side had it so.
                use std::os::unix::fs::PermissionsExt;
                if entry.mode & 0o111 != 0 {
                    if let Ok(meta) = std::fs::metadata(&file) {
                        let mut perm = meta.permissions();
                        perm.set_mode(perm.mode() | 0o111);
                        let _ = std::fs::set_permissions(&file, perm);
                    }
                }
            }
            index.add_path(Path::new(path))?;
        }
        None => {
            // That side deleted the file.
            let _ = std::fs::remove_file(&file);
            index.conflict_remove(Path::new(path))?;
        }
    }
    index.write()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{branches, status};

    fn write_commit(p: &Path, file: &str, content: &str, msg: &str) {
        std::fs::write(p.join(file), content).unwrap();
        status::stage(p, &[file.into()]).unwrap();
        status::commit(p, msg).unwrap();
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
    fn fast_forward_and_clean_merge() {
        let (dir, main) = setup();
        let p = dir.path();
        branches::create_and_checkout(p, "feature").unwrap();
        write_commit(p, "b", "b\n", "feature work");
        branches::checkout(p, &main).unwrap();
        assert_eq!(merge(p, "feature").unwrap(), MergeOutcome::FastForward);
        assert!(p.join("b").exists());
        assert_eq!(merge(p, "feature").unwrap(), MergeOutcome::UpToDate);

        // Diverge on different files: clean merge commit with two parents.
        branches::create_and_checkout(p, "other").unwrap();
        write_commit(p, "c", "c\n", "other work");
        branches::checkout(p, &main).unwrap();
        write_commit(p, "d", "d\n", "main work");
        assert_eq!(merge(p, "other").unwrap(), MergeOutcome::Merged);
        let repo = Repository::open(p).unwrap();
        assert_eq!(repo.head().unwrap().peel_to_commit().unwrap().parent_count(), 2);
        assert_eq!(repo.state(), RepositoryState::Clean);
    }

    #[test]
    fn conflict_resolve_commit_and_abort() {
        let (dir, main) = setup();
        let p = dir.path();
        branches::create_and_checkout(p, "feature").unwrap();
        write_commit(p, "a", "theirs\n", "feature edit");
        branches::checkout(p, &main).unwrap();
        write_commit(p, "a", "ours\n", "main edit");

        // Dirty tracked files block a merge.
        std::fs::write(p.join("a"), "dirty\n").unwrap();
        assert!(merge(p, "feature").is_err());
        std::fs::write(p.join("a"), "ours\n").unwrap();

        assert_eq!(merge(p, "feature").unwrap(), MergeOutcome::Conflicts(1));
        let s = status::status(p).unwrap();
        assert_eq!(s.unstaged[0].kind, crate::diff::ChangeKind::Conflicted);
        assert!(s.merging);
        assert!(s.merge_message.unwrap().contains("Merge branch 'feature'"));
        assert!(status::commit(p, "too early").is_err(), "conflicts remain");
        assert!(branches::checkout(p, "feature").is_err(), "busy during merge");

        // Abort restores the pre-merge state.
        abort(p).unwrap();
        assert_eq!(std::fs::read_to_string(p.join("a")).unwrap(), "ours\n");
        assert!(!status::status(p).unwrap().merging);

        // Merge again, resolve with theirs, and commit a two-parent commit.
        assert_eq!(merge(p, "feature").unwrap(), MergeOutcome::Conflicts(1));
        resolve(p, "a", Side::Theirs).unwrap();
        assert_eq!(std::fs::read_to_string(p.join("a")).unwrap(), "theirs\n");
        status::commit(p, "Merge feature").unwrap();
        let repo = Repository::open(p).unwrap();
        assert_eq!(repo.head().unwrap().peel_to_commit().unwrap().parent_count(), 2);
        assert_eq!(repo.state(), RepositoryState::Clean);
    }
}
