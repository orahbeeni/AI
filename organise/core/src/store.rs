//! The data folder on disk:
//!
//! ```text
//! Organise/
//!   workspace.yaml          categories, list order, settings
//!   board.yaml              board columns and their cards
//!   links.yaml              what each link points at (name, size, fingerprint, url)
//!   lists/<title>.md        one Markdown file per list
//!   .machines/<host>.yaml   where each linked file is on this computer (path, inode) + recent moves
//!   .trash/                 list files of deleted lists
//! ```

use crate::links::{Link, LinkKind, LinkStatus};
use crate::markdown;
use crate::model::{Category, Column, Doc, Node, Settings};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceFile {
    version: u32,
    categories: Vec<Category>,
    /// List ids in sidebar order.
    lists: Vec<String>,
    #[serde(default)]
    settings: Settings,
}

#[derive(Serialize, Deserialize)]
struct BoardFile {
    columns: Vec<Column>,
}

/// The part of a link that is the same on every machine.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedLink {
    kind: LinkKind,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mtime: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
}

/// Where a link's file is on this machine.
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LocalLink {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inode: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dev: Option<u64>,
    #[serde(default, skip_serializing_if = "is_ok")]
    status: LinkStatus,
}

fn is_ok(s: &LinkStatus) -> bool {
    *s == LinkStatus::Ok
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MoveRecord {
    pub link_id: String,
    pub name: String,
    pub from: String,
    pub to: String,
    /// Unix seconds.
    pub at: u64,
    /// How it was found: "watcher", "inode", "content", "you".
    pub how: String,
}

#[derive(Serialize, Deserialize, Default)]
struct MachineFile {
    #[serde(default)]
    links: BTreeMap<String, LocalLink>,
    #[serde(default)]
    moves: Vec<MoveRecord>,
}

pub struct Store {
    pub dir: PathBuf,
    pub host: String,
    pub doc: Doc,
    pub links: BTreeMap<String, Link>,
    pub moves: Vec<MoveRecord>,
    /// list id -> file name inside lists/
    files: HashMap<String, String>,
    /// Links created recently are never garbage-collected (the UI may not have saved the item yet).
    fresh: HashMap<String, Instant>,
}

pub fn host_name() -> String {
    let h = gethostname::gethostname().to_string_lossy().into_owned();
    let h: String = h.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    if h.is_empty() {
        "this-computer".into()
    } else {
        h
    }
}

/// Writes `content` to `path` through a temporary file, unless the file already holds exactly that.
pub fn write_if_changed(path: &Path, content: &str) -> Result<bool> {
    if std::fs::read_to_string(path).map(|old| old == content).unwrap_or(false) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(".{}.tmp", path.file_name().unwrap().to_string_lossy()));
    std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(true)
}

impl Store {
    /// Opens the data folder, creating it with a starter workspace if it doesn't exist yet.
    pub fn open(dir: &Path) -> Result<Store> {
        let mut st = Store {
            dir: dir.to_path_buf(),
            host: host_name(),
            doc: Doc::default(),
            links: BTreeMap::new(),
            moves: vec![],
            files: HashMap::new(),
            fresh: HashMap::new(),
        };
        if !dir.join("workspace.yaml").exists() {
            std::fs::create_dir_all(dir.join("lists"))?;
            st.doc = Doc::starter();
            st.save_doc()?;
            st.save_links()?;
            return Ok(st);
        }
        st.load()?;
        Ok(st)
    }

    fn machine_file(&self) -> PathBuf {
        self.dir.join(".machines").join(format!("{}.yaml", self.host))
    }

    pub fn load(&mut self) -> Result<()> {
        let ws: WorkspaceFile = serde_yaml::from_str(&std::fs::read_to_string(self.dir.join("workspace.yaml"))?)
            .context("reading workspace.yaml")?;
        let board: BoardFile = std::fs::read_to_string(self.dir.join("board.yaml"))
            .ok()
            .and_then(|s| serde_yaml::from_str(&s).ok())
            .unwrap_or(BoardFile { columns: Doc::starter().columns.into_iter().map(|mut c| { c.cards.clear(); c }).collect() });

        // lists
        let mut by_id: HashMap<String, Node> = HashMap::new();
        let mut file_order: Vec<String> = vec![];
        let mut dirty_lists = false;
        self.files.clear();
        if let Ok(rd) = std::fs::read_dir(self.dir.join("lists")) {
            let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "md")).collect();
            entries.sort();
            for p in entries {
                let Ok(text) = std::fs::read_to_string(&p) else { continue };
                let stem = p.file_stem().unwrap().to_string_lossy().replace('-', " ");
                let (mut list, dirty) = markdown::parse_list(&text, &stem);
                if by_id.contains_key(&list.id) {
                    list.id = crate::model::new_id("L"); // a copied file: give it its own id
                    dirty_lists = true;
                }
                dirty_lists |= dirty;
                self.files.insert(list.id.clone(), p.file_name().unwrap().to_string_lossy().into_owned());
                file_order.push(list.id.clone());
                by_id.insert(list.id.clone(), list);
            }
        }
        let mut lists = vec![];
        for id in ws.lists.iter().chain(file_order.iter()) {
            if let Some(l) = by_id.remove(id) {
                lists.push(l);
            }
        }
        let mut doc = Doc { categories: ws.categories, columns: board.columns, lists, settings: ws.settings };
        let ids = doc.node_ids();
        for c in &mut doc.columns {
            c.cards.retain(|id| ids.contains(id));
        }
        self.doc = doc;

        // links
        let shared: BTreeMap<String, SharedLink> = std::fs::read_to_string(self.dir.join("links.yaml"))
            .ok()
            .and_then(|s| serde_yaml::from_str(&s).ok())
            .unwrap_or_default();
        let machine: MachineFile = std::fs::read_to_string(self.machine_file())
            .ok()
            .and_then(|s| serde_yaml::from_str(&s).ok())
            .unwrap_or_default();
        self.links.clear();
        for (id, s) in shared {
            let local = machine.links.get(&id);
            let mut l = Link {
                id: id.clone(),
                kind: s.kind,
                name: s.name,
                url: s.url,
                size: s.size,
                mtime: s.mtime,
                fingerprint: s.fingerprint,
                path: local.and_then(|m| m.path.clone()),
                inode: local.and_then(|m| m.inode),
                dev: local.and_then(|m| m.dev),
                status: local.map(|m| m.status).unwrap_or_default(),
                candidates: vec![],
            };
            if l.is_fs() && l.path.is_none() {
                l.status = LinkStatus::Missing; // linked on another machine
            }
            self.links.insert(id, l);
        }
        self.moves = machine.moves;
        if dirty_lists {
            self.save_doc()?;
        }
        Ok(())
    }

    /// Writes workspace.yaml, board.yaml and every list file that changed.
    pub fn save_doc(&mut self) -> Result<()> {
        std::fs::create_dir_all(self.dir.join("lists"))?;
        let ws = WorkspaceFile {
            version: 1,
            categories: self.doc.categories.clone(),
            lists: self.doc.lists.iter().map(|l| l.id.clone()).collect(),
            settings: self.doc.settings.clone(),
        };
        write_if_changed(&self.dir.join("workspace.yaml"), &serde_yaml::to_string(&ws)?)?;
        write_if_changed(&self.dir.join("board.yaml"), &serde_yaml::to_string(&BoardFile { columns: self.doc.columns.clone() })?)?;

        // decide file names: keep a list's file unless its title's slug changed
        let mut taken: HashSet<String> = HashSet::new();
        let mut wanted: Vec<(String, String)> = vec![];
        for l in &self.doc.lists {
            let slug = markdown::slug(&l.text);
            let current = self.files.get(&l.id).cloned();
            let keep = current.as_ref().filter(|f| {
                let stem = f.trim_end_matches(".md");
                (stem == slug || stem.strip_prefix(&slug).is_some_and(|r| r.starts_with('-') && r[1..].chars().all(|c| c.is_ascii_digit())))
                    && !taken.contains(*f)
            });
            let name = match keep {
                Some(f) => f.clone(),
                None => {
                    let mut n = format!("{slug}.md");
                    let mut i = 2;
                    // never overwrite a file this list doesn't own (another list's, or a stray file)
                    while taken.contains(&n) || (self.dir.join("lists").join(&n).exists() && self.files.get(&l.id) != Some(&n)) {
                        n = format!("{slug}-{i}.md");
                        i += 1;
                    }
                    n
                }
            };
            taken.insert(name.clone());
            wanted.push((l.id.clone(), name));
        }
        let links = &self.links;
        let name_of = |id: &str| links.get(id).map(|l| l.name.clone());
        for (l, (id, name)) in self.doc.lists.iter().zip(wanted.iter()) {
            let text = markdown::write_list(l, &name_of);
            write_if_changed(&self.dir.join("lists").join(name), &text)?;
            if let Some(old) = self.files.get(id) {
                if old != name && !taken.contains(old) {
                    let _ = std::fs::remove_file(self.dir.join("lists").join(old));
                }
            }
        }
        // lists that were deleted: move their files to .trash
        let live: HashSet<&String> = self.doc.lists.iter().map(|l| &l.id).collect();
        for (id, file) in self.files.iter() {
            if !live.contains(id) && !taken.contains(file) {
                let src = self.dir.join("lists").join(file);
                if src.exists() {
                    let trash = self.dir.join(".trash");
                    std::fs::create_dir_all(&trash)?;
                    let _ = std::fs::rename(&src, trash.join(format!("{}-{}", crate::links::now_secs(), file)));
                }
            }
        }
        self.files = wanted.into_iter().collect();
        Ok(())
    }

    /// Writes links.yaml and this machine's file.
    pub fn save_links(&mut self) -> Result<()> {
        let mut shared = BTreeMap::new();
        let mut machine = MachineFile { links: BTreeMap::new(), moves: self.moves.clone() };
        for (id, l) in &self.links {
            shared.insert(
                id.clone(),
                SharedLink { kind: l.kind, name: l.name.clone(), url: l.url.clone(), size: l.size, mtime: l.mtime, fingerprint: l.fingerprint.clone() },
            );
            if l.is_fs() {
                machine.links.insert(id.clone(), LocalLink { path: l.path.clone(), inode: l.inode, dev: l.dev, status: l.status });
            }
        }
        write_if_changed(&self.dir.join("links.yaml"), &serde_yaml::to_string(&shared)?)?;
        write_if_changed(&self.machine_file(), &serde_yaml::to_string(&machine)?)?;
        Ok(())
    }

    pub fn add_link(&mut self, l: Link) -> Link {
        self.fresh.insert(l.id.clone(), Instant::now());
        self.links.insert(l.id.clone(), l.clone());
        l
    }

    /// An existing link to the same path, if there is one.
    pub fn link_for_path(&self, path: &str) -> Option<&Link> {
        self.links.values().find(|l| l.path.as_deref() == Some(path))
    }

    /// Removes links no item refers to any more. Returns true if any were removed.
    pub fn gc_links(&mut self) -> bool {
        let used = self.doc.link_ids();
        let fresh = &self.fresh;
        let before = self.links.len();
        self.links.retain(|id, _| used.contains(id) || fresh.get(id).is_some_and(|t| t.elapsed().as_secs() < 120));
        self.fresh.retain(|_, t| t.elapsed().as_secs() < 120);
        self.links.len() != before
    }

    pub fn record_move(&mut self, m: MoveRecord) {
        self.moves.insert(0, m);
        self.moves.truncate(50);
    }
}
