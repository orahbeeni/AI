//! Shared folders: each side can publish folders, and the other side browses them and copies
//! files in either direction.
//!
//! Every operation runs on its own connection (`Purpose::Files`), which is always dialled by the
//! client. The side that makes the requests is the *requester* and the other the *responder*;
//! when the server is the requester it sends `Msg::FilesDial` over the control link and the
//! client dials back with the matching token. Transfers are jobs that survive the link dropping:
//! they wait for the peer to come back and carry on from the bytes already on disk.

pub mod cli;
mod jobs;
mod serve;
mod session;

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::sync::oneshot;

pub use self::jobs::{Job, JobKind, JobState};
pub use self::session::Session;
use crate::config::{Identity, Paths};
use crate::proto::{DirEntry, FsOpen, Msg, Share};
use crate::server::core::ServerCore;
use crate::status::Hub;
use crate::wire::{self, Purpose, Secured};

/// How long the server waits for a client to dial back.
const DIAL_BACK: Duration = Duration::from_secs(8);
/// Suffix of a file still being received; it is renamed when complete and picked up again by a
/// retry.
const PART: &str = ".wifishare-part";

/// An error that trying again cannot fix: refused by the other side, a local disk problem.
#[derive(Debug)]
pub struct Fatal(pub String);

impl std::fmt::Display for Fatal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Fatal {}

#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub fn fatal(msg: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Fatal(msg.into()))
}

/// A failed local file operation.
fn local(what: &str, e: std::io::Error) -> anyhow::Error {
    fatal(format!("{what}: {e}"))
}

/// Network trouble that is worth waiting out, as opposed to a refusal or a disk error.
fn is_transient(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Fatal>().is_none() && e.downcast_ref::<Cancelled>().is_none()
}

/// `name`, or `name (2)`, `name (3)`, ... (before the extension) until `taken` is false.
pub fn unique_name(taken: impl Fn(&str) -> bool, name: &str) -> String {
    if !taken(name) {
        return name.into();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    };
    (2..).map(|n| format!("{stem} ({n}){ext}")).find(|c| !taken(c)).unwrap()
}

/// `a/b` + `c` -> `a/b/c`; either side may be empty.
pub fn join(dir: &str, name: &str) -> String {
    match (dir.is_empty(), name.is_empty()) {
        (true, _) => name.into(),
        (_, true) => dir.into(),
        _ => format!("{dir}/{name}"),
    }
}

/// `a/b/c` -> (`a/b`, `c`).
pub fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// The result of an operation running in the background, polled by the window.
pub struct Pending<T>(Arc<Mutex<Option<Result<T, String>>>>);

impl<T: Send + 'static> Pending<T> {
    fn spawn(files: &Files, fut: impl Future<Output = Result<T>> + Send + 'static) -> Pending<T> {
        let slot = Arc::new(Mutex::new(None));
        let (out, hub) = (slot.clone(), files.hub.clone());
        files.rt.spawn(async move {
            let r = fut.await.map_err(|e| format!("{e:#}"));
            *out.lock().unwrap() = Some(r);
            hub.update(|_| {});
        });
        Pending(slot)
    }

    pub fn take(&self) -> Option<Result<T, String>> {
        self.0.lock().unwrap().take()
    }
}

enum Endpoint {
    Server(Arc<Mutex<ServerCore>>),
    Client {
        server_key: Vec<u8>,
        /// Where the control link currently runs; `None` while disconnected.
        link: Mutex<Option<SocketAddr>>,
    },
}

pub struct Files {
    paths: Paths,
    id: Identity,
    hub: Arc<Hub>,
    rt: tokio::runtime::Handle,
    endpoint: Endpoint,
    /// Server: requests waiting for a client to dial back.
    waiting: Mutex<HashMap<u64, oneshot::Sender<Secured>>>,
    jobs: Mutex<Vec<Arc<Job>>>,
}

impl Files {
    /// Must be called inside the tokio runtime that will run the transfers.
    pub fn server(paths: Paths, id: Identity, hub: Arc<Hub>, core: Arc<Mutex<ServerCore>>) -> Arc<Files> {
        Files::new(paths, id, hub, Endpoint::Server(core))
    }

    pub fn client(paths: Paths, id: Identity, hub: Arc<Hub>, server_key: Vec<u8>) -> Arc<Files> {
        Files::new(paths, id, hub, Endpoint::Client { server_key, link: Mutex::new(None) })
    }

    fn new(paths: Paths, id: Identity, hub: Arc<Hub>, endpoint: Endpoint) -> Arc<Files> {
        Arc::new(Files {
            paths,
            id,
            hub,
            rt: tokio::runtime::Handle::current(),
            endpoint,
            waiting: Mutex::default(),
            jobs: Mutex::default(),
        })
    }

    /// Client: the control link came up at `addr` (or went down).
    pub fn set_link(&self, addr: Option<SocketAddr>) {
        if let Endpoint::Client { link, .. } = &self.endpoint {
            *link.lock().unwrap() = addr;
        }
    }

    /// Opens a connection on which this side makes requests of `peer` (a client's name when this
    /// is the server; ignored on a client, which has one server).
    pub async fn open(&self, peer: &str) -> Result<Session> {
        match &self.endpoint {
            Endpoint::Client { server_key, link } => {
                let addr = (*link.lock().unwrap()).context("not connected to the server")?;
                let mut s = wire::connect(addr, &self.id, Purpose::Files, Duration::from_secs(5)).await?;
                if &s.remote_key != server_key {
                    bail!("{addr} is a different machine than the server");
                }
                s.writer.send(&FsOpen { token: None }).await?;
                Ok(Session::new(s))
            }
            Endpoint::Server(core) => {
                let conn = self
                    .hub
                    .snapshot()
                    .clients
                    .iter()
                    .find(|c| c.name == peer)
                    .map(|c| c.conn)
                    .with_context(|| format!("{peer} is not connected"))?;
                let mut token = [0u8; 8];
                getrandom::fill(&mut token).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
                let token = u64::from_le_bytes(token);
                let (tx, rx) = oneshot::channel();
                self.waiting.lock().unwrap().insert(token, tx);
                core.lock().unwrap().send_to(conn, Msg::FilesDial { token });
                let got = tokio::time::timeout(DIAL_BACK, rx).await;
                self.waiting.lock().unwrap().remove(&token);
                match got {
                    Ok(Ok(s)) => Ok(Session::new(s)),
                    _ => bail!("{peer} did not answer"),
                }
            }
        }
    }

    /// Server: a paired client opened a files connection.
    pub async fn accept(self: &Arc<Self>, mut s: Secured) -> Result<()> {
        let open: FsOpen = tokio::time::timeout(Duration::from_secs(5), s.reader.recv()).await.context("no request")??;
        match open.token {
            None => serve::respond(s, self.paths.clone()).await,
            Some(token) => match self.waiting.lock().unwrap().remove(&token) {
                Some(tx) => {
                    let _ = tx.send(s);
                    Ok(())
                }
                None => bail!("a files connection for an unknown request"),
            },
        }
    }

    /// Client: the server asked for a connection to answer its requests on.
    pub fn on_dial(self: &Arc<Self>, token: u64) {
        let me = self.clone();
        self.rt.spawn(async move {
            if let Err(e) = me.answer(token).await {
                log::warn!("files: {e:#}");
            }
        });
    }

    async fn answer(&self, token: u64) -> Result<()> {
        let Endpoint::Client { server_key, link } = &self.endpoint else { bail!("not a client") };
        let addr = (*link.lock().unwrap()).context("not connected to the server")?;
        let mut s = wire::connect(addr, &self.id, Purpose::Files, Duration::from_secs(5)).await?;
        if &s.remote_key != server_key {
            bail!("{addr} is a different machine than the server");
        }
        s.writer.send(&FsOpen { token: Some(token) }).await?;
        serve::respond(s, self.paths.clone()).await
    }

    pub fn shares(self: &Arc<Self>, peer: &str) -> Pending<Vec<Share>> {
        let (me, peer) = (self.clone(), peer.to_string());
        Pending::spawn(self, async move { me.open(&peer).await?.shares().await })
    }

    pub fn list(self: &Arc<Self>, peer: &str, share: &str, path: &str) -> Pending<Vec<DirEntry>> {
        let (me, peer, share, path) = (self.clone(), peer.to_string(), share.to_string(), path.to_string());
        Pending::spawn(self, async move { me.open(&peer).await?.list(&share, &path).await })
    }

    pub fn mkdir(self: &Arc<Self>, peer: &str, share: &str, path: &str) -> Pending<()> {
        let (me, peer, share, path) = (self.clone(), peer.to_string(), share.to_string(), path.to_string());
        Pending::spawn(self, async move { me.open(&peer).await?.mkdir(&share, &path).await })
    }

    /// Copies `names` from the folder `dir` of `peer`'s share into the local folder `dest`.
    pub fn download(self: &Arc<Self>, peer: &str, share: &str, dir: &str, names: Vec<String>, dest: PathBuf) -> Arc<Job> {
        jobs::download(self, peer, share, dir, names, dest)
    }

    /// Copies the local `sources` into the folder `dir` of `peer`'s share.
    pub fn upload(self: &Arc<Self>, peer: &str, share: &str, dir: &str, sources: Vec<PathBuf>) -> Arc<Job> {
        jobs::upload(self, peer, share, dir, sources)
    }

    pub fn jobs(&self) -> Vec<Arc<Job>> {
        self.jobs.lock().unwrap().clone()
    }

    /// Forgets finished jobs.
    pub fn clear_finished(&self) {
        self.jobs.lock().unwrap().retain(|j| !j.finished());
        self.hub.update(|_| {});
    }
}

#[cfg(test)]
pub(crate) mod tests;
