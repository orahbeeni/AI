//! The requesting side of a files connection.

use std::io::SeekFrom;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use super::{Cancelled, fatal, local};
use crate::proto::{DirEntry, FileEntry, FsReq, FsResp, Share};
use crate::wire::Secured;

const CHUNK: usize = 256 * 1024;
/// A peer that says nothing for this long is treated as gone.
const STALL: Duration = Duration::from_secs(20);

/// Where a transfer reports progress and learns it was cancelled.
pub struct Ctl<'a> {
    pub done: &'a AtomicU64,
    /// Bytes of the job finished before the current file.
    pub base: u64,
    pub cancel: &'a AtomicBool,
}

impl Ctl<'_> {
    fn progress(&self, in_file: u64) {
        self.done.store(self.base + in_file, Ordering::Relaxed);
    }

    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) { Err(anyhow::Error::new(Cancelled)) } else { Ok(()) }
    }
}

pub struct Session {
    s: Secured,
}

impl Session {
    pub(super) fn new(s: Secured) -> Session {
        Session { s }
    }

    pub(super) async fn send(&mut self, req: &FsReq) -> Result<()> {
        match tokio::time::timeout(STALL, self.s.writer.send(req)).await {
            Ok(r) => r,
            Err(_) => bail!("the other computer stopped answering"),
        }
    }

    /// The next answer; the peer's own error message becomes a `Fatal` error.
    async fn recv(&mut self) -> Result<FsResp> {
        match tokio::time::timeout(STALL, self.s.reader.recv::<FsResp>()).await {
            Err(_) => bail!("the other computer stopped answering"),
            Ok(r) => match r? {
                FsResp::Error(message) => Err(fatal(message)),
                other => Ok(other),
            },
        }
    }

    async fn call(&mut self, req: FsReq) -> Result<FsResp> {
        self.send(&req).await?;
        self.recv().await
    }

    pub async fn shares(&mut self) -> Result<Vec<Share>> {
        match self.call(FsReq::Shares).await? {
            FsResp::Shares(s) => Ok(s),
            other => unexpected(other),
        }
    }

    pub async fn list(&mut self, share: &str, path: &str) -> Result<Vec<DirEntry>> {
        match self.call(FsReq::List { share: share.into(), path: path.into() }).await? {
            FsResp::Listing(l) => Ok(l),
            other => unexpected(other),
        }
    }

    pub async fn walk(&mut self, share: &str, path: &str) -> Result<Vec<FileEntry>> {
        match self.call(FsReq::Walk { share: share.into(), path: path.into() }).await? {
            FsResp::Manifest(m) => Ok(m),
            other => unexpected(other),
        }
    }

    pub async fn mkdir(&mut self, share: &str, path: &str) -> Result<()> {
        match self.call(FsReq::Mkdir { share: share.into(), path: path.into() }).await? {
            FsResp::Ok => Ok(()),
            other => unexpected(other),
        }
    }

    pub async fn free_name(&mut self, share: &str, dir: &str, name: &str) -> Result<String> {
        match self.call(FsReq::FreeName { share: share.into(), dir: dir.into(), name: name.into() }).await? {
            FsResp::Name(n) => Ok(n),
            other => unexpected(other),
        }
    }

    /// Appends the bytes of a remote file, from `offset`, to `out` (which is positioned there).
    pub(super) async fn read_to(
        &mut self,
        share: &str,
        path: &str,
        offset: u64,
        size: u64,
        out: &mut tokio::fs::File,
        ctl: &Ctl<'_>,
    ) -> Result<()> {
        let result = self.read_loop(share, path, offset, size, out, ctl).await;
        // What reached the disk is what a retry resumes from.
        let _ = out.flush().await;
        result
    }

    async fn read_loop(&mut self, share: &str, path: &str, offset: u64, size: u64, out: &mut tokio::fs::File, ctl: &Ctl<'_>) -> Result<()> {
        self.send(&FsReq::Read { share: share.into(), path: path.into(), offset, size }).await?;
        let mut got = offset;
        ctl.progress(got);
        loop {
            match self.recv().await? {
                FsResp::Chunk(data) => {
                    got += data.len() as u64;
                    if got > size {
                        return Err(fatal("the other computer sent more than the file holds"));
                    }
                    out.write_all(&data).await.map_err(|e| local("writing the file", e))?;
                    ctl.progress(got);
                    ctl.check()?;
                }
                FsResp::End if got == size => return Ok(()),
                FsResp::End => return Err(fatal("the file ended early on the other computer")),
                other => return unexpected(other),
            }
        }
    }

    /// Stores the local file `src` (of `size` bytes) at `path`, carrying on where an interrupted
    /// attempt stopped.
    pub(super) async fn write_from(&mut self, share: &str, path: &str, src: &Path, size: u64, exec: bool, ctl: &Ctl<'_>) -> Result<()> {
        self.send(&FsReq::Write { share: share.into(), path: path.into(), size, exec }).await?;
        let offset = match self.recv().await? {
            FsResp::Ready { offset } if offset <= size => offset,
            other => return unexpected(other),
        };
        let mut file = tokio::fs::File::open(src).await.map_err(|e| local("opening the file", e))?;
        file.seek(SeekFrom::Start(offset)).await.map_err(|e| local("reading the file", e))?;
        let mut sent = offset;
        ctl.progress(sent);
        let mut buf = vec![0u8; CHUNK];
        while sent < size {
            let want = ((size - sent) as usize).min(CHUNK);
            let n = file.read(&mut buf[..want]).await.map_err(|e| local("reading the file", e))?;
            if n == 0 {
                return Err(fatal("the file shrank while it was being sent"));
            }
            if let Err(e) = self.send(&FsReq::Chunk(buf[..n].to_vec())).await {
                // The other side may have stopped on purpose and said why.
                return match tokio::time::timeout(Duration::from_secs(2), self.recv()).await {
                    Ok(Err(refusal)) if super::is_transient(&refusal) => Err(e),
                    Ok(Err(refusal)) => Err(refusal),
                    _ => Err(e),
                };
            }
            sent += n as u64;
            ctl.progress(sent);
            ctl.check()?;
        }
        self.send(&FsReq::End).await?;
        match self.recv().await? {
            FsResp::Ok => Ok(()),
            other => unexpected(other),
        }
    }
}

fn unexpected<T>(resp: FsResp) -> Result<T> {
    Err(fatal(format!("unexpected answer from the other computer: {resp:?}")))
}
