//! The download queue: spawning yt-dlp, tracking state, cancelling, persisting.

use crate::args::{self, ArgContext, JobOptions, TranscribeOptions};
use crate::binaries;
use crate::errors;
use crate::existing;
use crate::parser::{self, Line};
use crate::paths::Paths;
use crate::platform;
use crate::store::{self, Settings, SubFile, Subscription};
use crate::transcribe;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

const MAX_LOG_LINES: usize = 400;
/// A download that has printed nothing for this long is flagged as possibly stalled.
const STALL_SECS: u64 = 180;

fn stall_after() -> Duration {
    Duration::from_secs(std::env::var("YTGRAB_STALL_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(STALL_SECS))
}

/// "+03:25": how far into the run a log line was written.
fn offset(since: Instant) -> String {
    let s = since.elapsed().as_secs();
    format!("+{:02}:{:02}", s / 60, s % 60)
}
const KEEP_FINISHED: usize = 500;
const MIN_FREE_BYTES: u64 = 200 * 1024 * 1024;
const MIN_FREE_BYTES_LIVE: u64 = 1024 * 1024 * 1024;
/// When the site says "slow down" the whole queue stops starting new downloads for this long.
const RATE_PAUSE_SECS: i64 = 15 * 60;
/// A download that hit a rate limit is put back in the queue this many times before it is reported as failed.
const MAX_RATE_RETRIES: u32 = 2;

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

/// What the transcript step did, for the summary shown after a job.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptStats {
    pub model: String,
    pub language: String,
    /// Seconds of audio that were transcribed.
    pub audio_secs: f64,
    pub words: u64,
    /// Voices found, when speaker labels were asked for.
    #[serde(default)]
    pub speakers: u32,
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
    /// Transcript files written next to the downloaded media.
    #[serde(default)]
    pub transcripts: Vec<String>,
    /// Set when a transcript was asked for but could not be made; the download itself is fine.
    #[serde(default)]
    pub transcript_error: Option<String>,
    pub is_live: bool,
    pub source: Option<String>,
    pub command: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// Seconds spent in each finished phase: wait, setup, prepare, download,
    /// record, convert, transcribe.
    #[serde(default)]
    pub timings: BTreeMap<String, f64>,
    /// The phase running right now, and when it began (unix seconds).
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub phase_started: Option<f64>,
    /// Total size of the files this job produced.
    #[serde(default)]
    pub output_bytes: u64,
    #[serde(default)]
    pub transcript_stats: Option<TranscriptStats>,
    /// How many times this job was put back after the site rate-limited us.
    #[serde(default)]
    pub rate_retries: u32,
    /// Set after a "file already on disk" job failed while re-tagging it: the next run
    /// downloads the file again instead of trying to patch the old one.
    #[serde(default)]
    pub overwrite_retry: bool,
    /// A file that was already on this computer: the job only makes its transcript (no yt-dlp).
    #[serde(default)]
    pub local_file: bool,
    /// The job was continued after stopping part-way: transcripts that are already finished are kept.
    #[serde(default)]
    pub resumed: bool,
    pub log: Vec<String>,
}

/// Move a job into `phase` (or out of all phases with `None`), crediting the
/// time spent in the previous one.
fn set_phase(j: &mut Job, phase: Option<&str>) {
    if j.phase.as_deref() == phase {
        return;
    }
    let now = store::now_f();
    if let (Some(p), Some(t)) = (j.phase.take(), j.phase_started.take()) {
        *j.timings.entry(p).or_default() += (now - t).max(0.0);
    }
    j.phase = phase.map(str::to_string);
    j.phase_started = phase.map(|_| now);
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
    /// Unix time until which no new download starts (after a rate limit).
    paused_until: Mutex<Option<i64>>,
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
        let st = Arc::new(AppState {
            host,
            paths,
            settings: Mutex::new(settings),
            jobs: Mutex::new(jobs),
            subs: Mutex::new(subs),
            pids: Mutex::new(HashMap::new()),
            stopping: Mutex::new(HashSet::new()),
            batch_active: Mutex::new(false),
            paused_until: Mutex::new(None),
        });
        st.prune_logs();
        st
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

    /// The job's log: the full text kept on disk when there is one (it survives restarts and is
    /// not cut off after a few hundred lines), else what is in memory.
    pub fn log_of(&self, id: &str) -> Vec<String> {
        if let Ok(text) = std::fs::read_to_string(self.log_path(id)) {
            let lines: Vec<String> = text.lines().map(str::to_string).collect();
            return lines[lines.len().saturating_sub(3000)..].to_vec();
        }
        lock(&self.jobs).iter().find(|j| j.id == id).map(|j| j.log.clone()).unwrap_or_default()
    }

    /// Where the full log of a job is kept.
    pub fn log_path(&self, id: &str) -> PathBuf {
        self.paths.data_dir.join("logs").join(format!("{id}.log"))
    }

    /// What yt-dlp reported about each video of a job (date, channel, link), one JSON object per line.
    pub fn meta_path(&self, id: &str) -> PathBuf {
        self.paths.data_dir.join("logs").join(format!("{id}.meta.jsonl"))
    }

    fn open_log(&self, id: &str) -> Option<std::fs::File> {
        let p = self.log_path(id);
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::OpenOptions::new().create(true).append(true).open(p).ok()
    }

    /// Delete the logs of jobs that no longer exist.
    fn prune_logs(&self) {
        let keep: HashSet<String> = lock(&self.jobs).iter().map(|j| j.id.clone()).collect();
        let Ok(rd) = std::fs::read_dir(self.paths.data_dir.join("logs")) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_prefix("skip-").unwrap_or(&name).split('.').next().unwrap_or("").to_string();
            if !keep.contains(&id) {
                let _ = std::fs::remove_file(e.path());
            }
        }
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
            if job.status.is_finished() || job.status == Status::Queued {
                set_phase(job, None);
            }
            job.clone()
        };
        self.emit_job(snapshot.clone());
        Some(snapshot)
    }

    // ---- public operations ---------------------------------------------

    pub fn add(self: &Arc<Self>, req: AddJob) -> Job {
        self.add_inner(req, false)
    }

    /// Queue a transcript for a media file that is already on disk. The transcript is written
    /// next to the file.
    pub fn add_local(self: &Arc<Self>, path: &str, opts: TranscribeOptions) -> Result<Job, String> {
        let p = Path::new(path);
        if !p.is_file() {
            return Err(format!("“{path}” isn't a file that can be read."));
        }
        if !transcribe::is_media(path) {
            return Err("That isn't an audio or video file YT Grab can read (mp4, mkv, webm, mov, mp3, m4a, opus, wav, flac…).".into());
        }
        let options = JobOptions {
            url: path.to_string(),
            output_dir: p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
            transcribe: Some(opts),
            ..Default::default()
        };
        let title = p.file_stem().map(|s| s.to_string_lossy().into_owned());
        Ok(self.add_inner(AddJob { options, title, thumbnail: None, is_live: false, source: None }, true))
    }

    fn add_inner(self: &Arc<Self>, req: AddJob, local: bool) -> Job {
        let mut options = req.options;
        if options.output_dir.trim().is_empty() {
            options.output_dir = self.settings().download_dir;
        }
        let media = options.url.trim().to_string();
        let job = Job {
            id: make_id(),
            url: options.url.trim().to_string(),
            title: req.title,
            thumbnail: req.thumbnail,
            command: if local { format!("Whisper transcript of {}", options.url) } else { self.preview(&options) },
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
            files: if local { vec![media] } else { vec![] },
            transcripts: vec![],
            transcript_error: None,
            is_live: req.is_live,
            source: req.source,
            created_at: store::now(),
            started_at: None,
            finished_at: None,
            timings: BTreeMap::new(),
            phase: None,
            phase_started: None,
            output_bytes: 0,
            transcript_stats: None,
            rate_retries: 0,
            overwrite_retry: false,
            local_file: local,
            resumed: false,
            log: vec![],
        };
        lock(&self.jobs).push(job.clone());
        self.emit_job(job.clone());
        self.save_jobs();
        self.pump();
        job
    }

    /// Run a finished job again. A job that stopped part-way through a playlist carries on instead:
    /// finished videos are skipped (see existing.rs), a half-downloaded file resumes, and the time
    /// already spent stays in its stats.
    pub fn retry(self: &Arc<Self>, id: &str) {
        self.update(id, |j| {
            if j.status.is_finished() {
                let unfinished = j.status != Status::Done || j.transcript_error.is_some();
                let continuing = unfinished && j.playlist.is_some_and(|(_, n)| n > 1);
                j.resumed = unfinished;
                j.playlist = None;
                j.status = Status::Queued;
                j.percent = None;
                j.speed = None;
                j.eta = None;
                j.error = None;
                j.hint = None;
                j.message = None;
                j.finished_at = None;
                j.stream = 0;
                j.transcripts.clear();
                j.transcript_error = None;
                j.rate_retries = 0;
                j.overwrite_retry = false;
                j.output_bytes = 0;
                j.transcript_stats = None;
                if !continuing {
                    j.timings.clear();
                    j.log.clear();
                }
            }
        });
        self.save_jobs();
        self.pump();
    }

    /// Make a transcript for a download that has already finished.
    pub fn transcribe_existing(self: &Arc<Self>, id: &str, opts: TranscribeOptions) -> Result<(), String> {
        let job = self.get(id).ok_or("That download no longer exists.")?;
        if job.status != Status::Done {
            return Err("Wait for the download to finish first.".into());
        }
        if !job.files.iter().any(|f| transcribe::is_media(f)) {
            return Err("There is no audio or video file to transcribe.".into());
        }
        self.update(id, |j| {
            j.status = Status::Processing;
            j.percent = Some(0.0);
            j.message = Some("Preparing transcription…".into());
            set_phase(j, Some("setup"));
            j.transcript_error = None;
            j.finished_at = None;
            j.options.transcribe = Some(opts.clone());
            j.resumed = false; // asked for on purpose: make them again
        });
        let st = self.clone();
        let id = id.to_string();
        tauri::async_runtime::spawn(async move {
            let (made, error, stats) = match transcribe::ensure_ready(st.host.as_ref(), &st.paths, &opts).await {
                Ok(()) => transcribe_files(&st, &id, &opts).await,
                Err(e) => (vec![], Some(format!("Transcript failed: {e}")), None),
            };
            let stopped = lock(&st.stopping).remove(&id);
            st.update(&id, |j| {
                // The download itself is finished either way.
                j.status = Status::Done;
                j.percent = Some(100.0);
                j.message = None;
                for m in made {
                    if !j.transcripts.contains(&m) {
                        j.transcripts.push(m);
                    }
                }
                j.transcript_error = if stopped { Some("Transcription stopped.".into()) } else { error };
                if stats.is_some() {
                    j.transcript_stats = stats;
                }
                j.finished_at = Some(store::now());
            });
            st.save_jobs();
            st.pump();
        });
        Ok(())
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
        let _ = std::fs::remove_file(self.log_path(id));
        let _ = std::fs::remove_file(self.meta_path(id));
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
            let _ = std::fs::remove_file(self.log_path(&id));
            let _ = std::fs::remove_file(self.meta_path(&id));
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
        let paused = self.paused_until().is_some();
        let to_start: Vec<String> = {
            let mut jobs = lock(&self.jobs);
            let mut active = jobs
                .iter()
                .filter(|j| j.status.is_running() && j.options.wait_for_video.is_none())
                .count();
            // A "one at a time" download runs alone: nothing else starts while it runs, and it
            // waits for others to finish. Jobs behind a waiting one-at-a-time job hold back too,
            // so it cannot be starved by a stream of later downloads.
            let mut alone_running = jobs.iter().any(|j| j.status.is_running() && j.options.wait_for_video.is_none() && j.options.one_at_a_time);
            let mut hold = false;
            let mut ids = vec![];
            for j in jobs.iter_mut().filter(|j| j.status == Status::Queued) {
                let waits = j.options.wait_for_video.is_some();
                let alone = j.options.one_at_a_time;
                let can_start = waits
                    || (!paused && !hold && !alone_running && active < max && (!alone || active == 0));
                if !waits && alone && !can_start && !paused {
                    hold = true;
                }
                if can_start {
                    j.status = Status::Starting;
                    j.message = Some("Starting…".into());
                    j.started_at = Some(store::now());
                    set_phase(j, Some("prepare"));
                    if !waits {
                        active += 1;
                        alone_running |= alone;
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

    // ---- rate-limit pause -------------------------------------------------

    /// Unix time the queue resumes at, if it is paused.
    pub fn paused_until(&self) -> Option<i64> {
        let mut p = lock(&self.paused_until);
        if p.is_some_and(|t| t <= store::now()) {
            *p = None;
        }
        *p
    }

    /// Stop starting new downloads for `secs` (running ones carry on). Resumes by itself.
    pub fn pause_queue(self: &Arc<Self>, secs: i64) {
        let until = store::now() + secs;
        *lock(&self.paused_until) = Some(until);
        self.host.emit("queue-paused", serde_json::json!({ "until": until }));
        let st = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(secs.max(0) as u64)).await;
            // Only if it was not resumed, or paused again for longer, in the meantime.
            if *lock(&st.paused_until) == Some(until) {
                st.resume_queue();
            }
        });
    }

    pub fn resume_queue(self: &Arc<Self>) {
        *lock(&self.paused_until) = None;
        self.host.emit("queue-paused", serde_json::json!({ "until": null }));
        self.pump();
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
                Status::Done => st.notify(if job.local_file { "Transcript ready" } else { "Download finished" }, &name),
                Status::Failed => st.notify(if job.local_file { "Transcript failed" } else { "Download failed" }, &format!("{name}\n{}", job.error.clone().unwrap_or_default())),
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
    if job.local_file {
        run_local(&st, &id, &job).await;
        return;
    }
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

    // Transcription needs an engine and a model. Fetch them now (first use
    // only) rather than after a long recording. If that fails the download
    // still goes ahead; the transcript can be made later.
    let mut transcript_setup_error: Option<String> = None;
    if let Some(t) = &job.options.transcribe {
        if !transcribe::is_ready(&st.paths, t) {
            st.update(&id, |j| {
                j.message = Some("Setting up transcription (first time only)…".into());
                set_phase(j, Some("setup"));
            });
            if let Err(e) = transcribe::ensure_ready(st.host.as_ref(), &st.paths, t).await {
                transcript_setup_error = Some(e);
            }
        }
    }

    let ctx = st.arg_context();
    let mut argv = args::build_args(&job.options, &ctx);
    if job.overwrite_retry {
        argv.insert(0, "--force-overwrites".into());
    }
    // Videos already saved in the folder are left alone (see existing.rs): no re-tagging, no re-download.
    let saved = existing::saved_in(&out_dir, &job.options);
    let skip_file = st.paths.data_dir.join("logs").join(format!("skip-{id}.txt"));
    let skipping = !saved.is_empty() && !job.overwrite_retry && std::fs::create_dir_all(skip_file.parent().unwrap_or(Path::new("."))).is_ok() && std::fs::write(&skip_file, existing::archive_text(&saved)).is_ok();
    if job.options.transcribe.is_some() {
        // The transcript's header carries the upload date, channel and link of each video.
        argv.insert(0, st.meta_path(&id).to_string_lossy().into_owned());
        argv.insert(0, "after_move:%(.{id,upload_date,uploader,title,webpage_url})j".into());
        argv.insert(0, "--print-to-file".into());
    }
    if skipping {
        argv.insert(0, skip_file.to_string_lossy().into_owned());
        argv.insert(0, "--download-archive".into());
    }
    let mut logf = st.open_log(&id);
    {
        use std::io::Write;
        if let Some(f) = logf.as_mut() {
            let _ = writeln!(f, "\n==== run started (unix time {}) ====\nyt-dlp {}", store::now(), argv.join(" "));
            if skipping {
                let _ = writeln!(f, "[app] {} video(s) already saved in this folder are skipped.", saved.len());
            }
        }
    }
    if skipping {
        let note = format!("[app] {} video(s) already saved in this folder are skipped.", saved.len());
        let transcribing = job.options.transcribe.clone();
        st.update(&id, |j| {
            push_log(j, &note);
            // They still get a transcript if one was asked for and is missing.
            if let Some(t) = &transcribing {
                for s in &saved {
                    let base = transcribe::output_base(&s.path);
                    let has = transcribe::clean_formats(&t.formats).split(',').all(|f| PathBuf::from(format!("{}.{f}", base.display())).is_file());
                    if !has && !j.files.contains(&s.path.to_string_lossy().into_owned()) {
                        j.files.push(s.path.to_string_lossy().into_owned());
                    }
                }
            }
        });
    }
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
        set_phase(j, Some("prepare"));
    });

    let mut last_error: Option<String> = None;
    // Every problem yt-dlp reported, with the playlist item it belonged to.
    let mut item_errors: Vec<(Option<u32>, String)> = vec![];
    let mut current_item: Option<u32> = None;
    let mut want_detail = false;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let run_started = Instant::now();
    let mut stalled_since: Option<Instant> = None;
    loop {
        let raw = match tokio::time::timeout(stall_after(), rx.recv()).await {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(_) => {
                // Nothing for a long while in the middle of a download: say so, once.
                if stalled_since.is_none() && st.get(&id).is_some_and(|j| j.status == Status::Downloading) {
                    stalled_since = Some(Instant::now());
                    let note = format!("[app] No data from yt-dlp for {} s — the connection may have stalled. Still waiting.", stall_after().as_secs());
                    {
                        use std::io::Write;
                        if let Some(f) = logf.as_mut() {
                            let _ = writeln!(f, "{} {note}", offset(run_started));
                        }
                    }
                    st.update(&id, |j| {
                        j.message = Some("No data for a while — the connection may have stalled".into());
                        push_log(j, &note);
                    });
                }
                continue;
            }
        };
        if let Some(t) = stalled_since.take() {
            use std::io::Write;
            if let Some(f) = logf.as_mut() {
                let _ = writeln!(f, "{} [app] Data is flowing again after {} s.", offset(run_started), t.elapsed().as_secs());
            }
        }
        let parsed = parser::parse_line(&raw);
        let clean = parser::strip_ansi(&raw).trim().to_string();
        if !clean.is_empty() && !matches!(parsed, Line::Progress(_) | Line::PostProcess { .. } | Line::File(_)) {
            use std::io::Write;
            if let Some(f) = logf.as_mut() {
                let _ = writeln!(f, "{} {clean}", offset(run_started));
            }
        }
        match parsed {
            Line::Progress(p) => {
                if last_emit.elapsed() < Duration::from_millis(200) && p.status != "finished" {
                    continue;
                }
                last_emit = Instant::now();
                st.update(&id, |j| {
                    let live_now = j.is_live && p.percent().is_none();
                    j.status = if live_now { Status::Live } else { Status::Downloading };
                    set_phase(j, Some(if live_now { "record" } else { "download" }));
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
                        set_phase(j, Some("convert"));
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
                current_item = Some(index);
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
                    set_phase(j, Some("wait"));
                    push_log(j, &clean);
                });
            }
            Line::Error(msg) => {
                // An "ERROR:" line with nothing after it: the reason is on the next line.
                if msg.is_empty() {
                    want_detail = true;
                } else {
                    item_errors.push((current_item, msg.clone()));
                    last_error = Some(msg);
                }
                st.update(&id, |j| push_log(j, &clean));
            }
            Line::Warning(_) | Line::Other(_) => {
                if want_detail && !clean.is_empty() {
                    want_detail = false;
                    item_errors.push((current_item, clean.clone()));
                    last_error = Some(clean.clone());
                }
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
    {
        use std::io::Write;
        if let Some(f) = logf.as_mut() {
            let how = match &exit {
                Ok(s) => s.to_string(),
                Err(e) => format!("could not be waited for: {e}"),
            };
            let _ = writeln!(f, "==== run ended after {}: {how}; {} error line(s) ====", offset(run_started).trim_start_matches('+'), item_errors.len());
        }
    }
    let _ = std::fs::remove_file(&skip_file);
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
        let output_bytes = media_bytes(&st, &id);
        let (transcripts, transcript_error, transcript_stats) = match (&job.options.transcribe, transcript_setup_error) {
            (Some(t), None) => transcribe_files(&st, &id, t).await,
            (Some(_), Some(e)) => (vec![], Some(format!("Transcript skipped: {e}")), None),
            (None, _) => (vec![], None, None),
        };
        // Stopping during the transcript keeps the download, drops the transcript.
        let stopped_while_transcribing = lock(&st.stopping).remove(&id);
        st.update(&id, |j| {
            j.status = if stopped_while_transcribing { Status::Cancelled } else { Status::Done };
            j.percent = Some(100.0);
            j.speed = None;
            j.eta = None;
            j.message = stopped_while_transcribing.then(|| "Stopped".to_string());
            j.transcripts = transcripts;
            j.transcript_error = if stopped_while_transcribing { None } else { transcript_error };
            j.output_bytes = output_bytes;
            j.transcript_stats = transcript_stats;
            j.finished_at = Some(store::now());
        });
    } else {
        let raw = last_error.filter(|e| !e.trim().is_empty()).unwrap_or_else(|| match &exit {
            Ok(s) => format!("yt-dlp stopped with an error ({s}) but did not say why. The full log is in Details."),
            Err(e) => e.to_string(),
        });
        // The site said "slow down": hold the whole queue back and try this one again later,
        // instead of burning through the rest of a big batch.
        if errors::is_rate_limited(&raw) && st.get(&id).is_some_and(|j| j.rate_retries < MAX_RATE_RETRIES) {
            st.pause_queue(RATE_PAUSE_SECS);
            st.update(&id, |j| {
                j.rate_retries += 1;
                j.status = Status::Queued;
                j.error = None;
                j.hint = None;
                j.percent = None;
                j.speed = None;
                j.eta = None;
                j.message = Some("The site is limiting requests — will try again after a pause".into());
            });
            if st.settings().notifications {
                st.notify("Downloads paused", "YouTube is limiting requests. Resuming in 15 minutes.");
            }
            st.save_jobs();
            st.pump();
            return;
        }
        // The file was already on disk (an earlier run, with its cover art embedded) and
        // re-tagging it failed. Download it afresh once instead of giving up.
        let one_video = st.get(&id).is_some_and(|j| j.playlist.is_none_or(|(_, n)| n <= 1));
        let reuse_failed = one_video && errors::is_retag_failure(&raw, st.get(&id).map(|j| j.log).unwrap_or_default().iter().map(String::as_str));
        if reuse_failed && st.get(&id).is_some_and(|j| !j.overwrite_retry) {
            st.update(&id, |j| {
                j.overwrite_retry = true;
                j.status = Status::Queued;
                j.error = None;
                j.hint = None;
                j.percent = None;
                j.speed = None;
                j.eta = None;
                j.message = Some("The earlier file couldn't be updated — downloading it again".into());
            });
            st.save_jobs();
            st.pump();
            return;
        }
        let explained = errors::explain(&raw);
        let mut error = explained.as_ref().map(|x| x.message.clone()).unwrap_or_else(|| raw.clone());
        let mut hint = explained.map(|x| x.hint);
        // A playlist where some videos failed: say which, and that the rest were saved.
        let total = st.get(&id).and_then(|j| j.playlist).map(|(_, n)| n).unwrap_or(0);
        let failed: Vec<u32> = {
            let mut v: Vec<u32> = item_errors.iter().filter_map(|(i, _)| *i).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        if total > 1 && !failed.is_empty() {
            let first = failed[0];
            error = format!("{} of {total} videos failed — item {first}: {error}", failed.len());
            let saved_note = format!("The other {} were saved. Press Continue: finished videos are skipped and an unfinished download resumes.", total as usize - failed.len().min(total as usize));
            hint = Some(match hint {
                Some(h) => format!("{h} {saved_note}"),
                None => saved_note,
            });
        }
        st.update(&id, |j| {
            j.status = Status::Failed;
            j.error = Some(error);
            j.hint = hint;
            j.speed = None;
            j.eta = None;
            j.message = None;
            j.finished_at = Some(store::now());
        });
    }
    finish(&st, &id);
}

/// A job for a file that was already on disk: set up the engine if needed, then transcribe.
async fn run_local(st: &Shared, id: &str, job: &Job) {
    let Some(opts) = job.options.transcribe.clone() else {
        fail_job(st, id, "No transcript settings were given.", None);
        return;
    };
    let file = job.options.url.clone();
    if !Path::new(&file).is_file() {
        fail_job(st, id, "The file can't be found any more.", Some("It may have been moved or deleted. Add it again from its new place."));
        return;
    }
    st.update(id, |j| {
        j.files = vec![file.clone()];
        j.message = Some("Preparing transcription…".into());
        set_phase(j, Some("setup"));
    });
    let (made, error, stats) = if transcribe::is_ready(&st.paths, &opts) {
        transcribe_files(st, id, &opts).await
    } else {
        st.update(id, |j| j.message = Some("Setting up transcription (first time only)…".into()));
        match transcribe::ensure_ready(st.host.as_ref(), &st.paths, &opts).await {
            Ok(()) => transcribe_files(st, id, &opts).await,
            Err(e) => (vec![], Some(e), None),
        }
    };
    let stopped = lock(&st.stopping).remove(id);
    let bytes = media_bytes(st, id);
    if stopped {
        st.update(id, |j| {
            j.status = Status::Cancelled;
            j.message = Some("Stopped".into());
            j.finished_at = Some(store::now());
        });
    } else if made.is_empty() {
        let raw = error.unwrap_or_else(|| "No transcript was written.".into());
        st.update(id, |j| {
            j.status = Status::Failed;
            j.error = Some(raw);
            j.hint = Some("Open Details to see the reason. Retry once it is fixed.".into());
            j.message = None;
            j.finished_at = Some(store::now());
        });
    } else {
        st.update(id, |j| {
            j.status = Status::Done;
            j.percent = Some(100.0);
            j.message = None;
            j.transcripts = made;
            j.transcript_error = error;
            j.output_bytes = bytes;
            j.transcript_stats = stats;
            j.finished_at = Some(store::now());
        });
    }
    finish(st, id);
}

/// Transcribe the media files a job produced, reporting progress on the job.
/// Returns the transcript files and, if something went wrong, a message.
async fn transcribe_files(st: &Shared, id: &str, opts: &TranscribeOptions) -> (Vec<String>, Option<String>, Option<TranscriptStats>) {
    let job = st.get(id);
    let keep_finished = job.as_ref().is_some_and(|j| j.resumed && !j.local_file);
    let files: Vec<String> = job.map(|j| j.files).unwrap_or_default().into_iter().filter(|f| transcribe::is_media(f)).collect();
    let n = files.len();
    let metas = transcribe::read_metas(&st.meta_path(id));
    let mut done = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    let mut stats: Option<TranscriptStats> = None;
    for (i, file) in files.iter().enumerate() {
        if lock(&st.stopping).contains(id) {
            break;
        }
        if keep_finished {
            if let Some(mut have) = transcribe::finished_outputs(Path::new(file), &opts.formats) {
                done.append(&mut have);
                continue;
            }
        }
        let label = if n > 1 { format!("Transcribing {}/{n}", i + 1) } else { "Transcribing".to_string() };
        let speakers_label = if n > 1 { format!("Identifying speakers {}/{n}", i + 1) } else { "Identifying speakers".to_string() };
        st.update(id, |j| {
            j.status = Status::Processing;
            j.percent = Some(0.0);
            j.speed = None;
            j.eta = None;
            j.message = Some(format!("{label}… 0%"));
            set_phase(j, Some("transcribe"));
        });
        let identifying = std::sync::atomic::AtomicBool::new(false);
        let on_progress = |p: u32| {
            st.update(id, |j| {
                j.percent = Some(p as f64);
                j.message = Some(if identifying.load(std::sync::atomic::Ordering::Relaxed) { format!("{speakers_label}… {p}%") } else { format!("{label}… {p}%") });
            });
        };
        let on_phase = |phase: &str| {
            if phase == "diarize" {
                identifying.store(true, std::sync::atomic::Ordering::Relaxed);
                st.update(id, |j| {
                    j.percent = Some(0.0);
                    j.message = Some(format!("{speakers_label}… 0%"));
                    set_phase(j, Some("diarize"));
                });
            }
        };
        let on_pid = |pid: Option<u32>| match pid {
            Some(p) => {
                lock(&st.pids).insert(id.to_string(), p);
            }
            None => {
                lock(&st.pids).remove(id);
            }
        };
        let about = transcribe::about_file(Path::new(file), &metas);
        match transcribe::transcribe_file(&st.paths, opts, Path::new(file), &about, on_progress, on_phase, on_pid).await {
            Ok(mut out) => {
                done.append(&mut out.files);
                let t = stats.get_or_insert_with(|| TranscriptStats { model: opts.model.clone(), language: out.language.clone(), ..Default::default() });
                t.audio_secs += out.audio_secs;
                t.words += out.words;
                t.speakers = t.speakers.max(out.speakers);
            }
            Err(e) => {
                // One file that cannot be transcribed must not cost the others their transcripts.
                let name = title_from_path(file).unwrap_or_else(|| file.clone());
                st.update(id, |j| push_log(j, &format!("Transcript failed for {name}: {e}")));
                failed.push((name, e));
            }
        }
    }
    let error = match failed.as_slice() {
        [] => None,
        [(_, e)] if n == 1 => Some(format!("Transcript failed: {e}")),
        [(name, e), ..] => Some(format!(
            "{} of {n} transcripts failed — {name}: {e}. The others were saved; press Continue to try the failed ones again.",
            failed.len()
        )),
    };
    (done, error, stats)
}

/// Combined size of the media files a job produced.
fn media_bytes(st: &Shared, id: &str) -> u64 {
    st.get(id)
        .map(|j| j.files.iter().filter(|f| transcribe::is_media(f)).filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum())
        .unwrap_or(0)
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
    async fn phase_times_add_up_and_stop_when_the_job_ends() {
        let dir = temp_dir("phases");
        let st = state_in(&dir);
        lock(&st.settings).max_concurrent = 0; // keep the queue from starting it
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/v".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        st.update(&job.id, |j| {
            j.status = Status::Downloading;
            set_phase(j, Some("download"));
        });
        tokio::time::sleep(Duration::from_millis(120)).await;
        st.update(&job.id, |j| set_phase(j, Some("download"))); // same phase: no reset
        st.update(&job.id, |j| set_phase(j, Some("convert")));
        tokio::time::sleep(Duration::from_millis(60)).await;
        let mid = st.get(&job.id).unwrap();
        assert!(mid.timings["download"] >= 0.11 && mid.phase.as_deref() == Some("convert") && mid.phase_started.is_some());
        let done = st.update(&job.id, |j| j.status = Status::Done).unwrap();
        assert_eq!(done.phase, None);
        assert!(done.timings["convert"] >= 0.05 && done.timings["download"] < 0.5, "{:?}", done.timings);
    }

    /// "One at a time" jobs run alone and in order; others queue behind them.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn one_at_a_time_jobs_do_not_overlap() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("serial");
        let st = state_in(&dir);
        lock(&st.settings).max_concurrent = 4;
        let fake = st.paths.bin_dir.join("yt-dlp");
        std::fs::write(&fake, "#!/bin/sh\nsleep 1\nexit 0\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mk = |alone: bool| AddJob { options: JobOptions { url: "https://x.test/v".into(), one_at_a_time: alone, ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None };
        let (a, b, c) = (st.add(mk(true)), st.add(mk(true)), st.add(mk(false)));
        tokio::time::sleep(Duration::from_millis(400)).await;
        let state = |id: &str| st.get(id).unwrap().status;
        assert!(state(&a.id).is_running(), "first starts: {:?}", state(&a.id));
        assert_eq!(state(&b.id), Status::Queued, "second waits for the first");
        assert_eq!(state(&c.id), Status::Queued, "a normal job does not slip past a waiting one-at-a-time job");
        let (da, db, dc) = (
            wait_for(&st, &a.id, |j| j.status.is_finished(), 15).await,
            wait_for(&st, &b.id, |j| j.status.is_finished(), 15).await,
            wait_for(&st, &c.id, |j| j.status.is_finished(), 15).await,
        );
        assert!(da.finished_at <= db.started_at.map(|s| s + 1) && db.finished_at <= dc.started_at.map(|s| s + 1), "ran in order, not overlapping");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_paused_queue_holds_jobs_until_resumed() {
        let dir = temp_dir("paused");
        let st = state_in(&dir);
        st.pause_queue(600);
        assert!(st.paused_until().is_some());
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/v".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(st.get(&job.id).unwrap().status, Status::Queued, "must not start while paused");
        st.resume_queue();
        assert!(st.paused_until().is_none());
        // Without yt-dlp installed it now starts and fails: proof it left the queue.
        wait_for(&st, &job.id, |j| j.status != Status::Queued, 10).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failures_are_explained() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("fail");
        let st = state_in(&dir);
        let fake = st.paths.bin_dir.join("yt-dlp");
        std::fs::write(&fake, "#!/bin/sh\necho \"ERROR: [youtube] abc: Private video. Sign in if you've been granted access\" >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/v".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        assert_eq!(done.status, Status::Failed);
        assert!(done.error.unwrap().contains("private"));
        assert!(st.paused_until().is_none(), "an ordinary failure must not pause the queue");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "Not a bot" / 429: pause the queue and put the job back, twice, then give up with the explanation.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn rate_limits_pause_the_queue_and_retry_the_job() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("ratelimit");
        let st = state_in(&dir);
        let fake = st.paths.bin_dir.join("yt-dlp");
        std::fs::write(&fake, "#!/bin/sh\necho \"ERROR: [youtube] abc: Sign in to confirm you're not a bot\" >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/v".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        for attempt in 1..=MAX_RATE_RETRIES {
            let j = wait_for(&st, &job.id, |j| j.rate_retries == attempt && j.status == Status::Queued, 10).await;
            assert!(j.message.unwrap().contains("limiting"));
            assert!(st.paused_until().is_some(), "queue should be paused after attempt {attempt}");
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert_eq!(st.get(&job.id).unwrap().status, Status::Queued, "must stay queued while paused");
            st.resume_queue();
        }
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        assert_eq!(done.status, Status::Failed);
        assert!(done.error.unwrap().contains("not a bot") && done.hint.unwrap().contains("cookies"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file left by an earlier run can't be re-tagged: the job downloads it again, once.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_file_that_cannot_be_retagged_is_downloaded_again() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("retag");
        let st = state_in(&dir);
        let fake = st.paths.bin_dir.join("yt-dlp");
        let script = r#"#!/bin/sh
case " $* " in
  *" --force-overwrites "*) echo "[download] 100% of 1MiB"; exit 0 ;;
esac
echo "[download] /x/a.opus has already been downloaded"
echo "ERROR: Postprocessing: Conversion failed!" >&2
exit 1
"#;
        std::fs::write(&fake, script).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/v".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        assert_eq!(done.status, Status::Done, "error: {:?}", done.error);
        assert!(done.overwrite_retry);

        // Without the "already downloaded" line a conversion failure is final, with a clear message.
        std::fs::write(&fake, "#!/bin/sh\necho \"ERROR: Postprocessing: Conversion failed!\" >&2\nexit 1\n").unwrap();
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/w".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        let failed = wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        assert_eq!(failed.status, Status::Failed);
        assert!(!failed.overwrite_retry);
        assert!(failed.error.unwrap().contains("ffmpeg couldn't"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One failing video in a playlist is named, with its reason, and the rest are reported as saved.
    /// An "ERROR:" line with nothing after it takes its reason from the next line. The full log is kept.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_playlist_item_is_named_and_the_log_is_kept() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("itemfail");
        let st = state_in(&dir);
        let fake = st.paths.bin_dir.join("yt-dlp");
        let script = "#!/bin/sh\necho '[download] Downloading item 1 of 3'\nsleep 0.3\necho '[download] Downloading item 2 of 3'\nsleep 0.3\necho 'ERROR:' >&2\nsleep 0.3\necho 'unable to download video data: connection reset by peer' >&2\nsleep 0.3\necho '[download] Downloading item 3 of 3'\nsleep 0.3\necho '[download] Finished downloading playlist: X'\nexit 1\n";
        std::fs::write(&fake, script).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/list".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        assert_eq!(done.status, Status::Failed);
        let err = done.error.unwrap();
        assert!(err.contains("1 of 3 videos failed") && err.contains("item 2") && err.contains("connection reset"), "{err}");
        assert!(done.hint.unwrap().contains("other 2 were saved"));
        // The whole log is on disk and is what Details shows; removing the job removes it.
        let on_disk = std::fs::read_to_string(st.log_path(&job.id)).unwrap();
        assert!(on_disk.contains("Downloading item 3 of 3") && on_disk.contains("connection reset"), "{on_disk}");
        assert!(st.log_of(&job.id).iter().any(|l| l.contains("connection reset")));
        st.remove(&job.id);
        assert!(!st.log_path(&job.id).exists());

        // No reason given at all: still a sentence, never an empty box.
        std::fs::write(&fake, "#!/bin/sh\necho 'ERROR:' >&2\nexit 1\n").unwrap();
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/v".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        assert!(done.error.unwrap().contains("did not say why"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A download that goes silent is flagged in the job and in the log, and the log records how the run ended.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_silent_download_is_flagged_as_stalled() {
        use std::os::unix::fs::PermissionsExt;
        std::env::set_var("YTGRAB_STALL_SECS", "1");
        let dir = temp_dir("stall");
        let st = state_in(&dir);
        let fake = st.paths.bin_dir.join("yt-dlp");
        // One progress line, then silence for 3 s, then it finishes.
        let line = "echo 'PRG|downloading|1000|9000|NA|100|80|NA|NA|1'";
        std::fs::write(&fake, format!("#!/bin/sh\n{line}\nsleep 3\nexit 0\n")).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/v".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        let mut flagged = false;
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            let j = st.get(&job.id).unwrap();
            flagged |= j.message.as_deref().is_some_and(|m| m.contains("stalled"));
            if j.status.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        std::env::remove_var("YTGRAB_STALL_SECS");
        let log = std::fs::read_to_string(st.log_path(&job.id)).unwrap();
        assert!(flagged || log.contains("may have stalled"), "stall should be flagged:\n{log}");
        assert!(log.contains("may have stalled") && log.contains("==== run ended after"), "{log}");
        assert!(log.contains("+00:"), "lines carry a time offset: {log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Continuing a partly finished playlist keeps its time stats; a plain retry starts them over.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn continuing_a_playlist_keeps_its_stats() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("continue");
        let st = state_in(&dir);
        let fake = st.paths.bin_dir.join("yt-dlp");
        std::fs::write(&fake, "#!/bin/sh\necho '[download] Downloading item 1 of 2'\nsleep 0.3\necho 'ERROR: boom' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let job = st.add(AddJob { options: JobOptions { url: "https://x.test/list".into(), ..Default::default() }, title: None, thumbnail: None, is_live: false, source: None });
        let first = wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        let spent = first.timings.get("prepare").copied().unwrap_or(0.0);
        assert!(spent > 0.0);
        std::fs::write(&fake, "#!/bin/sh\nsleep 0.3\nexit 0\n").unwrap();
        st.retry(&job.id);
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        assert_eq!(done.status, Status::Done);
        assert!(done.timings.get("prepare").copied().unwrap_or(0.0) > spent, "earlier time is kept and added to");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Videos already saved are handed to yt-dlp as an archive, so it never re-tags them; other formats are not.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn videos_already_saved_are_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("skipsaved");
        let st = state_in(&dir);
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("Have it [AAAAAAAAAAA].opus"), "x").unwrap();
        std::fs::write(out.join("Other format [BBBBBBBBBBB].mp3"), "x").unwrap();
        let seen = dir.join("seen.txt");
        let fake = st.paths.bin_dir.join("yt-dlp");
        // Copy the archive file yt-dlp is given (argument 2 after --download-archive) somewhere we can read it.
        let script = format!("#!/bin/sh\n[ \"$1\" = \"--download-archive\" ] && cp \"$2\" {} \nexit 0\n", seen.display());
        std::fs::write(&fake, script).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let opts = JobOptions {
            url: "https://www.youtube.com/@x/videos".into(),
            output_dir: out.to_string_lossy().into_owned(),
            audio_only: true,
            audio_format: Some("opus".into()),
            ..Default::default()
        };
        let job = st.add(AddJob { options: opts, title: None, thumbnail: None, is_live: false, source: None });
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 10).await;
        assert_eq!(done.status, Status::Done, "error: {:?}", done.error);
        assert_eq!(std::fs::read_to_string(&seen).unwrap(), "youtube AAAAAAAAAAA\n");
        assert!(st.log_of(&job.id).iter().any(|l| l.contains("1 video(s) already saved")));
        assert!(!st.paths.data_dir.join("logs").join(format!("skip-{}.txt", job.id)).exists(), "temporary list is removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file already on disk gets a transcript without yt-dlp. A fake engine stands in for Whisper.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_local_file_is_transcribed_through_the_queue() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("localfile");
        let st = state_in(&dir);
        let opts = TranscribeOptions { model: "tiny".into(), ..Default::default() };

        // Things that cannot be transcribed are refused at once, with a reason.
        assert!(st.add_local("/definitely/not/here.mp3", opts.clone()).unwrap_err().contains("isn't a file"));
        let notes = dir.join("notes.txt");
        std::fs::write(&notes, "hi").unwrap();
        assert!(st.add_local(notes.to_str().unwrap(), opts.clone()).unwrap_err().contains("audio or video"));

        // A fake engine: marker, python, and a model file are all it takes to count as installed.
        let marker = transcribe::marker_path(&st.paths);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, "faster-whisper\n").unwrap();
        let py = transcribe::python_path(&st.paths);
        std::fs::create_dir_all(py.parent().unwrap()).unwrap();
        // args: script transcribe MODEL_DIR INPUT OUT_BASE …  -> write OUT_BASE.txt and report it
        std::fs::write(&py, "#!/bin/sh\necho LANG\\|en; echo DURATION\\|60.0; echo PROGRESS\\|50\nprintf 'hello' > \"$5.txt\"\necho WORDS\\|1; echo DONE\\|$5.txt\n").unwrap();
        std::fs::set_permissions(&py, std::fs::Permissions::from_mode(0o755)).unwrap();
        let model = transcribe::model_dir(&st.paths, "tiny");
        std::fs::create_dir_all(&model).unwrap();
        std::fs::write(model.join("model.bin"), "x").unwrap();

        let talk = dir.join("talk.mp3");
        std::fs::write(&talk, "not really audio").unwrap();
        let job = st.add_local(talk.to_str().unwrap(), opts).unwrap();
        assert!(job.local_file && job.title.as_deref() == Some("talk") && job.files == vec![talk.to_string_lossy().to_string()]);
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 15).await;
        assert_eq!(done.status, Status::Done, "error: {:?}", done.error);
        assert_eq!(done.transcripts, vec![dir.join("talk.txt").to_string_lossy().to_string()]);
        assert_eq!(std::fs::read_to_string(dir.join("talk.txt")).unwrap(), "hello");
        assert_eq!(done.output_bytes, 16);

        // Retry runs it again; a vanished file fails with a clear message instead of crashing.
        std::fs::remove_file(&talk).unwrap();
        st.retry(&job.id);
        let failed = wait_for(&st, &job.id, |j| j.status.is_finished(), 15).await;
        assert_eq!(failed.status, Status::Failed);
        assert!(failed.error.unwrap().contains("can't be found"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file whose Whisper run is killed (out of memory) must not stop the files after it.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn one_crashed_transcript_does_not_stop_the_rest() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("crashmid");
        let st = state_in(&dir);
        let opts = TranscribeOptions { model: "tiny".into(), ..Default::default() };
        let marker = transcribe::marker_path(&st.paths);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, "faster-whisper\n").unwrap();
        let py = transcribe::python_path(&st.paths);
        std::fs::create_dir_all(py.parent().unwrap()).unwrap();
        // args: script transcribe MODEL_DIR INPUT OUT_BASE …  ($4 is the input)
        std::fs::write(&py, "#!/bin/sh\necho \"$4\" >> \"$0.calls\"\ncase \"$4\" in *big*) kill -9 $$;; esac\necho PROGRESS\\|50\nprintf 'hi' > \"$5.txt\"\necho DONE\\|$5.txt\n").unwrap();
        std::fs::set_permissions(&py, std::fs::Permissions::from_mode(0o755)).unwrap();
        let model = transcribe::model_dir(&st.paths, "tiny");
        std::fs::create_dir_all(&model).unwrap();
        std::fs::write(model.join("model.bin"), "x").unwrap();

        let names = ["a.mp3", "big [x].mp3", "c.mp3"];
        let paths: Vec<String> = names.iter().map(|n| dir.join(n).to_string_lossy().to_string()).collect();
        for p in &paths {
            std::fs::write(p, "x").unwrap();
        }
        let job = st.add_local(&paths[0], opts.clone()).unwrap();
        wait_for(&st, &job.id, |j| j.status.is_finished(), 15).await;
        st.update(&job.id, |j| j.files = paths.clone());
        let (made, error, _) = transcribe_files(&st, &job.id, &opts).await;
        assert_eq!(made.len(), 2, "a and c are transcribed: {made:?}");
        let error = error.expect("the failure is reported");
        assert!(error.contains("1 of 3") && error.contains("big") && error.contains("out of memory") && error.contains("Continue"), "{error}");
        assert!(st.get(&job.id).unwrap().log.iter().any(|l| l.contains("Transcript failed for big")));

        // Continuing: the two finished transcripts are kept, only the failed file is tried again.
        let calls = || std::fs::read_to_string(format!("{}.calls", py.display())).unwrap().lines().count();
        assert_eq!(calls(), 4); // one from the job itself, three from the pass
        st.update(&job.id, |j| { j.resumed = true; j.local_file = false; });
        let (made, error, _) = transcribe_files(&st, &job.id, &opts).await;
        assert_eq!(calls(), 5, "only the failed file was run again");
        assert_eq!(made.len(), 2);
        assert!(error.is_some());
        // Asking for transcripts on purpose redoes everything.
        st.update(&job.id, |j| j.resumed = false);
        transcribe_files(&st, &job.id, &opts).await;
        assert_eq!(calls(), 8);
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

    /// Runs a real, already-installed Whisper engine on a file through the queue.
    ///   YTGRAB_TEST_DATA=~/.local/share/app.ytgrab.desktop YTGRAB_TEST_AUDIO=clip.opus YTGRAB_TEST_MODEL=small \
    ///     cargo test e2e_local_file -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn e2e_local_file_with_the_real_engine() {
        struct NullHost;
        impl Host for NullHost {
            fn emit(&self, _: &str, _: serde_json::Value) {}
            fn notify(&self, _: &str, _: &str) {}
            fn exit(&self) {}
        }
        let dir = temp_dir("e2elocal");
        let data = PathBuf::from(std::env::var("YTGRAB_TEST_DATA").expect("set YTGRAB_TEST_DATA"));
        let audio = PathBuf::from(std::env::var("YTGRAB_TEST_AUDIO").expect("set YTGRAB_TEST_AUDIO"));
        let model = std::env::var("YTGRAB_TEST_MODEL").unwrap_or_else(|_| "small".into());
        let st = AppState::new(Arc::new(NullHost), Paths::installed_at(data, dir.join("dl")));
        let media = dir.join("clip.opus");
        std::fs::copy(&audio, &media).unwrap();
        let job = st.add_local(media.to_str().unwrap(), TranscribeOptions { model, ..Default::default() }).unwrap();
        let done = wait_for(&st, &job.id, |j| j.status.is_finished(), 600).await;
        println!("{:?} {:?} stats={:?} timings={:?}", done.status, done.error, done.transcript_stats, done.timings);
        assert_eq!(done.status, Status::Done);
        let txt = std::fs::read_to_string(dir.join("clip.txt")).unwrap();
        println!("{}", txt.lines().take(8).collect::<Vec<_>>().join("\n"));
        assert!(txt.contains("[00:00:"));
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
