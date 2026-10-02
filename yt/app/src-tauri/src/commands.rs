//! The functions the web UI can call.

use crate::args::{self, JobOptions};
use crate::binaries::{self, BinInfo};
use crate::errors;
use crate::jobs::{make_id, AddJob, Job, Shared};
use crate::platform;
use crate::store::{Settings, Subscription};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub message: String,
    pub hint: Option<String>,
}

impl From<String> for ApiError {
    fn from(message: String) -> Self {
        ApiError { message, hint: None }
    }
}

type ApiResult<T> = Result<T, ApiError>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub os: String,
    pub arch: String,
    pub portable: bool,
    pub data_dir: String,
    pub bin_dir: String,
    pub default_download_dir: String,
    pub bins: Vec<BinInfo>,
}

#[tauri::command]
pub async fn app_info(st: State<'_, Shared>) -> ApiResult<AppInfo> {
    let settings = st.settings();
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        os: format!("{:?}", platform::os()).to_lowercase(),
        arch: format!("{:?}", platform::arch()).to_lowercase(),
        portable: st.paths.portable,
        data_dir: st.paths.data_dir.to_string_lossy().into(),
        bin_dir: st.paths.bin_dir.to_string_lossy().into(),
        default_download_dir: st.paths.default_download_dir.to_string_lossy().into(),
        bins: binaries::inspect(&st.paths, &settings).await,
    })
}

#[tauri::command]
pub async fn install_tools(st: State<'_, Shared>, names: Vec<String>) -> ApiResult<Vec<BinInfo>> {
    for name in names.iter().filter(|n| binaries::TOOLS.contains(&n.as_str())) {
        binaries::install(st.host.as_ref(), &st.paths, name).await?;
    }
    Ok(binaries::inspect(&st.paths, &st.settings()).await)
}

#[tauri::command]
pub async fn latest_ytdlp_version() -> ApiResult<String> {
    Ok(binaries::latest_ytdlp_version().await?)
}

#[tauri::command]
pub fn get_settings(st: State<'_, Shared>) -> Settings {
    st.settings()
}

#[tauri::command]
pub fn save_settings(st: State<'_, Shared>, settings: Settings) -> ApiResult<Settings> {
    let mut s = settings;
    s.max_concurrent = s.max_concurrent.clamp(1, 8);
    s.save(&st.paths).map_err(|e| e.to_string())?;
    *st.settings.lock().unwrap_or_else(|p| p.into_inner()) = s.clone();
    st.pump();
    Ok(s)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReq {
    pub url: String,
    pub cookies_browser: Option<String>,
    pub cookies_file: Option<String>,
    pub proxy: Option<String>,
}

/// Ask yt-dlp what a link is (video, playlist, live, scheduled...) without downloading it.
#[tauri::command]
pub async fn probe(st: State<'_, Shared>, req: ProbeReq) -> ApiResult<Value> {
    let settings = st.settings();
    let (ytdlp, _) = binaries::locate(&st.paths, &settings, "yt-dlp").ok_or_else(|| ApiError {
        message: "yt-dlp isn't installed yet.".into(),
        hint: Some("Open Settings → Tools and click Install.".into()),
    })?;
    let opts = JobOptions {
        url: req.url.clone(),
        cookies_browser: req.cookies_browser,
        cookies_file: req.cookies_file,
        proxy: req.proxy,
        ..Default::default()
    };
    let ctx = st.arg_context_pub();
    let mut argv: Vec<String> = ["-J", "--flat-playlist", "--no-warnings", "--no-update", "--ignore-no-formats-error"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    argv.extend(args::common_flags(&opts, &ctx));
    argv.push("--".into());
    argv.push(req.url.trim().to_string());

    let mut cmd = Command::new(ytdlp);
    cmd.args(&argv)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1");
    platform::configure_command(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start yt-dlp: {e}"))?;
    let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());

    let run = async {
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let _ = tokio::join!(out.read_to_end(&mut o), err.read_to_end(&mut e));
        let status = child.wait().await;
        (o, e, status)
    };
    let (o, e, status) = tokio::time::timeout(Duration::from_secs(90), run)
        .await
        .map_err(|_| ApiError { message: "Looking up the link took too long.".into(), hint: Some("Check your connection and try again.".into()) })?;

    let stderr_text = String::from_utf8_lossy(&e).to_string();
    if let Ok(v) = serde_json::from_slice::<Value>(&o) {
        return Ok(v);
    }
    // A video that hasn't started yet reports an "error" but is still a valid target.
    let lower = stderr_text.to_lowercase();
    if lower.contains("will begin in") || lower.contains("premieres in") || lower.contains("will premiere") {
        let msg = stderr_text.lines().rev().find(|l| l.contains("ERROR")).unwrap_or("").trim_start_matches("ERROR:").trim();
        return Ok(serde_json::json!({ "_upcoming": true, "webpage_url": req.url.trim(), "title": null, "note": msg }));
    }
    let raw = stderr_text
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR"))
        .map(|l| l.trim_start_matches("ERROR:").trim().to_string())
        .unwrap_or_else(|| format!("yt-dlp could not read this link ({:?})", status.ok().and_then(|s| s.code())));
    match errors::explain(&raw) {
        Some(x) => Err(ApiError { message: x.message, hint: Some(x.hint) }),
        None => Err(ApiError::from(raw)),
    }
}

#[tauri::command]
pub fn preview_command(st: State<'_, Shared>, options: JobOptions) -> String {
    st.preview(&options)
}

#[tauri::command]
pub fn list_jobs(st: State<'_, Shared>) -> Vec<Job> {
    st.list()
}

#[tauri::command]
pub fn job_log(st: State<'_, Shared>, id: String) -> Vec<String> {
    st.log_of(&id)
}

#[tauri::command]
pub fn add_job(st: State<'_, Shared>, job: AddJob) -> Job {
    st.inner().add(job)
}

#[tauri::command]
pub fn cancel_job(st: State<'_, Shared>, id: String) {
    st.inner().cancel(&id)
}

#[tauri::command]
pub fn retry_job(st: State<'_, Shared>, id: String) {
    st.inner().retry(&id)
}

#[tauri::command]
pub fn remove_job(st: State<'_, Shared>, id: String) {
    st.inner().remove(&id)
}

#[tauri::command]
pub fn clear_finished(st: State<'_, Shared>) {
    st.clear_finished()
}

#[tauri::command]
pub fn list_subscriptions(st: State<'_, Shared>) -> Vec<Subscription> {
    st.subs.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

#[tauri::command]
pub fn save_subscription(st: State<'_, Shared>, subscription: Subscription) -> Vec<Subscription> {
    let mut sub = subscription;
    if sub.id.is_empty() {
        sub.id = make_id();
    }
    {
        let mut subs = st.subs.lock().unwrap_or_else(|p| p.into_inner());
        match subs.iter_mut().find(|s| s.id == sub.id) {
            Some(existing) => {
                sub.last_check = existing.last_check;
                sub.last_note = existing.last_note.clone();
                *existing = sub;
            }
            None => subs.push(sub),
        }
    }
    st.save_subs();
    list_subscriptions(st)
}

#[tauri::command]
pub fn delete_subscription(st: State<'_, Shared>, id: String) -> Vec<Subscription> {
    st.subs.lock().unwrap_or_else(|p| p.into_inner()).retain(|s| s.id != id);
    st.save_subs();
    list_subscriptions(st)
}

#[tauri::command]
pub fn check_subscription_now(st: State<'_, Shared>, id: String) -> bool {
    st.inner().check_subscription(&id)
}

#[tauri::command]
pub fn reveal_path(app: AppHandle, path: String) -> ApiResult<()> {
    let p = std::path::PathBuf::from(&path);
    if p.exists() {
        app.opener().reveal_item_in_dir(&p).map_err(|e| e.to_string())?;
    } else if let Some(parent) = p.parent().filter(|d| d.exists()) {
        app.opener().open_path(parent.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())?;
    } else {
        return Err("That file or folder no longer exists.".to_string().into());
    }
    Ok(())
}

#[tauri::command]
pub fn open_path(app: AppHandle, path: String) -> ApiResult<()> {
    if !std::path::Path::new(&path).exists() {
        return Err("That file or folder no longer exists.".to_string().into());
    }
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn quit_app(st: State<'_, Shared>, app: AppHandle) {
    st.stop_all();
    app.exit(0);
}
