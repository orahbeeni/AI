//! Repository history loading, designed to stream into a UI.

use crate::graph::{GraphLayout, GraphRow};
use git2::{Oid, Repository, Sort};
use std::collections::{BTreeSet, HashMap};
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

impl RefLabel {
    /// The full reference name, e.g. `refs/heads/main`.
    pub fn full_name(&self) -> String {
        full_ref_name(self.kind, &self.name)
    }
}

/// The full reference name for a short name of the given kind.
pub fn full_ref_name(kind: RefKind, name: &str) -> String {
    let prefix = match kind {
        RefKind::Head | RefKind::Local => "refs/heads/",
        RefKind::Remote => "refs/remotes/",
        RefKind::Tag => "refs/tags/",
    };
    format!("{prefix}{name}")
}

/// What to load.
#[derive(Clone, Debug, Default)]
pub struct LoadOptions {
    /// Full names of branches and tags to leave out of the graph. Commits only
    /// reachable from them are not listed. The checked-out branch is always shown.
    pub hidden: BTreeSet<String>,
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
    /// Saved change sets (`refs/keep/*`).
    pub change_sets: Vec<crate::changesets::ChangeSet>,
    pub refs: HashMap<Oid, Vec<RefLabel>>,
    /// Local branch (short name) -> (commits to push, files changed by upstream
    /// commits it doesn't have yet). Only branches with something to push or pull appear.
    pub unpulled: HashMap<String, (usize, usize)>,
}

/// For every local branch with an upstream: the commits ahead, and how many
/// files the unpulled commits touch (merge base tree vs. upstream tree).
fn unpulled_files(repo: &Repository) -> HashMap<String, (usize, usize)> {
    let mut out = HashMap::new();
    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else { return out };
    for (b, _) in branches.flatten() {
        let Ok(Some(name)) = b.name().map(|n| n.map(str::to_owned)) else { continue };
        let Ok(up) = b.upstream() else { continue };
        let (Some(local), Some(remote)) = (b.get().target(), up.get().target()) else { continue };
        let Ok((ahead, behind)) = repo.graph_ahead_behind(local, remote) else { continue };
        if ahead == 0 && behind == 0 {
            continue;
        }
        let mut files = 0;
        if behind > 0 {
            if let Ok(base) = repo.merge_base(local, remote) {
                let tree = |oid| repo.find_commit(oid).and_then(|c| c.tree());
                if let (Ok(a), Ok(z)) = (tree(base), tree(remote)) {
                    if let Ok(d) = repo.diff_tree_to_tree(Some(&a), Some(&z), None) {
                        files = d.deltas().len();
                    }
                }
            }
        }
        out.insert(name, (ahead, files));
    }
    out
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
pub fn load(path: &Path, opts: &LoadOptions, mut emit: impl FnMut(LoadEvent) -> bool) {
    if let Err(e) = load_inner(path, opts, &mut emit) {
        emit(LoadEvent::Error(e.message().to_owned()));
    }
}

fn load_inner(
    path: &Path,
    opts: &LoadOptions,
    emit: &mut impl FnMut(LoadEvent) -> bool,
) -> Result<(), git2::Error> {
    crate::gitlog::read(path, "git log --all --topo-order   # plus git for-each-ref for branches, tags and remotes");
    let mut repo = Repository::discover(path)?;
    let info = collect_info(&mut repo)?;
    if !emit(LoadEvent::Info(info)) {
        return Ok(());
    }

    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    if opts.hidden.is_empty() {
        for glob in ["refs/heads/*", "refs/remotes/*", "refs/tags/*"] {
            walk.push_glob(glob)?;
        }
    } else {
        // Only the visible branches and tags start the walk.
        for r in repo.references()? {
            let r = r?;
            let Some(name) = r.name() else { continue };
            let listed = name.starts_with("refs/heads/") || name.starts_with("refs/tags/")
                || (name.starts_with("refs/remotes/") && !name.ends_with("/HEAD"));
            if !listed || opts.hidden.contains(name) {
                continue;
            }
            if let Ok(c) = r.peel_to_commit() {
                walk.push(c.id())?;
            }
        }
    }
    let _ = walk.push_head(); // the checked-out commit is always shown; fails harmlessly when unborn

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
    let change_sets = crate::changesets::list(repo);
    let head_oid = repo.head().ok().and_then(|h| h.target());
    let remotes = crate::remotes::list(repo);
    let unpulled = unpulled_files(repo);
    Ok(RepoInfo { path, head_branch, upstream, head_oid, remotes, stashes, change_sets, refs, unpulled })
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
        load(dir.path(), &LoadOptions::default(), |ev| {
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

    #[test]
    fn hidden_branches_and_tags_are_left_out_of_the_graph() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("T", "t@example.com").unwrap();
        let tree = repo.find_tree(repo.treebuilder(None).unwrap().write().unwrap()).unwrap();
        let a = repo.commit(Some("HEAD"), &sig, &sig, "A", &tree, &[]).unwrap();
        let main = repo.head().unwrap().name().unwrap().to_owned();
        let a_commit = repo.find_commit(a).unwrap();
        let b = repo.commit(Some("refs/heads/feature"), &sig, &sig, "B (feature only)", &tree, &[&a_commit]).unwrap();
        let t = repo.commit(None, &sig, &sig, "T (tagged only)", &tree, &[&a_commit]).unwrap();
        repo.reference("refs/tags/v1", t, false, "tag").unwrap();
        let c = repo.commit(Some(&main), &sig, &sig, "C", &tree, &[&a_commit]).unwrap();

        let summaries = |hidden: &[&str]| {
            let opts = LoadOptions { hidden: hidden.iter().map(|s| s.to_string()).collect() };
            let mut rows = Vec::new();
            load(dir.path(), &opts, |ev| {
                if let LoadEvent::Rows(r) = ev {
                    rows.extend(r.into_iter().map(|r| r.summary.to_string()));
                }
                true
            });
            rows.sort();
            rows
        };
        assert_eq!(summaries(&[]), ["A", "B (feature only)", "C", "T (tagged only)"]);
        assert_eq!(summaries(&["refs/heads/feature"]), ["A", "C", "T (tagged only)"]);
        assert_eq!(summaries(&["refs/tags/v1", "refs/heads/feature"]), ["A", "C"]);
        // The checked-out branch can't be hidden.
        assert_eq!(summaries(&[main.as_str(), "refs/heads/feature", "refs/tags/v1"]), ["A", "C"]);
        let _ = (b, c);
        assert_eq!(full_ref_name(RefKind::Remote, "origin/x"), "refs/remotes/origin/x");
    }

    /// Rough speed check on a generated history: `cargo test -p gitgoing-core --release -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn loads_a_large_history_quickly() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        const COMMITS: usize = 50_000;
        let dir = tempfile::tempdir().unwrap();
        Command::new("git").arg("-C").arg(dir.path()).args(["init", "-q", "-b", "main"]).status().unwrap();
        // A branchy history: every 50 commits fork a side branch and merge it back.
        let mut stream = String::new();
        let mut mark = 0usize;
        let mut last_main = 0usize;
        let mut ts = 1_600_000_000usize;
        let mut commit = |stream: &mut String, branch: &str, parents: &[usize], mark: &mut usize| -> usize {
            *mark += 1;
            ts += 60;
            stream.push_str(&format!(
                "commit refs/heads/{branch}\nmark :{mark}\ncommitter T <t@e.com> {ts} +0000\ndata 2\nc\n"
            ));
            if let Some((first, rest)) = parents.split_first() {
                stream.push_str(&format!("from :{first}\n"));
                for r in rest {
                    stream.push_str(&format!("merge :{r}\n"));
                }
            }
            stream.push('\n');
            *mark
        };
        for i in 0..COMMITS {
            let parents: Vec<usize> = if last_main == 0 { vec![] } else { vec![last_main] };
            last_main = commit(&mut stream, "main", &parents, &mut mark);
            if i % 50 == 49 {
                let side = commit(&mut stream, "side", &[last_main], &mut mark);
                last_main = commit(&mut stream, "main", &[last_main, side], &mut mark);
            }
        }
        let mut child = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["fast-import", "--quiet"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stream.as_bytes()).unwrap();
        assert!(child.wait().unwrap().success());

        let start = std::time::Instant::now();
        let mut rows = 0usize;
        let mut first_batch = None;
        load(dir.path(), &LoadOptions::default(), |ev| {
            if let LoadEvent::Rows(r) = ev {
                first_batch.get_or_insert(start.elapsed());
                rows += r.len();
            }
            true
        });
        let total = start.elapsed();
        eprintln!(
            "loaded {rows} commits in {total:?} (first rows after {:?}); {:.1} bytes/row in memory (shallow estimate)",
            first_batch.unwrap(),
            std::mem::size_of::<CommitRow>() as f64
        );
        assert!(rows >= COMMITS);
        assert!(total.as_secs() < 30, "loading should stay fast");
    }
}
