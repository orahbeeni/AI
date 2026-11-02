//! Finding, installing and querying ffmpeg / ffprobe.

use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// A `Command` that does not flash a console window on Windows.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// Testing hook: ignore any system-wide ffmpeg (and Homebrew) and only use the copy this app downloads.
fn managed_only() -> bool {
    std::env::var_os("VIDEO_MANAGED_FFMPEG_ONLY").is_some_and(|v| !v.is_empty())
}

/// Where a downloaded copy of ffmpeg lives.
pub fn managed_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("video-app").join("ffmpeg")
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if !managed_only() {
        if let Some(path) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&path));
        }
        // An app started from Finder / a desktop launcher does not inherit the shell's PATH.
        #[cfg(unix)]
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/snap/bin"].map(PathBuf::from));
        #[cfg(windows)]
        {
            let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
            dirs.extend(var("LOCALAPPDATA").map(|p| p.join(r"Microsoft\WinGet\Links")));
            dirs.extend(var("ProgramData").map(|p| p.join(r"chocolatey\bin")));
            dirs.extend(var("USERPROFILE").map(|p| p.join(r"scoop\shims")));
        }
    }
    dirs.push(managed_dir());
    dirs
}

fn works(program: &Path) -> bool {
    command(program).arg("-version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn find_tool(name: &str) -> Option<PathBuf> {
    search_dirs().into_iter().map(|d| d.join(exe(name))).find(|p| p.is_file() && works(p))
}

/// Both ffmpeg and ffprobe, if they are installed and run.
pub fn locate() -> Option<Tools> {
    Some(Tools { ffmpeg: find_tool("ffmpeg")?, ffprobe: find_tool("ffprobe")? })
}

// ---------------------------------------------------------------------------------------------
// Installing
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Archive {
    Zip,
    TarXz,
}

/// Each entry is one archive, with the URLs to try in order.
fn downloads() -> Result<(Vec<Vec<String>>, Archive), String> {
    let arch = std::env::consts::ARCH;
    let s = |v: &[&str]| v.iter().map(|u| u.to_string()).collect::<Vec<_>>();
    match std::env::consts::OS {
        "linux" => {
            let a = match arch {
                "x86_64" => "amd64",
                "aarch64" => "arm64",
                "arm" => "armhf",
                other => return Err(format!("no ffmpeg download for Linux/{other}; install ffmpeg with your package manager")),
            };
            let url = format!("https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-{a}-static.tar.xz");
            Ok((vec![vec![url]], Archive::TarXz))
        }
        "windows" => Ok((
            vec![s(&[
                "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip",
                "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip",
            ])],
            Archive::Zip,
        )),
        "macos" => {
            let a = if arch == "aarch64" { "arm64" } else { "amd64" };
            let site = |tool: &str| {
                ["release", "snapshot"].map(|c| format!("https://ffmpeg.martin-riedl.de/redirect/latest/macos/{a}/{c}/{tool}.zip")).to_vec()
            };
            Ok((vec![site("ffmpeg"), site("ffprobe")], Archive::Zip))
        }
        other => Err(format!("no ffmpeg download for {other}; install ffmpeg yourself")),
    }
}

/// Installs ffmpeg and returns where it ended up. `report` gets a status line and, when known, a 0..1 fraction.
pub fn install(report: &dyn Fn(String, Option<f32>)) -> Result<Tools, String> {
    #[cfg(target_os = "macos")]
    if !managed_only() {
        if let Some(brew) = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"].iter().map(PathBuf::from).find(|p| p.is_file()) {
            report("Installing ffmpeg with Homebrew (this can take a few minutes)…".into(), None);
            let out = command(&brew).args(["install", "ffmpeg"]).env("HOMEBREW_NO_ENV_HINTS", "1").stdin(Stdio::null()).output();
            match out {
                Ok(o) if o.status.success() => {
                    if let Some(tools) = locate() {
                        return Ok(tools);
                    }
                }
                Ok(o) => report(format!("Homebrew failed ({}); downloading ffmpeg instead…", last_line(&o.stderr)), None),
                Err(e) => report(format!("Homebrew failed ({e}); downloading ffmpeg instead…"), None),
            }
        }
    }
    download_install(report)
}

fn last_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim().to_string()
}

fn download_install(report: &dyn Fn(String, Option<f32>)) -> Result<Tools, String> {
    let (archives, kind) = downloads()?;
    let dest = managed_dir();
    let work = dest.join("download");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("cannot create {}: {e}", work.display()))?;

    let count = archives.len() as f32;
    for (i, urls) in archives.iter().enumerate() {
        let file = work.join(format!("archive{i}"));
        let base = i as f32 / count;
        let mut last_err = String::new();
        let mut ok = false;
        for url in urls {
            let mut last_shown = -1.0f32;
            let result = download(url, &file, &mut |frac, bytes| {
                let shown = frac.unwrap_or(0.0);
                if frac.is_none() || shown - last_shown >= 0.005 {
                    last_shown = shown;
                    let mb = bytes as f32 / 1e6;
                    report(format!("Downloading ffmpeg… {mb:.0} MB"), Some(base + frac.unwrap_or(0.0) * 0.85 / count));
                }
            });
            match result {
                Ok(()) => {
                    ok = true;
                    break;
                }
                Err(e) => last_err = format!("{url}: {e}"),
            }
        }
        if !ok {
            let _ = fs::remove_dir_all(&work);
            return Err(format!("download failed ({last_err})"));
        }
        report("Unpacking ffmpeg…".into(), Some(base + 0.9 / count));
        extract(&file, &work.join(format!("x{i}")), kind)?;
    }

    report("Setting up ffmpeg…".into(), Some(0.97));
    for name in ["ffmpeg", "ffprobe"] {
        let found = find_file(&work, &exe(name)).ok_or_else(|| format!("{name} was not in the download"))?;
        let target = dest.join(exe(name));
        let _ = fs::remove_file(&target);
        fs::copy(&found, &target).map_err(|e| format!("cannot install {name}: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o755));
        }
    }
    let _ = fs::remove_dir_all(&work);
    locate().ok_or_else(|| "ffmpeg was downloaded but does not run on this machine".to_string())
}

fn download(url: &str, dest: &Path, progress: &mut dyn FnMut(Option<f32>, u64)) -> Result<(), String> {
    let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    let total = resp.body().content_length();
    let mut reader = resp.into_body().into_reader();
    let mut file = fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 128 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(total.map(|t| (done as f64 / t as f64) as f32), done);
    }
    match total {
        Some(t) if t != done => Err(format!("incomplete download ({done} of {t} bytes)")),
        _ => Ok(()),
    }
}

/// Unpacks with the system `tar` (bsdtar reads zips on macOS and Windows 10+; GNU tar reads .tar.xz on Linux).
fn extract(archive: &Path, into: &Path, kind: Archive) -> Result<(), String> {
    fs::create_dir_all(into).map_err(|e| e.to_string())?;
    #[allow(unused_mut)]
    let mut tar = PathBuf::from("tar");
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let system = PathBuf::from(root).join(r"System32\tar.exe");
        if system.is_file() {
            tar = system;
        }
    }
    let flags = match kind {
        Archive::Zip => "-xf",
        Archive::TarXz => "-xJf",
    };
    let out = command(&tar).arg(flags).arg(archive).arg("-C").arg(into).stdin(Stdio::null()).output().map_err(|e| format!("cannot run tar: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("unpacking failed: {}", last_line(&out.stderr))) }
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if kind.is_file() && entry.file_name() == name {
            return Some(path);
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Probing
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Info {
    pub duration: f64,
    /// Size of the picture as it is shown (rotation applied).
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
    pub bytes: u64,
    /// Overall bit rate of the file, if it says.
    pub bitrate_kbps: Option<u32>,
    pub codec: String,
}

pub fn probe(tools: &Tools, path: &Path) -> Result<Info, String> {
    probe_with(tools, path, true)
}

/// Like [`probe`], but a file with only sound is fine (the picture size is then 0×0).
#[cfg(test)]
pub fn probe_any(tools: &Tools, path: &Path) -> Result<Info, String> {
    probe_with(tools, path, false)
}

fn probe_with(tools: &Tools, path: &Path, need_video: bool) -> Result<Info, String> {
    let out = command(&tools.ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(format!("not a video ffmpeg can read: {}", last_line(&out.stderr)));
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    parse_probe(&json, need_video)
}

static NO_VIDEO: serde_json::Value = serde_json::Value::Null;

fn parse_probe(json: &serde_json::Value, need_video: bool) -> Result<Info, String> {
    let streams = json["streams"].as_array().map(Vec::as_slice).unwrap_or_default();
    let is_kind = |s: &serde_json::Value, kind: &str| s["codec_type"] == kind;
    let video = streams.iter().find(|s| is_kind(s, "video") && s["disposition"]["attached_pic"].as_i64() != Some(1));
    let video = match (video, need_video) {
        (Some(v), _) => v,
        (None, true) => return Err("this file has no video".into()),
        (None, false) => &NO_VIDEO,
    };

    let num = |v: &serde_json::Value| v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64());
    let duration = num(&json["format"]["duration"]).or_else(|| num(&video["duration"])).filter(|d| *d > 0.0).ok_or("cannot tell how long this video is")?;

    let (mut width, mut height) = (video["width"].as_u64().unwrap_or(0) as u32, video["height"].as_u64().unwrap_or(0) as u32);
    if need_video && (width == 0 || height == 0) {
        return Err("cannot read the picture size".into());
    }
    // Phones store portrait video as rotated landscape; ffmpeg turns it upright, so report it that way.
    let rotation = video["side_data_list"]
        .as_array()
        .and_then(|l| l.iter().find_map(|d| d["rotation"].as_f64()))
        .or_else(|| num(&video["tags"]["rotate"]))
        .unwrap_or(0.0);
    if (rotation.abs().round() as i64 / 90) % 2 == 1 {
        std::mem::swap(&mut width, &mut height);
    }

    let rate = |s: &str| {
        let (n, d) = s.split_once('/')?;
        let (n, d) = (n.parse::<f64>().ok()?, d.parse::<f64>().ok()?);
        (d > 0.0 && n > 0.0).then_some(n / d)
    };
    let fps = video["avg_frame_rate"].as_str().and_then(rate).or_else(|| video["r_frame_rate"].as_str().and_then(rate)).unwrap_or(30.0);

    Ok(Info {
        duration,
        width,
        height,
        fps,
        has_audio: streams.iter().any(|s| is_kind(s, "audio")),
        bytes: num(&json["format"]["size"]).unwrap_or(0.0) as u64,
        bitrate_kbps: num(&json["format"]["bit_rate"]).map(|b| (b / 1000.0) as u32).filter(|b| *b > 0),
        codec: video["codec_name"].as_str().unwrap_or("?").to_string(),
    })
}

// ---------------------------------------------------------------------------------------------
// Preview frames
// ---------------------------------------------------------------------------------------------

pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// One decoded picture at `t` seconds, at most `max_width` wide.
pub fn frame_at(tools: &Tools, path: &Path, t: f64, max_width: u32) -> Result<Frame, String> {
    let out = command(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-ss", &format!("{t:.3}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", &format!("scale='min({max_width},iw)':-1"), "-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if out.stdout.is_empty() {
        return Err(format!("no picture at {t:.1}s: {}", last_line(&out.stderr)));
    }
    decode_png(&out.stdout)
}

fn decode_png(bytes: &[u8]) -> Result<Frame, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("picture too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Indexed => return Err("unexpected palette picture".into()),
    };
    Ok(Frame { width: info.width as usize, height: info.height as usize, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_swaps_rotated_video() {
        let json = serde_json::json!({
            "format": { "duration": "12.5", "size": "1000", "bit_rate": "640000" },
            "streams": [
                { "codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080,
                  "avg_frame_rate": "30000/1001", "side_data_list": [{ "rotation": -90 }] },
                { "codec_type": "audio" }
            ]
        });
        let info = parse_probe(&json, true).unwrap();
        assert_eq!((info.width, info.height), (1080, 1920));
        assert!(info.has_audio);
        assert_eq!(info.bitrate_kbps, Some(640));
        assert!((info.fps - 29.97).abs() < 0.01);
    }

    #[test]
    fn probe_rejects_audio_only() {
        let json = serde_json::json!({ "format": { "duration": "3" }, "streams": [{ "codec_type": "audio" }] });
        assert!(parse_probe(&json, true).is_err());
    }
}
