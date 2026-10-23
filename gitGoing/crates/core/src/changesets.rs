//! Saved change sets: named snapshots of uncommitted changes that can be re-applied any number of
//! times, e.g. local configuration that should never be committed.
//!
//! Each set is a stash-shaped commit kept under `refs/keep/<name>`, so `git stash apply
//! refs/keep/<name>` works from the command line too, and `git stash drop` / `clear` never touch
//! it. Saving takes a snapshot only: the working tree and the index are left exactly as they were.

use std::path::Path;

use git2::{Oid, Repository};

pub const PREFIX: &str = "refs/keep/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSet {
    /// The name the user gave it.
    pub name: String,
    /// Full reference name, `refs/keep/...`.
    pub refname: String,
    /// The snapshot commit (shown like a stash: its changes against the commit it was saved on).
    pub id: Oid,
    /// Branch it was saved on.
    pub branch: String,
    /// Seconds since the epoch.
    pub time: i64,
    /// Changed and new files in it.
    pub files: usize,
}

/// The reference a name is stored under: characters git does not allow become `-`.
pub fn refname_for(name: &str) -> Result<String, String> {
    let slug: String = name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/') { c } else { '-' })
        .collect();
    let slug = slug.trim_matches(|c| c == '/' || c == '.' || c == '-');
    let full = format!("{PREFIX}{slug}");
    if slug.is_empty() || !git2::Reference::is_valid_name(&full) {
        return Err(format!("'{}' can't be used as a name", name.trim()));
    }
    Ok(full)
}

/// "On <branch>: <name>", as `git stash` writes it.
fn parse_message(msg: &str) -> Option<(String, String)> {
    let rest = msg.lines().next()?.strip_prefix("On ")?;
    let (branch, name) = rest.split_once(": ")?;
    // `git stash create -m x` stores "-m x" (create takes no -m); show just "x".
    let name = name.trim();
    let name = name.strip_prefix("-m ").unwrap_or(name).trim_matches(|c| c == '\'' || c == '"');
    Some((branch.to_owned(), name.to_owned()))
}

fn count_blobs(tree: &git2::Tree) -> usize {
    let mut n = 0;
    let _ = tree.walk(git2::TreeWalkMode::PreOrder, |_, e| {
        if e.kind() == Some(git2::ObjectType::Blob) {
            n += 1;
        }
        git2::TreeWalkResult::Ok
    });
    n
}

pub fn list(repo: &Repository) -> Vec<ChangeSet> {
    let mut out = Vec::new();
    let Ok(refs) = repo.references_glob(&format!("{PREFIX}*")) else { return out };
    for r in refs.flatten() {
        let (Some(refname), Ok(commit)) = (r.name().map(str::to_owned), r.peel_to_commit()) else { continue };
        let slug = refname.trim_start_matches(PREFIX).to_owned();
        let (branch, name) = parse_message(commit.message().unwrap_or("")).unwrap_or_else(|| (String::new(), slug.clone()));
        let mut files = 0;
        if let (Ok(base), Ok(tree)) = (commit.parent(0).and_then(|p| p.tree()), commit.tree()) {
            files += repo.diff_tree_to_tree(Some(&base), Some(&tree), None).map(|d| d.deltas().len()).unwrap_or(0);
        }
        if let Ok(untracked) = commit.parent(2).and_then(|p| p.tree()) {
            files += count_blobs(&untracked);
        }
        out.push(ChangeSet {
            name: if name.is_empty() { slug } else { name },
            refname,
            id: commit.id(),
            branch,
            time: commit.time().seconds(),
            files,
        });
    }
    out.sort_by_key(|c| c.name.to_lowercase());
    out
}

fn git(repo_path: &Path, args: &[&str], index: Option<&Path>) -> Result<String, String> {
    let mut cmd = crate::gitcmd::command(Some(repo_path));
    if let Some(i) = index {
        cmd.env("GIT_INDEX_FILE", i);
    }
    cmd.args(args);
    crate::gitcmd::run_cmd(&mut cmd, args.first().copied().unwrap_or("git")).map(|s| s.trim().to_owned())
}

/// Temporary index files in the repository's git folder, removed when dropped.
struct TempIndex(std::path::PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Save the current uncommitted changes (and, with `include_untracked`, new files) as `name`,
/// replacing a set of the same name. Nothing in the working tree or index changes.
pub fn save(repo_path: &Path, name: &str, include_untracked: bool) -> Result<(), String> {
    let name = name.trim();
    let refname = refname_for(name)?;
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    let root = repo.workdir().ok_or("This repository has no working folder.")?.to_path_buf();
    let gitdir = repo.path().to_path_buf();
    let head = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .ok_or("Make a first commit before saving changes.")?;
    let branch = repo.head().ok().filter(|h| h.is_branch()).and_then(|h| h.shorthand().map(str::to_owned)).unwrap_or_else(|| "(no branch)".into());
    let head_id = head.id().to_string();
    let subject = format!("{} {}", &head_id[..7], head.summary().unwrap_or(""));

    // Staged changes, as `git stash` records them.
    let itree = git(&root, &["write-tree"], None).map_err(|_| "Resolve the merge conflicts before saving changes.".to_owned())?;
    let icommit = git(&root, &["commit-tree", &itree, "-p", &head_id, "-m", &format!("index on {branch}: {subject}")], None)?;

    // Everything tracked, staged or not: a copy of the index brought up to date with the files.
    let wfile = TempIndex(gitdir.join("gitgoing-changeset-w.index"));
    let real_index = gitdir.join("index");
    if real_index.exists() {
        std::fs::copy(&real_index, &wfile.0).map_err(|e| format!("Couldn't copy the index: {e}"))?;
    }
    git(&root, &["add", "-u", "--", ":/"], Some(&wfile.0))?;
    let wtree = git(&root, &["write-tree"], Some(&wfile.0))?;

    // New files git doesn't track yet (ignored files stay out).
    let mut parents = vec![head_id.clone(), icommit];
    let mut has_untracked = false;
    if include_untracked {
        let list = git(&root, &["ls-files", "--others", "--exclude-standard", "-z", "--", ":/"], None)?;
        let paths: Vec<&str> = list.split('\0').filter(|p| !p.is_empty()).collect();
        if !paths.is_empty() {
            has_untracked = true;
            let ufile = TempIndex(gitdir.join("gitgoing-changeset-u.index"));
            let spec = TempIndex(gitdir.join("gitgoing-changeset-u.paths"));
            std::fs::write(&spec.0, paths.join("\0")).map_err(|e| format!("Couldn't list the new files: {e}"))?;
            let spec_arg = format!("--pathspec-from-file={}", spec.0.display());
            git(&root, &["add", "--pathspec-file-nul", &spec_arg], Some(&ufile.0))?;
            let utree = git(&root, &["write-tree"], Some(&ufile.0))?;
            parents.push(git(&root, &["commit-tree", &utree, "-m", &format!("untracked files on {branch}: {subject}")], None)?);
        }
    }

    let head_tree = head.tree_id().to_string();
    if wtree == head_tree && itree == head_tree && !has_untracked {
        return Err("There are no changes to save.".into());
    }
    let mut args = vec!["commit-tree".to_owned(), wtree];
    for p in &parents {
        args.push("-p".into());
        args.push(p.clone());
    }
    args.push("-m".into());
    args.push(format!("On {branch}: {name}"));
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let commit = git(&root, &argv, None)?;
    git(&root, &["update-ref", "-m", &format!("gitgoing: save changes '{name}'"), &refname, &commit], None)?;
    Ok(())
}

/// Apply a saved set to the working tree. The set stays saved.
pub fn apply(repo_path: &Path, refname: &str) -> Result<(), String> {
    let out = git(repo_path, &["stash", "apply", refname], None);
    out.map(|_| ()).map_err(|e| {
        if e.contains("CONFLICT") {
            format!("Applied with conflicts; resolve them in the marked files.\n{e}")
        } else {
            e
        }
    })
}

/// Give a set a new name (its contents are unchanged).
pub fn rename(repo_path: &Path, refname: &str, new_name: &str) -> Result<(), String> {
    let new_name = new_name.trim();
    let new_ref = refname_for(new_name)?;
    let repo = Repository::discover(repo_path).map_err(|e| e.message().to_owned())?;
    if new_ref != refname && repo.find_reference(&new_ref).is_ok() {
        return Err(format!("There is already a saved set called '{new_name}'."));
    }
    let old = repo.find_reference(refname).and_then(|r| r.peel_to_commit()).map_err(|e| e.message().to_owned())?;
    let branch = parse_message(old.message().unwrap_or("")).map(|(b, _)| b).unwrap_or_else(|| "(no branch)".into());
    let tree = old.tree_id().to_string();
    let mut args = vec!["commit-tree".to_owned(), tree];
    for p in old.parent_ids() {
        args.push("-p".into());
        args.push(p.to_string());
    }
    args.push("-m".into());
    args.push(format!("On {branch}: {new_name}"));
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let commit = git(repo_path, &argv, None)?;
    git(repo_path, &["update-ref", "-m", &format!("gitgoing: rename saved changes to '{new_name}'"), &new_ref, &commit], None)?;
    if new_ref != refname {
        git(repo_path, &["update-ref", "-d", refname], None)?;
    }
    Ok(())
}

pub fn delete(repo_path: &Path, refname: &str) -> Result<(), String> {
    git(repo_path, &["update-ref", "-d", refname], None).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::status;

    fn repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        std::fs::write(dir.path().join("a"), "1\n").unwrap();
        std::fs::write(dir.path().join("b"), "1\n").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
        crate::status::stage(dir.path(), &["a".into(), "b".into(), ".gitignore".into()]).unwrap();
        crate::status::commit(dir.path(), "init").unwrap();
        (dir, repo)
    }

    fn read(p: &Path, f: &str) -> String {
        std::fs::read_to_string(p.join(f)).unwrap_or_default()
    }

    #[test]
    fn saving_keeps_the_working_changes_and_apply_can_repeat() {
        let (dir, repo) = repo();
        let p = dir.path();
        assert!(save(p, "local", true).is_err(), "nothing to save yet");

        std::fs::write(p.join("a"), "local config\n").unwrap();
        std::fs::write(p.join("b"), "staged\n").unwrap();
        crate::status::stage(p, &["b".into()]).unwrap();
        std::fs::write(p.join("new.txt"), "new file\n").unwrap();
        std::fs::write(p.join("debug.log"), "ignored\n").unwrap();
        let before = status(p).unwrap();

        save(p, "my local changes", true).unwrap();
        // Nothing moved: same files, same staged/unstaged split.
        let after = status(p).unwrap();
        assert_eq!((after.staged.len(), after.unstaged.len()), (before.staged.len(), before.unstaged.len()));
        assert_eq!(read(p, "a"), "local config\n");

        let sets = list(&repo);
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].name, "my local changes");
        assert_eq!(sets[0].refname, "refs/keep/my-local-changes");
        assert_eq!(sets[0].branch, repo.head().unwrap().shorthand().unwrap());
        assert_eq!(sets[0].files, 3, "a, b and the new file; the ignored log is left out");

        // Throw the changes away, then bring them back twice.
        for round in 0..2 {
            git(p, &["reset", "--hard"], None).unwrap();
            let _ = std::fs::remove_file(p.join("new.txt"));
            assert_eq!(read(p, "a"), "1\n");
            apply(p, &sets[0].refname).unwrap();
            assert_eq!(read(p, "a"), "local config\n", "round {round}");
            assert_eq!(read(p, "b"), "staged\n");
            assert_eq!(read(p, "new.txt"), "new file\n");
            assert_eq!(list(&repo).len(), 1, "applying keeps the set");
        }
        // Stash commands don't touch it.
        let _ = git(p, &["stash", "clear"], None);
        assert_eq!(list(&repo).len(), 1);
    }

    #[test]
    fn update_rename_and_delete() {
        let (dir, repo) = repo();
        let p = dir.path();
        std::fs::write(p.join("a"), "v1\n").unwrap();
        save(p, "cfg", false).unwrap();
        std::fs::write(p.join("a"), "v2\n").unwrap();
        save(p, "cfg", false).unwrap(); // same name: replaced
        let sets = list(&repo);
        assert_eq!(sets.len(), 1);

        rename(p, &sets[0].refname, "Local config").unwrap();
        let sets = list(&repo);
        assert_eq!((sets[0].name.as_str(), sets[0].refname.as_str()), ("Local config", "refs/keep/Local-config"));

        git(p, &["checkout", "--", "a"], None).unwrap();
        apply(p, &sets[0].refname).unwrap();
        assert_eq!(read(p, "a"), "v2\n", "the updated snapshot is the one applied");

        std::fs::write(p.join("b"), "x\n").unwrap();
        save(p, "other", false).unwrap();
        assert!(rename(p, &list(&repo)[1].refname, "Local config").is_err(), "names stay unique");

        delete(p, "refs/keep/Local-config").unwrap();
        assert_eq!(list(&repo).iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["other"]);
    }

    #[test]
    fn sets_made_with_plain_git_are_listed() {
        let (dir, repo) = repo();
        let p = dir.path();
        std::fs::write(p.join("a"), "cli\n").unwrap();
        let id = git(p, &["stash", "create", "-m", "from the command line"], None).unwrap();
        git(p, &["update-ref", "refs/keep/cli-set", &id], None).unwrap();
        let sets = list(&repo);
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].name, "from the command line");
        assert_eq!(sets[0].files, 1);
        assert!(refname_for("   ").is_err());
    }
}
