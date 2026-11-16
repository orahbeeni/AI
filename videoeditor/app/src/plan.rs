//! Turns the clips and the person's choices into exact FFmpeg commands.
//!
//! Nothing here runs anything, so every rule can be tested on its own.

use crate::media::MediaInfo;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Size limits use decimal megabytes: a file under "20 MB" this way is also under 20 MiB, so it
/// fits whichever one a phone or website means.
pub const MB: f64 = 1_000_000.0;

/// Room left for the container and the encoder's small overshoots.
const SIZE_MARGIN: f64 = 0.97;
const MIN_VIDEO_KBPS: f64 = 40.0;
const NO_LIMIT_CRF_X264: &str = "20";
const NO_LIMIT_CRF_VP9: &str = "31";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    /// Whole picture, black bars if the shape differs.
    Bars,
    /// Fill the frame and trim the edges.
    Crop,
    /// Fill the frame and distort.
    Stretch,
}

/// One entry in the format list.
#[derive(Clone, Copy, Debug)]
pub struct FormatSpec {
    pub label: &'static str,
    pub ext: &'static str,
    pub video: &'static [&'static str],
    pub audio: &'static [&'static str],
}

const H264: &[&str] = &["libx264", "h264_videotoolbox", "h264_nvenc", "h264_qsv", "h264_amf", "h264_mf", "mpeg4"];
const AAC: &[&str] = &["aac", "aac_at", "libfdk_aac"];

pub const FORMATS: &[FormatSpec] = &[
    FormatSpec { label: "MP4", ext: "mp4", video: H264, audio: AAC },
    FormatSpec { label: "MOV", ext: "mov", video: H264, audio: AAC },
    FormatSpec { label: "MKV", ext: "mkv", video: H264, audio: AAC },
    FormatSpec { label: "WebM", ext: "webm", video: &["libvpx-vp9", "libvpx"], audio: &["libopus", "libvorbis"] },
    FormatSpec { label: "AVI", ext: "avi", video: &["mpeg4", "libxvid"], audio: &["libmp3lame", "ac3"] },
    FormatSpec { label: "M4V", ext: "m4v", video: H264, audio: AAC },
    FormatSpec { label: "MPEG-TS", ext: "ts", video: H264, audio: AAC },
    FormatSpec { label: "MPEG", ext: "mpg", video: &["mpeg2video"], audio: &["mp2"] },
    FormatSpec { label: "FLV", ext: "flv", video: H264, audio: AAC },
    FormatSpec { label: "WMV", ext: "wmv", video: &["wmv2", "msmpeg4"], audio: &["wmav2"] },
    FormatSpec { label: "GIF (no sound)", ext: "gif", video: &["gif"], audio: &[] },
];

/// The chosen output format.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FormatChoice {
    Listed(usize),
    /// Any other file extension; FFmpeg chooses the codecs.
    Other(String),
}

impl FormatChoice {
    pub fn spec(&self) -> Option<&'static FormatSpec> {
        match self {
            FormatChoice::Listed(index) => FORMATS.get(*index),
            FormatChoice::Other(ext) => FORMATS.iter().find(|format| format.ext.eq_ignore_ascii_case(ext.trim())),
        }
    }

    pub fn extension(&self) -> String {
        match self {
            FormatChoice::Listed(index) => FORMATS.get(*index).map_or("mp4", |format| format.ext).to_string(),
            FormatChoice::Other(ext) => ext.trim().trim_start_matches('.').to_lowercase(),
        }
    }
}

/// Is this format's encoder present in the FFmpeg we found?
pub fn format_available(format: &FormatSpec, encoders: &HashSet<String>) -> bool {
    encoders.is_empty() || format.video.iter().any(|name| encoders.contains(*name))
}

/// What the person chose in the output panel.
#[derive(Clone, Debug)]
pub struct Settings {
    pub format: FormatChoice,
    /// Final picture size; `None` keeps the source size.
    pub frame: Option<(u32, u32)>,
    pub fit: Fit,
    /// One entry per result video: its size limit in MB, or `None` for no limit.
    pub limits_mb: Vec<Option<f64>>,
    pub out_dir: PathBuf,
    pub base_name: String,
    /// Per clip, in the same order: stretches of the source (start, end in seconds) to leave out.
    pub removed: Vec<Vec<(f64, f64)>>,
}

/// Pieces shorter than this are dropped rather than encoded as a flash of a frame or two.
const MIN_PIECE: f64 = 0.05;

/// Sort and join overlapping stretches.
pub fn merge_ranges(ranges: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut sorted: Vec<(f64, f64)> = ranges.iter().copied().filter(|(start, end)| end > start).collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(sorted.len());
    for (start, end) in sorted {
        match merged.last_mut() {
            Some(last) if start <= last.1 + 1e-6 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// The parts of a clip that stay, as (start, end) in the source.
pub fn kept_ranges(duration: f64, removed: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut kept = Vec::new();
    let mut from = 0.0;
    for (start, end) in merge_ranges(removed) {
        let start = start.clamp(0.0, duration);
        if start - from >= MIN_PIECE {
            kept.push((from, start));
        }
        from = from.max(end.clamp(0.0, duration));
    }
    if duration - from >= MIN_PIECE {
        kept.push((from, duration));
    }
    kept
}

/// A piece of one source clip inside a result video.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub clip: usize,
    pub start: f64,
    pub duration: f64,
}

/// How good a result video should look, estimated from the bits each pixel gets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    Best,
    Great,
    Good,
    Fair,
    Low,
    TooSmall,
}

impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Quality::Best => "Best",
            Quality::Great => "Great",
            Quality::Good => "Good",
            Quality::Fair => "Fair",
            Quality::Low => "Low",
            Quality::TooSmall => "Too small",
        }
    }
}

/// One result video.
#[derive(Clone, Debug)]
pub struct PartPlan {
    pub index: usize,
    pub start: f64,
    pub duration: f64,
    pub limit_bytes: Option<u64>,
    pub video_kbps: Option<f64>,
    pub audio_kbps: u32,
    pub quality: Quality,
    pub output: PathBuf,
    pub segments: Vec<Segment>,
}

/// Which encoding run a set of arguments is for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pass {
    /// Analysis only, writes no video.
    First,
    /// The run that writes the file.
    Final,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub clips: Vec<MediaInfo>,
    pub total_duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub fit: Fit,
    pub video_encoder: Option<String>,
    pub audio_encoder: Option<String>,
    pub has_audio: bool,
    pub ext: String,
    pub is_gif: bool,
    pub parts: Vec<PartPlan>,
    /// Why this plan can't be exported, in words for the person.
    pub problem: Option<String>,
}

fn even(value: f64) -> u32 {
    let rounded = (value.round() as u32).max(2);
    rounded - rounded % 2
}

/// The picture size when the person keeps the source: the largest clip's.
pub fn source_frame(clips: &[MediaInfo]) -> (u32, u32) {
    clips
        .iter()
        .max_by_key(|clip| clip.width as u64 * clip.height as u64)
        .map_or((1920, 1080), |clip| (even(clip.width as f64), even(clip.height as f64)))
}

/// The filter that fits one clip's picture into a frame of the final size.
pub fn fit_filter(width: u32, height: u32, fit: Fit) -> String {
    match fit {
        Fit::Bars => format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black"
        ),
        Fit::Crop => format!("scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height}"),
        Fit::Stretch => format!("scale={width}:{height}"),
    }
}

/// How well each codec family uses its bits, compared with H.264.
fn codec_efficiency(encoder: Option<&str>) -> f64 {
    match encoder {
        Some("libvpx-vp9") => 1.4,
        Some("libvpx" | "mpeg4" | "libxvid" | "wmv2" | "msmpeg4" | "mpeg2video") => 0.6,
        _ => 1.0,
    }
}

fn rate_quality(video_kbps: f64, width: u32, height: u32, fps: f64, encoder: Option<&str>) -> Quality {
    let bits_per_pixel = video_kbps * 1000.0 / (width as f64 * height as f64 * fps) * codec_efficiency(encoder);
    match bits_per_pixel {
        bpp if bpp >= 0.08 => Quality::Great,
        bpp if bpp >= 0.04 => Quality::Good,
        bpp if bpp >= 0.018 => Quality::Fair,
        _ => Quality::Low,
    }
}

/// How much of a part's budget goes to sound.
fn audio_share(total_kbps: f64) -> u32 {
    match total_kbps {
        kbps if kbps < 250.0 => 48,
        kbps if kbps < 500.0 => 64,
        kbps if kbps < 1000.0 => 96,
        _ => 128,
    }
}

/// Don't spend a generous limit on bits the picture can't use, or a short clip would balloon up to
/// the limit for nothing.
fn useful_ceiling_kbps(width: u32, height: u32, fps: f64, source_kbps: f64) -> f64 {
    let by_pixels = width as f64 * height as f64 * fps * 0.20 / 1000.0;
    by_pixels.min((source_kbps * 1.5).max(500.0))
}

fn pick(candidates: &[&str], available: &HashSet<String>) -> Option<String> {
    let found = candidates.iter().find(|name| available.is_empty() || available.contains(**name));
    found.map(|name| name.to_string())
}

fn output_path(settings: &Settings, index: usize, count: usize, ext: &str, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    let base = if count > 1 {
        let width = count.to_string().len();
        format!("{}_part{:0width$}of{count}", settings.base_name, index + 1)
    } else {
        settings.base_name.clone()
    };
    let first = settings.out_dir.join(format!("{base}.{ext}"));
    if !exists(&first) {
        return first;
    }
    (2..)
        .map(|n| settings.out_dir.join(format!("{base} ({n}).{ext}")))
        .find(|path| !exists(path))
        .unwrap_or(first)
}

/// Everything that stays, in order: whole clips and the pieces left between removed stretches.
fn pieces(clips: &[MediaInfo], removed: &[Vec<(f64, f64)>]) -> Vec<Segment> {
    let mut pieces = Vec::new();
    for (clip, info) in clips.iter().enumerate() {
        let cuts = removed.get(clip).map_or(&[][..], Vec::as_slice);
        for (start, end) in kept_ranges(info.duration, cuts) {
            pieces.push(Segment { clip, start, duration: end - start });
        }
    }
    pieces
}

fn segments_between(pieces: &[Segment], start: f64, end: f64) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut piece_start = 0.0;
    for piece in pieces {
        let piece_end = piece_start + piece.duration;
        let (from, to) = (start.max(piece_start), end.min(piece_end));
        if to - from > 0.01 {
            segments.push(Segment { clip: piece.clip, start: piece.start + from - piece_start, duration: to - from });
        }
        piece_start = piece_end;
    }
    segments
}

/// Work out everything an export needs. `encoders` is what this FFmpeg can do, `exists` protects
/// files that are already there.
pub fn build(
    clips: &[MediaInfo],
    settings: &Settings,
    encoders: &HashSet<String>,
    exists: &dyn Fn(&Path) -> bool,
) -> Plan {
    let pieces = pieces(clips, &settings.removed);
    let total_duration: f64 = pieces.iter().map(|piece| piece.duration).sum();
    let (width, height) = match settings.frame {
        Some((w, h)) => (even(w as f64), even(h as f64)),
        None => source_frame(clips),
    };
    let spec = settings.format.spec();
    let is_gif = spec.is_some_and(|format| format.ext == "gif");
    let source_fps = clips.iter().map(|clip| clip.fps).fold(0.0, f64::max);
    let mut fps = source_fps.clamp(1.0, 60.0);
    if is_gif {
        fps = fps.min(15.0);
    }
    let video_encoder = spec.and_then(|format| pick(format.video, encoders));
    let audio_encoder = spec.and_then(|format| pick(format.audio, encoders));
    let can_carry_sound = !is_gif && (spec.is_none() || audio_encoder.is_some());
    let has_audio = can_carry_sound && clips.iter().any(|clip| clip.has_audio);
    let source_kbps = if total_duration > 0.0 {
        clips.iter().map(|clip| clip.kbps() * clip.duration).sum::<f64>() / total_duration
    } else {
        0.0
    };

    let count = settings.limits_mb.len().max(1);
    let part_duration = total_duration / count as f64;
    let ext = settings.format.extension();
    let mut parts = Vec::with_capacity(count);
    for index in 0..count {
        let limit_mb = settings.limits_mb.get(index).copied().flatten();
        let limit_bytes = limit_mb.map(|mb| (mb * MB) as u64);
        let start = part_duration * index as f64;
        let end = if index + 1 == count { total_duration } else { part_duration * (index + 1) as f64 };
        let mut audio_kbps = if has_audio { 128 } else { 0 };
        let (video_kbps, quality) = match limit_bytes {
            None => (None, Quality::Best),
            Some(_) if is_gif => (None, Quality::Good),
            Some(bytes) => {
                let total_kbps = bytes as f64 * 8.0 / 1000.0 / part_duration.max(0.001) * SIZE_MARGIN;
                if has_audio {
                    audio_kbps = audio_share(total_kbps);
                }
                let budget = total_kbps - audio_kbps as f64;
                if budget < MIN_VIDEO_KBPS {
                    (Some(budget.max(0.0)), Quality::TooSmall)
                } else {
                    let kbps = budget.min(useful_ceiling_kbps(width, height, fps, source_kbps));
                    (Some(kbps), rate_quality(kbps, width, height, fps, video_encoder.as_deref()))
                }
            }
        };
        parts.push(PartPlan {
            index,
            start,
            duration: end - start,
            limit_bytes,
            video_kbps,
            audio_kbps,
            quality,
            output: output_path(settings, index, count, &ext, exists),
            segments: segments_between(&pieces, start, end),
        });
    }

    let problem = if clips.is_empty() {
        Some("Add a video to get started.".to_string())
    } else if total_duration < 0.1 {
        Some("Everything has been cut out. Undo a cut to bring some back.".to_string())
    } else if settings.base_name.trim().is_empty() {
        Some("Give your video a name.".to_string())
    } else if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some("Enter a file format, like mkv.".to_string())
    } else if spec.is_some_and(|format| video_encoder.is_none() && !format_available(format, encoders)) {
        Some(format!("Your FFmpeg can't write {} files.", ext.to_uppercase()))
    } else {
        parts.iter().find(|part| part.quality == Quality::TooSmall).map(|part| {
            format!("Part {} can't fit in that size. Raise its limit or split into more parts.", part.index + 1)
        })
    };

    Plan {
        clips: clips.to_vec(),
        total_duration,
        width,
        height,
        fps,
        fit: settings.fit,
        video_encoder,
        audio_encoder,
        has_audio,
        ext,
        is_gif,
        parts,
        problem,
    }
}

fn push(args: &mut Vec<OsString>, items: &[&str]) {
    args.extend(items.iter().map(OsString::from));
}

impl Plan {
    /// Two passes hit a size target far more closely; only some encoders support it.
    pub fn two_pass(&self, part: &PartPlan) -> bool {
        part.video_kbps.is_some() && matches!(self.video_encoder.as_deref(), Some("libx264" | "libvpx-vp9" | "libvpx"))
    }

    /// The joining and resizing for one result video.
    pub fn filter_graph(&self, part: &PartPlan, width: u32, height: u32, with_audio: bool) -> String {
        let mut graph = String::new();
        let mut labels = String::new();
        let pixel_format = if self.is_gif { "" } else { ",format=yuv420p" };
        for (input, segment) in part.segments.iter().enumerate() {
            graph.push_str(&format!(
                "[{input}:v:0]setpts=PTS-STARTPTS,{},setsar=1,fps={:.3}{pixel_format}[v{input}];",
                fit_filter(width, height, self.fit),
                self.fps,
            ));
            labels.push_str(&format!("[v{input}]"));
            if with_audio {
                let duration = segment.duration;
                if self.clips[segment.clip].has_audio {
                    graph.push_str(&format!(
                        "[{input}:a:0]aresample=48000,aformat=channel_layouts=stereo,apad,atrim=0:{duration:.3},asetpts=PTS-STARTPTS[a{input}];"
                    ));
                } else {
                    graph.push_str(&format!(
                        "anullsrc=r=48000:cl=stereo,atrim=0:{duration:.3},asetpts=PTS-STARTPTS[a{input}];"
                    ));
                }
                labels.push_str(&format!("[a{input}]"));
            }
        }
        let audio_count = i32::from(with_audio);
        graph.push_str(&format!("{labels}concat=n={}:v=1:a={audio_count}", part.segments.len()));
        if self.is_gif {
            graph.push_str("[joined];[joined]split[first][second];[first]palettegen=stats_mode=diff[palette];[second][palette]paletteuse=dither=bayer:bayer_scale=3[v]");
        } else if with_audio {
            graph.push_str("[v][a]");
        } else {
            graph.push_str("[v]");
        }
        graph
    }

    /// The FFmpeg arguments for one run. `scale` shrinks the picture (only GIFs use it, to meet a limit).
    pub fn args(&self, part: &PartPlan, pass: Pass, video_kbps: Option<f64>, scale: f64, passlog: &Path) -> Vec<OsString> {
        let mut args = Vec::new();
        push(&mut args, &["-y", "-hide_banner", "-loglevel", "level+info", "-nostats", "-progress", "pipe:1"]);
        for segment in &part.segments {
            push(&mut args, &["-ss", &format!("{:.3}", segment.start), "-t", &format!("{:.3}", segment.duration), "-i"]);
            args.push(self.clips[segment.clip].path.as_os_str().to_os_string());
        }
        let (width, height) = if self.is_gif {
            // Very wide GIFs are huge and rarely wanted, start at a size that can share.
            let start = (480.0 / self.width as f64).min(1.0) * scale;
            (even(self.width as f64 * start).max(16), even(self.height as f64 * start).max(16))
        } else {
            (self.width, self.height)
        };
        let with_audio = self.has_audio && pass == Pass::Final;
        push(&mut args, &["-filter_complex", &self.filter_graph(part, width, height, with_audio), "-map", "[v]"]);
        if with_audio {
            push(&mut args, &["-map", "[a]"]);
        }
        self.video_args(&mut args, part, pass, video_kbps, passlog);
        if pass == Pass::First {
            push(&mut args, &["-an", "-f", "null", "-"]);
            return args;
        }
        if with_audio {
            if let Some(encoder) = &self.audio_encoder {
                push(&mut args, &["-c:a", encoder]);
            }
            push(&mut args, &["-b:a", &format!("{}k", part.audio_kbps), "-ar", "48000"]);
        }
        if matches!(self.ext.as_str(), "mp4" | "mov" | "m4v") {
            push(&mut args, &["-movflags", "+faststart"]);
        }
        args.push(part.output.as_os_str().to_os_string());
        args
    }

    fn video_args(&self, args: &mut Vec<OsString>, part: &PartPlan, pass: Pass, video_kbps: Option<f64>, passlog: &Path) {
        let encoder = self.video_encoder.as_deref();
        if let Some(encoder) = encoder {
            push(args, &["-c:v", encoder]);
        }
        match (encoder, video_kbps) {
            (Some("libx264"), Some(kbps)) => push(args, &["-preset", "medium", "-b:v", &format!("{kbps:.0}k")]),
            (Some("libx264"), None) => push(args, &["-preset", "medium", "-crf", NO_LIMIT_CRF_X264]),
            (Some("libvpx-vp9"), Some(kbps)) => {
                push(args, &["-row-mt", "1", "-cpu-used", "3", "-b:v", &format!("{kbps:.0}k")])
            }
            (Some("libvpx-vp9"), None) => push(args, &["-row-mt", "1", "-cpu-used", "3", "-crf", NO_LIMIT_CRF_VP9, "-b:v", "0"]),
            (Some("libvpx"), Some(kbps)) => push(args, &["-cpu-used", "3", "-b:v", &format!("{kbps:.0}k")]),
            (Some("libvpx"), None) => push(args, &["-cpu-used", "3", "-crf", "10", "-b:v", "2M"]),
            (Some("gif"), _) => {}
            (Some("mpeg4" | "libxvid" | "wmv2" | "msmpeg4" | "mpeg2video"), None) => push(args, &["-q:v", "3"]),
            (_, Some(kbps)) => push(args, &["-b:v", &format!("{kbps:.0}k")]),
            (_, None) => {}
        }
        if self.two_pass(part) {
            push(args, &["-pass", if pass == Pass::First { "1" } else { "2" }, "-passlogfile"]);
            args.push(passlog.as_os_str().to_os_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(name: &str, duration: f64, audio: bool) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from(format!("/in/{name}")),
            name: name.to_string(),
            duration,
            width: 1920,
            height: 1080,
            fps: 30.0,
            has_audio: audio,
            bytes: (duration * 2_000_000.0) as u64,
        }
    }

    fn settings(limits: Vec<Option<f64>>) -> Settings {
        Settings {
            format: FormatChoice::Listed(0),
            frame: None,
            fit: Fit::Bars,
            limits_mb: limits,
            out_dir: PathBuf::from("/out"),
            base_name: "movie".to_string(),
            removed: Vec::new(),
        }
    }

    fn encoders() -> HashSet<String> {
        ["libx264", "aac", "libvpx-vp9", "libopus", "gif"].iter().map(|s| s.to_string()).collect()
    }

    fn make(clips: &[MediaInfo], settings: &Settings) -> Plan {
        build(clips, settings, &encoders(), &|_| false)
    }

    #[test]
    fn splits_the_timeline_into_equal_parts_across_clips() {
        let clips = vec![clip("a", 40.0, true), clip("b", 20.0, true)];
        let plan = make(&clips, &settings(vec![Some(20.0); 3]));
        assert_eq!(plan.parts.len(), 3);
        for part in &plan.parts {
            assert!((part.duration - 20.0).abs() < 1e-6);
        }
        assert_eq!(plan.parts[0].segments, vec![Segment { clip: 0, start: 0.0, duration: 20.0 }]);
        assert_eq!(plan.parts[1].segments, vec![Segment { clip: 0, start: 20.0, duration: 20.0 }]);
        assert_eq!(plan.parts[2].segments, vec![Segment { clip: 1, start: 0.0, duration: 20.0 }]);
    }

    #[test]
    fn a_part_crossing_a_join_uses_both_clips() {
        let clips = vec![clip("a", 30.0, true), clip("b", 30.0, true)];
        let plan = make(&clips, &settings(vec![None; 3]));
        assert_eq!(plan.parts[1].segments.len(), 2);
        assert_eq!(plan.parts[1].segments[0], Segment { clip: 0, start: 20.0, duration: 10.0 });
        assert_eq!(plan.parts[1].segments[1], Segment { clip: 1, start: 0.0, duration: 10.0 });
    }

    #[test]
    fn budget_stays_under_the_limit() {
        let clips = vec![clip("a", 200.0, true)];
        let plan = make(&clips, &settings(vec![Some(20.0)]));
        let part = &plan.parts[0];
        let kbps = part.video_kbps.unwrap() + part.audio_kbps as f64;
        assert!(kbps * 1000.0 / 8.0 * part.duration <= 20.0 * MB);
        assert!(kbps > 700.0, "should use the room it has, got {kbps}");
    }

    #[test]
    fn equal_limits_mean_equal_quality() {
        let clips = vec![clip("a", 100.0, true)];
        let plan = make(&clips, &settings(vec![Some(5.0); 4]));
        let rate = plan.parts[0].video_kbps.unwrap();
        assert!(plan.parts.iter().all(|part| (part.video_kbps.unwrap() - rate).abs() < 1e-6));
    }

    #[test]
    fn short_clips_dont_balloon_to_the_limit() {
        let clips = vec![clip("a", 5.0, true)];
        let plan = make(&clips, &settings(vec![Some(500.0)]));
        // 5s at 500 MB would be 800 Mbps; the source is only 16 Mbps.
        assert!(plan.parts[0].video_kbps.unwrap() < 30_000.0);
    }

    #[test]
    fn an_impossible_limit_is_refused_with_a_reason() {
        let clips = vec![clip("a", 3600.0, true)];
        let plan = make(&clips, &settings(vec![Some(1.0)]));
        assert_eq!(plan.parts[0].quality, Quality::TooSmall);
        assert!(plan.problem.unwrap().contains("Part 1"));
    }

    #[test]
    fn clips_without_sound_get_silence_when_others_have_it() {
        let clips = vec![clip("a", 10.0, true), clip("b", 10.0, false)];
        let plan = make(&clips, &settings(vec![None]));
        let graph = plan.filter_graph(&plan.parts[0], 1280, 720, true);
        assert!(graph.contains("[0:a:0]"));
        assert!(!graph.contains("[1:a:0]"));
        assert!(graph.contains("anullsrc"));
        assert!(graph.contains("concat=n=2:v=1:a=1[v][a]"));
    }

    #[test]
    fn no_sound_anywhere_means_no_audio_stream() {
        let clips = vec![clip("a", 10.0, false)];
        let plan = make(&clips, &settings(vec![None]));
        assert!(!plan.has_audio);
        let args = plan.args(&plan.parts[0], Pass::Final, None, 1.0, Path::new("/tmp/x"));
        assert!(!args.iter().any(|arg| arg == "-c:a"));
    }

    #[test]
    fn two_pass_first_run_writes_nothing() {
        let clips = vec![clip("a", 100.0, true)];
        let plan = make(&clips, &settings(vec![Some(10.0)]));
        let part = &plan.parts[0];
        assert!(plan.two_pass(part));
        let first = plan.args(part, Pass::First, part.video_kbps, 1.0, Path::new("/tmp/x"));
        assert!(first.windows(2).any(|pair| pair[0] == "-f" && pair[1] == "null"));
        assert!(!first.iter().any(|arg| arg.to_string_lossy().contains("movie")));
        let last = plan.args(part, Pass::Final, part.video_kbps, 1.0, Path::new("/tmp/x"));
        assert!(last.last().unwrap().to_string_lossy().ends_with("movie.mp4"));
    }

    #[test]
    fn existing_files_are_never_overwritten() {
        let clips = vec![clip("a", 10.0, false)];
        let taken = PathBuf::from("/out/movie.mp4");
        let plan = build(&clips, &settings(vec![None]), &encoders(), &|path| path == taken);
        assert_eq!(plan.parts[0].output, PathBuf::from("/out/movie (2).mp4"));
    }

    #[test]
    fn parts_are_named_in_a_sortable_way() {
        let clips = vec![clip("a", 100.0, false)];
        let plan = make(&clips, &settings(vec![None; 12]));
        assert!(plan.parts[0].output.ends_with("movie_part01of12.mp4"));
        assert!(plan.parts[11].output.ends_with("movie_part12of12.mp4"));
    }

    #[test]
    fn webm_picks_webm_codecs() {
        let clips = vec![clip("a", 100.0, true)];
        let mut chosen = settings(vec![Some(10.0)]);
        chosen.format = FormatChoice::Listed(FORMATS.iter().position(|f| f.ext == "webm").unwrap());
        let plan = make(&clips, &chosen);
        assert_eq!(plan.video_encoder.as_deref(), Some("libvpx-vp9"));
        assert_eq!(plan.audio_encoder.as_deref(), Some("libopus"));
    }

    #[test]
    fn a_typed_format_leaves_codecs_to_ffmpeg() {
        let clips = vec![clip("a", 100.0, true)];
        let mut chosen = settings(vec![None]);
        chosen.format = FormatChoice::Other(".OGV".to_string());
        let plan = make(&clips, &chosen);
        assert_eq!(plan.ext, "ogv");
        assert!(plan.video_encoder.is_none());
        assert!(plan.problem.is_none());
    }

    #[test]
    fn removed_stretches_are_left_out() {
        let clips = vec![clip("a", 30.0, true), clip("b", 30.0, true)];
        let mut chosen = settings(vec![None; 2]);
        chosen.removed = vec![vec![(10.0, 20.0)], vec![(0.0, 5.0), (25.0, 40.0)]];
        let plan = make(&clips, &chosen);
        // 20 s of a and 20 s of b stay, cut into two parts of 20 s.
        assert!((plan.total_duration - 40.0).abs() < 1e-6);
        assert_eq!(
            plan.parts[0].segments,
            vec![Segment { clip: 0, start: 0.0, duration: 10.0 }, Segment { clip: 0, start: 20.0, duration: 10.0 }]
        );
        assert_eq!(plan.parts[1].segments, vec![Segment { clip: 1, start: 5.0, duration: 20.0 }]);
    }

    #[test]
    fn overlapping_cuts_merge() {
        assert_eq!(merge_ranges(&[(5.0, 8.0), (1.0, 3.0), (2.0, 6.0)]), vec![(1.0, 8.0)]);
        assert_eq!(kept_ranges(10.0, &[(0.0, 2.0), (9.99, 12.0)]), vec![(2.0, 9.99)]);
    }

    #[test]
    fn cutting_everything_is_explained() {
        let clips = vec![clip("a", 10.0, true)];
        let mut chosen = settings(vec![None]);
        chosen.removed = vec![vec![(0.0, 10.0)]];
        assert!(make(&clips, &chosen).problem.unwrap().contains("cut out"));
    }

    #[test]
    fn picture_sizes_stay_even() {
        let mut odd = clip("a", 10.0, false);
        odd.width = 853;
        odd.height = 479;
        let plan = make(&[odd], &settings(vec![None]));
        assert_eq!((plan.width % 2, plan.height % 2), (0, 0));
    }
}
