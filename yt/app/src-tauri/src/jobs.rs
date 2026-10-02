//! The download queue: spawning yt-dlp, tracking state, cancelling, persisting.

use crate::args::{self, ArgContext, JobOptions};
use crate::binaries;
use crate::errors;
use crate::parser::{self, Line};
use crate::paths::Paths;
use crate::platform;
use crate::store::{self, Settings, SubFile, Subscription};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

const MAX_LOG_LINES: usize = 400;
const KEEP_FINISHED: usize = 500;
const MIN_FREE_BYTES: u64 = 200 * 1024 * 1024;
const MIN_FREE_BYTES_LIVE: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Starting,
    Waiting,
    Downloading,
    Live,
    Processing,
    Done,
    Failed,
    Cancelled,
}

impl Status {
    pub fn is_finished(self) -> bool {
        matches!(self, Status::Done | Status::Failed | Status::Cancelled)
    }
    fn is_running(self) -> bool {
        matches!(self, Status::Starting | Status::Waiting | Status::Downloading | Status::Live | Status::Processing)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub url: String,
    pub title: Option<String>,
    pub thumbnail: Option<String>,
    pub options: JobOptions,
    pub status: Status,
    pub percent: Option<f64>,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub elapsed: Option<f64>,
    pub stream: u32,
    pub playlist: Option<(u32, u32)>,
    pub message: Option<String>,
    pub error: Option<String>,
    pub hint: Option<String>,
    pub files: Vec<String>,
    pub is_live: bool,
    pub source: Option<String>,
    pub command: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub log: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddJob {
    pub options: JobOptions,
    pub title: Option<String>,
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub is_live: bool,
    pub source: Option<String>,
}

/// Everything the job manager needs from the outside world. The real app
/// implements this with Tauri; tests use a recording stand-in.
pub trait Host: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
    fn notify(&self, title: &str, body: &str);
    fn exit(&self);
}

pub struct TauriHost(pub AppHandle);

impl Host for TauriHost {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.0.emit(event, payload);
    }
    fn notify(&self, title: &str, body: &str) {
        let _ = self.0.notification().builder().title(title).body(body).show();
    }
    fn exit(&self) {
        self.0.exit(0);
    }
}

pub struct AppState {
    pub host: Arc<dyn Host>,
    pub paths: Paths,
    pub settings: Mutex<Settings>,
    pub jobs: Mutex<Vec<Job>>,
    pub subs: Mutex<Vec<Subscription>>,
    pids: Mutex<HashMap<String, u32>>,
    stopping: Mutex<HashSet<String>>,
    batch_active: Mutex<bool>,
}

pub type Shared = Arc<AppState>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn make_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

impl AppState {
    pub fn new(host: Arc<dyn Host>, paths: Paths) -> Shared {
        paths.ensure_dirs();
        let settings = Settings::load(&paths);
        let subs = store::load::<SubFile>(&paths.data_dir.join("subscriptions.json"))
            .subscriptions
            .into_iter()
            .map(|mut s| {
                s.options.output_dir = paths.decode(&s.options.output_dir);
                s
            })
            .collect();
        let mut jobs: Vec<Job> = store::load::<Vec<Job>>(&paths.data_dir.join("jobs.json"));
        for j in jobs.iter_mut() {
            j.options.output_dir = paths.decode(&j.options.output_dir);
            if j.status.is_running() {
                j.status = Status::Queued;
                j.message = Some("Resumed after restart".into());
            }
        }
        Arc::new(AppState {
            host,
            paths,
            settings: Mutex::new(settings),
            jobs: Mutex::new(jobs),
            subs: Mutex::new(subs),
            pids: Mutex::new(HashMap::new()),
            stopping: Mutex::new(HashSet::new()),
            batch_active: Mutex::new(false),
        })
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    pub fn arg_context_pub(&self) -> ArgContext {
        self.arg_context()
    }

    fn arg_context(&self) -> ArgContext {
        let s = self.settings();
        ArgContext {
            ffmpeg_dir: binaries::locate(&self.paths, &s, "ffmpeg").and_then(|(p, _)| p.parent().map(Path::to_path_buf)),
            deno_path: binaries::locate(&self.paths, &s, "deno").map(|(p, _)| p),
            archive_dir: self.paths.archive_dir.clone(),
        }
    }

    pub fn preview(&self, o: &JobOptions) -> String {
        args::preview_command(o, &self.arg_context())
    }

    // ---- persistence ----------------------------------------------------

    pub fn save_jobs(&self) {
        let mut jobs = lock(&self.jobs).clone();
        // Cap history: keep every unfinished job plus the newest finished ones.
        let mut finished_seen = 0usize;
        jobs.reverse();
        jobs.retain(|j| {
            if j.status.is_finished() {
                finished_seen += 1;
                finished_seen <= KEEP_FINISHED
            } else {
                true
            }
        });
        jobs.reverse();
        for j in jobs.iter_mut() {
            j.options.output_dir = self.paths.encode(&j.options.output_dir);
            j.log.drain(..j.log.len().saturating_sub(60));
        }
        let _ = store::save(&self.paths.data_dir.join("jobs.json"), &jobs);
    }

    pub fn save_subs(&self) {
        let mut subs = lock(&self.subs).clone();
        for s in subs.iter_mut() {
            s.options.output_dir = self.paths.encode(&s.options.output_dir);
        }
        let _ = store::save(&self.paths.data_dir.join("subscriptions.json"), &SubFile { subscriptions: subs });
    }

    // ---- job access -----------------------------------------------------

    pub fn list(&self) -> Vec<Job> {
        lock(&self.jobs).iter().cloned().map(|mut j| {
            j.log.drain(..j.log.len().saturating_sub(5));
            j
        }).collect()
    }

    pub fn log_of(&self, id: &str) -> Vec<String> {
        lock(&self.jobs).iter().find(|j| j.id == id).map(|j| j.log.clone()).unwrap_or_default()
    }

    fn get(&self, id: &str) -> Option<Job> {
        lock(&self.jobs).iter().find(|j| j.id == id).cloned()
    }

    fn emit_job(&self, mut job: Job) {
        job.log.drain(..job.log.len().saturating_sub(5));
        self.host.emit("job-update", serde_json::to_value(&job).unwrap_or_default());
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Job)) -> Option<Job> {
        let snapshot = {
            let mut jobs = lock(&self.jobs);
            let job = jobs.iter_mut().find(|j| j.id == id)?;
            f(job);
            job.clone()
        };
        self.emit_job(snapshot.clone());
        Some(snapshot)
    }

    // ---- public operations ---------------------------------------------

    pub fn add(self: &Arc<Self>, req: AddJob) -> Job {
        let mut options = req.options;
        if options.output_dir.trim().is_empty() {
            options.output_dir = self.settings().download_dir;
        }
        let job = Job {
            id: make_id(),
            url: options.url.trim().to_string(),
            title: req.title,
            thumbnail: req.thumbnail,
            command: self.preview(&options),
            options,
            status: Status::Queued,
            percent: None,
            speed: None,
            eta: None,
            downloaded: 0,
            total: None,
            elapsed: None,
            stream: 0,
            playlist: None,
            message: None,
            error: None,
            hint: None,
            files: vec![],
            is_live: req.is_live,
            source: req.source,
            created_at: store::now(),
            started_at: None,
            finished_at: None,
            log: vec![],
        };
        lock(&self.jobs).push(job.clone());
        self.emit_job(job.clone());
        self.save_jobs();
        self.pump();
        job
    }

    pub fn retry(self: &Arc<Self>, id: &str) {
        self.update(id, |j| {
            if j.status.is_finished() {
                j.status = Status::Queued;
                j.percent = None;
                j.speed = None;
                j.eta = None;
                j.error = None;
                j.hint = None;
                j.message = None;
                j.finished_at = None;
                j.stream = 0;
                j.log.clear();
            }
        });
        self.save_jobs();
        self.pump();
    }

    /// Stop a job. Running jobs are asked to stop gracefully first (so partial
    /// live recordings are left tidy), then killed if they do not exit.
    pub fn cancel(self: &Arc<Self>, id: &str) {
        let Some(job) = self.get(id) else { return };
        if job.status == Status::Queued {
            self.update(id, |j| {
                j.status = Status::Cancelled;
                j.message = Some("Cancelled".into());
                j.finished_at = Some(store::now());
            });
            self.save_jobs();
            return;
        }
        let pid = lock(&self.pids).get(id).copied();
        if let Some(pid) = pid {
            lock(&self.stopping).insert(id.to_string());
            self.update(id, |j| j.message = Some("Stopping…".into()));
            if !platform::request_stop(pid) {
                platform::kill_tree(pid);
            }
            let st = self.clone();
            let id = id.to_string();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(8)).await;
                if let Some(pid) = lock(&st.pids).get(&id).copied() {
                    platform::kill_tree(pid);
                }
            });
        }
    }

    pub fn remove(self: &Arc<Self>, id: &str) {
        if self.get(id).is_some_and(|j| j.status.is_running()) {
            self.cancel(id);
        }
        lock(&self.jobs).retain(|j| j.id != id);
        self.host.emit("job-removed", serde_json::json!(id));
        self.save_jobs();
    }

    pub fn clear_finished(&self) {
        let removed: Vec<String> = {
            let mut jobs = lock(&self.jobs);
            let ids = jobs.iter().filter(|j| j.status.is_finished()).map(|j| j.id.clone()).collect();
            jobs.retain(|j| !j.status.is_finished());
            ids
        };
        for id in removed {
            self.host.emit("job-removed", serde_json::json!(id));
        }
        self.save_jobs();
    }

    /// Shut down everything on quit so no yt-dlp/ffmpeg is left behind.
    pub fn stop_all(&self) {
        for pid in lock(&self.pids).values() {
            platform::kill_tree(*pid);
        }
    }

    pub fn has_active(&self) -> bool {
        lock(&self.jobs).iter().any(|j| j.status.is_running() || j.status == Status::Queued)
    }

    // ---- scheduling -----------------------------------------------------

    /// Start as many queued jobs as the concurrency limit allows. Jobs that
    /// wait for a scheduled video do not count against the limit — they sit
    /// idle, and you may be waiting on several streams at once.
    pub fn pump(self: &Arc<Self>) {
        let max = self.settings().max_concurrent as usize;
        let to_start: Vec<String> = {
            let mut jobs = lock(&self.jobs);
            let mut active = jobs
                .iter()
                .filter(|j| j.status.is_running() && j.options.wait_for_video.is_none())
                .count();
            let mut ids = vec![];
            for j in jobs.iter_mut().filter(|j| j.status == Status::Queued) {
                let waits = j.options.wait_for_video.is_some();
                if waits || active < max {
                    j.status = Status::Starting;
                    j.message = Some("Starting…".into());
                    j.started_at = Some(store::now());
                    if !waits {
                        active += 1;
                    }
                    ids.push(j.id.clone());
                }
            }
            ids
        };
        if !to_start.is_empty() {
            *lock(&self.batch_active) = true;
        }
        for id in to_start {
            if let Some(j) = self.get(&id) {
                self.emit_job(j);
            }
            let st = self.clone();
            tauri::async_runtime::spawn(async move { run_job(st, id).await });
        }
    }

    fn check_queue_finished(self: &Arc<Self>) {
        if self.has_active() {
            return;
        }
        {
            let mut b = lock(&self.batch_active);
            if !*b {
                return;
            }
            *b = false;
        }
        match self.settings().after_queue.as_str() {
            "notify" => self.notify("All downloads finished", "The queue is empty."),
            "quit" => self.host.exit(),
            "shutdown" => {
                self.notify("All downloads finished", "Shutting down the computer…");
                platform::shutdown_machine();
            }
            _ => {}
        }
    }

    fn notify(&self, title: &str, body: &str) {
        self.host.notify(title, body);
    }

    // ---- subscriptions --------------------------------------------------

    /// Queue a check for one subscription now.
    pub fn check_subscription(self: &Arc<Self>, id: &str) -> bool {
        let sub = {
            let mut subs = lock(&self.subs);
            let Some(s) = subs.iter_mut().find(|s| s.id == id) else { return false };
            s.last_check = Some(store::now());
            s.last_note = Some("Checking for new videos…".into());
            s.clone()
        };
        // Don't stack checks: skip if the previous one is still going.
        let source = format!("sub:{}", sub.id);
        if lock(&self.jobs).iter().any(|j| j.source.as_deref() == Some(&source) && !j.status.is_finished()) {
            return true;
        }
        let mut options = sub.options.clone();
        options.url = sub.url.clone();
        options.use_archive = true;
        options.archive_name = Some(sub.id.clone());
        self.add(AddJob { options, title: Some(sub.name.clone()), thumbnail: None, is_live: false, source: Some(source) });
        self.save_subs();
        self.host.emit("subscriptions-changed", serde_json::Value::Null);
        true
    }

    pub fn due_subscriptions(&self) -> Vec<String> {
        let now = store::now();
        lock(&self.subs)
            .iter()
            .filter(|s| s.enabled && s.interval_minutes > 0)
            .filter(|s| s.last_check.is_none_or(|t| now - t >= s.interval_minutes as i64 * 60))
            .map(|s| s.id.clone())
            .collect()
    }

    fn note_subscription_result(&self, source: &str, note: String) {
        if let Some(id) = source.strip_prefix("sub:") {
            if let Some(s) = lock(&self.subs).iter_mut().find(|s| s.id == id) {
                s.last_note = Some(note);
            }
            self.save_subs();
            self.host.emit("subscriptions-changed", serde_json::Value::Null);
        }
    }
}

// ---- running one job ------------------------------------------------------

fn humanize_pp(name: &str) -> String {
    match name {
        "Merger" => "Merging video and audio…",
        "ExtractAudio" => "Converting audio…",
        "EmbedThumbnail" => "Adding cover art…",
        "EmbedSubtitle" | "FFmpegEmbedSubtitle" => "Embedding subtitles…",
        "FFmpegMetadata" | "Metadata" => "Writing metadata…",
        "SponsorBlock" | "ModifyChapters" => "Cutting sponsor segments…",
        "FFmpegSubtitlesConvertor" => "Converting subtitles…",
        "FFmpegSplitChapters" => "Splitting chapters…",
        "MoveFiles" => "Finishing up…",
        _ => "Processing…",
    }
    .to_string()
}

/// Best-effort title from a filename yt-dlp is writing: strips "[id]" and ".f137".
pub fn title_from_path(path: &str) -> Option<String> {
    let stem = Path::new(path).file_stem()?.to_string_lossy().to_string();
    let stem = match stem.rfind(".f") {
        Some(i) if stem[i + 2..].chars().all(|c| c.is_ascii_digit() || c == '-') && i + 2 < stem.len() => stem[..i].to_string(),
        _ => stem,
    };
    let stem = match (stem.rfind(" ["), stem.ends_with(']')) {
        (Some(i), true) => stem[..i].to_string(),
        _ => stem,
    };
    let stem = stem.trim().to_string();
    (!stem.is_empty()).then_some(stem)
}

async fn pipe_lines<R: tokio::io::AsyncRead + Unpin>(mut r: R, tx: mpsc::UnboundedSender<String>) {
    // yt-dlp separates lines with \n, but "waiting" updates use \r.
    let mut buf = vec![0u8; 8192];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !pending.is_empty() {
                            let _ = tx.send(String::from_utf8_lossy(&pending).into_owned());
                            pending.clear();
                        }
                    } else {
                        pending.push(b);
                    }
                }
            }
        }
    }
    if !pending.is_empty() {
        let _ = tx.send(String::from_utf8_lossy(&pending).into_owned());
    }
}

fn fail_job(st: &Shared, id: &str, message: &str, hint: Option<&str>) {
    st.update(id, |j| {
        j.status = Status::Failed;
        j.error = Some(message.to_string());
        j.hint = hint.map(str::to_string);
        j.finished_at = Some(store::now());
        j.message = None;
    });
    finish(st, id);
}

fn finish(st: &Shared, id: &str) {
    st.save_jobs();
    if let Some(job) = st.get(id) {
        if st.settings().notifications && !matches!(job.status, Status::Cancelled) {
            let name = job.title.clone().unwrap_or_else(|| job.url.clone());
            match job.status {
                Status::Done => st.notify("Download finished", &name),
                Status::Failed => st.notify("Download failed", &format!("{name}\n{}", job.error.clone().unwrap_or_default())),
                _ => {}
            }
        }
        if let Some(src) = &job.source {
            let note = match job.status {
                Status::Done if job.files.is_empty() => "Checked — nothing new".to_string(),
                Status::Done => format!("Downloaded {} new file(s)", job.files.len()),
                Status::Failed => format!("Check failed: {}", job.error.clone().unwrap_or_default()),
                _ => "Check stopped".to_string(),
            };
            st.note_subscription_result(src, note);
        }
    }
    st.pump();
    st.check_queue_finished();
}

pub async fn run_job(st: Shared, id: String) {
    let Some(job) = st.get(&id) else { return };
    let settings = st.settings();

    let Some((ytdlp, _)) = binaries::locate(&st.paths, &settings, "yt-dlp") else {
        fail_job(&st, &id, "yt-dlp isn't installed yet.", Some("Open Settings → Tools and click Install."));
        return;
    };

    // Make sure there is somewhere to write, and enough room to do so.
    let out_dir = PathBuf::from(&job.options.output_dir);
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        fail_job(&st, &id, &format!("Can't create the download folder: {e}"), Some("Choose a different download folder."));
        return;
    }
    if let Some(free) = crate::disk::free_space(&out_dir) {
        let need = if job.is_live || job.options.live_from_start { MIN_FREE_BYTES_LIVE } else { MIN_FREE_BYTES };
        if free < need {
            fail_job(
                &st,
                &id,
                &format!("Only {} MB free on the download drive.", free / 1024 / 1024),
                Some("Free some space or choose a different download folder."),
            );
            return;
        }
    }

    let ctx = st.arg_context();
    let argv = args::build_args(&job.options, &ctx);
    let mut cmd = Command::new(&ytdlp);
    cmd.args(&argv)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1");
    let mut path_var = std::ffi::OsString::from(&st.paths.bin_dir);
    if let Some(existing) = std::env::var_os("PATH") {
        path_var.push(if cfg!(windows) { ";" } else { ":" });
        path_var.push(existing);
    }
    cmd.env("PATH", path_var);
    platform::configure_command(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            fail_job(&st, &id, &format!("Couldn't start yt-dlp: {e}"), Some("Try reinstalling it from Settings → Tools."));
            return;
        }
    };
    if let Some(pid) = child.id() {
        lock(&st.pids).insert(id.clone(), pid);
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    if let Some(o) = child.stdout.take() {
        tauri::async_runtime::spawn(pipe_lines(o, tx.clone()));
    }
    if let Some(e) = child.stderr.take() {
        tauri::async_runtime::spawn(pipe_lines(e, tx.clone()));
    }
    drop(tx);

    st.update(&id, |j| {
        j.message = Some(if j.options.wait_for_video.is_some() { "Checking the video…".into() } else { "Connecting…".into() });
    });

    let mut last_error: Option<String> = None;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    while let Some(raw) = rx.recv().await {
        let parsed = parser::parse_line(&raw);
        let clean = parser::strip_ansi(&raw).trim().to_string();
        match parsed {
            Line::Progress(p) => {
                if last_emit.elapsed() < Duration::from_millis(200) && p.status != "finished" {
                    continue;
                }
                last_emit = Instant::now();
                st.update(&id, |j| {
                    let live_now = j.is_live && p.percent().is_none();
                    j.status = if live_now { Status::Live } else { Status::Downloading };
                    j.percent = p.percent();
                    j.speed = p.speed;
                    j.eta = p.eta;
                    j.downloaded = p.downloaded;
                    j.total = p.total;
                    j.elapsed = p.elapsed;
                    j.message = None;
                });
            }
            Line::PostProcess { name, status } => {
                if status == "started" {
                    st.update(&id, |j| {
                        j.status = Status::Processing;
                        j.percent = None;
                        j.speed = None;
                        j.eta = None;
                        j.message = Some(humanize_pp(&name));
                    });
                }
            }
            Line::File(path) => {
                st.update(&id, |j| {
                    if !j.files.contains(&path) {
                        j.files.push(path.clone());
                    }
                    if j.title.is_none() {
                        j.title = title_from_path(&path);
                    }
                });
            }
            Line::Destination(path) => {
                st.update(&id, |j| {
                    j.stream += 1;
                    if j.title.is_none() {
                        j.title = title_from_path(&path);
                    }
                    push_log(j, &clean);
                });
            }
            Line::PlaylistItem { index, total } => {
                st.update(&id, |j| {
                    j.playlist = Some((index, total));
                    j.stream = 0;
                    j.percent = None;
                    push_log(j, &clean);
                });
            }
            Line::Waiting(msg) => {
                st.update(&id, |j| {
                    j.status = Status::Waiting;
                    j.percent = None;
                    j.message = Some(msg.clone());
                    push_log(j, &clean);
                });
            }
            Line::Error(msg) => {
                last_error = Some(msg);
                st.update(&id, |j| push_log(j, &clean));
            }
            Line::Warning(_) | Line::Other(_) => {
                if !clean.is_empty() {
                    st.update(&id, |j| {
                        if j.status == Status::Starting {
                            j.message = Some("Fetching video info…".into());
                        }
                        push_log(j, &clean);
                    });
                }
            }
        }
    }

    let exit = child.wait().await;
    lock(&st.pids).remove(&id);
    let stopped = lock(&st.stopping).remove(&id);
    let ok = exit.as_ref().is_ok_and(|s| s.success());

    if stopped {
        st.update(&id, |j| {
            j.status = Status::Cancelled;
            j.message = Some("Stopped".into());
            j.speed = None;
            j.eta = None;
            j.finished_at = Some(store::now());
        });
    } else if ok {
        st.update(&id, |j| {
            j.status = Status::Done;
            j.percent = Some(100.0);
            j.speed = None;
            j.eta = None;
            j.message = None;
            j.finished_at = Some(store::now());
        });
    } else {
        let raw = last_error.unwrap_or_else(|| match &exit {
            Ok(s) => format!("yt-dlp stopped unexpectedly ({s})"),
            Err(e) => e.to_string(),
        });
        let explained = errors::explain(&raw);
        st.update(&id, |j| {
            j.status = Status::Failed;
            j.error = Some(explained.as_ref().map(|x| x.message.clone()).unwrap_or_else(|| raw.clone()));
            j.hint = explained.map(|x| x.hint);
            j.speed = None;
            j.eta = None;
            j.message = None;
            j.finished_at = Some(store::now());
        });
    }
    finish(&st, &id);
}

fn push_log(j: &mut Job, line: &str) {
    j.log.push(line.to_string());
    if j.log.len() > MAX_LOG_LINES {
        j.log.drain(..j.log.len() - MAX_LOG_LINES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_from_yt_dlp_filenames() {
        assert_eq!(title_from_path("./Me at the zoo [jNQXAC9IVRw].f140.m4a").as_deref(), Some("Me at the zoo"));
        assert_eq!(title_from_path("/x/y/Me at the zoo [jNQXAC9IVRw].mp4").as_deref(), Some("Me at the zoo"));
        assert_eq!(title_from_path("C:\\Videos\\Plain name.mp4").is_some(), true);
    }

    #[test]
    fn status_helpers() {
        assert!(Status::Done.is_finished());
        assert!(!Status::Waiting.is_finished());
        assert!(Status::Waiting.is_running());
        assert!(!Status::Queued.is_running());
    }
}

#[cfg(test)]
mod integration {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ytgrab-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn state_in(dir: &Path) -> Shared {
        struct NullHost;
        impl Host for NullHost {
            fn emit(&self, _: &str, _: serde_json::Value) {}
            fn notify(&self, _: &str, _: &str) {}
            fn exit(&self) {}
        }
        let paths = Paths::installed_at(dir.join("data"), dir.join("dl"));
        paths.ensure_dirs();
        let st = AppState::new(Arc::new(NullHost), paths);
        lock(&st.settings).notifications = false;
        st
    }

    async fn wait_for(st: &Shared, id: &str, want: impl Fn(&Job) -> bool, secs: u64) -> Job {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            let j = st.get(id).unwrap();
            if want(&j) {
                return j;
            }
            assert!(Instant::now() < end, "timed out; last state: {:?} msg={:?} err={:?}\nlog={:#?}", j.status, j.message, j.error, j.log);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Real yt-dlp + network. Run with:
    ///   YTGRAB_TEST_BIN=/dir/with/yt-dlp,ffmpeg,deno cargo test e2e_real -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn e2e_real_download_with_the_original_format() {
        let bin = PathBuf::from(std::env::var("YTGRAB_TEST_BIN").expect("set YTGRAB_TEST_BIN"));
        let dir = temp_dir("e2e");
        let st = state_in(&dir);
        for name in binaries::TOOLS {
            let src = bin.join(platform::exe_name(name));
            if src.exists() {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&src, st.paths.bin_dir.join(platform::exe_name(name))).unwrap();
            }
        }
        let job = st.add(AddJob {
            options: JobOptions {
                url: "https://www.youtube.com/watch?v=jNQXAC9IVRw".into(),
                format: Some("bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]".into()),
                wait_for_video: Some(args::WaitRange { min: 30, max: 60 }),
                ..Default::default()
            },
            title: None,
            thumbnail: None,
            is_live: false,
            source: None,
        });
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 180).await;
        println!("final: {:?} files={:?} title={:?}", done.status, done.files, done.title);
        assert_eq!(done.status, Status::Done, "error: {:?}\n{:#?}", done.error, done.log);
        assert_eq!(done.files.len(), 1);
        assert!(Path::new(&done.files[0]).exists());
        assert!(done.files[0].ends_with(".mp4"));
        assert_eq!(done.title.as_deref(), Some("Me at the zoo"));
        assert_eq!(done.percent, Some(100.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stand-in yt-dlp that behaves like a scheduled live stream:
    /// waits, then records forever until interrupted.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn waiting_then_live_then_graceful_stop() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("fake");
        let st = state_in(&dir);
        let fake = st.paths.bin_dir.join("yt-dlp");
        std::fs::write(
            &fake,
            r#"#!/bin/sh
trap 'echo "ERROR: Interrupted by user" >&2; exit 130' INT TERM
printf '[youtube] abc: This live event will begin in 2 hours.\n'
printf '[wait] Waiting for 00:00:01 - Press Ctrl+C to try now\033[K\r'
sleep 1
i=0
while true; do
  i=$((i+1))
  echo "PRG|downloading|$((i*1000))|NA|NA|NA|NA|$i|NA|$i.0"
  sleep 0.3
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let job = st.add(AddJob {
            options: JobOptions {
                url: "https://example.com/live".into(),
                wait_for_video: Some(args::WaitRange { min: 30, max: 60 }),
                live_from_start: true,
                ..Default::default()
            },
            title: Some("Fake stream".into()),
            thumbnail: None,
            is_live: true,
            source: None,
        });

        let waiting = wait_for(&st, &job.id, |j| j.status == Status::Waiting, 10).await;
        assert!(waiting.message.unwrap().contains("Waiting for 00:00:01"));

        let live = wait_for(&st, &job.id, |j| j.status == Status::Live && j.elapsed.unwrap_or(0.0) >= 2.0, 15).await;
        assert!(live.downloaded >= 2000);
        assert_eq!(live.percent, None);

        // Stop: must exit gracefully via SIGINT and be reported as stopped, not failed.
        st.cancel(&job.id);
        let end = wait_for(&st, &job.id, |j| j.status.is_finished(), 12).await;
        assert_eq!(end.status, Status::Cancelled, "log: {:#?}", end.log);
        assert!(lock(&st.pids).is_empty(), "process was not cleaned up");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An error from yt-dlp is explained in plain language.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn failures_are_explained() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("fail");
        let st = state_in(&dir);
        let fake = st.paths.bin_dir.join("yt-dlp");
        std::fs::write(&fake, "#!/bin/sh\necho \"ERROR: [youtube] abc: Sign in to confirm you're not a bot\" >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/v".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        assert_eq!(done.status, Status::Failed);
        assert!(done.error.unwrap().contains("not a bot"));
        assert!(done.hint.unwrap().contains("cookies"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Jobs survive a restart, and active ones come back queued.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn queue_persists_across_restart() {
        let dir = temp_dir("persist");
        let st = state_in(&dir);
        // No yt-dlp installed, so the job fails fast — but it must still be saved.
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/v".into(), ..Default::default() }, title: Some("T".into()), thumbnail: None, is_live: false, source: None });
        wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        let again = state_in(&dir);
        assert_eq!(again.list().len(), 1);
        assert_eq!(again.list()[0].title.as_deref(), Some("T"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Downloads the real tools from the internet into a temp folder and runs them.
    ///   cargo test e2e_install -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn e2e_install_tools_from_the_internet() {
        struct Print;
        impl Host for Print {
            fn emit(&self, _: &str, _: serde_json::Value) {}
            fn notify(&self, _: &str, _: &str) {}
            fn exit(&self) {}
        }
        let dir = temp_dir("install");
        let paths = Paths::installed_at(dir.join("data"), dir.join("dl"));
        for name in binaries::TOOLS {
            binaries::install(&Print, &paths, name).await.unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let settings = Settings::default();
        let bins = binaries::inspect(&paths, &settings).await;
        for b in &bins {
            println!("{:8} {:?} {:?}", b.name, b.source, b.version);
            assert_eq!(b.source, "managed");
            assert!(b.version.is_some(), "{} did not run", b.name);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
