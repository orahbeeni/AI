//! Local speech-to-text with Whisper (faster-whisper).
//!
//! Like yt-dlp and ffmpeg, the engine is downloaded on first use into the
//! app's own data folder, so it needs nothing installed on the computer and
//! travels with a portable install. It is a small private Python environment:
//! `uv` (one static binary) fetches its own Python and installs faster-whisper.
//! Models are downloaded on demand, one folder each. Everything runs offline
//! once installed.

use crate::args::TranscribeOptions;
use crate::binaries::{self, emit, ProgressEvent};
use crate::jobs::Host;
use crate::paths::Paths;
use crate::platform::{self, Os};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Pinned: unpinned installs pull the newest PyAV, which faster-whisper
/// does not support yet.
const PACKAGES: [&str; 3] = ["faster-whisper==1.2.1", "av==15.1.0", "sherpa-onnx==2.0.1"];
/// Added to engines installed before speaker labels existed.
const SPEAKER_PACKAGE: &str = "sherpa-onnx==2.0.1";
/// Size of the two speaker models, for the progress bar.
const SPEAKER_MODELS_MB: u64 = 36;
const PYTHON_VERSION: &str = "3.12";
const SCRIPT: &str = include_str!("whisper_run.py");
const MEDIA_EXTENSIONS: [&str; 13] =
    ["mp4", "mkv", "webm", "mov", "avi", "ts", "mp3", "m4a", "opus", "ogg", "flac", "wav", "aac"];

/// One engine install or model download at a time.
static SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub struct ModelSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub size_mb: u64,
    pub hint: &'static str,
}

pub const MODELS: [ModelSpec; 6] = [
    ModelSpec { name: "tiny", label: "Tiny", size_mb: 75, hint: "Fastest. Rough accuracy — fine for a quick look." },
    ModelSpec { name: "base", label: "Base", size_mb: 145, hint: "Fast. Misses words with accents or background noise." },
    ModelSpec { name: "small", label: "Small", size_mb: 485, hint: "Good balance for clear speech and conversations." },
    ModelSpec { name: "medium", label: "Medium", size_mb: 1500, hint: "Noticeably more accurate. About 2.5 times slower than Small." },
    ModelSpec { name: "turbo", label: "Turbo (large-v3)", size_mb: 1620, hint: "Near-best accuracy. On a CPU it is about as slow as Medium." },
    ModelSpec { name: "large-v3", label: "Large v3", size_mb: 3100, hint: "Most accurate, but about 5 times slower than Small without a GPU." },
];

pub fn model_spec(name: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.name == name)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub name: &'static str,
    pub label: &'static str,
    pub size_mb: u64,
    pub hint: &'static str,
    pub installed: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhisperStatus {
    pub engine_installed: bool,
    pub speakers_installed: bool,
    pub speakers_mb: u64,
    pub dir: String,
    pub models: Vec<ModelStatus>,
}

// ---- locations --------------------------------------------------------

fn engine_dir(paths: &Paths) -> PathBuf {
    paths.data_dir.join("whisper")
}
fn uv_path(paths: &Paths) -> PathBuf {
    engine_dir(paths).join(platform::exe_name("uv"))
}
fn venv_dir(paths: &Paths) -> PathBuf {
    engine_dir(paths).join("venv")
}
pub(crate) fn python_path(paths: &Paths) -> PathBuf {
    platform::venv_python(&venv_dir(paths))
}
pub(crate) fn marker_path(paths: &Paths) -> PathBuf {
    engine_dir(paths).join("engine.ok")
}
pub(crate) fn model_dir(paths: &Paths, name: &str) -> PathBuf {
    engine_dir(paths).join("models").join(name)
}
fn speaker_dir(paths: &Paths) -> PathBuf {
    engine_dir(paths).join("speakers")
}
fn script_path(paths: &Paths) -> PathBuf {
    engine_dir(paths).join("whisper_run.py")
}

pub fn engine_installed(paths: &Paths) -> bool {
    marker_path(paths).is_file() && python_path(paths).is_file()
}

pub fn model_installed(paths: &Paths, name: &str) -> bool {
    model_dir(paths, name).join("model.bin").is_file()
}

pub fn speakers_installed(paths: &Paths) -> bool {
    let d = speaker_dir(paths);
    d.join("segmentation.onnx").is_file() && d.join("embedding.onnx").is_file()
}

/// Does the installed engine already have the package that identifies speakers?
fn engine_has_speaker_package(paths: &Paths) -> bool {
    std::fs::read_to_string(marker_path(paths)).is_ok_and(|s| s.lines().any(|l| l.starts_with("sherpa-onnx")))
}

pub fn status(paths: &Paths) -> WhisperStatus {
    WhisperStatus {
        engine_installed: engine_installed(paths),
        speakers_installed: speakers_installed(paths) && engine_has_speaker_package(paths),
        speakers_mb: SPEAKER_MODELS_MB,
        dir: engine_dir(paths).to_string_lossy().into_owned(),
        models: MODELS
            .iter()
            .map(|m| ModelStatus {
                name: m.name,
                label: m.label,
                size_mb: m.size_mb,
                hint: m.hint,
                installed: model_installed(paths, m.name),
            })
            .collect(),
    }
}

/// Is this a file Whisper can read (as opposed to a thumbnail or subtitle)?
pub fn is_media(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| MEDIA_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

// ---- installing ---------------------------------------------------------

fn stage(host: &dyn Host, name: &str, stage: &str) {
    emit(host, ProgressEvent { name, stage, downloaded: 0, total: None });
}

/// Run a command to completion, keeping the tail of its output for error messages.
async fn run_checked(mut cmd: Command, what: &str) -> Result<(), String> {
    platform::configure_command(&mut cmd);
    let out = cmd.output().await.map_err(|e| format!("could not run {what}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let text = format!("{}{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
    let tail: Vec<&str> = text.lines().rev().filter(|l| !l.trim().is_empty()).take(6).collect();
    Err(format!("{what} failed: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" | ")))
}

async fn ensure_uv(host: &dyn Host, paths: &Paths) -> Result<(), String> {
    let dest = uv_path(paths);
    if dest.is_file() {
        return Ok(());
    }
    let (os, arch) = (platform::os(), platform::arch());
    let asset = platform::uv_asset(os, arch);
    let base = "https://github.com/astral-sh/uv/releases/latest/download";
    let tmp = engine_dir(paths).join("uv.download");
    binaries::download_to(host, "whisper", &format!("{base}/{asset}"), &tmp).await?;
    stage(host, "whisper", "verifying");
    let sum = binaries::client()?
        .get(format!("{base}/{asset}.sha256"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("could not fetch the checksum for uv: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let want = binaries::checksum_for(&sum, asset).or_else(|| sum.split_whitespace().next().map(str::to_lowercase));
    let got = binaries::sha256_file(&tmp).map_err(|e| e.to_string())?;
    if want.as_deref() != Some(got.as_str()) {
        let _ = std::fs::remove_file(&tmp);
        return Err("uv failed its integrity check; not installed".into());
    }
    stage(host, "whisper", "unpacking");
    let out = tmp.with_extension("bin");
    let (tmp2, out2, exe) = (tmp.clone(), out.clone(), platform::exe_name("uv"));
    let unpacked = tokio::task::spawn_blocking(move || extract_one(&tmp2, &out2, &exe, os)).await.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);
    unpacked.map_err(|e| format!("could not unpack uv: {e}"))?;
    binaries::replace_file(&out, &dest)?;
    platform::prepare_binary(&dest).map_err(|e| e.to_string())
}

/// Pull a single executable out of a .zip (Windows) or .tar.gz (macOS, Linux).
fn extract_one(archive: &Path, out: &Path, exe: &str, os: Os) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    if os == Os::Windows {
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut f = zip.by_name(exe).map_err(|e| e.to_string())?;
        let mut dst = std::fs::File::create(out).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut dst).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if entry.path().map_err(|e| e.to_string())?.file_name().is_some_and(|n| n == exe) {
            let mut dst = std::fs::File::create(out).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut dst).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err(format!("{exe} not found in the archive"))
}

fn uv_command(paths: &Paths) -> Command {
    let dir = engine_dir(paths);
    let mut cmd = Command::new(uv_path(paths));
    cmd.env("UV_CACHE_DIR", dir.join("cache"))
        // Don't keep a second copy of every downloaded package around.
        .env("UV_NO_CACHE", "1")
        .env("UV_PYTHON_INSTALL_DIR", dir.join("python"))
        // Never pick up a system Python: its version may have no wheels yet.
        .env("UV_PYTHON_PREFERENCE", "only-managed")
        .env("UV_NO_CONFIG", "1")
        .stdin(std::process::Stdio::null());
    cmd
}

async fn install_engine_locked(host: &dyn Host, paths: &Paths) -> Result<(), String> {
    std::fs::create_dir_all(engine_dir(paths)).map_err(|e| e.to_string())?;
    stage(host, "whisper", "downloading");
    ensure_uv(host, paths).await?;
    write_script(paths)?;

    stage(host, "whisper", "setting up Python");
    let _ = std::fs::remove_file(marker_path(paths));
    let mut venv = uv_command(paths);
    venv.arg("venv").arg("--python").arg(PYTHON_VERSION).arg("--allow-existing").arg(venv_dir(paths));
    run_checked(venv, "creating the Python environment").await?;

    stage(host, "whisper", "installing packages");
    let mut pip = uv_command(paths);
    pip.args(["pip", "install", "--python"]).arg(python_path(paths)).args(PACKAGES);
    run_checked(pip, "installing the transcription packages").await?;

    std::fs::write(marker_path(paths), PACKAGES.join("\n")).map_err(|e| e.to_string())?;
    stage(host, "whisper", "done");
    Ok(())
}

pub async fn install_engine(host: &dyn Host, paths: &Paths) -> Result<(), String> {
    let _guard = SETUP_LOCK.lock().await;
    install_engine_locked(host, paths).await
}

/// Remove the engine (frees roughly 1 GB). Downloaded models are kept.
pub fn remove_engine(paths: &Paths) -> Result<(), String> {
    let dir = engine_dir(paths);
    for name in ["venv", "python", "cache"] {
        let p = dir.join(name);
        if p.exists() {
            std::fs::remove_dir_all(&p).map_err(|e| format!("could not remove {}: {e}", p.display()))?;
        }
    }
    for p in [uv_path(paths), marker_path(paths)] {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

fn write_script(paths: &Paths) -> Result<(), String> {
    let path = script_path(paths);
    if std::fs::read_to_string(&path).is_ok_and(|s| s == SCRIPT) {
        return Ok(());
    }
    std::fs::create_dir_all(engine_dir(paths)).map_err(|e| e.to_string())?;
    std::fs::write(&path, SCRIPT).map_err(|e| format!("could not write the transcription script: {e}"))
}

fn dir_size(path: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else { return 0 };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

fn python_command(paths: &Paths) -> Command {
    let mut cmd = Command::new(python_path(paths));
    cmd.env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1")
        .env("HF_HOME", engine_dir(paths).join("hf"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd
}

async fn download_model_locked(host: &dyn Host, paths: &Paths, name: &str) -> Result<(), String> {
    let spec = model_spec(name).ok_or_else(|| format!("unknown model “{name}”"))?;
    if !engine_installed(paths) {
        install_engine_locked(host, paths).await?;
    }
    write_script(paths)?;
    let final_dir = model_dir(paths, name);
    let part = final_dir.with_extension("part");
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(part.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;

    let event = format!("model:{name}");
    let total = spec.size_mb * 1024 * 1024;
    let mut cmd = python_command(paths);
    cmd.arg(script_path(paths)).arg("download").arg(name).arg(&part);
    platform::configure_command(&mut cmd);
    let child = cmd.spawn().map_err(|e| format!("could not start the model download: {e}"))?;
    let wait = child.wait_with_output();
    tokio::pin!(wait);
    let out = loop {
        tokio::select! {
            r = &mut wait => break r,
            _ = tokio::time::sleep(std::time::Duration::from_millis(400)) => {
                emit(host, ProgressEvent { name: &event, stage: "downloading", downloaded: dir_size(&part).min(total), total: Some(total) });
            }
        }
    }
    .map_err(|e| e.to_string())?;

    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() || !part.join("model.bin").is_file() {
        let _ = std::fs::remove_dir_all(&part);
        let why = text
            .lines()
            .find_map(|l| l.strip_prefix("ERROR|"))
            .map(str::to_string)
            .unwrap_or_else(|| String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("download failed").to_string());
        return Err(format!("could not download the {} model: {why}", spec.label));
    }
    let _ = std::fs::remove_dir_all(&final_dir);
    std::fs::rename(&part, &final_dir).map_err(|e| format!("could not finish installing the model: {e}"))?;
    emit(host, ProgressEvent { name: &event, stage: "done", downloaded: total, total: Some(total) });
    Ok(())
}

/// Make sure the engine can identify speakers (older installs lack the package) and has its two models.
async fn ensure_speakers_locked(host: &dyn Host, paths: &Paths) -> Result<(), String> {
    if !engine_installed(paths) {
        install_engine_locked(host, paths).await?;
    }
    write_script(paths)?;
    if !engine_has_speaker_package(paths) {
        stage(host, "speakers", "installing packages");
        let mut pip = uv_command(paths);
        pip.args(["pip", "install", "--python"]).arg(python_path(paths)).arg(SPEAKER_PACKAGE);
        run_checked(pip, "installing the speaker package").await?;
        let mut marker = std::fs::read_to_string(marker_path(paths)).unwrap_or_default();
        if !marker.is_empty() && !marker.ends_with('\n') {
            marker.push('\n');
        }
        marker.push_str(SPEAKER_PACKAGE);
        std::fs::write(marker_path(paths), marker).map_err(|e| e.to_string())?;
    }
    if speakers_installed(paths) {
        return Ok(());
    }
    let final_dir = speaker_dir(paths);
    let part = final_dir.with_extension("part");
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| e.to_string())?;
    let total = SPEAKER_MODELS_MB * 1024 * 1024;
    let mut cmd = python_command(paths);
    cmd.arg(script_path(paths)).arg("download-speakers").arg(&part);
    platform::configure_command(&mut cmd);
    let child = cmd.spawn().map_err(|e| format!("could not start the speaker model download: {e}"))?;
    let wait = child.wait_with_output();
    tokio::pin!(wait);
    let out = loop {
        tokio::select! {
            r = &mut wait => break r,
            _ = tokio::time::sleep(std::time::Duration::from_millis(400)) => {
                emit(host, ProgressEvent { name: "speakers", stage: "downloading", downloaded: dir_size(&part).min(total), total: Some(total) });
            }
        }
    }
    .map_err(|e| e.to_string())?;
    if !out.status.success() || !(part.join("segmentation.onnx").is_file() && part.join("embedding.onnx").is_file()) {
        let _ = std::fs::remove_dir_all(&part);
        let why = String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.strip_prefix("ERROR|").map(str::to_string))
            .unwrap_or_else(|| String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("download failed").to_string());
        return Err(format!("could not download the speaker models: {why}"));
    }
    let _ = std::fs::remove_dir_all(&final_dir);
    std::fs::rename(&part, &final_dir).map_err(|e| format!("could not finish installing the speaker models: {e}"))?;
    emit(host, ProgressEvent { name: "speakers", stage: "done", downloaded: total, total: Some(total) });
    Ok(())
}

pub async fn download_speakers(host: &dyn Host, paths: &Paths) -> Result<(), String> {
    let _guard = SETUP_LOCK.lock().await;
    ensure_speakers_locked(host, paths).await
}

pub fn delete_speakers(paths: &Paths) -> Result<(), String> {
    let d = speaker_dir(paths);
    if d.exists() {
        std::fs::remove_dir_all(&d).map_err(|e| format!("could not remove the speaker models: {e}"))?;
    }
    Ok(())
}

pub async fn download_model(host: &dyn Host, paths: &Paths, name: &str) -> Result<(), String> {
    let _guard = SETUP_LOCK.lock().await;
    if model_installed(paths, name) {
        return Ok(());
    }
    download_model_locked(host, paths, name).await
}

pub fn delete_model(paths: &Paths, name: &str) -> Result<(), String> {
    model_spec(name).ok_or_else(|| format!("unknown model “{name}”"))?;
    let dir = model_dir(paths, name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("could not remove the model: {e}"))?;
    }
    Ok(())
}

/// Is everything these options need already installed?
pub fn is_ready(paths: &Paths, opts: &TranscribeOptions) -> bool {
    let speakers_ok = clean_speakers(&opts.speakers) == "off" || (speakers_installed(paths) && engine_has_speaker_package(paths));
    engine_installed(paths) && model_installed(paths, &opts.model) && speakers_ok
}

/// Install whatever is missing for `model` (and for speaker labels, if asked): the engine, the model, the speaker models.
pub async fn ensure_ready(host: &dyn Host, paths: &Paths, opts: &TranscribeOptions) -> Result<(), String> {
    let model = opts.model.as_str();
    model_spec(model).ok_or_else(|| format!("unknown model “{model}”"))?;
    let want_speakers = clean_speakers(&opts.speakers) != "off";
    if is_ready(paths, opts) {
        return Ok(());
    }
    let _guard = SETUP_LOCK.lock().await;
    if !engine_installed(paths) {
        install_engine_locked(host, paths).await?;
    }
    if !model_installed(paths, model) {
        download_model_locked(host, paths, model).await?;
    }
    if want_speakers {
        ensure_speakers_locked(host, paths).await?;
    }
    Ok(())
}

// ---- transcribing -------------------------------------------------------

/// What the Python runner printed.
#[derive(Debug, PartialEq)]
pub enum RunnerLine {
    Progress(u32),
    Language(String),
    Duration(f64),
    Words(u64),
    Phase(String),
    Speakers(u32),
    Done(Vec<String>),
    Error(String),
}

pub fn parse_runner_line(line: &str) -> Option<RunnerLine> {
    let line = line.trim();
    let (tag, rest) = line.split_once('|')?;
    match tag {
        "PROGRESS" => rest.trim().parse().ok().map(RunnerLine::Progress),
        "LANG" => Some(RunnerLine::Language(rest.to_string())),
        "DURATION" => rest.trim().parse().ok().map(RunnerLine::Duration),
        "WORDS" => rest.trim().parse().ok().map(RunnerLine::Words),
        "PHASE" => Some(RunnerLine::Phase(rest.trim().to_string())),
        "SPEAKERS" => rest.trim().parse().ok().map(RunnerLine::Speakers),
        "DONE" => Some(RunnerLine::Done(rest.split(';').filter(|s| !s.is_empty()).map(str::to_string).collect())),
        "ERROR" => Some(RunnerLine::Error(rest.to_string())),
        _ => None,
    }
}

/// The file the transcript is written next to: same name, new extension.
pub fn output_base(media: &Path) -> PathBuf {
    media.with_extension("")
}

/// The transcript files already sitting next to `media` for the formats asked for, when all of them
/// exist and are not empty. Used when a stopped job is continued, so finished work is not redone.
pub(crate) fn finished_outputs(media: &Path, formats: &[String]) -> Option<Vec<String>> {
    let base = output_base(media);
    let mut out = Vec::new();
    for ext in clean_formats(formats).split(',') {
        let p = base.with_file_name(format!("{}.{ext}", base.file_name()?.to_string_lossy()));
        if std::fs::metadata(&p).map(|m| m.len() == 0).unwrap_or(true) {
            return None;
        }
        out.push(p.to_string_lossy().to_string());
    }
    Some(out)
}

/// "off", "auto" or a number from 2 to 8; anything else means off.
fn clean_speakers(s: &str) -> String {
    let t = s.trim().to_ascii_lowercase();
    match t.as_str() {
        "auto" => t,
        _ => t.parse::<u32>().ok().filter(|n| (2..=8).contains(n)).map(|n| n.to_string()).unwrap_or_else(|| "off".into()),
    }
}

pub(crate) fn clean_formats(f: &[String]) -> String {
    let wanted: Vec<&str> = ["txt", "srt", "vtt"].into_iter().filter(|k| f.iter().any(|x| x.eq_ignore_ascii_case(k))).collect();
    if wanted.is_empty() { "txt".into() } else { wanted.join(",") }
}

/// What a finished transcription produced.
#[derive(Debug, Default)]
pub struct Transcribed {
    pub files: Vec<String>,
    /// Length of the audio that was transcribed, in seconds.
    pub audio_secs: f64,
    pub words: u64,
    pub language: String,
    /// Voices found when speaker labels were asked for (0 = not asked or none found).
    pub speakers: u32,
}

/// Transcribe one file. `on_progress` gets 0..=100; `on_pid` gets the runner's
/// process id so the job can be stopped (and `None` when it has ended).
/// What is known about a video, for the transcript's header.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct About {
    pub title: String,
    pub channel: String,
    /// `YYYY-MM-DD`
    pub uploaded: String,
    pub url: String,
}

impl About {
    fn as_json(&self) -> String {
        serde_json::json!({ "title": self.title, "channel": self.channel, "uploaded": self.uploaded, "url": self.url }).to_string()
    }
}

/// `20260930` or `2026-09-30` -> `2026-09-30`; anything else -> empty.
fn pretty_date(s: &str) -> String {
    let d: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.len() == 8 && s.len() <= 10 && !d.starts_with("0000") { format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..]) } else { String::new() }
}

/// The per-video records yt-dlp printed for a job, by video id.
pub(crate) fn read_metas(path: &Path) -> std::collections::HashMap<String, About> {
    let mut out = std::collections::HashMap::new();
    let Ok(text) = std::fs::read_to_string(path) else { return out };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let get = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let id = get("id");
        if !id.is_empty() {
            out.insert(id, About { title: get("title"), channel: get("uploader"), uploaded: pretty_date(&get("upload_date")), url: get("webpage_url") });
        }
    }
    out
}

/// What is known about the video in `media`: from yt-dlp's record, else from a date at the start of the file name.
pub(crate) fn about_file(media: &Path, metas: &std::collections::HashMap<String, About>) -> About {
    let stem = media.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut a = crate::existing::video_id(&stem).and_then(|id| metas.get(id).cloned()).unwrap_or_default();
    if a.uploaded.is_empty() {
        a.uploaded = stem.get(..10).map(pretty_date).filter(|d| stem.get(..10) == Some(d.as_str())).unwrap_or_default();
    }
    a
}

pub async fn transcribe_file(
    paths: &Paths,
    opts: &TranscribeOptions,
    media: &Path,
    about: &About,
    on_progress: impl Fn(u32),
    on_phase: impl Fn(&str),
    on_pid: impl Fn(Option<u32>),
) -> Result<Transcribed, String> {
    write_script(paths)?;
    let threads = std::thread::available_parallelism().map(|n| n.get().saturating_sub(1).max(1)).unwrap_or(2);
    let mut cmd = python_command(paths);
    cmd.env("HF_HUB_OFFLINE", "1")
        .env("YTGRAB_ABOUT", about.as_json())
        .arg(script_path(paths))
        .arg("transcribe")
        .arg(model_dir(paths, &opts.model))
        .arg(media)
        .arg(output_base(media))
        .arg(clean_formats(&opts.formats))
        .arg(opts.language.trim())
        .arg(opts.interval_secs.to_string())
        .arg(threads.to_string())
        .arg(clean_speakers(&opts.speakers))
        .arg(speaker_dir(paths));
    platform::configure_command(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("could not start Whisper: {e}"))?;
    on_pid(child.id());

    let stderr = child.stderr.take();
    let err_task = tokio::spawn(async move {
        let mut tail = Vec::new();
        if let Some(e) = stderr {
            let mut lines = BufReader::new(e).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                tail.push(l);
                if tail.len() > 8 {
                    tail.remove(0);
                }
            }
        }
        tail
    });

    let mut out_info = Transcribed::default();
    let mut error = None;
    let identifying = std::sync::atomic::AtomicBool::new(false);
    if let Some(out) = child.stdout.take() {
        let mut lines = BufReader::new(out).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            match parse_runner_line(&l) {
                Some(RunnerLine::Progress(p)) => on_progress(p.min(100)),
                Some(RunnerLine::Done(f)) => out_info.files = f,
                Some(RunnerLine::Duration(d)) => out_info.audio_secs = d,
                Some(RunnerLine::Words(w)) => out_info.words = w,
                Some(RunnerLine::Language(l)) => out_info.language = l,
                Some(RunnerLine::Phase(p)) => {
                    if p == "diarize" {
                        identifying.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    on_phase(&p)
                }
                Some(RunnerLine::Speakers(n)) => out_info.speakers = n,
                Some(RunnerLine::Error(e)) => error = Some(e),
                None => {}
            }
        }
    }
    let status = child.wait().await;
    on_pid(None);
    let stderr_tail = err_task.await.unwrap_or_default();
    match status {
        Ok(s) if s.success() && !out_info.files.is_empty() => Ok(out_info),
        _ => Err(error.unwrap_or_else(|| {
            let t = stderr_tail.into_iter().rev().find(|l| !l.trim().is_empty()).unwrap_or_default();
            let why = status.as_ref().ok().and_then(platform::describe_exit);
            let where_ = if identifying.load(std::sync::atomic::Ordering::Relaxed) { "while identifying speakers" } else { "while transcribing" };
            match (t.is_empty(), why) {
                (false, _) => t,
                (true, Some(w)) => format!("Whisper stopped {where_}: {w}"),
                (true, None) => format!("Whisper stopped unexpectedly {where_}"),
            }
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_the_runner_prints() {
        assert_eq!(parse_runner_line("PROGRESS|42"), Some(RunnerLine::Progress(42)));
        assert_eq!(parse_runner_line("LANG|en\n"), Some(RunnerLine::Language("en".into())));
        assert_eq!(
            parse_runner_line("DONE|/a/b.txt;/a/b.srt"),
            Some(RunnerLine::Done(vec!["/a/b.txt".into(), "/a/b.srt".into()]))
        );
        assert_eq!(parse_runner_line("DURATION|3600.4"), Some(RunnerLine::Duration(3600.4)));
        assert_eq!(parse_runner_line("WORDS|10925"), Some(RunnerLine::Words(10925)));
        assert_eq!(parse_runner_line("PHASE|diarize"), Some(RunnerLine::Phase("diarize".into())));
        assert_eq!(parse_runner_line("SPEAKERS|3"), Some(RunnerLine::Speakers(3)));
        assert_eq!(parse_runner_line("ERROR|ValueError: bad"), Some(RunnerLine::Error("ValueError: bad".into())));
        assert_eq!(parse_runner_line("some library noise"), None);
        // A path containing a pipe must survive.
        assert_eq!(parse_runner_line("DONE|/a|b.txt"), Some(RunnerLine::Done(vec!["/a|b.txt".into()])));
    }

    #[test]
    fn the_header_knows_the_upload_date() {
        assert_eq!(pretty_date("20260930"), "2026-09-30");
        assert_eq!(pretty_date("2026-09-30"), "2026-09-30");
        assert_eq!(pretty_date("NA"), "");
        assert_eq!(pretty_date("00000000"), "");
        let dir = std::env::temp_dir().join(format!("ytgrab-meta-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("m.jsonl");
        std::fs::write(&f, "{\"id\":\"AAAAAAAAAAA\",\"upload_date\":\"20260102\",\"uploader\":\"Chan\",\"title\":\"T\",\"webpage_url\":\"https://y/x\"}\nnot json\n").unwrap();
        let metas = read_metas(&f);
        let a = about_file(Path::new("/d/Old name [AAAAAAAAAAA].opus"), &metas);
        assert_eq!((a.uploaded.as_str(), a.channel.as_str(), a.url.as_str()), ("2026-01-02", "Chan", "https://y/x"));
        // No record: fall back on a date written at the start of the file name.
        let b = about_file(Path::new("/d/2025-12-31 - Title [BBBBBBBBBBB].opus"), &metas);
        assert_eq!((b.uploaded.as_str(), b.channel.as_str()), ("2025-12-31", ""));
        assert_eq!(about_file(Path::new("/d/Title [CCCCCCCCCCC].opus"), &metas), About::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transcript_sits_next_to_the_media() {
        assert_eq!(output_base(Path::new("/d/Talk [abc].opus")), PathBuf::from("/d/Talk [abc]"));
    }

    #[test]
    fn only_media_files_are_transcribed() {
        assert!(is_media("/d/a.MP4") && is_media("x.opus") && is_media("x.m4a"));
        assert!(!is_media("/d/a.jpg") && !is_media("/d/a.txt") && !is_media("/d/a.info.json") && !is_media("noext"));
    }

    #[test]
    fn format_list_is_sanitised() {
        assert_eq!(clean_formats(&[]), "txt");
        assert_eq!(clean_formats(&["VTT".into(), "txt".into(), "evil;rm".into()]), "txt,vtt");
    }

    #[test]
    fn speaker_choice_is_sanitised() {
        assert_eq!(clean_speakers("auto"), "auto");
        assert_eq!(clean_speakers(" AUTO "), "auto");
        assert_eq!(clean_speakers("3"), "3");
        for bad in ["", "off", "1", "9", "-2", "two", "3; rm -rf /"] {
            assert_eq!(clean_speakers(bad), "off", "{bad}");
        }
    }

    #[test]
    fn every_model_is_known_to_faster_whisper() {
        let known = ["tiny", "base", "small", "medium", "turbo", "large-v3"];
        assert!(MODELS.iter().all(|m| known.contains(&m.name)));
    }

    #[test]
    fn uv_assets_exist_for_every_platform() {
        for os in [Os::Linux, Os::MacOs, Os::Windows] {
            for arch in [platform::Arch::X64, platform::Arch::Arm64] {
                let a = platform::uv_asset(os, arch);
                assert!(a.starts_with("uv-") && (a.ends_with(".zip") == (os == Os::Windows)));
            }
        }
    }

    struct NullHost;
    impl Host for NullHost {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            if std::env::var_os("YTGRAB_TEST_VERBOSE").is_some() {
                println!("{event} {payload}");
            }
        }
        fn notify(&self, _: &str, _: &str) {}
        fn exit(&self) {}
    }

    /// Installs the real engine from the internet (uv, Python, faster-whisper), downloads the
    /// Tiny model and transcribes a real file. Run with:
    ///   YTGRAB_TEST_AUDIO=/path/to/speech.opus cargo test e2e_whisper -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn e2e_whisper_install_and_transcribe() {
        let audio = PathBuf::from(std::env::var("YTGRAB_TEST_AUDIO").expect("set YTGRAB_TEST_AUDIO"));
        let dir = std::env::temp_dir().join(format!("ytgrab-whisper-{}", uuid::Uuid::new_v4()));
        let paths = Paths::installed_at(dir.join("data"), dir.join("dl"));
        paths.ensure_dirs();
        assert!(!engine_installed(&paths));

        let t = std::time::Instant::now();
        ensure_ready(&NullHost, &paths, &TranscribeOptions { model: "tiny".into(), speakers: "auto".into(), ..Default::default() }).await.unwrap();
        println!("engine + model ready in {:.0}s", t.elapsed().as_secs_f32());
        assert!(engine_installed(&paths) && model_installed(&paths, "tiny"));
        assert!(status(&paths).models.iter().any(|m| m.name == "tiny" && m.installed));

        let media = dir.join("talk.opus");
        std::fs::copy(&audio, &media).unwrap();
        let opts = TranscribeOptions { model: "tiny".into(), formats: vec!["txt".into(), "srt".into(), "vtt".into()], interval_secs: 30, speakers: "auto".into(), ..Default::default() };
        let seen = std::sync::Mutex::new(Vec::new());
        let t = std::time::Instant::now();
        let done = transcribe_file(&paths, &opts, &media, &About::default(), |p| seen.lock().unwrap().push(p), |_| {}, |_| {}).await.unwrap();
        println!("transcribed in {:.0}s -> {done:?}", t.elapsed().as_secs_f32());
        assert_eq!(done.files.len(), 3);
        assert!(done.audio_secs > 250.0 && done.words > 300 && done.language == "en", "{done:?}");
        assert!(done.speakers >= 1, "speaker labels were asked for: {done:?}");
        let txt = std::fs::read_to_string(dir.join("talk.txt")).unwrap();
        println!("{}", txt.lines().take(12).collect::<Vec<_>>().join("\n"));
        assert!(txt.contains("[00:00:"), "periodic time stamps expected");
        assert!(txt.contains("Speaker 1: "), "speaker labels expected");
        assert!(std::fs::read_to_string(dir.join("talk.srt")).unwrap().contains("Speaker 1: "));
        assert!(std::fs::read_to_string(dir.join("talk.vtt")).unwrap().contains("<v Speaker 1>"));
        assert!(speakers_installed(&paths) && status(&paths).speakers_installed);
        assert!(std::fs::read_to_string(dir.join("talk.srt")).unwrap().contains(" --> "));
        assert!(std::fs::read_to_string(dir.join("talk.vtt")).unwrap().starts_with("WEBVTT"));
        // The bar restarts for the speaker step, so it is only monotonic within each step.
        let p = seen.lock().unwrap().clone();
        assert!(p.last() == Some(&100) && p.iter().all(|v| *v <= 100), "progress should end at 100: {p:?}");

        delete_speakers(&paths).unwrap();
        assert!(!speakers_installed(&paths));
        delete_model(&paths, "tiny").unwrap();
        assert!(!model_installed(&paths, "tiny"));
        remove_engine(&paths).unwrap();
        assert!(!engine_installed(&paths));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
