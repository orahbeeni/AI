//! Finding files that left their path while nobody was watching.
//!
//! The walk starts at the folder the file was last seen in and works outward (that folder, then its parent,
//! then the grandparent...), so a file moved to a nearby folder is found in a moment even in a home folder
//! with a million files. A match is:
//! - **inode**: same inode on the same device (a move or rename within one disk keeps it), checked
//!   against the name, size or content so a reused inode isn't mistaken for the file;
//! - **content**: same name, same size and same content fingerprint (a copy to another disk, or a move made
//!   while the old file was deleted).
//!
//! Weaker matches (same name only) are returned as candidates for the person to confirm.

use crate::links::{self, Candidate, Link, LinkKind};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub inode: Option<u64>,
    pub dev: Option<u64>,
    pub fingerprint: Option<String>,
    /// Where it was last seen.
    pub origin: Option<PathBuf>,
}

impl Target {
    pub fn from_link(l: &Link) -> Target {
        Target {
            id: l.id.clone(),
            name: l.name.clone(),
            is_dir: l.kind == LinkKind::Folder,
            size: l.size,
            inode: l.inode,
            dev: l.dev,
            fingerprint: l.fingerprint.clone(),
            origin: l.path.as_ref().map(PathBuf::from),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    /// Confidently the same file; `how` is "inode" or "content".
    Exact { path: PathBuf, how: &'static str },
    /// Possible matches to show the person.
    Candidates(Vec<Candidate>),
    NotFound,
}

/// Folders never worth walking.
pub fn skip_dir(name: &str, path: &Path, skip_hidden: bool) -> bool {
    (skip_hidden && name.starts_with('.'))
        || matches!(name, "node_modules" | "__pycache__" | "proc" | "sys")
        || (name == "target" && path.join("CACHEDIR.TAG").exists())
}

/// The folders to walk, nearest first: each origin's folder, then its parents up to (and including) the
/// root that contains it, then the remaining roots. Each entry says which child was already walked.
pub fn walk_plan(targets: &[Target], roots: &[PathBuf]) -> Vec<(PathBuf, Option<PathBuf>)> {
    let mut plan: Vec<(PathBuf, Option<PathBuf>)> = vec![];
    let mut covered: Vec<PathBuf> = vec![];
    let is_covered = |p: &Path, covered: &[PathBuf]| covered.iter().any(|c| p.starts_with(c));
    for t in targets {
        let Some(origin) = &t.origin else { continue };
        // the root that contains the old place; outside every root, only look two levels up
        let root = roots.iter().filter(|r| origin.starts_with(r)).max_by_key(|r| r.as_os_str().len()).cloned();
        let mut cur = origin.parent().map(Path::to_path_buf);
        let mut prev: Option<PathBuf> = None;
        let mut levels = 0;
        while let Some(dir) = cur {
            if is_covered(&dir, &covered) {
                break;
            }
            if dir.is_dir() {
                plan.push((dir.clone(), prev.clone()));
                covered.push(dir.clone());
            }
            levels += 1;
            match &root {
                Some(r) if r == &dir => break,
                None if levels >= 2 => break,
                _ => {}
            }
            prev = Some(dir.clone());
            cur = dir.parent().map(Path::to_path_buf);
        }
    }
    for r in roots {
        if !is_covered(r, &covered) && r.is_dir() {
            plan.push((r.clone(), None));
            covered.push(r.clone());
        }
    }
    plan
}

/// Searches `roots` (nearest to each target's old place first) for the targets.
pub fn search(targets: &[Target], roots: &[PathBuf], skip_hidden: bool, cancel: &AtomicBool) -> HashMap<String, Found> {
    let mut results: HashMap<String, Found> = HashMap::new();
    let mut by_inode: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, t) in targets.iter().enumerate() {
        if let Some(ino) = t.inode {
            by_inode.entry(ino).or_default().push(i);
        }
        by_name.entry(t.name.clone()).or_default().push(i);
    }
    let mut exact: HashSet<usize> = HashSet::new();
    let mut cands: HashMap<usize, Vec<Candidate>> = HashMap::new();
    let mut content_match: HashMap<usize, PathBuf> = HashMap::new();

    let mut check = |path: &Path, ino: u64, is_dir: bool, exact: &mut HashSet<usize>| {
        if let Some(ids) = by_inode.get(&ino) {
            for &i in ids {
                let t = &targets[i];
                if exact.contains(&i) || t.is_dir != is_dir || t.origin.as_deref() == Some(path) {
                    continue;
                }
                let Ok(info) = links::stat(path) else { continue };
                if t.dev.is_some_and(|d| d != info.dev) {
                    continue;
                }
                let same_name = links::file_name(path) == t.name;
                let same_size = !is_dir && t.size == Some(info.size);
                let same_fp = !is_dir && !same_size && t.fingerprint.is_some() && links::fingerprint(path, info.size).ok() == t.fingerprint;
                if is_dir || same_name || same_size || same_fp {
                    results.insert(t.id.clone(), Found::Exact { path: path.to_path_buf(), how: "inode" });
                    exact.insert(i);
                } else {
                    cands.entry(i).or_default().push(Candidate { path: path.display().to_string(), reason: "Same file identity, but renamed and changed".into() });
                }
            }
        }
        let name = links::file_name(path);
        if let Some(ids) = by_name.get(&name) {
            for &i in ids {
                let t = &targets[i];
                if exact.contains(&i) || t.is_dir != is_dir || t.origin.as_deref() == Some(path) {
                    continue;
                }
                if is_dir {
                    cands.entry(i).or_default().push(Candidate { path: path.display().to_string(), reason: "Folder with the same name".into() });
                    continue;
                }
                let Ok(info) = links::stat(path) else { continue };
                if t.size == Some(info.size) && t.fingerprint.is_some() && links::fingerprint(path, info.size).ok() == t.fingerprint {
                    content_match.entry(i).or_insert_with(|| path.to_path_buf());
                    cands.entry(i).or_default().push(Candidate { path: path.display().to_string(), reason: "Same name, size and content".into() });
                } else {
                    cands.entry(i).or_default().push(Candidate { path: path.display().to_string(), reason: "Same name".into() });
                }
            }
        }
    };

    let plan = walk_plan(targets, roots);
    'outer: for (dir, skip_child) in plan {
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                break 'outer;
            }
            if skip_child.as_ref() == Some(&d) {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_symlink() {
                    continue;
                }
                let p = e.path();
                #[cfg(unix)]
                let ino = std::os::unix::fs::DirEntryExt::ino(&e);
                #[cfg(not(unix))]
                let ino = 0u64;
                if ft.is_dir() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    check(&p, ino, true, &mut exact);
                    if !skip_dir(&name, &p, skip_hidden) && skip_child.as_ref() != Some(&p) {
                        stack.push(p);
                    }
                } else {
                    check(&p, ino, false, &mut exact);
                }
            }
            if exact.len() == targets.len() {
                break 'outer;
            }
        }
    }

    for (i, t) in targets.iter().enumerate() {
        if exact.contains(&i) {
            continue;
        }
        let found = if let Some(p) = content_match.get(&i).filter(|_| cands.get(&i).map(|c| c.iter().filter(|c| c.reason.starts_with("Same name, size")).count()).unwrap_or(0) == 1) {
            // exactly one file with the same name, size and content: that's it
            Found::Exact { path: p.clone(), how: "content" }
        } else if let Some(mut c) = cands.remove(&i) {
            c.sort_by_key(|c| if c.reason.starts_with("Same name, size") { 0 } else if c.reason.starts_with("Same file") { 1 } else { 2 });
            c.truncate(5);
            Found::Candidates(c)
        } else {
            Found::NotFound
        };
        results.insert(t.id.clone(), found);
    }
    results
}
