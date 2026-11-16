//! Reading facts and pictures from video files.

use crate::log::{LogKind, LogLine};
use crate::tools::{display_command, quiet_command, Tools};
use serde_json::Value;
use std::cmp::Ordering;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const THUMB_WIDTH: u32 = 160;
pub const THUMB_HEIGHT: u32 = 90;

/// What Cutline knows about one video file.
#[derive(Clone, Debug)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub name: String,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
    pub bytes: u64,
}

impl MediaInfo {
    pub fn kbps(&self) -> f64 {
        if self.duration > 0.0 {
            self.bytes as f64 * 8.0 / 1000.0 / self.duration
        } else {
            0.0
        }
    }
}

/// Filename order as people expect it: case doesn't matter and `clip 2` comes before `clip 10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b): (Vec<char>, Vec<char>) = (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let start_i = i;
            let start_j = j;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let left: String = a[start_i..i].iter().collect();
            let right: String = b[start_j..j].iter().collect();
            let (left_trim, right_trim) = (left.trim_start_matches('0'), right.trim_start_matches('0'));
            let order = left_trim.len().cmp(&right_trim.len()).then_with(|| left_trim.cmp(right_trim));
            if order != Ordering::Equal {
                return order;
            }
        } else {
            if a[i] != b[j] {
                return a[i].cmp(&b[j]);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

fn number(value: &Value) -> Option<f64> {
    value.as_str().and_then(|text| text.parse().ok()).or_else(|| value.as_f64())
}

fn fraction(value: &Value) -> Option<f64> {
    let text = value.as_str()?;
    let (top, bottom) = text.split_once('/')?;
    let (top, bottom): (f64, f64) = (top.parse().ok()?, bottom.parse().ok()?);
    (top > 0.0 && bottom > 0.0).then(|| top / bottom)
}

fn rotation(stream: &Value) -> i64 {
    let from_side = stream["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|item| item["rotation"].as_f64()));
    let degrees = from_side.or_else(|| number(&stream["tags"]["rotate"])).unwrap_or(0.0);
    (degrees.round() as i64).rem_euclid(360)
}

/// Ask ffprobe about a file. Errors are in words for the person.
pub fn probe(tools: &Tools, path: &Path, log: &mut dyn FnMut(LogLine)) -> Result<MediaInfo, String> {
    let args: Vec<OsString> = ["-v", "error", "-show_format", "-show_streams", "-of", "json"]
        .iter()
        .map(OsString::from)
        .chain(std::iter::once(path.as_os_str().to_os_string()))
        .collect();
    log(LogLine::new(LogKind::Command, display_command(tools.ffprobe.as_os_str(), &args)));
    let output = quiet_command(tools.ffprobe.as_os_str())
        .args(&args)
        .output()
        .map_err(|error| format!("Couldn't run ffprobe: {error}"))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !detail.is_empty() {
            log(LogLine::new(LogKind::Warning, detail));
        }
        return Err(format!("{name} isn't a video Cutline can read."));
    }
    let info: Value = serde_json::from_slice(&output.stdout).map_err(|_| format!("{name} couldn't be read."))?;
    let streams = info["streams"].as_array().cloned().unwrap_or_default();
    let is_real_video = |stream: &&Value| {
        stream["codec_type"] == "video" && stream["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0
    };
    let video = streams.iter().find(is_real_video).ok_or_else(|| format!("{name} has no video in it."))?;
    let duration = number(&info["format"]["duration"])
        .filter(|d| *d > 0.0)
        .or_else(|| number(&video["duration"]).filter(|d| *d > 0.0))
        .ok_or_else(|| format!("{name} has no length Cutline can read."))?;
    if duration < 0.1 {
        return Err(format!("{name} is a still picture or too short to use."));
    }
    let (mut width, mut height) = (
        video["width"].as_u64().unwrap_or(0) as u32,
        video["height"].as_u64().unwrap_or(0) as u32,
    );
    if width == 0 || height == 0 {
        return Err(format!("{name} has no picture size Cutline can read."));
    }
    if matches!(rotation(video), 90 | 270) {
        std::mem::swap(&mut width, &mut height);
    }
    let fps = fraction(&video["avg_frame_rate"]).or_else(|| fraction(&video["r_frame_rate"])).unwrap_or(30.0);
    let has_audio = streams.iter().any(|stream| stream["codec_type"] == "audio");
    let bytes = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    Ok(MediaInfo { path: path.to_path_buf(), name, duration, width, height, fps, has_audio, bytes })
}

/// One frame as RGBA pixels of exactly `width` x `height`, shaped by `filter`.
pub fn grab_frame(tools: &Tools, path: &Path, at: f64, filter: &str, width: u32, height: u32) -> Option<Vec<u8>> {
    let output = quiet_command(tools.ffmpeg.as_os_str())
        .args(["-hide_banner", "-loglevel", "error", "-ss"])
        .arg(format!("{at:.3}"))
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-vf", filter, "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .ok()?;
    let expected = (width * height * 4) as usize;
    (output.stdout.len() >= expected).then(|| {
        let mut pixels = output.stdout;
        pixels.truncate(expected);
        pixels
    })
}

/// A small frame for the clip list and the timeline.
pub fn grab_thumbnail(tools: &Tools, path: &Path, at: f64) -> Option<Vec<u8>> {
    let filter = format!(
        "scale={THUMB_WIDTH}:{THUMB_HEIGHT}:force_original_aspect_ratio=increase,crop={THUMB_WIDTH}:{THUMB_HEIGHT}"
    );
    grab_frame(tools, path, at, &filter, THUMB_WIDTH, THUMB_HEIGHT)
}

/// `90`, `1:30`, `1:02:03`.
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (hours, minutes, secs) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

/// `1:30.25`, for the playhead.
pub fn fine_clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let hundredths = ((seconds.fract() * 100.0).floor() as u32).min(99);
    format!("{}.{hundredths:02}", clock(seconds.floor()))
}

/// `20 MB`, `640 KB`, `1.4 GB`.
pub fn human_bytes(bytes: f64) -> String {
    if bytes >= 1e9 {
        format!("{:.2} GB", bytes / 1e9)
    } else if bytes >= 1e6 {
        format!("{:.1} MB", bytes / 1e6)
    } else {
        format!("{:.0} KB", bytes / 1e3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_reads_numbers_as_numbers() {
        let mut names = vec!["clip10.mp4", "Clip2.mp4", "clip1.mp4", "alpha.mov", "Zed.mkv"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["alpha.mov", "clip1.mp4", "Clip2.mp4", "clip10.mp4", "Zed.mkv"]);
    }

    #[test]
    fn clocks_read_well() {
        assert_eq!(clock(5.0), "0:05");
        assert_eq!(clock(90.0), "1:30");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(fine_clock(90.25), "1:30.25");
    }
}
