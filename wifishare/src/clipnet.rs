//! Clipboard sharing over the network.
//!
//! Small copies travel inline on the control link. Large text/images and files are *offered*;
//! the receiver pulls the bytes over a separate bulk connection so they never queue in front of
//! mouse and keyboard events. Received files land in the cache folder and the clipboard is
//! pointed at them, so pasting in Finder/Nautilus/Explorer copies them from there.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::clipboard::{self, ClipHandle, Copied, Local};
use crate::config::{ClipboardCfg, Identity};
use crate::proto::{Bulk, ClipContent, ClipMeta, ClipMsg, FileEntry, Msg, OfferKind};
use crate::server::core::ServerCore;
use crate::status::{Hub, TransferGuard};
use crate::wire::{self, FrameReader, FrameWriter, Purpose, Secured};

/// Encoded clipboard contents up to this size go inline on the control link.
const INLINE_MAX: usize = 128 * 1024;
const BULK_CHUNK: usize = 256 * 1024;
/// Received file sets kept in the cache; older ones are deleted.
const KEEP_INCOMING: usize = 3;
const KEEP_OUTGOING: usize = 4;

pub enum Role {
    /// Messages go to every connected client.
    Server(Arc<Mutex<ServerCore>>),
    Client(ClientCtx),
}

pub struct ClientCtx {
    pub identity: Identity,
    pub server_key: Vec<u8>,
    /// The current control link and the server address it is connected to.
    pub link: Mutex<Option<(UnboundedSender<Msg>, SocketAddr)>>,
}

enum Outgoing {
    Blob(Vec<u8>),
    Files(Vec<(PathBuf, FileEntry)>),
}

struct Pending {
    kind: OfferKind,
    bytes: u64,
}

#[derive(Default)]
struct State {
    next_id: u64,
    outgoing: VecDeque<(u64, Arc<Outgoing>)>,
    /// Our most recent copy, re-sent when a link comes (back) up.
    latest: Option<ClipMsg>,
    pending: HashMap<u64, Pending>,
    /// The offer whose data should end up on the clipboard; a newer copy on either side
    /// replaces it so a slow transfer cannot clobber something copied after it.
    wanted: Option<u64>,
    /// Newest copy time seen from either side (Unix ms), for judging replays.
    newest_at: u64,
}

pub struct ClipNet {
    cfg: ClipboardCfg,
    incoming_dir: PathBuf,
    handle: ClipHandle,
    role: Role,
    hub: Arc<Hub>,
    state: Mutex<State>,
}

impl ClipNet {
    pub fn new(cfg: ClipboardCfg, incoming_dir: PathBuf, handle: ClipHandle, role: Role, hub: Arc<Hub>) -> Arc<ClipNet> {
        let state = State { next_id: clipboard::now_ms() << 8, ..State::default() };
        Arc::new(ClipNet { cfg, incoming_dir, handle, role, hub, state: Mutex::new(state) })
    }

    pub fn client_ctx(&self) -> Option<&ClientCtx> {
        match &self.role {
            Role::Client(c) => Some(c),
            Role::Server(_) => None,
        }
    }

    fn send(&self, msg: ClipMsg, except: Option<u64>) {
        match &self.role {
            Role::Server(core) => core.lock().unwrap().broadcast(&Msg::Clip(msg), except),
            Role::Client(c) => {
                if let Some((tx, _)) = &*c.link.lock().unwrap() {
                    let _ = tx.send(Msg::Clip(msg));
                }
            }
        }
    }

    /// Publishes local copies as they happen.
    pub async fn run_local(self: Arc<Self>, mut copied: UnboundedReceiver<Copied>) {
        while let Some(c) = copied.recv().await {
            self.hub.clipboard_event(format!("Copied {} here", clipboard::describe(&c.local)));
            match self.prepare(c).await {
                Ok(Some(msg)) => self.send(msg, None),
                Ok(None) => {}
                Err(e) => log::warn!("clipboard: {e:#}"),
            }
        }
    }

    async fn prepare(&self, c: Copied) -> Result<Option<ClipMsg>> {
        let id = {
            let mut st = self.state.lock().unwrap();
            st.next_id += 1;
            st.wanted = None;
            st.newest_at = st.newest_at.max(c.at);
            st.next_id
        };
        let meta = ClipMeta { id, at: c.at, replay: false };
        let (msg, outgoing) = match c.local {
            Local::Content(content) => {
                let bytes = postcard::to_stdvec(&content)?;
                if bytes.len() <= INLINE_MAX {
                    (ClipMsg::Inline { meta, content }, None)
                } else {
                    let len = bytes.len() as u64;
                    (ClipMsg::Offer { meta, kind: OfferKind::Blob, bytes: len }, Some(Outgoing::Blob(bytes)))
                }
            }
            Local::Files(paths) => {
                let list = tokio::task::spawn_blocking(move || walk(&paths)).await??;
                let total: u64 = list.iter().map(|(_, e)| e.size).sum();
                if total > self.cfg.max_files_mb * 1024 * 1024 {
                    log::info!("clipboard: copied files total {}, over max_files_mb; not sent", human(total));
                    return Ok(None);
                }
                let entries = list.iter().map(|(_, e)| e.clone()).collect();
                (ClipMsg::Offer { meta, kind: OfferKind::Files(entries), bytes: total }, Some(Outgoing::Files(list)))
            }
        };
        let mut st = self.state.lock().unwrap();
        if let Some(o) = outgoing {
            st.outgoing.push_back((id, Arc::new(o)));
            while st.outgoing.len() > KEEP_OUTGOING {
                st.outgoing.pop_front();
            }
        }
        st.latest = Some(msg.clone());
        Ok(Some(msg))
    }

    /// A link came up: re-send our latest copy in case the peer missed it while apart.
    pub fn on_link_up(&self, tx: &UnboundedSender<Msg>) {
        let latest = self.state.lock().unwrap().latest.clone();
        let msg = match latest {
            Some(ClipMsg::Inline { meta, content }) => ClipMsg::Inline { meta: ClipMeta { replay: true, ..meta }, content },
            Some(ClipMsg::Offer { meta, kind, bytes }) => ClipMsg::Offer { meta: ClipMeta { replay: true, ..meta }, kind, bytes },
            _ => return,
        };
        let _ = tx.send(Msg::Clip(msg));
    }

    fn accept(&self, meta: &ClipMeta) -> bool {
        let mut st = self.state.lock().unwrap();
        if meta.replay && meta.at <= st.newest_at {
            return false;
        }
        st.newest_at = st.newest_at.max(meta.at);
        true
    }

    /// Handles a clipboard message from the peer. `from` is the sending client's connection id
    /// (server only), so it can be relayed to the other clients.
    pub fn on_message(self: &Arc<Self>, msg: ClipMsg, from: Option<u64>) {
        match msg {
            ClipMsg::Inline { meta, content } => {
                if !self.accept(&meta) {
                    return;
                }
                self.state.lock().unwrap().wanted = None;
                let what = clipboard::describe(&Local::Content(content.clone()));
                log::info!("clipboard: received {what}");
                self.hub.clipboard_event(format!("Received {what}"));
                self.handle.apply(content.clone());
                if from.is_some() {
                    self.send(ClipMsg::Inline { meta, content }, from);
                }
            }
            ClipMsg::Offer { meta, kind, bytes } => {
                if !self.accept(&meta) {
                    return;
                }
                let limit = match kind {
                    OfferKind::Blob => self.cfg.max_text_mb.max(self.cfg.max_image_mb),
                    OfferKind::Files(_) => self.cfg.max_files_mb,
                } * 1024
                    * 1024;
                if bytes > limit {
                    log::info!("clipboard: peer offered {}, over this machine's limit; skipped", human(bytes));
                    return;
                }
                {
                    let mut st = self.state.lock().unwrap();
                    st.wanted = Some(meta.id);
                    st.pending.insert(meta.id, Pending { kind, bytes });
                }
                match (&self.role, from) {
                    (Role::Server(core), Some(conn)) => core.lock().unwrap().send_to(conn, Msg::Clip(ClipMsg::Fetch { id: meta.id })),
                    (Role::Client(_), _) => {
                        let me = self.clone();
                        tokio::spawn(async move {
                            if let Err(e) = me.pull_from_server(meta.id).await {
                                log::warn!("clipboard: transfer failed: {e:#}");
                            }
                        });
                    }
                    (Role::Server(_), None) => {}
                }
            }
            ClipMsg::Fetch { id } => {
                let me = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = me.push_to_server(id).await {
                        log::warn!("clipboard: transfer failed: {e:#}");
                    }
                });
            }
        }
    }

    fn outgoing(&self, id: u64) -> Option<Arc<Outgoing>> {
        self.state.lock().unwrap().outgoing.iter().find(|(i, _)| *i == id).map(|(_, o)| o.clone())
    }

    async fn dial_bulk(&self) -> Result<Secured> {
        let ctx = self.client_ctx().context("not a client")?;
        let addr = ctx.link.lock().unwrap().as_ref().map(|(_, a)| *a).context("not connected")?;
        let s = wire::connect(addr, &ctx.identity, Purpose::Bulk, Duration::from_secs(5)).await?;
        if s.remote_key != ctx.server_key {
            bail!("bulk connection reached a different machine");
        }
        Ok(s)
    }

    async fn pull_from_server(self: Arc<Self>, id: u64) -> Result<()> {
        let mut s = self.dial_bulk().await?;
        s.writer.send(&Bulk::Open { id, push: false }).await?;
        self.receive(&mut s.reader, id).await
    }

    async fn push_to_server(self: Arc<Self>, id: u64) -> Result<()> {
        let out = self.outgoing(id).context("server asked for an expired clipboard offer")?;
        let mut s = self.dial_bulk().await?;
        s.writer.send(&Bulk::Open { id, push: true }).await?;
        let progress = self.hub.transfer(format!("Sending {}", out.describe()), out.size());
        stream(&mut s.writer, &out, &progress).await
    }

    /// Server: a client opened a bulk connection.
    pub async fn serve_bulk(self: Arc<Self>, mut s: Secured) -> Result<()> {
        match s.reader.recv::<Bulk>().await? {
            Bulk::Open { id, push: true } => self.receive(&mut s.reader, id).await,
            Bulk::Open { id, push: false } => match self.outgoing(id) {
                Some(out) => {
                    let progress = self.hub.transfer(format!("Sending {}", out.describe()), out.size());
                    stream(&mut s.writer, &out, &progress).await
                }
                None => {
                    s.writer.send(&Bulk::Error("offer expired".into())).await?;
                    Ok(())
                }
            },
            other => bail!("unexpected first bulk message {other:?}"),
        }
    }

    async fn receive(&self, reader: &mut FrameReader, id: u64) -> Result<()> {
        let pending = self.state.lock().unwrap().pending.remove(&id).context("unknown clipboard offer")?;
        let started = Instant::now();
        let label = match &pending.kind {
            OfferKind::Blob => "Receiving a large clipboard".to_string(),
            OfferKind::Files(e) => format!("Receiving {}", describe_entries(e)),
        };
        let progress = self.hub.transfer(label, pending.bytes);
        let mut data = DataStream { reader, buf: Vec::new(), pos: 0, done: false, progress: &progress };
        let applied = match pending.kind {
            OfferKind::Blob => {
                let mut bytes = Vec::with_capacity(pending.bytes as usize);
                while let Some(chunk) = data.next(usize::MAX).await? {
                    bytes.extend_from_slice(chunk);
                }
                let content: ClipContent = postcard::from_bytes(&bytes)?;
                let what = clipboard::describe(&Local::Content(content.clone()));
                self.apply_if_wanted(id, || self.handle.apply(content)).then_some(what)
            }
            OfferKind::Files(entries) => {
                log::info!("clipboard: receiving {} ({} entries)", human(pending.bytes), entries.len());
                let roots = receive_files(&mut data, &entries, &self.incoming_dir, id).await?;
                let what = clipboard::describe(&Local::Files(roots.clone()));
                self.apply_if_wanted(id, || self.handle.apply_files(roots)).then_some(what)
            }
        };
        let secs = started.elapsed().as_secs_f64();
        match applied {
            Some(what) => {
                log::info!(
                    "clipboard: received {what}, {} in {secs:.1}s ({}/s)",
                    human(pending.bytes),
                    human((pending.bytes as f64 / secs.max(0.001)) as u64)
                );
                self.hub.clipboard_event(format!("Received {what} ({})", human(pending.bytes)));
            }
            None => log::info!("clipboard: transfer finished after something newer was copied; left the clipboard alone"),
        }
        Ok(())
    }

    fn apply_if_wanted(&self, id: u64, apply: impl FnOnce()) -> bool {
        let wanted = self.state.lock().unwrap().wanted == Some(id);
        if wanted {
            apply();
        }
        wanted
    }
}

/// Reads `Bulk::Data` chunks as one continuous byte stream.
struct DataStream<'a> {
    reader: &'a mut FrameReader,
    buf: Vec<u8>,
    pos: usize,
    done: bool,
    progress: &'a TransferGuard,
}

impl DataStream<'_> {
    /// Up to `max` bytes, or `None` at the end of the stream.
    async fn next(&mut self, max: usize) -> Result<Option<&[u8]>> {
        while self.pos == self.buf.len() {
            if self.done {
                return Ok(None);
            }
            match self.reader.recv::<Bulk>().await? {
                Bulk::Data(d) => {
                    self.progress.add(d.len() as u64);
                    self.buf = d;
                    self.pos = 0;
                }
                Bulk::End => self.done = true,
                Bulk::Error(e) => bail!("sender failed: {e}"),
                Bulk::Open { .. } => bail!("unexpected Open"),
            }
        }
        let n = (self.buf.len() - self.pos).min(max);
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(Some(out))
    }
}

impl Outgoing {
    fn size(&self) -> u64 {
        match self {
            Outgoing::Blob(b) => b.len() as u64,
            Outgoing::Files(list) => list.iter().map(|(_, e)| e.size).sum(),
        }
    }

    fn describe(&self) -> String {
        match self {
            Outgoing::Blob(_) => "a large clipboard".into(),
            Outgoing::Files(list) => describe_entries(&list.iter().map(|(_, e)| e.clone()).collect::<Vec<_>>()),
        }
    }
}

fn describe_entries(entries: &[FileEntry]) -> String {
    let top: Vec<&FileEntry> = entries.iter().filter(|e| !e.path.contains('/')).collect();
    match top.as_slice() {
        [one] => format!("\"{}\"", one.path),
        many => format!("{} items", many.len()),
    }
}

async fn stream(writer: &mut FrameWriter, out: &Outgoing, progress: &TransferGuard) -> Result<()> {
    match out {
        Outgoing::Blob(bytes) => {
            for chunk in bytes.chunks(BULK_CHUNK) {
                writer.send(&Bulk::Data(chunk.to_vec())).await?;
                progress.add(chunk.len() as u64);
            }
        }
        Outgoing::Files(list) => {
            let mut buf = vec![0u8; BULK_CHUNK];
            for (path, entry) in list.iter().filter(|(_, e)| !e.dir) {
                let mut file = tokio::fs::File::open(path).await.with_context(|| format!("opening {}", path.display()))?;
                let mut left = entry.size;
                while left > 0 {
                    let want = (left as usize).min(BULK_CHUNK);
                    let n = file.read(&mut buf[..want]).await?;
                    if n == 0 {
                        let _ = writer.send(&Bulk::Error(format!("{} shrank while sending", entry.path))).await;
                        bail!("{} shrank while sending", path.display());
                    }
                    writer.send(&Bulk::Data(buf[..n].to_vec())).await?;
                    progress.add(n as u64);
                    left -= n as u64;
                }
            }
        }
    }
    writer.send(&Bulk::End).await
}

async fn receive_files(data: &mut DataStream<'_>, entries: &[FileEntry], incoming: &Path, id: u64) -> Result<Vec<PathBuf>> {
    let dest = incoming.join(id.to_string());
    tokio::fs::create_dir_all(&dest).await.with_context(|| format!("creating {}", dest.display()))?;
    prune_incoming(incoming);
    let mut roots = Vec::new();
    for entry in entries {
        let path = safe_join(&dest, &entry.path)?;
        if !entry.path.contains('/') {
            roots.push(path.clone());
        }
        if entry.dir {
            tokio::fs::create_dir_all(&path).await?;
            continue;
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::File::create(&path).await.with_context(|| format!("creating {}", path.display()))?;
        let mut left = entry.size;
        while left > 0 {
            let chunk = data.next(left.min(usize::MAX as u64) as usize).await?.context("stream ended early")?;
            file.write_all(chunk).await?;
            left -= chunk.len() as u64;
        }
        file.flush().await?;
        #[cfg(unix)]
        if entry.exec {
            use std::os::unix::fs::PermissionsExt;
            let _ = tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).await;
        }
    }
    if data.next(1).await?.is_some() {
        bail!("sender sent more data than announced");
    }
    Ok(roots)
}

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

fn prune_incoming(incoming: &Path) {
    let Ok(dir) = std::fs::read_dir(incoming) else { return };
    let mut ids: Vec<(u64, PathBuf)> = dir.flatten().filter_map(|e| Some((e.file_name().to_str()?.parse().ok()?, e.path()))).collect();
    ids.sort();
    let excess = ids.len().saturating_sub(KEEP_INCOMING);
    for (_, path) in ids.into_iter().take(excess) {
        let _ = std::fs::remove_dir_all(path);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_refuses_escapes() {
        let base = Path::new("/tmp/x");
        assert_eq!(safe_join(base, "a/b.txt").unwrap(), base.join("a").join("b.txt"));
        for bad in ["../etc/passwd", "a/../../b", "/abs", "a//b", "c:\\x", "./a"] {
            assert!(safe_join(base, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn walk_names_entries_relative_to_their_roots() {
        let dir = std::env::temp_dir().join(format!("wifishare-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/sub")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(dir.join("a/sub/f.txt"), "hello").unwrap();
        std::fs::write(dir.join("b/sub"), "x").unwrap();
        let list = walk(&[dir.join("a/sub"), dir.join("b/sub")]).unwrap();
        let names: Vec<_> = list.iter().map(|(_, e)| (e.path.as_str(), e.size, e.dir)).collect();
        assert_eq!(names, [("sub", 0, true), ("sub/f.txt", 5, false), ("sub (2)", 1, false)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
