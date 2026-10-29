//! The responding side of a files connection: answers requests about the folders this computer
//! shares. Everything a peer sends is untrusted: names are checked, and every path is resolved
//! and confirmed to lie inside its share (so symlinks cannot lead out of it). Sharing never
//! overwrites or deletes anything.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use super::{PART, split, unique_name};
use crate::clipnet::{safe_join, walk_into};
use crate::config::{Config, FilesCfg, Paths};
use crate::proto::{DirEntry, FileEntry, FsReq, FsResp, Share};
use crate::wire::Secured;

const CHUNK: usize = 256 * 1024;
/// A connection with no request for this long is closed.
const IDLE: Duration = Duration::from_secs(60);
/// Larger folders are refused rather than listed in one message.
const MAX_ENTRIES: usize = 200_000;

/// Answers requests until the peer hangs up.
pub async fn respond(mut s: Secured, paths: Paths) -> Result<()> {
    loop {
        let req: FsReq = match tokio::time::timeout(IDLE, s.reader.recv()).await {
            Ok(Ok(r)) => r,
            _ => return Ok(()),
        };
        // Read fresh each time so shares added or removed in the window apply at once.
        let cfg = Config::load(&paths).map(|c| c.files).unwrap_or_default();
        let mut streaming = false;
        if let Err(e) = handle(&cfg, req, &mut s, &mut streaming).await {
            log::debug!("files: request failed: {e:#}");
            // Once file data is flowing, trouble (this side shutting down, a disk error) just
            // ends the connection: the requester treats that as an interruption and tries again,
            // rather than as a verdict on its request.
            if streaming {
                return Ok(());
            }
            let _ = s.writer.send(&FsResp::Error(format!("{e:#}"))).await;
        }
    }
}

/// `streaming` is set once a transfer's data has started to flow.
async fn handle(cfg: &FilesCfg, req: FsReq, s: &mut Secured, streaming: &mut bool) -> Result<()> {
    match req {
        FsReq::Shares => {
            let shares = cfg.shares.iter().map(|x| Share { name: x.name.clone(), write: x.write }).collect();
            s.writer.send(&FsResp::Shares(shares)).await
        }
        FsReq::List { share, path } => {
            let (root, dir) = resolve(cfg, &share, &path, false)?;
            let entries = tokio::task::spawn_blocking(move || list_dir(&root, &dir)).await??;
            s.writer.send(&FsResp::Listing(entries)).await
        }
        FsReq::Walk { share, path } => {
            if path.is_empty() {
                bail!("choose a file or folder inside the share");
            }
            let (_, real) = resolve(cfg, &share, &path, false)?;
            let name = split(&path).1.to_string();
            let entries = tokio::task::spawn_blocking(move || -> Result<Vec<FileEntry>> {
                let mut out = Vec::new();
                walk_into(&real, &name, &mut out)?;
                if out.len() > MAX_ENTRIES {
                    bail!("{} items is too many to copy at once; copy smaller folders", out.len());
                }
                Ok(out.into_iter().map(|(_, e)| e).collect())
            })
            .await??;
            s.writer.send(&FsResp::Manifest(entries)).await
        }
        FsReq::Read { share, path, offset, size } => read(cfg, &share, &path, offset, size, s, streaming).await,
        FsReq::Mkdir { share, path } => {
            let target = resolve_new(cfg, &share, &path)?;
            match tokio::fs::create_dir(&target).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && tokio::fs::metadata(&target).await.is_ok_and(|m| m.is_dir()) => {
                }
                Err(e) => bail!("cannot create the folder: {e}"),
            }
            s.writer.send(&FsResp::Ok).await
        }
        FsReq::FreeName { share, dir, name } => {
            let (_, dir) = resolve(cfg, &share, &dir, true)?;
            safe_join(Path::new("x"), &name)?;
            let taken = |n: &str| dir.join(n).symlink_metadata().is_ok() || dir.join(format!("{n}{PART}")).symlink_metadata().is_ok();
            s.writer.send(&FsResp::Name(unique_name(taken, &name))).await
        }
        FsReq::Write { share, path, size, exec } => write(cfg, &share, &path, size, exec, s, streaming).await,
        FsReq::Chunk(_) | FsReq::End => bail!("unexpected file data"),
    }
}

/// The canonical root of the share and the canonical path of `rel` inside it, which must exist.
/// With `write`, the share must also allow changes.
fn resolve(cfg: &FilesCfg, share: &str, rel: &str, write: bool) -> Result<(PathBuf, PathBuf)> {
    let s = cfg.shares.iter().find(|s| s.name == share).with_context(|| format!("\"{share}\" is not shared"))?;
    if write && !s.write {
        bail!("\"{share}\" is shared read-only");
    }
    let root = std::fs::canonicalize(&s.path).with_context(|| format!("the folder shared as \"{share}\" is not available"))?;
    let wanted = if rel.is_empty() { root.clone() } else { safe_join(&root, rel)? };
    let real = std::fs::canonicalize(&wanted).context("no such file or folder")?;
    if !real.starts_with(&root) {
        bail!("that is outside the shared folder");
    }
    Ok((root, real))
}

/// The path for a new item `rel` in a share that allows changes: its folder must exist.
fn resolve_new(cfg: &FilesCfg, share: &str, rel: &str) -> Result<PathBuf> {
    let (dir, name) = split(rel);
    if name.is_empty() {
        bail!("no name given");
    }
    let (_, parent) = resolve(cfg, share, dir, true)?;
    safe_join(&parent, name)
}

fn list_dir(root: &Path, dir: &Path) -> Result<Vec<DirEntry>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).context("cannot read that folder")?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Names the other platforms could not use anyway are left out.
        if safe_join(Path::new("x"), &name).is_err() {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        let meta = if kind.is_symlink() {
            match std::fs::canonicalize(entry.path()) {
                Ok(target) if target.starts_with(root) => std::fs::metadata(target),
                _ => continue,
            }
        } else {
            entry.metadata()
        };
        let Ok(meta) = meta else { continue };
        let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
        out.push(DirEntry { name, dir: meta.is_dir(), size: if meta.is_dir() { 0 } else { meta.len() }, modified });
    }
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

async fn read(cfg: &FilesCfg, share: &str, path: &str, offset: u64, size: u64, s: &mut Secured, streaming: &mut bool) -> Result<()> {
    let (_, real) = resolve(cfg, share, path, false)?;
    let mut file = tokio::fs::File::open(&real).await.context("cannot open the file")?;
    let meta = file.metadata().await?;
    if !meta.is_file() {
        bail!("that is not a file");
    }
    if meta.len() != size {
        bail!("the file changed since it was listed");
    }
    if offset > size {
        bail!("nothing to read past the end");
    }
    file.seek(SeekFrom::Start(offset)).await?;
    *streaming = true;
    let mut left = size - offset;
    let mut buf = vec![0u8; CHUNK];
    while left > 0 {
        let n = file.read(&mut buf[..(left as usize).min(CHUNK)]).await?;
        if n == 0 {
            bail!("the file shrank while it was being read");
        }
        s.writer.send(&FsResp::Chunk(buf[..n].to_vec())).await?;
        left -= n as u64;
    }
    s.writer.send(&FsResp::End).await
}

fn part_of(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(PART);
    target.with_file_name(name)
}

async fn write(cfg: &FilesCfg, share: &str, path: &str, size: u64, exec: bool, s: &mut Secured, streaming: &mut bool) -> Result<()> {
    let target = resolve_new(cfg, share, path)?;
    if tokio::fs::symlink_metadata(&target).await.is_ok() {
        bail!("\"{}\" already exists there", split(path).1);
    }
    let part = part_of(&target);
    // A leftover from an interrupted copy is carried on from; anything else starts over.
    let have = match tokio::fs::symlink_metadata(&part).await {
        Ok(m) if m.is_file() && m.len() <= size => m.len(),
        Ok(m) if !m.is_file() => bail!("\"{}\" is in the way", split(path).1),
        _ => 0,
    };
    let mut file =
        tokio::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&part).await.context("cannot create the file")?;
    file.set_len(have).await?;
    file.seek(SeekFrom::Start(have)).await?;
    *streaming = true;
    s.writer.send(&FsResp::Ready { offset: have }).await?;
    let received = receive(&mut file, have, size, s).await;
    // Whatever reached the disk is what a retry resumes from.
    let _ = file.flush().await;
    received?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&part, std::fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 })).await;
    }
    #[cfg(not(unix))]
    let _ = exec;
    if tokio::fs::symlink_metadata(&target).await.is_ok() {
        bail!("\"{}\" already exists there", split(path).1);
    }
    tokio::fs::rename(&part, &target).await.context("cannot finish the file")?;
    s.writer.send(&FsResp::Ok).await
}

async fn receive(file: &mut tokio::fs::File, mut got: u64, size: u64, s: &mut Secured) -> Result<()> {
    loop {
        let req: FsReq = match tokio::time::timeout(IDLE, s.reader.recv()).await {
            Ok(r) => r?,
            Err(_) => bail!("the data stopped coming"),
        };
        match req {
            FsReq::Chunk(data) => {
                got += data.len() as u64;
                if got > size {
                    bail!("more data than the file holds");
                }
                file.write_all(&data).await.context("cannot write the file")?;
            }
            FsReq::End if got == size => return Ok(()),
            FsReq::End => bail!("the file ended early"),
            _ => bail!("unexpected request while receiving a file"),
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::config::ShareCfg;

    fn cfg(root: &Path, write: bool) -> FilesCfg {
        FilesCfg { shares: vec![ShareCfg { name: "docs".into(), path: root.to_path_buf(), write }] }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wifishare-serve-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_keeps_requests_inside_the_share() {
        let dir = scratch("resolve");
        let root = dir.join("share");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(dir.join("secret.txt"), "no").unwrap();
        std::fs::write(root.join("sub/a.txt"), "yes").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, root.join("out")).unwrap();
            std::os::unix::fs::symlink(root.join("sub"), root.join("in")).unwrap();
        }
        let c = cfg(&root, false);
        assert!(resolve(&c, "docs", "sub/a.txt", false).is_ok());
        assert!(resolve(&c, "docs", "", false).is_ok());
        for bad in ["../secret.txt", "sub/../../secret.txt", "/etc/passwd", "sub//a.txt"] {
            assert!(resolve(&c, "docs", bad, false).is_err(), "{bad}");
        }
        assert!(resolve(&c, "other", "", false).is_err());
        #[cfg(unix)]
        {
            assert!(resolve(&c, "docs", "out/secret.txt", false).is_err(), "a symlink out of the share");
            assert!(resolve(&c, "docs", "in/a.txt", false).is_ok(), "a symlink inside the share");
            let listed: Vec<_> = list_dir(&std::fs::canonicalize(&root).unwrap(), &std::fs::canonicalize(&root).unwrap())
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            assert_eq!(listed, ["in", "sub"], "the link out is not listed");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writing_needs_a_writable_share_and_never_overwrites() {
        let dir = scratch("write");
        std::fs::write(dir.join("have.txt"), "x").unwrap();
        assert!(resolve_new(&cfg(&dir, false), "docs", "new.txt").is_err(), "read-only share");
        let c = cfg(&dir, true);
        assert_eq!(resolve_new(&c, "docs", "new.txt").unwrap(), std::fs::canonicalize(&dir).unwrap().join("new.txt"));
        assert!(resolve_new(&c, "docs", "../new.txt").is_err());
        assert!(resolve_new(&c, "docs", "missing/new.txt").is_err(), "the folder must exist");
        assert!(resolve_new(&c, "docs", "").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unique_names_keep_the_extension() {
        let taken = |n: &str| ["a.txt", "a (2).txt", "dir"].contains(&n);
        assert_eq!(unique_name(taken, "b.txt"), "b.txt");
        assert_eq!(unique_name(taken, "a.txt"), "a (3).txt");
        assert_eq!(unique_name(taken, "dir"), "dir (2)");
        assert_eq!(split("a/b/c"), ("a/b", "c"));
        assert_eq!(split("c"), ("", "c"));
    }
}
