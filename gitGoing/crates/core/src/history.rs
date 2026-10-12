//! Repository history loading, designed to stream into a UI.

use crate::graph::{GraphLayout, GraphRow};
use git2::{Oid, Repository, Sort};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct CommitRow {
    pub id: Oid,
    pub summary: Box<str>,
    pub author: Box<str>,
    /// Seconds since the unix epoch.
    pub time: i64,
    pub graph: GraphRow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Head,
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
}

#[derive(Clone, Debug)]
pub struct Upstream {
    pub name: String,
    pub ahead: usize,
    pub behind: usize,
}

pub struct RepoInfo {
    pub path: PathBuf,
    pub head_branch: Option<String>,
    pub upstream: Option<Upstream>,
    /// The commit HEAD points at (also when detached).
    pub head_oid: Option<Oid>,
    pub remotes: Vec<crate::remotes::RemoteInfo>,
    pub stashes: Vec<crate::stash::StashEntry>,
    pub refs: HashMap<Oid, Vec<RefLabel>>,
}

pub enum LoadEvent {
    Info(RepoInfo),
    Rows(Vec<CommitRow>),
    Done { max_cols: u16 },
    Error(String),
}

const BATCH: usize = 2000;

/// Walk the full history of `path`, calling `emit` with batches of rows.
/// Returns early (without error) when `emit` returns `false`, so a caller
/// can cancel by dropping its receiver.
pub fn load(path: &Path, mut emit: impl FnMut(LoadEvent) -> bool) {
    if let Err(e) = load_inner(path, &mut emit) {
        emit(LoadEvent::Error(e.message().to_owned()));
    }
}

fn load_inner(path: &Path, emit: &mut impl FnMut(LoadEvent) -> bool) -> Result<(), git2::Error> {
    let mut repo = Repository::discover(path)?;
    let info = collect_info(&mut repo)?;
    if !emit(LoadEvent::Info(info)) {
        return Ok(());
    }

    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    for glob in ["refs/heads/*", "refs/remotes/*", "refs/tags/*"] {
        walk.push_glob(glob)?;
    }
    let _ = walk.push_head(); // fails harmlessly on an unborn branch

    let mut layout = GraphLayout::new();
    let mut batch = Vec::with_capacity(BATCH);
    for oid in walk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let parents: Vec<Oid> = commit.parent_ids().collect();
        let graph = layout.push(oid, &parents);
        batch.push(CommitRow {
            id: oid,
            summary: commit.summary().unwrap_or("").into(),
            author: commit.author().name().unwrap_or("").into(),
            time: commit.time().seconds(),
            graph,
        });
        if batch.len() == BATCH {
            let full = std::mem::replace(&mut batch, Vec::with_capacity(BATCH));
            if !emit(LoadEvent::Rows(full)) {
                return Ok(());
            }
        }
    }
    if !batch.is_empty() && !emit(LoadEvent::Rows(batch)) {
        return Ok(());
    }
    emit(LoadEvent::Done { max_cols: layout.max_cols });
    Ok(())
}

fn collect_info(repo: &mut Repository) -> Result<RepoInfo, git2::Error> {
    let mut refs: HashMap<Oid, Vec<RefLabel>> = HashMap::new();
    let mut head_branch = None;
    if let Ok(head) = repo.head() {
        if head.is_branch() {
            head_branch = head.shorthand().map(str::to_owned);
        }
    }
    for r in repo.references()? {
        let r = r?;
        let Some(name) = r.name() else { continue };
        let (short, kind) = if let Some(s) = name.strip_prefix("refs/heads/") {
            let k = if head_branch.as_deref() == Some(s) { RefKind::Head } else { RefKind::Local };
            (s, k)
        } else if let Some(s) = name.strip_prefix("refs/remotes/") {
            if s.ends_with("/HEAD") {
                continue;
            }
            (s, RefKind::Remote)
        } else if let Some(s) = name.strip_prefix("refs/tags/") {
            (s, RefKind::Tag)
        } else {
            continue;
        };
        // Peel annotated tags to the commit they point at.
        let Ok(target) = r.peel_to_commit() else { continue };
        refs.entry(target.id())
            .or_default()
            .push(RefLabel { name: short.to_owned(), kind });
    }
    for labels in refs.values_mut() {
        labels.sort_by_key(|l| l.kind as u8);
    }
    let upstream = repo
        .head()
        .ok()
        .filter(|h| h.is_branch())
        .and_then(|h| {
            let local = h.target()?;
            let up = git2::Branch::wrap(h).upstream().ok()?;
            let name = up.name().ok()??.to_owned();
            let (ahead, behind) = repo.graph_ahead_behind(local, up.get().target()?).ok()?;
            Some(Upstream { name, ahead, behind })
        });
    let path = repo.workdir().unwrap_or_else(|| repo.path()).to_path_buf();
    let stashes = crate::stash::list(repo);
    let head_oid = repo.head().ok().and_then(|h| h.target());
    let remotes = crate::remotes::list(repo);
    Ok(RepoInfo { path, head_branch, upstream, head_oid, remotes, stashes, refs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_history_with_refs() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("T", "t@example.com").unwrap();
        let tree = repo.find_tree(repo.treebuilder(None).unwrap().write().unwrap()).unwrap();
        let a = repo.commit(Some("HEAD"), &sig, &sig, "first", &tree, &[]).unwrap();
        let a = repo.find_commit(a).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&a]).unwrap();

        let mut rows = Vec::new();
        let mut info = None;
        load(dir.path(), |ev| {
            match ev {
                LoadEvent::Info(i) => info = Some(i),
                LoadEvent::Rows(r) => rows.extend(r),
                LoadEvent::Done { .. } => {}
                LoadEvent::Error(e) => panic!("{e}"),
            }
            true
        });
        assert_eq!(rows.len(), 2);
        assert_eq!(&*rows[0].summary, "second");
        let info = info.unwrap();
        assert!(info.refs.values().flatten().any(|l| l.kind == RefKind::Head));
    }
}
