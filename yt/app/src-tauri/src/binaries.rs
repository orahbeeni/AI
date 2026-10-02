//! Finds, downloads and updates yt-dlp, ffmpeg/ffprobe and deno.
//! The app prefers its own copies (in the app's `bin` folder) so it behaves
//! identically on every machine and can run from a flash drive.

use crate::paths::Paths;
use crate::platform::{self, Arch, Os};
use crate::store::Settings;
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use crate::jobs::Host;

pub const TOOLS: [&str; 4] = ["yt-dlp", "ffmpeg", "ffprobe", "deno"];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinInfo {
    pub name: String,
    pub path: Option<String>,
    pub version: Option<String>,
    /// "managed" | "system" | "custom" | "missing"
    pub source: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    name: &'a str,
    stage: &'a str,
    downloaded: u64,
    total: Option<u64>,
}

fn emit(host: &dyn Host, ev: ProgressEvent) {
    host.emit("binary-progress", serde_json::to_value(ev).unwrap_or_default());
}

fn managed_path(paths: &Paths, name: &str) -> PathBuf {
    paths.bin_dir.join(platform::exe_name(name))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let exe = platform::exe_name(name);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// Resolve where a tool lives, honouring the user's "managed vs system" choice.
pub fn locate(paths: &Paths, settings: &Settings, name: &str) -> Option<(PathBuf, &'static str)> {
    if name == "yt-dlp" && !settings.custom_ytdlp_path.trim().is_empty() {
        let p = PathBuf::from(settings.custom_ytdlp_path.trim());
        if p.is_file() {
            return Some((p, "custom"));
        }
    }
    let managed = Some(managed_path(paths, name)).filter(|p| p.is_file()).map(|p| (p, "managed"));
    if settings.binary_source == "system" {
        find_on_path(name).map(|p| (p, "system")).or(managed)
    } else {
        // Default: only the app's own copies. A stale copy on PATH (an old
        // yt-dlp, no JS runtime...) would silently break downloads and would
        // not travel with a portable install.
        managed
    }
}

async fn version_of(path: &Path, name: &str) -> Option<String> {
    let arg = if name == "yt-dlp" { "--version" } else if name == "deno" { "--version" } else { "-version" };
    let mut cmd = tokio::process::Command::new(path);
    cmd.arg(arg).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    platform::configure_command(&mut cmd);
    let out = tokio::time::timeout(std::time::Duration::from_secs(15), cmd.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?.trim().to_string();
    // "ffmpeg version 6.1.1-static ..." -> "6.1.1-static"
    let v = first
        .strip_prefix("ffmpeg version ")
        .or_else(|| first.strip_prefix("ffprobe version "))
        .or_else(|| first.strip_prefix("deno "))
        .unwrap_or(&first);
    Some(v.split_whitespace().next().unwrap_or(v).to_string())
}

pub async fn inspect(paths: &Paths, settings: &Settings) -> Vec<BinInfo> {
    let mut out = Vec::new();
    for name in TOOLS {
        match locate(paths, settings, name) {
            Some((p, source)) => out.push(BinInfo {
                name: name.into(),
                version: version_of(&p, name).await,
                path: Some(p.to_string_lossy().into_owned()),
                source: source.into(),
            }),
            None => out.push(BinInfo { name: name.into(), path: None, version: None, source: "missing".into() }),
        }
    }
    out
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("YTGrab/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

async fn download_to(host: &dyn Host, name: &str, url: &str, dest: &Path) -> Result<(), String> {
    let resp = client()?.get(url).send().await.map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {} for {url}", resp.status()));
    }
    let total = resp.content_length();
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut downloaded = 0u64;
    let mut last_emit = std::time::Instant::now();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("download interrupted: {e}"))?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        if last_emit.elapsed() > std::time::Duration::from_millis(150) {
            last_emit = std::time::Instant::now();
            emit(host, ProgressEvent { name, stage: "downloading", downloaded, total });
        }
    }
    emit(host, ProgressEvent { name, stage: "downloading", downloaded, total });
    Ok(())
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Find `asset`'s hash inside a `SHA2-256SUMS` file ("<hash>  <name>" per line).
pub fn checksum_for(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (hash, file) = (it.next()?, it.next()?);
        (file.trim_start_matches('*') == asset).then(|| hash.to_lowercase())
    })
}

fn replace_file(tmp: &Path, dest: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(dest);
    std::fs::rename(tmp, dest)
        .or_else(|_| std::fs::copy(tmp, dest).map(|_| ()).and_then(|_| std::fs::remove_file(tmp)))
        .map_err(|e| format!("could not install {}: {e}", dest.display()))
}

pub async fn install(host: &dyn Host, paths: &Paths, name: &str) -> Result<(), String> {
    paths.ensure_dirs();
    let (os, arch) = (platform::os(), platform::arch());
    let dest = managed_path(paths, name);
    let tmp = paths.bin_dir.join(format!("{name}.download"));
    let result = install_inner(host, name, os, arch, &dest, &tmp).await;
    let _ = std::fs::remove_file(&tmp);
    result?;
    platform::prepare_binary(&dest).map_err(|e| e.to_string())?;
    emit(host, ProgressEvent { name, stage: "done", downloaded: 0, total: None });
    Ok(())
}

async fn install_inner(host: &dyn Host, name: &str, os: Os, arch: Arch, dest: &Path, tmp: &Path) -> Result<(), String> {
    match name {
        "yt-dlp" => {
            let asset = platform::ytdlp_asset(os, arch);
            let base = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
            download_to(host, name, &format!("{base}/{asset}"), tmp).await?;
            emit(host, ProgressEvent { name, stage: "verifying", downloaded: 0, total: None });
            let sums = client()?
                .get(format!("{base}/SHA2-256SUMS"))
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| format!("could not fetch checksums: {e}"))?
                .text()
                .await
                .map_err(|e| e.to_string())?;
            let want = checksum_for(&sums, asset).ok_or("checksum for this platform not found")?;
            let got = sha256_file(tmp).map_err(|e| e.to_string())?;
            if want != got {
                return Err("yt-dlp download failed its integrity check; not installed".into());
            }
            replace_file(tmp, dest)
        }
        "ffmpeg" | "ffprobe" => {
            let url = format!(
                "https://github.com/eugeneware/ffmpeg-static/releases/latest/download/{name}-{}.gz",
                platform::ffmpeg_suffix(os, arch)
            );
            download_to(host, name, &url, tmp).await?;
            emit(host, ProgressEvent { name, stage: "unpacking", downloaded: 0, total: None });
            let (tmp2, out2) = (tmp.to_path_buf(), tmp.with_extension("bin"));
            let out3 = out2.clone();
            tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                let mut dec = flate2::read::GzDecoder::new(std::fs::File::open(&tmp2)?);
                let mut out = std::fs::File::create(&out2)?;
                std::io::copy(&mut dec, &mut out)?;
                Ok(())
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("could not unpack {name}: {e}"))?;
            let r = replace_file(&out3, dest);
            let _ = std::fs::remove_file(&out3);
            r
        }
        "deno" => {
            let url = format!(
                "https://github.com/denoland/deno/releases/latest/download/deno-{}.zip",
                platform::deno_triple(os, arch)
            );
            download_to(host, name, &url, tmp).await?;
            emit(host, ProgressEvent { name, stage: "unpacking", downloaded: 0, total: None });
            let (zip_path, out_path) = (tmp.to_path_buf(), tmp.with_extension("bin"));
            let exe = platform::exe_name("deno");
            let out_clone = out_path.clone();
            tokio::task::spawn_blocking(move || -> Result<(), String> {
                let mut z = zip::ZipArchive::new(std::fs::File::open(&zip_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                let mut f = z.by_name(&exe).map_err(|e| e.to_string())?;
                let mut out = std::fs::File::create(&out_clone).map_err(|e| e.to_string())?;
                std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
                Ok(())
            })
            .await
            .map_err(|e| e.to_string())??;
            let r = replace_file(&out_path, dest);
            let _ = std::fs::remove_file(&out_path);
            r
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Latest yt-dlp release tag (e.g. "2026.08.19"), for the "update available" hint.
pub async fn latest_ytdlp_version() -> Result<String, String> {
    let v: serde_json::Value = client()?
        .get("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    v["tag_name"].as_str().map(str::to_string).ok_or_else(|| "no version in response".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_checksum_lines() {
        let sums = "aaa111  yt-dlp\nBBB222 *yt-dlp.exe\nccc333  yt-dlp_linux\n";
        assert_eq!(checksum_for(sums, "yt-dlp_linux").as_deref(), Some("ccc333"));
        assert_eq!(checksum_for(sums, "yt-dlp.exe").as_deref(), Some("bbb222"));
        assert_eq!(checksum_for(sums, "nope"), None);
    }

    #[test]
    fn managed_copy_wins_unless_system_preferred() {
        let dir = std::env::temp_dir().join(format!("ytgrab-bin-{}", uuid::Uuid::new_v4()));
        let paths = Paths::installed_at(dir.clone(), dir.join("dl"));
        std::fs::create_dir_all(&paths.bin_dir).unwrap();
        let exe = paths.bin_dir.join(platform::exe_name("yt-dlp"));
        std::fs::write(&exe, b"x").unwrap();
        let s = Settings::default();
        let (p, src) = locate(&paths, &s, "yt-dlp").unwrap();
        assert_eq!((p, src), (exe, "managed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_mode_ignores_tools_on_path() {
        let dir = std::env::temp_dir().join(format!("ytgrab-bin-{}", uuid::Uuid::new_v4()));
        let paths = Paths::installed_at(dir.clone(), dir.join("dl"));
        std::fs::create_dir_all(&paths.bin_dir).unwrap();
        // Nothing installed by the app: a system copy must not be picked up.
        let managed_mode = Settings::default();
        assert!(locate(&paths, &managed_mode, "yt-dlp").is_none());
        // ...unless the user explicitly prefers system tools.
        let system_mode = Settings { binary_source: "system".into(), ..Default::default() };
        if find_on_path("yt-dlp").is_some() {
            assert_eq!(locate(&paths, &system_mode, "yt-dlp").unwrap().1, "system");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
