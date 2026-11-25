//! Links to files, folders and web pages, and the facts used to find a file again after it moves.

use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    File,
    Folder,
    Url,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LinkStatus {
    #[default]
    Ok,
    /// Gone from its path; a search is running.
    Searching,
    /// Gone and not found automatically.
    Missing,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub id: String,
    pub kind: LinkKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    // ---- this machine only (stored in .machines/<host>.yaml) ----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inode: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev: Option<u64>,
    #[serde(default)]
    pub status: LinkStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Candidate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
    pub inode: u64,
    pub dev: u64,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Facts about a path, without following a final symlink's target type (it does follow it for size).
pub fn stat(path: &Path) -> std::io::Result<FileInfo> {
    let m = std::fs::metadata(path)?;
    let mtime = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    #[cfg(unix)]
    let (inode, dev) = {
        use std::os::unix::fs::MetadataExt;
        (m.ino(), m.dev())
    };
    #[cfg(not(unix))]
    let (inode, dev) = (0u64, 0u64);
    Ok(FileInfo { is_dir: m.is_dir(), size: if m.is_dir() { 0 } else { m.len() }, mtime, inode, dev })
}

const CHUNK: u64 = 64 * 1024;

/// A cheap content fingerprint: the size plus a hash of the first and last 64 KB.
pub fn fingerprint(path: &Path, size: u64) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    h.update(&size.to_le_bytes());
    let mut buf = vec![0u8; CHUNK as usize];
    let n = read_full(&mut f, &mut buf)?;
    h.update(&buf[..n]);
    if size > CHUNK {
        f.seek(SeekFrom::Start(size.saturating_sub(CHUNK).max(CHUNK)))?;
        let n = read_full(&mut f, &mut buf)?;
        h.update(&buf[..n]);
    }
    Ok(format!("xxh3:{:016x}", h.digest()))
}

fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = f.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

impl Link {
    /// A new link to a file or folder that exists now.
    pub fn from_path(path: &Path) -> std::io::Result<Link> {
        let path = std::path::absolute(path)?;
        let info = stat(&path)?;
        let mut l = Link {
            id: crate::model::new_id(if info.is_dir { "d" } else { "f" }),
            kind: if info.is_dir { LinkKind::Folder } else { LinkKind::File },
            name: file_name(&path),
            url: None,
            size: None,
            mtime: None,
            fingerprint: None,
            path: Some(path.display().to_string()),
            inode: None,
            dev: None,
            status: LinkStatus::Ok,
            candidates: vec![],
        };
        l.refresh_from(&path, &info);
        Ok(l)
    }

    pub fn url(url: &str, title: Option<&str>) -> Link {
        let name = title.map(|s| s.to_string()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| short_url(url));
        Link {
            id: crate::model::new_id("u"),
            kind: LinkKind::Url,
            name,
            url: Some(url.to_string()),
            size: None,
            mtime: None,
            fingerprint: None,
            path: None,
            inode: None,
            dev: None,
            status: LinkStatus::Ok,
            candidates: vec![],
        }
    }

    pub fn is_fs(&self) -> bool {
        matches!(self.kind, LinkKind::File | LinkKind::Folder)
    }

    /// Points the link at `path` and records what the file looks like there now.
    pub fn refresh_from(&mut self, path: &Path, info: &FileInfo) {
        self.path = Some(path.display().to_string());
        self.name = file_name(path);
        self.inode = Some(info.inode);
        self.dev = Some(info.dev);
        self.status = LinkStatus::Ok;
        self.candidates.clear();
        if info.is_dir {
            self.kind = LinkKind::Folder;
            self.size = None;
            self.fingerprint = None;
        } else {
            self.kind = LinkKind::File;
            if self.size != Some(info.size) || self.mtime != Some(info.mtime) || self.fingerprint.is_none() {
                self.fingerprint = fingerprint(path, info.size).ok();
            }
            self.size = Some(info.size);
        }
        self.mtime = Some(info.mtime);
    }
}

fn short_url(url: &str) -> String {
    let s = url.split("://").nth(1).unwrap_or(url);
    let s = s.trim_end_matches('/');
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(59).collect::<String>())
    } else {
        s.to_string()
    }
}
