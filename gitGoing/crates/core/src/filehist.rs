//! A single file's history and blame.

use git2::{BlameOptions, DiffOptions, Oid, Repository, Sort};
use std::path::Path;

/// What to compare a commit against: its first parent, or the empty tree for a root commit.
pub fn diff_base(repo_path: &Path, commit: Oid) -> String {
    const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    let root = Repository::discover(repo_path)
        .and_then(|r| r.find_commit(commit).map(|c| c.parent_count() == 0))
        .unwrap_or(false);
    if root { EMPTY_TREE.to_owned() } else { format!("{commit}^") }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCommit {
    pub id: Oid,
    pub summary: String,
    pub author: String,
    pub time: i64,
    pub offset_minutes: i32,
}

/// Commits that changed `path`, newest first, starting at `start` (default HEAD).
/// Stops after `limit` commits.
pub fn history(repo_path: &Path, path: &str, start: Option<Oid>, limit: usize) -> Result<Vec<FileCommit>, git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git log --follow -- {}", crate::gitlog::quote(path)));
    let repo = Repository::discover(repo_path)?;
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TIME)?;
    match start {
        Some(id) => walk.push(id)?,
        None => walk.push_head()?,
    }
    let mut out = Vec::new();
    for id in walk {
        let commit = repo.find_commit(id?)?;
        let tree = commit.tree()?;
        let mut opts = DiffOptions::new();
        opts.pathspec(path);
        // A merge only "touches" the file if it differs from every parent.
        let touched = if commit.parent_count() == 0 {
            tree.get_path(Path::new(path)).is_ok()
        } else {
            let mut all = true;
            for p in commit.parents() {
                let d = repo.diff_tree_to_tree(Some(&p.tree()?), Some(&tree), Some(&mut opts))?;
                if d.deltas().len() == 0 {
                    all = false;
                    break;
                }
            }
            all
        };
        if touched {
            let a = commit.author();
            out.push(FileCommit {
                id: commit.id(),
                summary: commit.summary().unwrap_or("").to_owned(),
                author: a.name().unwrap_or("").to_owned(),
                time: commit.time().seconds(),
                offset_minutes: commit.time().offset_minutes(),
            });
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    pub text: String,
    pub id: Oid,
    pub author: String,
    pub time: i64,
    pub summary: String,
}

/// Who last changed each line of `path` as of `at` (default HEAD).
pub fn blame(repo_path: &Path, path: &str, at: Option<Oid>) -> Result<Vec<BlameLine>, git2::Error> {
    crate::gitlog::equiv(repo_path, format!("git blame -- {}", crate::gitlog::quote(path)));
    let repo = Repository::discover(repo_path)?;
    let commit_id = match at {
        Some(id) => id,
        None => repo.head()?.peel_to_commit()?.id(),
    };
    let tree = repo.find_commit(commit_id)?.tree()?;
    let blob = repo.find_blob(tree.get_path(Path::new(path))?.id())?;
    if blob.is_binary() {
        return Err(git2::Error::from_str("Binary files can't be blamed."));
    }
    let text = String::from_utf8_lossy(blob.content()).into_owned();
    let mut opts = BlameOptions::new();
    opts.newest_commit(commit_id);
    let blame = repo.blame_file(Path::new(path), Some(&mut opts))?;
    let mut cache: std::collections::HashMap<Oid, (String, String, i64)> = Default::default();
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let Some(h) = blame.get_line(i + 1) else { continue };
        let id = h.final_commit_id();
        let (author, summary, time) = match cache.get(&id) {
            Some(v) => v.clone(),
            None => {
                let v = repo
                    .find_commit(id)
                    .map(|c| (c.author().name().unwrap_or("").to_owned(), c.summary().unwrap_or("").to_owned(), c.time().seconds()))
                    .unwrap_or_default();
                cache.insert(id, v.clone());
                v
            }
        };
        out.push(BlameLine { text: line.to_owned(), id, author, time, summary });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn commit(repo: &Repository, dir: &Path, file: &str, body: &str, msg: &str) -> Oid {
        fs::write(dir.join(file), body).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new(file)).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("T", "t@x").unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents).unwrap()
    }

    #[test]
    fn history_lists_only_commits_touching_the_file_and_blame_names_them() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let c1 = commit(&repo, dir.path(), "a.txt", "one\ntwo\n", "add a");
        commit(&repo, dir.path(), "b.txt", "x\n", "add b");
        let c3 = commit(&repo, dir.path(), "a.txt", "one\nTWO\n", "edit a");
        let h = history(dir.path(), "a.txt", None, 50).unwrap();
        assert_eq!(h.iter().map(|c| c.id).collect::<Vec<_>>(), vec![c3, c1]);
        assert_eq!(history(dir.path(), "a.txt", None, 1).unwrap().len(), 1);
        let b = blame(dir.path(), "a.txt", None).unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].id, b[1].id), (c1, c3));
        assert_eq!(b[1].summary, "edit a");
        // As of the first commit the second line was still "two".
        let old = blame(dir.path(), "a.txt", Some(c1)).unwrap();
        assert_eq!(old[1].text, "two");
    }
}
