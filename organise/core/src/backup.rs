//! Backup = a .zip of the data folder. Restore replaces the folder's contents with the zip's,
//! after saving what was there into `.history/before-restore-<time>.zip`.

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if name != ".history" {
                    stack.push(p);
                }
            } else if ft.is_file() && !name.ends_with(".tmp") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Writes every file of the data folder (except .history) into `dest`.
pub fn export(dir: &Path, dest: &Path) -> Result<usize> {
    let f = std::fs::File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let files = files_under(dir);
    for p in &files {
        let rel = p.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
        z.start_file(rel, opts)?;
        z.write_all(&std::fs::read(p)?)?;
    }
    z.finish()?;
    Ok(files.len())
}

/// Replaces the data folder's contents with the backup's. The old contents are kept in .history.
pub fn restore(dir: &Path, zip_path: &Path) -> Result<PathBuf> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(zip_path)?).context("not a zip file")?;
    let mut has_ws = false;
    for i in 0..z.len() {
        if z.by_index(i)?.name() == "workspace.yaml" {
            has_ws = true;
        }
    }
    if !has_ws {
        bail!("this zip isn't an Organise backup (no workspace.yaml)");
    }
    let hist = dir.join(".history");
    std::fs::create_dir_all(&hist)?;
    let safety = hist.join(format!("before-restore-{}.zip", crate::links::now_secs()));
    export(dir, &safety)?;
    for p in files_under(dir) {
        std::fs::remove_file(p)?;
    }
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        let Some(rel) = f.enclosed_name() else { continue };
        if f.is_dir() {
            continue;
        }
        let out = dir.join(rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut buf = vec![];
        f.read_to_end(&mut buf)?;
        std::fs::write(out, buf)?;
    }
    Ok(safety)
}
