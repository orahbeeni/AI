//! File-system helpers shared by the clipboard and shared folders: safe path joining, walking
//! copied trees, and sizes for people.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::proto::FileEntry;

/// Joins a peer-supplied relative path under `base`, refusing anything that could escape it.
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let mut out = base.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':', '\0']) {
            bail!("refusing unsafe path {rel:?}");
        }
        out.push(part);
    }
    Ok(out)
}

/// Lists copied paths (recursing into folders) as wire entries named relative to their roots.
pub fn walk(paths: &[PathBuf]) -> Result<Vec<(PathBuf, FileEntry)>> {
    let mut out = Vec::new();
    let mut used = Vec::new();
    for root in paths {
        let base = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "copied".into());
        // Two copied items with the same name (from different folders) must not collide.
        let mut name = base.clone();
        let mut n = 2;
        while used.contains(&name) {
            name = format!("{base} ({n})");
            n += 1;
        }
        used.push(name.clone());
        walk_into(root, &name, &mut out)?;
    }
    Ok(out)
}

pub fn walk_into(path: &Path, rel: &str, out: &mut Vec<(PathBuf, FileEntry)>) -> Result<()> {
    let meta = std::fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if meta.file_type().is_symlink() {
        log::info!("clipboard: skipping symlink {}", path.display());
        return Ok(());
    }
    if meta.is_dir() {
        out.push((path.to_path_buf(), FileEntry { path: rel.into(), size: 0, dir: true, exec: false }));
        let mut children: Vec<_> = std::fs::read_dir(path)?.flatten().collect();
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let name = child.file_name().to_string_lossy().into_owned();
            walk_into(&child.path(), &format!("{rel}/{name}"), out)?;
        }
    } else {
        #[cfg(unix)]
        let exec = std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o111 != 0;
        #[cfg(not(unix))]
        let exec = false;
        out.push((path.to_path_buf(), FileEntry { path: rel.into(), size: meta.len(), dir: false, exec }));
    }
    Ok(())
}

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[u]) }
}
