//! Commit details and per-file diffs, computed on demand.

use git2::{Commit, Delta, Diff, DiffFindOptions, DiffOptions, Oid, Patch, Repository};
use std::cell::RefCell;
use std::path::Path;

/// Cap on diff lines kept per file, to bound memory for huge/generated files.
pub const MAX_DIFF_LINES: usize = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Conflicted,
    Other,
}

impl ChangeKind {
    pub fn letter(self) -> char {
        match self {
            Self::Added => 'A',
            Self::Modified => 'M',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
            Self::Copied => 'C',
            Self::Untracked => 'U',
            Self::Conflicted => '!',
            Self::Other => '?',
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

#[derive(Clone, Debug)]
pub struct CommitDetail {
    pub id: Oid,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub offset_minutes: i32,
    pub message: String,
    pub parents: Vec<Oid>,
    pub files: Vec<FileChange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Hunk,
    Add,
    Del,
    Context,
    Meta,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    /// Index of the hunk this line belongs to.
    pub hunk: u32,
    pub old_no: u32,
    pub new_no: u32,
    pub text: Box<str>,
}

#[derive(Clone, Debug)]
pub enum FileDiff {
    Binary,
    Text { lines: Vec<DiffLine>, truncated: bool, max_chars: usize },
}

/// Diff of `commit` against its first parent (or the empty tree for roots).
fn commit_diff<'r>(
    repo: &'r Repository,
    commit: &Commit<'r>,
    pathspecs: &[&str],
    ignore_ws: bool,
) -> Result<Diff<'r>, git2::Error> {
    let new_tree = commit.tree()?;
    let old_tree = commit.parent(0).ok().map(|p| p.tree()).transpose()?;
    let mut opts = DiffOptions::new();
    opts.disable_pathspec_match(true).ignore_whitespace(ignore_ws);
    for p in pathspecs {
        opts.pathspec(p);
    }
    let mut diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts))?;
    if pathspecs.is_empty() {
        diff.find_similar(Some(DiffFindOptions::new().renames(true)))?;
    }
    Ok(diff)
}

pub(crate) fn kind_of(d: Delta) -> ChangeKind {
    match d {
        Delta::Added => ChangeKind::Added,
        Delta::Modified | Delta::Typechange => ChangeKind::Modified,
        Delta::Deleted => ChangeKind::Deleted,
        Delta::Renamed => ChangeKind::Renamed,
        Delta::Copied => ChangeKind::Copied,
        _ => ChangeKind::Other,
    }
}

pub fn commit_detail(repo_path: &Path, id: Oid) -> Result<CommitDetail, git2::Error> {
    crate::gitlog::read(repo_path, format!("git show --stat --find-renames {}", id));
    let repo = Repository::discover(repo_path)?;
    let commit = repo.find_commit(id)?;
    let diff = commit_diff(&repo, &commit, &[], false)?;

    let files: RefCell<Vec<FileChange>> = RefCell::new(Vec::new());
    diff.foreach(
        &mut |delta, _| {
            let path = delta.new_file().path().map(|p| p.to_string_lossy().into_owned());
            let old = delta.old_file().path().map(|p| p.to_string_lossy().into_owned());
            let kind = kind_of(delta.status());
            let moved = matches!(kind, ChangeKind::Renamed | ChangeKind::Copied);
            files.borrow_mut().push(FileChange {
                path: path.or_else(|| old.clone()).unwrap_or_default(),
                old_path: if moved { old } else { None },
                kind,
                additions: 0,
                deletions: 0,
                binary: delta.flags().is_binary(),
            });
            true
        },
        Some(&mut |_, _| {
            if let Some(f) = files.borrow_mut().last_mut() {
                f.binary = true;
            }
            true
        }),
        None,
        Some(&mut |_, _, line| {
            if let Some(f) = files.borrow_mut().last_mut() {
                match line.origin() {
                    '+' => f.additions += 1,
                    '-' => f.deletions += 1,
                    _ => {}
                }
            }
            true
        }),
    )?;

    let author = commit.author();
    Ok(CommitDetail {
        id,
        author: author.name().unwrap_or("").to_owned(),
        email: author.email().unwrap_or("").to_owned(),
        time: author.when().seconds(),
        offset_minutes: author.when().offset_minutes(),
        message: commit.message().unwrap_or("").trim_end().to_owned(),
        parents: commit.parent_ids().collect(),
        files: files.into_inner(),
    })
}

pub fn file_diff(
    repo_path: &Path,
    id: Oid,
    path: &str,
    old_path: Option<&str>,
    ignore_ws: bool,
) -> Result<FileDiff, git2::Error> {
    crate::gitlog::read(repo_path, format!("git show {}{} -- {}", id, if ignore_ws { " -w" } else { "" }, crate::gitlog::quote(path)));
    let repo = Repository::discover(repo_path)?;
    let commit = repo.find_commit(id)?;
    let mut specs = vec![path];
    specs.extend(old_path);
    let diff = commit_diff(&repo, &commit, &specs, ignore_ws)?;
    diff_to_file_diff(&diff)
}

/// Render the first delta of `diff` as lines (capped at [`MAX_DIFF_LINES`]).
pub(crate) fn diff_to_file_diff(diff: &Diff) -> Result<FileDiff, git2::Error> {
    if diff.deltas().len() == 0 {
        return Ok(FileDiff::Text { lines: Vec::new(), truncated: false, max_chars: 0 });
    }
    let Some(patch) = Patch::from_diff(diff, 0)? else {
        return Ok(FileDiff::Binary);
    };
    if patch.delta().flags().is_binary() {
        return Ok(FileDiff::Binary);
    }

    let mut lines = Vec::new();
    let mut max_chars = 0;
    let mut truncated = false;
    'hunks: for h in 0..patch.num_hunks() {
        let (hunk, n) = patch.hunk(h)?;
        let header = String::from_utf8_lossy(hunk.header()).trim_end().to_owned();
        max_chars = max_chars.max(header.chars().count());
        lines.push(DiffLine { kind: LineKind::Hunk, hunk: h as u32, old_no: 0, new_no: 0, text: header.into() });
        for l in 0..n {
            if lines.len() >= MAX_DIFF_LINES {
                truncated = true;
                break 'hunks;
            }
            let line = patch.line_in_hunk(h, l)?;
            let kind = match line.origin() {
                '+' => LineKind::Add,
                '-' => LineKind::Del,
                ' ' => LineKind::Context,
                _ => LineKind::Meta,
            };
            let text = String::from_utf8_lossy(line.content())
                .trim_end_matches(['\n', '\r'])
                .replace('\t', "    ");
            max_chars = max_chars.max(text.chars().count() + 1);
            lines.push(DiffLine {
                kind,
                hunk: h as u32,
                old_no: line.old_lineno().unwrap_or(0),
                new_no: line.new_lineno().unwrap_or(0),
                text: text.into(),
            });
        }
    }
    Ok(FileDiff::Text { lines, truncated, max_chars })
}

/// Largest file shown in File View; bigger ones are refused rather than loaded.
pub const MAX_FILE_VIEW_BYTES: u64 = 8 * 1024 * 1024;

/// Where to read a file's full contents from.
#[derive(Clone, Copy, Debug)]
pub enum FileSource {
    /// The file as of this commit (or, if the commit deleted it, its parent).
    Commit(Oid),
    /// The staged version.
    Index,
    /// The file on disk.
    Workdir,
}

fn too_large(bytes: u64) -> git2::Error {
    git2::Error::from_str(&format!(
        "this file is {} MB, too large to show here (limit {} MB)",
        bytes / (1024 * 1024),
        MAX_FILE_VIEW_BYTES / (1024 * 1024)
    ))
}

/// A whole file as numbered context lines ("File View").
pub fn file_text(repo_path: &Path, source: FileSource, path: &str) -> Result<FileDiff, git2::Error> {
    crate::gitlog::read(repo_path, match source { FileSource::Commit(id) => format!("git show {}:{}", id, path), FileSource::Index => format!("git show :{}", path), FileSource::Workdir => format!("cat {}", crate::gitlog::quote(path)) });
    let repo = Repository::discover(repo_path)?;
    let bytes: Vec<u8> = match source {
        FileSource::Workdir => {
            let wd = repo
                .workdir()
                .ok_or_else(|| git2::Error::from_str("repository has no working directory"))?;
            let full = wd.join(path);
            let len = std::fs::metadata(&full).map_err(|e| git2::Error::from_str(&e.to_string()))?.len();
            if len > MAX_FILE_VIEW_BYTES {
                return Err(too_large(len));
            }
            std::fs::read(full).map_err(|e| git2::Error::from_str(&e.to_string()))?
        }
        FileSource::Index => {
            let index = repo.index()?;
            let entry = index
                .get_path(Path::new(path), 0)
                .ok_or_else(|| git2::Error::from_str("file is not in the index"))?;
            let blob = repo.find_blob(entry.id)?;
            if blob.size() as u64 > MAX_FILE_VIEW_BYTES {
                return Err(too_large(blob.size() as u64));
            }
            blob.content().to_vec()
        }
        FileSource::Commit(id) => {
            let commit = repo.find_commit(id)?;
            let blob_in = |c: &Commit| -> Result<Vec<u8>, git2::Error> {
                let entry = c.tree()?.get_path(Path::new(path))?;
                let blob = repo.find_blob(entry.id())?;
                if blob.size() as u64 > MAX_FILE_VIEW_BYTES {
                    return Err(too_large(blob.size() as u64));
                }
                Ok(blob.content().to_vec())
            };
            match blob_in(&commit) {
                Ok(b) => b,
                Err(_) => blob_in(&commit.parent(0)?)?,
            }
        }
    };
    if bytes.contains(&0) {
        return Ok(FileDiff::Binary);
    }
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    let truncated = all.len() > MAX_DIFF_LINES;
    let mut max_chars = 0;
    let lines = all
        .into_iter()
        .take(MAX_DIFF_LINES)
        .enumerate()
        .map(|(i, l)| {
            let l = l.replace('\t', "    ");
            max_chars = max_chars.max(l.chars().count() + 1);
            DiffLine { kind: LineKind::Context, hunk: 0, old_no: 0, new_no: i as u32 + 1, text: l.into() }
        })
        .collect();
    Ok(FileDiff::Text { lines, truncated, max_chars })
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;

    fn commit_file(repo: &Repository, name: &str, content: &str, parent: Option<Oid>) -> Oid {
        std::fs::write(repo.workdir().unwrap().join(name), content).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new(name)).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = Signature::now("T", "t@example.com").unwrap();
        let parents: Vec<Commit> = parent.map(|p| repo.find_commit(p).unwrap()).into_iter().collect();
        let refs: Vec<&Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "msg", &tree, &refs).unwrap()
    }

    #[test]
    fn detail_and_file_diff() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let a = commit_file(&repo, "f.txt", "one\ntwo\n", None);
        let b = commit_file(&repo, "f.txt", "one\n2\nthree\n", Some(a));

        let root = commit_detail(dir.path(), a).unwrap();
        assert_eq!(root.files.len(), 1);
        assert_eq!(root.files[0].kind, ChangeKind::Added);
        assert_eq!(root.files[0].additions, 2);

        let d = commit_detail(dir.path(), b).unwrap();
        assert_eq!((d.files[0].additions, d.files[0].deletions), (2, 1));
        assert_eq!(d.parents, vec![a]);

        let FileDiff::Text { lines, truncated, .. } =
            file_diff(dir.path(), b, "f.txt", None, false).unwrap()
        else {
            panic!("expected text diff")
        };
        assert!(!truncated);
        assert_eq!(lines[0].kind, LineKind::Hunk);
        assert_eq!(lines.iter().filter(|l| l.kind == LineKind::Add).count(), 2);
        assert_eq!(lines.iter().filter(|l| l.kind == LineKind::Del).count(), 1);

        // Full file contents at a commit.
        let FileDiff::Text { lines, .. } = file_text(dir.path(), FileSource::Commit(b), "f.txt").unwrap() else {
            panic!("expected text")
        };
        let texts: Vec<_> = lines.iter().map(|l| &*l.text).collect();
        assert_eq!(texts, ["one", "2", "three"]);
        assert_eq!(lines[2].new_no, 3);
    }
}
