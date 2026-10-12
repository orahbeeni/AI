//! Working-tree status, staging, and committing.

use crate::diff::{diff_to_file_diff, ChangeKind, FileDiff};
use git2::{DiffOptions, ObjectType, Oid, Repository, Status, StatusOptions};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub kind: ChangeKind,
}

#[derive(Clone, Debug, Default)]
pub struct WorkStatus {
    pub staged: Vec<StatusEntry>,
    pub unstaged: Vec<StatusEntry>,
    /// A merge is in progress (conflicts to resolve and/or a commit to make).
    pub merging: bool,
    /// The prepared merge commit message, if merging.
    pub merge_message: Option<String>,
    /// Name of the in-progress operation ("Merge", "Cherry-pick", "Revert").
    pub operation: Option<&'static str>,
    /// Message of the HEAD commit (used to pre-fill an amend).
    pub head_message: Option<String>,
}

impl WorkStatus {
    /// Number of distinct paths with any change.
    pub fn changed_paths(&self) -> usize {
        let both = self
            .staged
            .iter()
            .filter(|s| self.unstaged.iter().any(|u| u.path == s.path))
            .count();
        self.staged.len() + self.unstaged.len() - both
    }
}

pub fn status(repo_path: &Path) -> Result<WorkStatus, git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true).include_ignored(false);
    let mut out = WorkStatus::default();
    out.head_message = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .and_then(|c| c.message().map(|m| m.trim_end().to_owned()));
    let operation = match repo.state() {
        git2::RepositoryState::Merge => Some("Merge"),
        git2::RepositoryState::CherryPick | git2::RepositoryState::CherryPickSequence => Some("Cherry-pick"),
        git2::RepositoryState::Revert | git2::RepositoryState::RevertSequence => Some("Revert"),
        git2::RepositoryState::Rebase
        | git2::RepositoryState::RebaseInteractive
        | git2::RepositoryState::RebaseMerge => Some("Rebase"),
        _ => None,
    };
    if operation.is_some() {
        out.merging = true;
        out.operation = operation;
        out.merge_message = repo.message().ok();
    }
    for e in repo.statuses(Some(&mut opts))?.iter() {
        let Some(path) = e.path() else { continue };
        let s = e.status();
        let mut push = |staged: bool, kind| {
            let list = if staged { &mut out.staged } else { &mut out.unstaged };
            list.push(StatusEntry { path: path.to_owned(), kind });
        };
        if s.is_conflicted() {
            push(false, ChangeKind::Conflicted);
            continue;
        }
        if s.contains(Status::INDEX_NEW) {
            push(true, ChangeKind::Added);
        } else if s.intersects(Status::INDEX_MODIFIED | Status::INDEX_TYPECHANGE) {
            push(true, ChangeKind::Modified);
        } else if s.contains(Status::INDEX_DELETED) {
            push(true, ChangeKind::Deleted);
        }
        if s.contains(Status::WT_NEW) {
            push(false, ChangeKind::Untracked);
        } else if s.intersects(Status::WT_MODIFIED | Status::WT_TYPECHANGE) {
            push(false, ChangeKind::Modified);
        } else if s.contains(Status::WT_DELETED) {
            push(false, ChangeKind::Deleted);
        }
    }
    Ok(out)
}

pub fn stage(repo_path: &Path, paths: &[String]) -> Result<(), git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| git2::Error::from_str("repository has no working directory"))?
        .to_path_buf();
    let mut index = repo.index()?;
    for p in paths {
        if workdir.join(p).symlink_metadata().is_ok() {
            index.add_path(Path::new(p))?;
        } else {
            index.remove_path(Path::new(p))?;
        }
    }
    index.write()
}

pub fn unstage(repo_path: &Path, paths: &[String]) -> Result<(), git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let head = repo.head().ok().and_then(|h| h.peel(ObjectType::Commit).ok());
    repo.reset_default(head.as_ref(), paths.iter().map(Path::new))
}

pub fn commit(repo_path: &Path, message: &str) -> Result<Oid, git2::Error> {
    let message = message.trim();
    if message.is_empty() {
        return Err(git2::Error::from_str("commit message is empty"));
    }
    let mut repo = Repository::discover(repo_path)?;
    if crate::rebase::in_progress(&repo) {
        return Err(git2::Error::from_str("a rebase is in progress: continue or abort it instead of committing"));
    }
    let sig = repo.signature()?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
        return Err(git2::Error::from_str("resolve all conflicts before committing"));
    }
    let merging = repo.state() == git2::RepositoryState::Merge;
    let in_op = repo.state() != git2::RepositoryState::Clean;
    let mut merge_heads = Vec::new();
    if merging {
        repo.mergehead_foreach(|oid| {
            merge_heads.push(*oid);
            true
        })?;
    }
    let author_commit = if repo.state() == git2::RepositoryState::CherryPick {
        repo.revparse_single("CHERRY_PICK_HEAD").ok().and_then(|o| o.peel_to_commit().ok())
    } else {
        None
    };
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let nothing = !in_op && match &parent {
        Some(p) => p.tree_id() == tree_id,
        None => tree.is_empty(),
    };
    if nothing {
        return Err(git2::Error::from_str("nothing to commit"));
    }
    let merge_commits = merge_heads
        .iter()
        .map(|o| repo.find_commit(*o))
        .collect::<Result<Vec<_>, _>>()?;
    let parents: Vec<_> = parent.iter().chain(merge_commits.iter()).collect();
    let author = author_commit.as_ref().map_or_else(|| sig.clone(), |c| c.author().to_owned());
    let id = repo.commit(Some("HEAD"), &author, &sig, message, &tree, &parents)?;
    if in_op {
        repo.cleanup_state()?;
    }
    Ok(id)
}

/// Stage a single hunk (by index, as numbered in [`workdir_diff`]) of a modified file.
pub fn stage_hunk(repo_path: &Path, path: &str, hunk: usize) -> Result<(), git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let mut opts = DiffOptions::new();
    opts.pathspec(path).disable_pathspec_match(true);
    let diff = repo.diff_index_to_workdir(None, Some(&mut opts))?;
    apply_hunk(&repo, &diff, hunk)
}

/// Unstage a single hunk (by index, as numbered in the staged diff) of a modified file.
pub fn unstage_hunk(repo_path: &Path, path: &str, hunk: usize) -> Result<(), git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let head = repo
        .head()
        .and_then(|h| h.peel_to_tree())
        .map_err(|_| git2::Error::from_str("there is no commit to unstage against"))?;
    let mut opts = DiffOptions::new();
    // Reversed, the diff turns index content back into HEAD content.
    opts.pathspec(path).disable_pathspec_match(true).reverse(true);
    let diff = repo.diff_tree_to_index(Some(&head), None, Some(&mut opts))?;
    apply_hunk(&repo, &diff, hunk)
}

/// Apply only hunk number `target` of `diff` to the index.
fn apply_hunk(repo: &Repository, diff: &git2::Diff, target: usize) -> Result<(), git2::Error> {
    let mut seen = 0usize;
    let mut apply = git2::ApplyOptions::new();
    apply.hunk_callback(|_| {
        let take = seen == target;
        seen += 1;
        take
    });
    repo.apply(diff, git2::ApplyLocation::Index, Some(&mut apply))?;
    repo.index()?.write()
}

/// A line within a diff: (hunk index, line index within that hunk).
pub type LineId = (u32, u32);

/// Stage only the selected changed lines of a modified file.
pub fn stage_lines(repo_path: &Path, path: &str, selected: &[LineId]) -> Result<(), git2::Error> {
    edit_index_lines(repo_path, path, selected, true)
}

/// Unstage only the selected changed lines of a modified file.
pub fn unstage_lines(repo_path: &Path, path: &str, selected: &[LineId]) -> Result<(), git2::Error> {
    edit_index_lines(repo_path, path, selected, false)
}

/// Rebuild the index version of `path` with only the `selected` changes of the
/// relevant diff applied (or reverted, when unstaging).
///
/// Staging uses the index→workdir diff: removed lines are present in the index
/// and added lines are absent. Unstaging uses the HEAD→index diff, where it is
/// the other way round. Either way the base text is the current index blob.
fn edit_index_lines(
    repo_path: &Path,
    path: &str,
    selected: &[LineId],
    stage: bool,
) -> Result<(), git2::Error> {
    safe_relative(path)?;
    let repo = Repository::discover(repo_path)?;
    let mut index = repo.index()?;
    let mut entry = index
        .get_path(Path::new(path), 0)
        .ok_or_else(|| git2::Error::from_str("the file is not in the index"))?;
    let base = repo.find_blob(entry.id)?.content().to_vec();

    let mut opts = DiffOptions::new();
    opts.pathspec(path).disable_pathspec_match(true);
    let diff = if stage {
        repo.diff_index_to_workdir(Some(&index), Some(&mut opts))?
    } else {
        let head = repo.head().and_then(|h| h.peel_to_tree()).ok();
        repo.diff_tree_to_index(head.as_ref(), Some(&index), Some(&mut opts))?
    };
    if diff.deltas().len() == 0 {
        return Err(git2::Error::from_str("no changes to stage"));
    }
    let patch = git2::Patch::from_diff(&diff, 0)?
        .ok_or_else(|| git2::Error::from_str("binary files cannot be staged by line"))?;

    let base_lines: Vec<&[u8]> = base.split_inclusive(|b| *b == b'\n').collect();
    let (present, absent) = if stage { ('-', '+') } else { ('+', '-') };
    let mut out: Vec<u8> = Vec::with_capacity(base.len());
    let mut cursor = 0usize; // next base line (0-based) to copy
    for h in 0..patch.num_hunks() {
        let (hunk, n) = patch.hunk(h)?;
        let (start, count) = if stage {
            (hunk.old_start(), hunk.old_lines())
        } else {
            (hunk.new_start(), hunk.new_lines())
        };
        // A zero-length side starts "after" the given line number.
        let first = if count == 0 { start as usize } else { (start as usize).saturating_sub(1) };
        for l in base_lines.iter().take(first).skip(cursor) {
            out.extend_from_slice(l);
        }
        for k in 0..n {
            let line = patch.line_in_hunk(h, k)?;
            let picked = selected.contains(&(h as u32, k as u32));
            match line.origin() {
                ' ' => out.extend_from_slice(line.content()),
                o if o == present => {
                    if !picked {
                        out.extend_from_slice(line.content());
                    }
                }
                o if o == absent => {
                    if picked {
                        out.extend_from_slice(line.content());
                    }
                }
                _ => {}
            }
        }
        cursor = first + count as usize;
    }
    for l in base_lines.iter().skip(cursor) {
        out.extend_from_slice(l);
    }

    entry.id = repo.blob(&out)?;
    entry.file_size = out.len() as u32;
    index.add(&entry)?;
    index.write()
}

/// Reject paths that could escape the working directory.
fn safe_relative(path: &str) -> Result<(), git2::Error> {
    let p = Path::new(path);
    if p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(git2::Error::from_str("path is outside the working directory"));
    }
    Ok(())
}

/// Throw away unstaged changes to one file: restores it from the index, or
/// deletes it if it is untracked. This cannot be undone.
pub fn discard_file(repo_path: &Path, path: &str) -> Result<(), git2::Error> {
    safe_relative(path)?;
    let repo = Repository::discover(repo_path)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| git2::Error::from_str("repository has no working directory"))?
        .to_path_buf();
    let in_index = repo.index()?.get_path(Path::new(path), 0).is_some();
    if in_index {
        let mut co = git2::build::CheckoutBuilder::new();
        co.force().path(path).disable_pathspec_match(true);
        repo.checkout_index(None, Some(&mut co))
    } else {
        let full = workdir.join(path);
        let meta = std::fs::symlink_metadata(&full).map_err(|e| git2::Error::from_str(&e.to_string()))?;
        if meta.is_dir() {
            std::fs::remove_dir_all(&full)
        } else {
            std::fs::remove_file(&full)
        }
        .map_err(|e| git2::Error::from_str(&e.to_string()))
    }
}

/// Discard every unstaged change (and, if `include_untracked`, delete new files).
pub fn discard_all(repo_path: &Path, include_untracked: bool) -> Result<(), git2::Error> {
    let st = status(repo_path)?;
    for e in &st.unstaged {
        match e.kind {
            ChangeKind::Untracked if !include_untracked => {}
            ChangeKind::Conflicted => {}
            _ => discard_file(repo_path, &e.path)?,
        }
    }
    Ok(())
}

/// Discard one hunk (numbered as in the unstaged diff) from the working tree.
pub fn discard_hunk(repo_path: &Path, path: &str, hunk: usize) -> Result<(), git2::Error> {
    safe_relative(path)?;
    let repo = Repository::discover(repo_path)?;
    let mut opts = DiffOptions::new();
    // Reversed, the diff turns working-tree content back into the index content.
    opts.pathspec(path).disable_pathspec_match(true).reverse(true);
    let diff = repo.diff_index_to_workdir(None, Some(&mut opts))?;
    let mut seen = 0usize;
    let mut apply = git2::ApplyOptions::new();
    apply.hunk_callback(|_| {
        let take = seen == hunk;
        seen += 1;
        take
    });
    repo.apply(&diff, git2::ApplyLocation::WorkDir, Some(&mut apply))
}

/// Replace the HEAD commit with one that has the current index as its tree.
/// The author, parents and (unless `message` is given) message are kept.
pub fn amend(repo_path: &Path, message: &str) -> Result<Oid, git2::Error> {
    let message = message.trim();
    if message.is_empty() {
        return Err(git2::Error::from_str("commit message is empty"));
    }
    let repo = Repository::discover(repo_path)?;
    crate::merge::ensure_clean_state(&repo)?;
    let sig = repo.signature()?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
        return Err(git2::Error::from_str("resolve all conflicts before committing"));
    }
    let head = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(|_| git2::Error::from_str("there is no commit to amend"))?;
    let tree = repo.find_tree(index.write_tree()?)?;
    head.amend(Some("HEAD"), None, Some(&sig), None, Some(message), Some(&tree))
}

/// Contents of a conflicted file (with its conflict markers) as plain lines.
pub fn conflict_view(repo_path: &Path, path: &str) -> Result<FileDiff, git2::Error> {
    crate::diff::file_text(repo_path, crate::diff::FileSource::Workdir, path)
}

/// Diff of one working-tree file: index vs HEAD (`staged`) or workdir vs index.
pub fn workdir_diff(
    repo_path: &Path,
    path: &str,
    staged: bool,
    ignore_ws: bool,
) -> Result<FileDiff, git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let mut opts = DiffOptions::new();
    opts.pathspec(path).disable_pathspec_match(true).ignore_whitespace(ignore_ws);
    let diff = if staged {
        let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        repo.diff_tree_to_index(head.as_ref(), None, Some(&mut opts))?
    } else {
        opts.include_untracked(true).show_untracked_content(true).recurse_untracked_dirs(true);
        repo.diff_index_to_workdir(None, Some(&mut opts))?
    };
    diff_to_file_diff(&diff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::LineKind;

    #[test]
    fn stage_and_unstage_single_hunks() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let repo = Repository::init(p).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        let lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
        std::fs::write(p.join("f"), lines.join("\n") + "\n").unwrap();
        stage(p, &["f".into()]).unwrap();
        commit(p, "base").unwrap();

        // Two edits far apart give two hunks.
        let mut edited = lines.clone();
        edited[1] = "CHANGED 2".into();
        edited[37] = "CHANGED 38".into();
        std::fs::write(p.join("f"), edited.join("\n") + "\n").unwrap();
        let hunks = |staged: bool| match workdir_diff(p, "f", staged, false).unwrap() {
            FileDiff::Text { lines, .. } => lines.iter().filter(|l| l.kind == crate::diff::LineKind::Hunk).count(),
            _ => panic!("text expected"),
        };
        assert_eq!((hunks(false), hunks(true)), (2, 0));

        stage_hunk(p, "f", 1).unwrap();
        assert_eq!((hunks(false), hunks(true)), (1, 1));
        let FileDiff::Text { lines, .. } = workdir_diff(p, "f", true, false).unwrap() else { panic!() };
        assert!(lines.iter().any(|l| &*l.text == "CHANGED 38"));
        assert!(!lines.iter().any(|l| &*l.text == "CHANGED 2"));
        // The working-tree file itself is untouched.
        assert!(std::fs::read_to_string(p.join("f")).unwrap().contains("CHANGED 2"));

        unstage_hunk(p, "f", 0).unwrap();
        assert_eq!((hunks(false), hunks(true)), (2, 0));
    }

    #[test]
    fn stage_and_unstage_selected_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let repo = Repository::init(p).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        std::fs::write(p.join("f"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        stage(p, &["f".into()]).unwrap();
        commit(p, "base").unwrap();
        std::fs::write(p.join("f"), "l1\nA\nB\nl4\nl5\nextra\n").unwrap();

        // Find line ids in the unstaged diff.
        let ids = |staged: bool| -> Vec<(LineId, crate::diff::LineKind, String)> {
            let FileDiff::Text { lines, .. } = workdir_diff(p, "f", staged, false).unwrap() else { panic!() };
            let mut k = 0u32;
            let mut out = Vec::new();
            for l in &lines {
                if l.kind == crate::diff::LineKind::Hunk {
                    k = 0;
                    continue;
                }
                out.push(((l.hunk, k), l.kind, l.text.to_string()));
                k += 1;
            }
            out
        };
        let pick = |staged: bool, wanted: &[(&str, crate::diff::LineKind)]| -> Vec<LineId> {
            ids(staged)
                .into_iter()
                .filter(|(_, kind, text)| wanted.iter().any(|(t, k)| t == text && k == kind))
                .map(|(id, _, _)| id)
                .collect()
        };
        use crate::diff::LineKind::{Add, Del};

        // Stage only "l2 -> A" (drop l2, add A); leave l3 -> B and the extra line.
        stage_lines(p, "f", &pick(false, &[("l2", Del), ("A", Add)])).unwrap();
        // A fresh handle: a long-lived one would keep its cached index.
        let index_text = || {
            let r = Repository::open(p).unwrap();
            let id = r.index().unwrap().get_path(Path::new("f"), 0).unwrap().id;
            let text = String::from_utf8(r.find_blob(id).unwrap().content().to_vec()).unwrap();
            text
        };
        // Same ordering `git add -p` gives: unselected removals stay where they were,
        // added lines follow the removed block.
        assert_eq!(index_text(), "l1\nl3\nA\nl4\nl5\n");
        // The working-tree file is untouched.
        assert_eq!(std::fs::read_to_string(p.join("f")).unwrap(), "l1\nA\nB\nl4\nl5\nextra\n");

        // Stage the trailing added line too.
        stage_lines(p, "f", &pick(false, &[("extra", Add)])).unwrap();
        assert!(index_text().ends_with("l5\nextra\n"), "got {:?}", index_text());

        // Unstage everything again, line by line.
        let all: Vec<LineId> = ids(true)
            .into_iter()
            .filter(|(_, k, _)| *k != crate::diff::LineKind::Context)
            .map(|(id, _, _)| id)
            .collect();
        unstage_lines(p, "f", &all).unwrap();
        assert!(status(p).unwrap().staged.is_empty(), "back to HEAD");
        assert_eq!(status(p).unwrap().unstaged.len(), 1);
    }

    #[test]
    fn discard_file_all_and_hunk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let repo = Repository::init(p).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        let lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
        let base = lines.join("\n") + "\n";
        std::fs::write(p.join("f"), &base).unwrap();
        std::fs::write(p.join("g"), "g\n").unwrap();
        stage(p, &["f".into(), "g".into()]).unwrap();
        commit(p, "base").unwrap();

        // Hunk discard: two hunks, drop only the second.
        let mut edited = lines.clone();
        edited[1] = "CHANGED 2".into();
        edited[37] = "CHANGED 38".into();
        std::fs::write(p.join("f"), edited.join("\n") + "\n").unwrap();
        discard_hunk(p, "f", 1).unwrap();
        let now = std::fs::read_to_string(p.join("f")).unwrap();
        assert!(now.contains("CHANGED 2") && !now.contains("CHANGED 38") && now.contains("line 38"));

        // File discard restores from the index; a staged change is kept.
        discard_file(p, "f").unwrap();
        assert_eq!(std::fs::read_to_string(p.join("f")).unwrap(), base);
        std::fs::write(p.join("g"), "staged\n").unwrap();
        stage(p, &["g".into()]).unwrap();
        std::fs::write(p.join("g"), "unstaged\n").unwrap();
        discard_file(p, "g").unwrap();
        assert_eq!(std::fs::read_to_string(p.join("g")).unwrap(), "staged\n");

        // Untracked: only removed when asked.
        std::fs::write(p.join("new.txt"), "n").unwrap();
        std::fs::write(p.join("f"), "changed\n").unwrap();
        discard_all(p, false).unwrap();
        assert!(p.join("new.txt").exists());
        assert_eq!(std::fs::read_to_string(p.join("f")).unwrap(), base);
        discard_all(p, true).unwrap();
        assert!(!p.join("new.txt").exists());

        assert!(discard_file(p, "../outside").is_err());
    }

    #[test]
    fn stage_commit_unstage_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        let p = dir.path();
        let file = |c: &str| std::fs::write(p.join("a.txt"), c).unwrap();

        file("one\n");
        let s = status(p).unwrap();
        assert_eq!(s.unstaged, vec![StatusEntry { path: "a.txt".into(), kind: ChangeKind::Untracked }]);
        assert!(commit(p, "x").is_err(), "nothing staged yet");

        stage(p, &["a.txt".into()]).unwrap();
        let s = status(p).unwrap();
        assert_eq!(s.staged[0].kind, ChangeKind::Added);
        assert!(s.unstaged.is_empty());
        commit(p, "first").unwrap();
        assert!(status(p).unwrap().changed_paths() == 0);

        file("one\ntwo\n");
        let s = status(p).unwrap();
        assert_eq!(s.unstaged[0].kind, ChangeKind::Modified);
        let FileDiff::Text { lines, .. } = workdir_diff(p, "a.txt", false, false).unwrap() else { panic!() };
        assert_eq!(lines.iter().filter(|l| l.kind == LineKind::Add).count(), 1);

        stage(p, &["a.txt".into()]).unwrap();
        assert_eq!(status(p).unwrap().staged[0].kind, ChangeKind::Modified);
        unstage(p, &["a.txt".into()]).unwrap();
        let s = status(p).unwrap();
        assert!(s.staged.is_empty() && s.unstaged.len() == 1);

        // Amend: message-only, then with newly staged content.
        let before = repo.head().unwrap().peel_to_commit().unwrap();
        let id = amend(p, "reworded").unwrap();
        let after = repo.find_commit(id).unwrap();
        assert_eq!(after.message(), Some("reworded"));
        assert_eq!(after.tree_id(), before.tree_id());
        assert_eq!(after.parent_count(), before.parent_count());
        assert_eq!(status(p).unwrap().head_message.as_deref(), Some("reworded"));
        file("one\ntwo\nthree\n");
        stage(p, &["a.txt".into()]).unwrap();
        let id2 = amend(p, "with three").unwrap();
        assert_ne!(repo.find_commit(id2).unwrap().tree_id(), before.tree_id());
        assert_eq!(status(p).unwrap().changed_paths(), 0);

        std::fs::remove_file(p.join("a.txt")).unwrap();
        stage(p, &["a.txt".into()]).unwrap();
        assert_eq!(status(p).unwrap().staged[0].kind, ChangeKind::Deleted);
    }
}
