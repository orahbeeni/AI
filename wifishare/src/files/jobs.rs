//! Transfers as jobs: planned once, then run over as many connections as it takes. A dropped link
//! only pauses a job; when the peer is back it carries on from the bytes already on disk.

use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use super::session::{Ctl, Session};
use super::{Cancelled, Files, PART, fatal, is_transient, join, local, unique_name};
use crate::clipnet::{safe_join, walk};
use crate::proto::FileEntry;

/// How long a job keeps waiting for a peer that cannot be reached.
const PATIENCE: Duration = Duration::from_secs(15 * 60);
/// After this many connections in a row that moved nothing, a job gives up.
const MAX_FRUITLESS: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Download,
    Upload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobState {
    Running,
    /// Paused until the peer can be reached again (the reason is shown).
    Waiting(String),
    Done,
    Failed(String),
    Cancelled,
}

pub struct Job {
    pub kind: JobKind,
    pub peer: String,
    /// "3 items", "report.pdf".
    pub what: String,
    /// Downloads: the local folder they go to. Uploads: the folder on the peer.
    pub place: String,
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
    state: Mutex<JobState>,
}

impl Job {
    pub fn state(&self) -> JobState {
        self.state.lock().unwrap().clone()
    }

    pub fn finished(&self) -> bool {
        !matches!(self.state(), JobState::Running | JobState::Waiting(_))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn set(&self, state: JobState) {
        *self.state.lock().unwrap() = state;
    }
}

fn describe(names: &[String]) -> String {
    match names {
        [one] => format!("\"{one}\""),
        many => format!("{} items", many.len()),
    }
}

fn start(files: &Arc<Files>, kind: JobKind, peer: &str, what: String, place: String) -> Arc<Job> {
    let job = Arc::new(Job {
        kind,
        peer: peer.into(),
        what,
        place,
        done: AtomicU64::new(0),
        total: AtomicU64::new(0),
        cancel: AtomicBool::new(false),
        state: Mutex::new(JobState::Running),
    });
    files.jobs.lock().unwrap().push(job.clone());
    files.hub.update(|_| {});
    job
}

type Step<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// Runs `attempt` on fresh connections until it succeeds, fails for good, or is cancelled.
async fn drive(files: Arc<Files>, job: Arc<Job>, mut attempt: impl for<'a> FnMut(&'a mut Session) -> Step<'a>) {
    let mut give_up = Instant::now() + PATIENCE;
    // Connections that were made but got nowhere, in a row: something is wrong that waiting
    // will not fix.
    let (mut fruitless, mut last_done) = (0, 0);
    let end = loop {
        if job.cancel.load(Ordering::Relaxed) {
            break JobState::Cancelled;
        }
        let outcome = match files.open(&job.peer).await {
            Ok(mut session) => {
                job.set(JobState::Running);
                give_up = Instant::now() + PATIENCE;
                let result = attempt(&mut session).await;
                let done = job.done.load(Ordering::Relaxed);
                fruitless = if done > last_done { 0 } else { fruitless + 1 };
                last_done = done;
                result
            }
            Err(e) => Err(e),
        };
        match outcome {
            Ok(()) => break JobState::Done,
            Err(e) if e.downcast_ref::<Cancelled>().is_some() => break JobState::Cancelled,
            Err(e) if is_transient(&e) => {
                if fruitless >= MAX_FRUITLESS {
                    break JobState::Failed(format!("the transfer keeps breaking off: {e:#}"));
                }
                if Instant::now() > give_up {
                    break JobState::Failed(format!("gave up waiting for {}: {e:#}", job.peer));
                }
                log::info!("files: waiting to carry on with {}: {e:#}", job.what);
                job.set(JobState::Waiting(format!("{e:#}")));
                files.hub.update(|_| {});
                for _ in 0..6 {
                    if job.cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
            Err(e) => break JobState::Failed(format!("{e:#}")),
        }
    };
    if end == JobState::Done {
        job.done.store(job.total.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    log::info!("files: {} {:?}", job.what, end);
    job.set(end);
    files.hub.update(|_| {});
}

// ---- downloads ----

struct Download {
    share: String,
    dir: String,
    names: Vec<String>,
    dest: PathBuf,
}

#[derive(Default)]
struct DownPlan {
    planned: bool,
    files: Vec<DownFile>,
    /// Index of the first file not finished yet, and the bytes of those before it.
    next: usize,
    completed: u64,
}

struct DownFile {
    remote: String,
    local: PathBuf,
    size: u64,
    #[cfg_attr(not(unix), allow(dead_code))]
    exec: bool,
}

pub(super) fn download(files: &Arc<Files>, peer: &str, share: &str, dir: &str, names: Vec<String>, dest: PathBuf) -> Arc<Job> {
    let job = start(files, JobKind::Download, peer, describe(&names), dest.display().to_string());
    let args = Arc::new(Download { share: share.into(), dir: dir.into(), names, dest });
    let plan = Arc::new(tokio::sync::Mutex::new(DownPlan::default()));
    let j = job.clone();
    files.rt.spawn(drive(files.clone(), job.clone(), move |session| {
        let (job, args, plan) = (j.clone(), args.clone(), plan.clone());
        Box::pin(async move { download_attempt(&job, session, &mut *plan.lock().await, &args).await })
    }));
    job
}

async fn download_attempt(job: &Job, s: &mut Session, plan: &mut DownPlan, a: &Download) -> Result<()> {
    if !plan.planned {
        plan_download(job, s, plan, a).await?;
    }
    while plan.next < plan.files.len() {
        let f = &plan.files[plan.next];
        let size = f.size;
        let mut part = f.local.clone().into_os_string();
        part.push(PART);
        let part = PathBuf::from(part);
        // Bytes already on disk from an earlier attempt are not fetched again.
        let have = match tokio::fs::metadata(&part).await {
            Ok(m) if m.len() <= f.size => m.len(),
            _ => 0,
        };
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&part)
            .await
            .map_err(|e| local("creating a file", e))?;
        file.set_len(have).await.map_err(|e| local("writing a file", e))?;
        file.seek(std::io::SeekFrom::Start(have)).await.map_err(|e| local("writing a file", e))?;
        if have < f.size {
            let ctl = Ctl { done: &job.done, base: plan.completed, cancel: &job.cancel };
            s.read_to(&a.share, &f.remote, have, f.size, &mut file, &ctl).await?;
        }
        file.flush().await.map_err(|e| local("writing a file", e))?;
        drop(file);
        #[cfg(unix)]
        if f.exec {
            use std::os::unix::fs::PermissionsExt;
            let _ = tokio::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755)).await;
        }
        tokio::fs::rename(&part, &f.local).await.map_err(|e| local("finishing a file", e))?;
        plan.completed += size;
        plan.next += 1;
        job.done.store(plan.completed, Ordering::Relaxed);
    }
    Ok(())
}

async fn plan_download(job: &Job, s: &mut Session, plan: &mut DownPlan, a: &Download) -> Result<()> {
    tokio::fs::create_dir_all(&a.dest).await.map_err(|e| local("creating the destination folder", e))?;
    let mut chosen = HashSet::new();
    let mut total = 0;
    for name in &a.names {
        let manifest = s.walk(&a.share, &join(&a.dir, name)).await?;
        let mine = unique_name(|n| chosen.contains(n) || a.dest.join(n).symlink_metadata().is_ok(), name);
        chosen.insert(mine.clone());
        for entry in manifest {
            let rel = match entry.path.split_once('/') {
                Some((_, rest)) => format!("{mine}/{rest}"),
                None => mine.clone(),
            };
            let local_path = safe_join(&a.dest, &rel).map_err(|e| fatal(format!("{e:#}")))?;
            if entry.dir {
                tokio::fs::create_dir_all(&local_path).await.map_err(|e| local("creating a folder", e))?;
                continue;
            }
            if let Some(parent) = local_path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| local("creating a folder", e))?;
            }
            total += entry.size;
            plan.files.push(DownFile { remote: join(&a.dir, &entry.path), local: local_path, size: entry.size, exec: entry.exec });
        }
    }
    job.total.store(total, Ordering::Relaxed);
    plan.planned = true;
    Ok(())
}

// ---- uploads ----

struct Upload {
    share: String,
    dir: String,
    sources: Vec<PathBuf>,
}

#[derive(Default)]
struct UpPlan {
    planned: bool,
    items: Vec<UpItem>,
    next: usize,
    completed: u64,
}

struct UpItem {
    src: PathBuf,
    remote: String,
    dir: bool,
    size: u64,
    exec: bool,
    modified: Option<SystemTime>,
}

pub(super) fn upload(files: &Arc<Files>, peer: &str, share: &str, dir: &str, sources: Vec<PathBuf>) -> Arc<Job> {
    let names: Vec<String> =
        sources.iter().map(|p| p.file_name().map_or_else(|| "files".into(), |n| n.to_string_lossy().into_owned())).collect();
    let job = start(files, JobKind::Upload, peer, describe(&names), format!("{share}/{dir}").trim_end_matches('/').to_string());
    let args = Arc::new(Upload { share: share.into(), dir: dir.into(), sources });
    let plan = Arc::new(tokio::sync::Mutex::new(UpPlan::default()));
    let j = job.clone();
    files.rt.spawn(drive(files.clone(), job.clone(), move |session| {
        let (job, args, plan) = (j.clone(), args.clone(), plan.clone());
        Box::pin(async move { upload_attempt(&job, session, &mut *plan.lock().await, &args).await })
    }));
    job
}

async fn upload_attempt(job: &Job, s: &mut Session, plan: &mut UpPlan, a: &Upload) -> Result<()> {
    if !plan.planned {
        plan_upload(job, s, plan, a).await?;
    }
    while plan.next < plan.items.len() {
        let item = &plan.items[plan.next];
        if item.dir {
            s.mkdir(&a.share, &item.remote).await?;
        } else {
            let meta = tokio::fs::metadata(&item.src).await.map_err(|e| local("reading a file", e))?;
            if meta.len() != item.size || meta.modified().ok() != item.modified {
                return Err(fatal(format!("\"{}\" changed while it was being copied", item.src.display())));
            }
            let ctl = Ctl { done: &job.done, base: plan.completed, cancel: &job.cancel };
            s.write_from(&a.share, &item.remote, &item.src, item.size, item.exec, &ctl).await?;
            plan.completed += item.size;
            job.done.store(plan.completed, Ordering::Relaxed);
        }
        plan.next += 1;
    }
    Ok(())
}

async fn plan_upload(job: &Job, s: &mut Session, plan: &mut UpPlan, a: &Upload) -> Result<()> {
    let sources = a.sources.clone();
    let list: Vec<(PathBuf, FileEntry)> =
        tokio::task::spawn_blocking(move || walk(&sources)).await?.map_err(|e| fatal(format!("{e:#}")))?;
    // Top-level names that are taken over there get a number, the same way for every file below.
    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut total = 0;
    for (src, entry) in list {
        let (top, rest) = match entry.path.split_once('/') {
            Some((top, rest)) => (top.to_string(), Some(rest.to_string())),
            None => (entry.path.clone(), None),
        };
        let mine = match renamed.iter().find(|(from, _)| *from == top) {
            Some((_, to)) => to.clone(),
            None => {
                let to = s.free_name(&a.share, &a.dir, &top).await?;
                renamed.push((top, to.clone()));
                to
            }
        };
        let remote = join(&a.dir, &join(&mine, rest.as_deref().unwrap_or("")));
        let modified = if entry.dir { None } else { tokio::fs::metadata(&src).await.ok().and_then(|m| m.modified().ok()) };
        total += entry.size;
        plan.items.push(UpItem { src, remote, dir: entry.dir, size: entry.size, exec: entry.exec, modified });
    }
    job.total.store(total, Ordering::Relaxed);
    plan.planned = true;
    Ok(())
}
