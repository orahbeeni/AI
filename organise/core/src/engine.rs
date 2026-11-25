//! Turns file-system events into link updates.
//!
//! Changes wait 1.5 s before they count, because many programs save a file by renaming the old one away
//! (`report.odt` → `report.odt~` or a temp name) and writing a new one under the same name. If the old path
//! exists again when the delay is up, the link stays where it was.

use crate::links::{self, Link, LinkStatus};
use crate::scan::Found;
use crate::store::MoveRecord;
use crate::tracker::FsEvent;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DELAY: Duration = Duration::from_millis(1500);

#[derive(Debug)]
struct Pending {
    id: String,
    old: PathBuf,
    new: Option<PathBuf>,
    due: Instant,
}

#[derive(Debug, Default)]
pub struct Outcome {
    /// Links whose details changed (send them to the UI and save).
    pub changed: Vec<String>,
    pub moves: Vec<MoveRecord>,
    /// Links that should be searched for.
    pub search: Vec<String>,
}

impl Outcome {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.moves.is_empty() && self.search.is_empty()
    }
    pub fn merge(&mut self, o: Outcome) {
        for c in o.changed {
            if !self.changed.contains(&c) {
                self.changed.push(c);
            }
        }
        self.moves.extend(o.moves);
        for s in o.search {
            if !self.search.contains(&s) {
                self.search.push(s);
            }
        }
    }
}

#[derive(Default)]
pub struct Engine {
    pending: Vec<Pending>,
}

fn link_path(l: &Link) -> Option<PathBuf> {
    l.path.as_ref().map(PathBuf::from)
}

fn move_record(l: &Link, from: &Path, to: &Path, how: &str) -> MoveRecord {
    MoveRecord {
        link_id: l.id.clone(),
        name: links::file_name(to),
        from: from.display().to_string(),
        to: to.display().to_string(),
        at: links::now_secs(),
        how: how.into(),
    }
}

/// Does the file at `p` look like this (missing) link's file?
fn same_file(l: &Link, p: &Path) -> bool {
    let Ok(info) = links::stat(p) else { return false };
    if info.is_dir != (l.kind == links::LinkKind::Folder) {
        return false;
    }
    if l.inode == Some(info.inode) && l.dev == Some(info.dev) {
        return true;
    }
    !info.is_dir
        && links::file_name(p) == l.name
        && l.size == Some(info.size)
        && l.fingerprint.is_some()
        && links::fingerprint(p, info.size).ok() == l.fingerprint
}

impl Engine {
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn schedule(&mut self, id: &str, old: PathBuf, new: Option<PathBuf>) {
        self.pending.retain(|p| p.id != id);
        self.pending.push(Pending { id: id.to_string(), old, new, due: Instant::now() + DELAY });
    }

    pub fn on_event(&mut self, ev: &FsEvent, links: &mut BTreeMap<String, Link>) -> Outcome {
        let mut out = Outcome::default();
        match ev {
            FsEvent::Moved { from, to } => {
                for l in links.values().filter(|l| l.is_fs()) {
                    let Some(p) = link_path(l) else { continue };
                    if let Ok(rest) = p.strip_prefix(from) {
                        let new = if rest.as_os_str().is_empty() { to.clone() } else { to.join(rest) };
                        self.schedule(&l.id, p.clone(), Some(new));
                    }
                }
                // a move can also bring a missing file back into view
                out.merge(self.appeared(to, links));
            }
            FsEvent::Gone(path) => {
                for l in links.values().filter(|l| l.is_fs()) {
                    let Some(p) = link_path(l) else { continue };
                    if p.starts_with(path) {
                        self.schedule(&l.id, p.clone(), None);
                    }
                }
            }
            FsEvent::Appeared(path) => out.merge(self.appeared(path, links)),
            FsEvent::Overflow => out.merge(self.verify(links)),
        }
        out
    }

    fn appeared(&mut self, path: &Path, links: &mut BTreeMap<String, Link>) -> Outcome {
        let mut out = Outcome::default();
        for l in links.values_mut().filter(|l| l.is_fs() && l.status != LinkStatus::Ok) {
            if same_file(l, path) {
                let old = link_path(l).unwrap_or_default();
                if let Ok(info) = links::stat(path) {
                    if old != path {
                        out.moves.push(move_record(l, &old, path, "watcher"));
                    }
                    l.refresh_from(path, &info);
                    self.pending.retain(|p| p.id != l.id);
                    out.changed.push(l.id.clone());
                }
            }
        }
        out
    }

    /// Commits changes whose delay is up.
    pub fn tick(&mut self, links: &mut BTreeMap<String, Link>) -> Outcome {
        let now = Instant::now();
        let mut out = Outcome::default();
        let (due, rest): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending).into_iter().partition(|p| p.due <= now);
        self.pending = rest;
        for p in due {
            let Some(l) = links.get_mut(&p.id) else { continue };
            if link_path(l).as_deref() != Some(p.old.as_path()) {
                continue; // changed by something else meanwhile
            }
            if let Ok(info) = links::stat(&p.old) {
                // the name is back: a "safe save" or a delete-and-recreate. Keep the link here.
                if l.inode != Some(info.inode) || l.size != Some(info.size) || l.status != LinkStatus::Ok {
                    l.refresh_from(&p.old, &info);
                    out.changed.push(l.id.clone());
                }
                continue;
            }
            if let Some(new) = &p.new {
                if let Ok(info) = links::stat(new) {
                    out.moves.push(move_record(l, &p.old, new, "watcher"));
                    l.refresh_from(new, &info);
                    out.changed.push(l.id.clone());
                    continue;
                }
            }
            l.status = LinkStatus::Searching;
            out.changed.push(l.id.clone());
            out.search.push(l.id.clone());
        }
        out
    }

    /// Checks every file link against the disk (at startup, or after the kernel dropped events).
    pub fn verify(&mut self, links: &mut BTreeMap<String, Link>) -> Outcome {
        let mut out = Outcome::default();
        for l in links.values_mut().filter(|l| l.is_fs()) {
            let Some(p) = link_path(l) else { continue };
            match links::stat(&p) {
                Ok(info) => {
                    let kind_ok = info.is_dir == (l.kind == links::LinkKind::Folder);
                    if kind_ok && (l.status != LinkStatus::Ok || l.inode != Some(info.inode) || l.size != Some(info.size) || l.mtime != Some(info.mtime)) {
                        l.refresh_from(&p, &info);
                        out.changed.push(l.id.clone());
                    }
                }
                Err(_) => {
                    if l.status != LinkStatus::Searching {
                        l.status = LinkStatus::Searching;
                        out.changed.push(l.id.clone());
                    }
                    out.search.push(l.id.clone());
                }
            }
        }
        out
    }

    /// Applies what a search found.
    pub fn apply_found(&mut self, found: HashMap<String, Found>, links: &mut BTreeMap<String, Link>) -> Outcome {
        let mut out = Outcome::default();
        for (id, f) in found {
            let Some(l) = links.get_mut(&id) else { continue };
            if l.status == LinkStatus::Ok {
                continue; // found meanwhile
            }
            match f {
                Found::Exact { path, how } => {
                    if let Ok(info) = links::stat(&path) {
                        let old = link_path(l).unwrap_or_default();
                        out.moves.push(move_record(l, &old, &path, how));
                        l.refresh_from(&path, &info);
                    }
                }
                Found::Candidates(c) => {
                    l.status = LinkStatus::Missing;
                    l.candidates = c;
                }
                Found::NotFound => {
                    l.status = LinkStatus::Missing;
                    l.candidates.clear();
                }
            }
            out.changed.push(id);
        }
        out
    }
}
