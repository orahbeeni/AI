//! Turning a [`Job`] (one or more clips, each with the ranges to keep and an optional crop, joined end to
//! end, optionally with a size ceiling) into a file in the chosen [`Format`].

use crate::ffmpeg::{self, Info, Tools};
use std::ffi::OsString;
use std::fmt;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// A rough guess of the bits per pixel per frame H.264 needs, used for the on-screen plan and as the fallback when
/// the picture size cannot be chosen by testing (see [`choose_height`]).
const BITS_PER_PIXEL: f64 = 0.04;
/// Heights the picture is scaled down to when the size budget is too small for the full size.
const HEIGHT_LADDER: [u32; 10] = [2160, 1440, 1080, 900, 720, 540, 480, 360, 270, 240];
/// With a size limit the frame rate is capped here: 60 fps costs bits that are better spent on sharpness.
const SIZE_LIMITED_MAX_FPS: f64 = 30.0;
/// The picture is tested at constant quality (CRF) on samples; the biggest size that still fits is used.
const FLOOR_CRF: u32 = 28;
/// The samples: this much video in total, in windows of this length.
const SAMPLE_TOTAL: f64 = 12.0;
const SAMPLE_WINDOW: f64 = 2.0;
/// Encodes tried before giving up on fitting the size ceiling.
const MAX_ATTEMPTS: usize = 4;
/// Share of the size ceiling the first attempt aims for, leaving room for the container and rate-control error.
const FIRST_MARGIN: f64 = 0.96;
/// Constant bit rates MP3 allows, in kbit/s.
const MP3_RATES: [u32; 14] = [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
/// Sample rate every clip's sound is brought to before joining.
const AUDIO_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mp4,
    Mov,
    Mkv,
    WebM,
    Gif,
    Mp3,
}

impl Format {
    pub const ALL: [Format; 6] = [Format::Mp4, Format::Mov, Format::Mkv, Format::WebM, Format::Gif, Format::Mp3];

    pub fn label(self) -> &'static str {
        match self {
            Format::Mp4 => "MP4 (H.264 + AAC)",
            Format::Mov => "MOV (H.264 + AAC)",
            Format::Mkv => "MKV (H.264 + AAC)",
            Format::WebM => "WebM (VP9 + Opus)",
            Format::Gif => "GIF (animated, no sound)",
            Format::Mp3 => "MP3 (sound only)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::Mov => "mov",
            Format::Mkv => "mkv",
            Format::WebM => "webm",
            Format::Gif => "gif",
            Format::Mp3 => "mp3",
        }
    }

    pub fn has_video(self) -> bool {
        self != Format::Mp3
    }

    pub fn has_audio(self) -> bool {
        self != Format::Gif
    }

    /// Formats whose size is steered with a bit rate (two passes) rather than by shrinking.
    fn two_pass(self) -> bool {
        matches!(self, Format::Mp4 | Format::Mov | Format::Mkv | Format::WebM)
    }
}

/// A rectangle of the picture, in pixels of the source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Crop {
    /// Clamped into the frame and rounded down to even numbers, which H.264 (4:2:0) requires.
    pub fn normalized(self, frame_w: u32, frame_h: u32) -> Option<Crop> {
        let x = self.x.min(frame_w) & !1;
        let y = self.y.min(frame_h) & !1;
        let w = self.w.min(frame_w - x) & !1;
        let h = self.h.min(frame_h - y) & !1;
        (w >= 16 && h >= 16).then_some(Crop { x, y, w, h })
    }
}

/// A stretch of a clip, `(from, to)` in seconds.
pub type Span = (f64, f64);

#[derive(Clone, Debug)]
pub struct ClipJob {
    pub input: PathBuf,
    pub info: Info,
    /// The parts to keep, in play order.
    pub spans: Vec<Span>,
    pub crop: Option<Crop>,
}

impl ClipJob {
    fn crop(&self) -> Option<Crop> {
        self.crop.and_then(|c| c.normalized(self.info.width, self.info.height))
    }

    /// Size of the picture after cropping.
    fn picture(&self) -> (u32, u32) {
        self.crop().map_or((self.info.width, self.info.height), |c| (c.w, c.h))
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    /// Joined in this order.
    pub clips: Vec<ClipJob>,
    pub output: PathBuf,
    pub format: Format,
    /// The file must not be bigger than this. `None` means good quality, whatever the size.
    pub target_bytes: Option<u64>,
}

impl Job {
    pub fn duration(&self) -> f64 {
        self.clips.iter().flat_map(|c| &c.spans).map(|(a, b)| b - a).sum::<f64>().max(0.01)
    }

    /// The picture size of the result: the first clip's, after cropping. Other clips are fitted into it.
    pub fn canvas(&self) -> (u32, u32) {
        let (w, h) = self.clips.first().map_or((2, 2), ClipJob::picture);
        ((w & !1).max(2), (h & !1).max(2))
    }

    fn fps(&self) -> f64 {
        self.clips.first().map_or(30.0, |c| c.info.fps)
    }

    /// The frame rate of the result.
    fn out_fps(&self) -> f64 {
        let fps = self.fps().clamp(1.0, 120.0);
        if self.target_bytes.is_some() { fps.min(SIZE_LIMITED_MAX_FPS) } else { fps }
    }

    fn has_audio(&self) -> bool {
        self.format.has_audio() && self.clips.iter().any(|c| c.info.has_audio)
    }

    /// The highest bit rate any clip had; encoding above that buys nothing. `None` if a clip does not say.
    fn source_kbps(&self) -> Option<u32> {
        self.clips.iter().map(|c| c.info.bitrate_kbps).collect::<Option<Vec<_>>>()?.into_iter().max()
    }

    /// Every kept span of every clip, in output order; each becomes one input of ffmpeg.
    fn pieces(&self) -> impl Iterator<Item = (&ClipJob, Span)> {
        self.clips.iter().flat_map(|c| c.spans.iter().map(move |s| (c, *s)))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub video_kbps: u32,
    pub audio_kbps: u32,
    /// Scale the picture down to this height (the width follows).
    pub height: Option<u32>,
}

/// Works out bit rates (and whether to shrink the picture) so the file lands inside `target_bytes`.
/// `margin` is the share of the ceiling to aim for. GIFs have no bit rate; they are shrunk by trial instead.
pub fn plan(job: &Job, target_bytes: u64, margin: f64) -> Result<Plan, String> {
    let too_small = || {
        format!("{:.1} MB is too small for {:.0} s; pick a shorter part or a bigger size", target_bytes as f64 / 1e6, job.duration())
    };
    if job.format == Format::Gif {
        return Ok(Plan { video_kbps: 0, audio_kbps: 0, height: None });
    }
    let total_kbps = target_bytes as f64 * 8.0 * margin / job.duration() / 1000.0;
    if job.format == Format::Mp3 {
        let kbps = MP3_RATES.iter().copied().rev().find(|&r| r as f64 <= total_kbps).ok_or_else(too_small)?;
        return Ok(Plan { video_kbps: 0, audio_kbps: kbps, height: None });
    }

    let audio_kbps = match (job.has_audio(), total_kbps) {
        (false, _) => 0,
        (_, t) if t >= 1500.0 => 128,
        (_, t) if t >= 600.0 => 96,
        (_, t) if t >= 250.0 => 64,
        _ => 32,
    };
    let video_kbps = total_kbps - audio_kbps as f64;
    if video_kbps < 40.0 {
        return Err(too_small());
    }

    // Decide the size from what the budget allows ...
    let (w, h) = job.canvas();
    let fps = job.out_fps().clamp(15.0, 60.0);
    let pixels = video_kbps * 1000.0 / (BITS_PER_PIXEL * fps);
    let fits_height = (pixels * h as f64 / w as f64).sqrt();
    let height = if fits_height >= h as f64 {
        None
    } else {
        let rung = HEIGHT_LADDER.iter().copied().find(|&l| l < h && l as f64 <= fits_height).unwrap_or(240);
        (rung < h).then_some(rung)
    };
    // ... then don't spend more bits than the source had; a small file is fine.
    let video_kbps = job.source_kbps().map_or(video_kbps, |cap| video_kbps.min(cap as f64));
    Ok(Plan { video_kbps: video_kbps.round().max(1.0) as u32, audio_kbps, height })
}

#[derive(Debug)]
pub enum Error {
    Cancelled,
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Cancelled => write!(f, "cancelled"),
            Error::Failed(msg) => write!(f, "{msg}"),
        }
    }
}

pub enum Event {
    Status(String),
    /// Overall progress, 0..1.
    Progress(f32),
}

pub struct Done {
    pub bytes: u64,
    /// What came out, for showing: picture size and frame rate, or the sound's bit rate.
    pub detail: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Pass {
    /// One pass: constant quality, or a fixed bit rate for MP3 and the GIF.
    Single,
    /// First of two passes at a target bit rate: analysis only, no output.
    First,
    Second,
}

#[derive(Clone, Copy)]
struct GifLook {
    width: u32,
    fps: f64,
}

fn secs(t: f64) -> OsString {
    format!("{t:.3}").into()
}

/// The filter graph: every kept span of every clip is brought to the same picture size, frame rate and sound
/// format, then they are joined. `audio` is off for the analysis pass.
fn graph(job: &Job, height: Option<u32>, audio: bool, gif: Option<GifLook>) -> String {
    let (cw, ch) = job.canvas();
    let fps = gif.map_or(job.out_fps(), |g| g.fps);
    let video = job.format.has_video();
    let mut parts = Vec::new();
    let mut joined = String::new();
    let mut count = 0;
    for (clip, (from, to)) in job.pieces() {
        let d = to - from;
        if video {
            let mut chain = Vec::new();
            if let Some(c) = clip.crop() {
                chain.push(format!("crop={}:{}:{}:{}", c.w, c.h, c.x, c.y));
            }
            if clip.picture() != (cw, ch) {
                chain.push(format!("scale={cw}:{ch}:flags=lanczos:force_original_aspect_ratio=decrease,pad={cw}:{ch}:(ow-iw)/2:(oh-ih)/2"));
            }
            chain.extend(["setsar=1".to_string(), format!("fps={fps:.3}"), format!("trim=duration={d:.3}"), "setpts=PTS-STARTPTS".into()]);
            parts.push(format!("[{count}:v:0]{}[v{count}]", chain.join(",")));
            joined.push_str(&format!("[v{count}]"));
        }
        if audio {
            if clip.info.has_audio {
                parts.push(format!(
                    "[{count}:a:0]aresample={AUDIO_RATE},aformat=sample_fmts=fltp:channel_layouts=stereo,atrim=duration={d:.3},asetpts=PTS-STARTPTS[a{count}]"
                ));
            } else {
                // A clip without sound gets silence, so the sound of the others stays in step with the picture.
                parts.push(format!("anullsrc=r={AUDIO_RATE}:cl=stereo:d={d:.3},aformat=sample_fmts=fltp[a{count}]"));
            }
            joined.push_str(&format!("[a{count}]"));
        }
        count += 1;
    }
    let mut outs = String::new();
    if video {
        outs.push_str("[v]");
    }
    if audio {
        outs.push_str("[a]");
    }
    parts.push(format!("{joined}concat=n={count}:v={}:a={}{outs}", video as u8, audio as u8));

    if video {
        parts.push(match (gif, height) {
            (Some(g), _) => format!(
                "[v]scale={}:-2:flags=lanczos,split[pa][pb];[pa]palettegen=stats_mode=diff[pal];[pb][pal]paletteuse=dither=bayer:bayer_scale=5[vout]",
                g.width
            ),
            (None, Some(h)) => format!("[v]scale=-2:{h}:flags=lanczos[vout]"),
            (None, None) => "[v]null[vout]".into(),
        });
    }
    parts.join(";")
}

/// The start of every ffmpeg command, with each kept span of each clip as an input.
fn input_args(job: &Job) -> Vec<OsString> {
    let mut a: Vec<OsString> = ["-hide_banner", "-nostdin", "-y", "-v", "error", "-progress", "pipe:1", "-nostats"].map(Into::into).to_vec();
    for (clip, (from, to)) in job.pieces() {
        if from > 0.0 {
            a.extend(["-ss".into(), secs(from)]);
        }
        a.extend(["-t".into(), secs(to - from), "-i".into(), clip.input.clone().into()]);
    }
    a
}

fn build_args(job: &Job, plan: Option<&Plan>, gif: Option<GifLook>, pass: Pass, log_prefix: &Path, output: &Path) -> Vec<OsString> {
    let mut a = input_args(job);
    // Slower presets squeeze more picture out of every bit, which matters when the size is limited.
    let preset = if plan.is_some() { "slow" } else { "medium" };
    let audio = job.has_audio() && pass != Pass::First;
    a.extend(["-filter_complex".into(), graph(job, plan.and_then(|p| p.height), audio, gif).into()]);
    if job.format.has_video() {
        a.extend(["-map", "[vout]"].map(OsString::from));
    }
    if audio {
        a.extend(["-map", "[a]"].map(OsString::from));
    }

    let rate = |codec: &[&str], quality: &[&str]| -> Vec<OsString> {
        let mut v: Vec<OsString> = codec.iter().map(OsString::from).collect();
        match (pass, plan) {
            (Pass::Single, _) | (_, None) => v.extend(quality.iter().map(OsString::from)),
            (pass, Some(plan)) => {
                let n = if pass == Pass::First { "1" } else { "2" };
                v.extend(["-b:v".into(), format!("{}k", plan.video_kbps).into(), "-pass".into(), n.into(), "-passlogfile".into(), log_prefix.into()]);
            }
        }
        v
    };
    match job.format {
        Format::Mp4 | Format::Mov | Format::Mkv => {
            a.extend(rate(&["-c:v", "libx264", "-preset", preset, "-pix_fmt", "yuv420p"], &["-crf", "20"]));
            if audio {
                a.extend(["-c:a".into(), "aac".into(), "-b:a".into(), format!("{}k", plan.map_or(160, |p| p.audio_kbps)).into()]);
            }
            if job.format != Format::Mkv {
                a.extend(["-movflags", "+faststart"].map(OsString::from));
            }
        }
        Format::WebM => {
            a.extend(rate(&["-c:v", "libvpx-vp9", "-deadline", "good", "-cpu-used", "4", "-row-mt", "1", "-pix_fmt", "yuv420p"], &["-crf", "32", "-b:v", "0"]));
            if audio {
                a.extend(["-c:a".into(), "libopus".into(), "-b:a".into(), format!("{}k", plan.map_or(128, |p| p.audio_kbps)).into()]);
            }
        }
        Format::Gif => a.extend(["-loop", "0"].map(OsString::from)),
        Format::Mp3 => match plan {
            Some(p) => a.extend(["-c:a".into(), "libmp3lame".into(), "-b:a".into(), format!("{}k", p.audio_kbps).into()]),
            None => a.extend(["-c:a", "libmp3lame", "-q:a", "2"].map(OsString::from)),
        },
    }
    if pass == Pass::First {
        a.extend(["-an", "-f", "null", "-"].map(OsString::from));
    } else {
        a.push(output.into());
    }
    a
}

struct Ctx<'a> {
    tools: &'a Tools,
    cancel: &'a AtomicBool,
    report: &'a dyn Fn(Event),
    part: PathBuf,
    log_prefix: PathBuf,
}

/// Runs the job. Writes `job.output` only once the result is complete (and inside the size ceiling).
pub fn run(tools: &Tools, job: &Job, cancel: &AtomicBool, report: &dyn Fn(Event)) -> Result<Done, Error> {
    if job.clips.iter().any(|c| c.input == job.output) {
        return Err(Error::Failed("the output would overwrite one of the original videos".into()));
    }
    if job.clips.is_empty() || job.clips.iter().any(|c| c.spans.is_empty()) {
        return Err(Error::Failed("nothing is left to keep in one of the videos".into()));
    }
    let ctx = Ctx {
        tools,
        cancel,
        report,
        part: job.output.with_extension(format!("part.{}", job.format.extension())),
        log_prefix: std::env::temp_dir().join(format!(
            "video-pass-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
        )),
    };
    let result = run_inner(job, &ctx);
    let _ = std::fs::remove_file(&ctx.part);
    remove_pass_logs(&ctx.log_prefix);
    result
}

fn run_inner(job: &Job, ctx: &Ctx) -> Result<Done, Error> {
    let finish = |plan: Option<&Plan>, gif: Option<GifLook>| -> Result<Done, Error> {
        let bytes = std::fs::metadata(&ctx.part).map_err(|e| Error::Failed(e.to_string()))?.len();
        std::fs::rename(&ctx.part, &job.output).map_err(|e| Error::Failed(format!("cannot write {}: {e}", job.output.display())))?;
        Ok(Done { bytes, detail: describe(job, plan, gif) })
    };
    let size = || std::fs::metadata(&ctx.part).map(|m| m.len()).map_err(|e| Error::Failed(e.to_string()));
    let too_big = |target: u64, best: u64| {
        Error::Failed(format!(
            "could not get under {:.1} MB (best try was {:.1} MB); pick a shorter part or a bigger size",
            target as f64 / 1e6,
            best as f64 / 1e6
        ))
    };

    // GIF: no bit rate to set, so shrink it until it fits.
    if job.format == Format::Gif {
        let (cw, _) = job.canvas();
        let mut look = GifLook { width: cw.min(if job.target_bytes.is_some() { 480 } else { 640 }), fps: job.fps().clamp(1.0, 15.0) };
        let Some(target) = job.target_bytes else {
            run_passes(job, ctx, None, Some(look), &[Pass::Single], "")?;
            return finish(None, Some(look));
        };
        let mut last = 0;
        for attempt in 1..=MAX_ATTEMPTS + 2 {
            let tag = if attempt > 1 { format!(" (try {attempt})") } else { String::new() };
            run_passes(job, ctx, None, Some(look), &[Pass::Single], &tag)?;
            last = size()?;
            if last <= target {
                return finish(None, Some(look));
            }
            let shrink = (target as f64 / last as f64).sqrt() * 0.92;
            look.width = (((look.width as f64 * shrink) as u32) & !1).max(64);
            if attempt >= 3 {
                look.fps = look.fps.min(10.0);
            }
        }
        return Err(too_big(target, last));
    }

    let Some(target) = job.target_bytes else {
        run_passes(job, ctx, None, None, &[Pass::Single], "")?;
        return finish(None, None);
    };
    let passes: &[Pass] = if job.format.two_pass() { &[Pass::First, Pass::Second] } else { &[Pass::Single] };
    let mut margin = FIRST_MARGIN;
    let mut last = 0;
    for attempt in 1..=MAX_ATTEMPTS {
        let mut plan = plan(job, target, margin).map_err(Error::Failed)?;
        if job.format.has_video() {
            // Pick the picture size by testing the video itself; the guess in `plan` is only the fallback.
            match choose_height(job, ctx, plan.video_kbps) {
                Ok(height) => plan.height = height,
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => {}
            }
        }
        let tag = if attempt > 1 { format!(" (try {attempt})") } else { String::new() };
        run_passes(job, ctx, Some(&plan), None, passes, &tag)?;
        last = size()?;
        if last <= target {
            return finish(Some(&plan), None);
        }
        // Over the ceiling: aim lower by the overshoot, plus a little extra.
        margin *= target as f64 / last as f64 * 0.98;
    }
    Err(too_big(target, last))
}

fn run_passes(job: &Job, ctx: &Ctx, plan: Option<&Plan>, gif: Option<GifLook>, passes: &[Pass], tag: &str) -> Result<(), Error> {
    let total = job.duration();
    for (i, pass) in passes.iter().enumerate() {
        let status = match (passes.len(), pass) {
            (1, _) => format!("Converting{tag}…"),
            (_, Pass::First) => format!("Analysing, pass 1 of 2{tag}…"),
            _ => format!("Encoding, pass 2 of 2{tag}…"),
        };
        (ctx.report)(Event::Status(status));
        let args = build_args(job, plan, gif, *pass, &ctx.log_prefix, &ctx.part);
        let (base, share) = (i as f32 / passes.len() as f32, 1.0 / passes.len() as f32);
        run_ffmpeg(ctx.tools, &args, total, ctx.cancel, &mut |p| (ctx.report)(Event::Progress(base + p * share)))?;
    }
    Ok(())
}

/// A short description of what came out, for the window.
fn describe(job: &Job, plan: Option<&Plan>, gif: Option<GifLook>) -> String {
    if job.format == Format::Mp3 {
        return plan.map_or_else(|| "VBR".to_string(), |p| format!("{} kbit/s", p.audio_kbps));
    }
    let (cw, ch) = job.canvas();
    if let Some(g) = gif {
        return format!("{} px wide, {:.0} fps", g.width, g.fps);
    }
    let (w, h) = match plan.and_then(|p| p.height) {
        Some(h) => ((((cw as f64 * h as f64 / ch as f64).round() as u32) & !1).max(2), h),
        None => (cw, ch),
    };
    format!("{w}×{h}, {:.0} fps", job.out_fps())
}

/// A job made of short windows spread over the whole of `job`, for judging how hard the video is to compress.
/// Short jobs are used whole.
fn sample_job(job: &Job) -> Job {
    let total = job.duration();
    let mut sample = Job { clips: Vec::new(), output: job.output.clone(), format: Format::Mp4, target_bytes: None };
    if total <= SAMPLE_TOTAL * 1.25 {
        sample.clips = job.clips.clone();
        return sample;
    }
    // Every piece with where it starts on the joined timeline.
    let mut pieces = Vec::new();
    let mut offset = 0.0;
    for (i, clip) in job.clips.iter().enumerate() {
        for &(from, to) in &clip.spans {
            pieces.push((i, from, to, offset));
            offset += to - from;
        }
    }
    let windows = (SAMPLE_TOTAL / SAMPLE_WINDOW) as usize;
    let mut spans: Vec<Vec<Span>> = vec![Vec::new(); job.clips.len()];
    for w in 0..windows {
        let t = ((w as f64 + 0.5) * total / windows as f64 - SAMPLE_WINDOW / 2.0).max(0.0);
        if let Some(&(i, from, to, start)) = pieces.iter().find(|p| t >= p.3 && t < p.3 + (p.2 - p.1)) {
            let begin = from + (t - start);
            let end = (begin + SAMPLE_WINDOW).min(to);
            if end - begin >= 0.4 {
                spans[i].push((begin, end));
            }
        }
    }
    for (clip, spans) in job.clips.iter().zip(spans) {
        if !spans.is_empty() {
            sample.clips.push(ClipJob { spans, ..clip.clone() });
        }
    }
    if sample.clips.is_empty() {
        sample.clips = job.clips.clone();
    }
    sample
}

/// The biggest picture height whose estimated size, at constant quality [`FLOOR_CRF`], fits `video_kbps`.
/// `None` means the full size fits. Easy video (a screen recording, say) keeps its full size; hard video (a lot of
/// motion) is shrunk. Estimated by encoding samples quickly, so it takes seconds, not a full pass.
fn choose_height(job: &Job, ctx: &Ctx, video_kbps: u32) -> Result<Option<u32>, Error> {
    let (_, ch) = job.canvas();
    let budget = video_kbps as f64 * 1000.0 / 8.0 * job.duration();
    let sample = sample_job(job);
    let factor = job.duration() / sample.duration();

    let mut candidates = vec![ch];
    for rung in HEIGHT_LADDER {
        // Rungs within a few per cent of the one before would only look the same.
        if rung < ch && rung as f64 <= *candidates.last().unwrap_or(&ch) as f64 * 0.92 {
            candidates.push(rung);
        }
    }
    let out = ctx.log_prefix.with_extension("probe.mp4");
    for (i, &h) in candidates.iter().enumerate() {
        (ctx.report)(Event::Status(format!("Choosing the picture size ({h}p)…")));
        let height = (h < ch).then_some(h);
        let mut a = input_args(&sample);
        a.extend(["-filter_complex".into(), graph(&sample, height, false, None).into(), "-map".into(), "[vout]".into()]);
        a.extend(["-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-an"].map(OsString::from));
        a.extend(["-crf".into(), FLOOR_CRF.to_string().into(), out.clone().into()]);
        run_ffmpeg(ctx.tools, &a, sample.duration(), ctx.cancel, &mut |_| {})?;
        let bytes = std::fs::metadata(&out).map_err(|e| Error::Failed(e.to_string()))?.len() as f64 * factor;
        let _ = std::fs::remove_file(&out);
        if bytes <= budget || i == candidates.len() - 1 {
            return Ok(height);
        }
    }
    Ok(None)
}

fn remove_pass_logs(prefix: &Path) {
    let (Some(dir), Some(name)) = (prefix.parent(), prefix.file_name().and_then(|n| n.to_str())) else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_str().is_some_and(|n| n.starts_with(name)) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Runs one ffmpeg invocation, reporting 0..1 progress against `total` seconds of output.
fn run_ffmpeg(tools: &Tools, args: &[OsString], total: f64, cancel: &AtomicBool, progress: &mut dyn FnMut(f32)) -> Result<(), Error> {
    let mut child = ffmpeg::command(&tools.ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Failed(format!("cannot run ffmpeg: {e}")))?;
    let stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");

    // ffmpeg blocks if either pipe fills up, so both are drained on their own threads.
    let errors = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                    progress((us / 1e6 / total).clamp(0.0, 1.0) as f32);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().map_err(|e| Error::Failed(e.to_string()))?;
    let errors = errors.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        let tail: Vec<&str> = errors.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = tail[tail.len().saturating_sub(3)..].join("\n");
        Err(Error::Failed(format!("ffmpeg failed: {tail}")))
    }
}

/// Removes `cut` from a sorted list of spans, splitting a span if the cut is in its middle. Slivers shorter than
/// a tenth of a second are dropped.
pub fn subtract(spans: &[Span], cut: Span) -> Vec<Span> {
    const MIN: f64 = 0.1;
    let mut out = Vec::new();
    for &(a, b) in spans {
        if cut.1 <= a || cut.0 >= b {
            out.push((a, b));
            continue;
        }
        if cut.0 - a >= MIN {
            out.push((a, cut.0));
        }
        if b - cut.1 >= MIN {
            out.push((cut.1, b));
        }
    }
    out
}

/// The parts of `spans` that lie inside `keep`.
pub fn intersect(spans: &[Span], keep: Span) -> Vec<Span> {
    spans.iter().map(|&(a, b)| (a.max(keep.0), b.min(keep.1))).filter(|(a, b)| b - a >= 0.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(duration: f64, w: u32, h: u32, audio: bool) -> Info {
        Info { duration, width: w, height: h, fps: 30.0, has_audio: audio, bytes: 0, bitrate_kbps: None, codec: "h264".into() }
    }

    fn job(duration: f64, w: u32, h: u32, audio: bool) -> Job {
        Job {
            clips: vec![ClipJob { input: "in.mov".into(), info: info(duration, w, h, audio), spans: vec![(0.0, duration)], crop: None }],
            output: "out.mp4".into(),
            format: Format::Mp4,
            target_bytes: Some(20_000_000),
        }
    }

    #[test]
    fn short_clip_keeps_its_size() {
        let p = plan(&job(20.0, 1920, 1080, true), 20_000_000, FIRST_MARGIN).unwrap();
        assert_eq!(p.height, None);
        assert_eq!(p.audio_kbps, 128);
        // 20 MB * 0.96 over 20 s is 7.68 Mbit/s in total.
        assert_eq!(p.video_kbps, 7680 - 128);
    }

    #[test]
    fn long_clip_is_scaled_down() {
        let p = plan(&job(300.0, 1920, 1080, true), 20_000_000, FIRST_MARGIN).unwrap();
        assert_eq!(p.height, Some(360));
        assert_eq!(p.audio_kbps, 64);
        assert!(p.video_kbps < 520);
    }

    #[test]
    fn plan_never_exceeds_the_ceiling() {
        for secs in [5.0, 30.0, 90.0, 240.0, 600.0] {
            let j = job(secs, 1920, 1080, true);
            if let Ok(p) = plan(&j, 20_000_000, FIRST_MARGIN) {
                let bytes = (p.video_kbps + p.audio_kbps) as f64 * 1000.0 / 8.0 * secs;
                assert!(bytes < 20_000_000.0, "{secs}s planned {bytes}");
            }
        }
    }

    #[test]
    fn source_bitrate_caps_the_plan() {
        let mut j = job(10.0, 1280, 720, false);
        j.clips[0].info.bitrate_kbps = Some(2000);
        assert_eq!(plan(&j, 20_000_000, FIRST_MARGIN).unwrap().video_kbps, 2000);
    }

    #[test]
    fn hopeless_budget_is_an_error() {
        assert!(plan(&job(3600.0, 1920, 1080, true), 5_000_000, FIRST_MARGIN).is_err());
    }

    #[test]
    fn mp3_rate_comes_from_the_size() {
        let mut j = job(600.0, 1920, 1080, true);
        j.format = Format::Mp3;
        // 5 MB * 0.96 over 10 minutes is 64 kbit/s in total, so the biggest rate that fits is 64.
        assert_eq!(plan(&j, 5_000_000, FIRST_MARGIN).unwrap().audio_kbps, 64);
        assert!(plan(&j, 500_000, FIRST_MARGIN).is_err());
    }

    #[test]
    fn crop_is_clamped_and_even() {
        let c = Crop { x: 11, y: 5, w: 5000, h: 301 }.normalized(1920, 1080).unwrap();
        assert_eq!(c, Crop { x: 10, y: 4, w: 1910, h: 300 });
        assert_eq!(Crop { x: 0, y: 0, w: 8, h: 8 }.normalized(1920, 1080), None);
    }

    #[test]
    fn graph_joins_every_span_with_silence_for_quiet_clips() {
        let mut j = job(10.0, 1920, 1080, true);
        j.clips[0].spans = vec![(0.0, 2.0), (5.0, 8.0)];
        j.clips[0].crop = Some(Crop { x: 100, y: 50, w: 800, h: 600 });
        j.clips.push(ClipJob { input: "b.mp4".into(), info: info(4.0, 640, 360, false), spans: vec![(0.0, 4.0)], crop: None });
        let g = graph(&j, Some(480), true, None);
        assert!(g.contains("[0:v:0]crop=800:600:100:50,setsar=1"));
        assert!(g.contains("[1:v:0]crop=800:600:100:50"));
        assert!(g.contains("[2:v:0]scale=800:600:flags=lanczos:force_original_aspect_ratio=decrease,pad=800:600"));
        assert!(g.contains("anullsrc=r=48000:cl=stereo:d=4.000"));
        assert!(g.contains("[v0][a0][v1][a1][v2][a2]concat=n=3:v=1:a=1[v][a]"));
        assert!(g.ends_with("[v]scale=-2:480:flags=lanczos[vout]"));
        assert_eq!(j.duration(), 9.0);
        assert_eq!(j.canvas(), (800, 600));
    }

    #[test]
    fn analysis_pass_has_no_sound_in_the_graph() {
        let g = graph(&job(10.0, 1280, 720, true), None, false, None);
        assert!(!g.contains("[a]") && !g.contains("aresample"));
        assert!(g.contains("concat=n=1:v=1:a=0[v]"));
    }

    #[test]
    fn samples_cover_the_whole_job() {
        let mut j = job(300.0, 1920, 1080, true);
        // Short jobs are used whole.
        assert_eq!(sample_job(&job(10.0, 1280, 720, true)).duration(), 10.0);
        // A long job: 6 windows of 2 s, spread out, inside the kept spans.
        j.clips[0].spans = vec![(10.0, 100.0), (200.0, 290.0)];
        let sample = sample_job(&j);
        assert_eq!(sample.clips.len(), 1);
        assert_eq!(sample.clips[0].spans.len(), 6);
        assert!((sample.duration() - 12.0).abs() < 1e-9);
        for &(a, b) in &sample.clips[0].spans {
            assert!(j.clips[0].spans.iter().any(|&(x, y)| a >= x && b <= y), "{a}..{b} is outside the kept parts");
        }
        // Two clips: both are sampled.
        j.clips.push(ClipJob { input: "b.mp4".into(), info: info(100.0, 1280, 720, true), spans: vec![(0.0, 100.0)], crop: None });
        assert_eq!(sample_job(&j).clips.len(), 2);
    }

    #[test]
    fn size_limited_video_is_capped_at_30_fps() {
        let mut j = job(60.0, 1920, 1080, true);
        j.clips[0].info.fps = 60.0;
        assert_eq!(j.out_fps(), 30.0);
        j.target_bytes = None;
        assert_eq!(j.out_fps(), 60.0);
    }

    #[test]
    fn spans_are_cut_and_kept() {
        let whole = vec![(0.0, 60.0)];
        assert_eq!(subtract(&whole, (20.0, 30.0)), vec![(0.0, 20.0), (30.0, 60.0)]);
        assert_eq!(subtract(&whole, (0.0, 10.0)), vec![(10.0, 60.0)]);
        assert_eq!(subtract(&[(0.0, 20.0), (30.0, 60.0)], (15.0, 40.0)), vec![(0.0, 15.0), (40.0, 60.0)]);
        assert!(subtract(&whole, (0.0, 60.0)).is_empty());
        assert_eq!(intersect(&[(0.0, 20.0), (30.0, 60.0)], (10.0, 40.0)), vec![(10.0, 20.0), (30.0, 40.0)]);
    }

    fn make_clip(tools: &Tools, path: &Path, size: &str, audio: bool, secs: u32) {
        let mut cmd = ffmpeg::command(&tools.ffmpeg);
        cmd.args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc2=size={size}:rate=30")]);
        if audio {
            cmd.args(["-f", "lavfi", "-i", "sine=frequency=440"]);
        }
        cmd.args(["-t", &secs.to_string(), "-c:v", "libx264", "-pix_fmt", "yuv420p"]);
        if audio {
            cmd.args(["-c:a", "aac"]);
        }
        assert!(cmd.arg(path).status().unwrap().success());
    }

    /// Needs ffmpeg on the machine; every format, two clips, a removed middle part, a crop and a size ceiling.
    #[test]
    fn every_format_joins_cuts_and_stays_under_the_ceiling() {
        let Some(tools) = ffmpeg::locate() else { return };
        let dir = std::env::temp_dir().join(format!("video-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.mov"), dir.join("b.mp4"));
        make_clip(&tools, &a, "1280x720", true, 10);
        make_clip(&tools, &b, "640x360", false, 5);
        let (info_a, info_b) = (ffmpeg::probe(&tools, &a).unwrap(), ffmpeg::probe(&tools, &b).unwrap());

        for format in Format::ALL {
            let output = dir.join(format!("out.{}", format.extension()));
            let job = Job {
                clips: vec![
                    // 1 s..9 s with 3 s..6 s removed, cropped to 800x600.
                    ClipJob {
                        input: a.clone(),
                        info: info_a.clone(),
                        spans: subtract(&[(1.0, 9.0)], (3.0, 6.0)),
                        crop: Some(Crop { x: 100, y: 50, w: 800, h: 600 }),
                    },
                    ClipJob { input: b.clone(), info: info_b.clone(), spans: vec![(0.0, 3.0)], crop: None },
                ],
                output: output.clone(),
                format,
                target_bytes: Some(1_500_000),
            };
            // 2 + 3 + 3 seconds
            assert!((job.duration() - 8.0).abs() < 1e-9);
            let done = run(&tools, &job, &AtomicBool::new(false), &|_| {}).unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert!(done.bytes <= 1_500_000, "{format:?}: {} bytes", done.bytes);
            assert!(done.bytes > 20_000, "{format:?}: suspiciously small ({} bytes)", done.bytes);
            assert_eq!(std::fs::metadata(&output).unwrap().len(), done.bytes);
            let out = ffmpeg::probe_any(&tools, &output).unwrap();
            assert!((out.duration - 8.0).abs() < 0.6, "{format:?}: {} s", out.duration);
            assert_eq!(out.has_audio, format.has_audio(), "{format:?}");
            if format.has_video() && format != Format::Gif {
                assert!(out.width <= 800 && out.width >= 400, "{format:?}: width {}", out.width);
            }
            assert!(!job.output.with_extension(format!("part.{}", format.extension())).exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Easy video (a still picture, like a screen recording that barely changes) keeps its full size even on a tiny
    /// budget; busy video is shrunk. The old guess shrank both to 240p.
    #[test]
    fn picture_size_follows_how_hard_the_video_is() {
        let Some(tools) = ffmpeg::locate() else { return };
        let dir = std::env::temp_dir().join(format!("video-size-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let encode = |name: &str, source: &str| {
            let path = dir.join(name);
            let ok = ffmpeg::command(&tools.ffmpeg)
                .args(["-v", "error", "-y", "-f", "lavfi", "-i", source, "-t", "30", "-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p"])
                .arg(&path)
                .status()
                .unwrap()
                .success();
            assert!(ok);
            path
        };
        let easy = encode("easy.mp4", "smptebars=size=1360x930:rate=30");
        let busy = encode("busy.mp4", "testsrc2=size=1360x930:rate=30,noise=alls=30:allf=t");
        let cancel = AtomicBool::new(false);
        let ctx = |name: &str| Ctx {
            tools: &tools,
            cancel: &cancel,
            report: &|_| {},
            part: dir.join(format!("{name}.part.mp4")),
            log_prefix: dir.join(format!("{name}-log")),
        };
        let pick = |path: &PathBuf, name: &str| {
            let info = ffmpeg::probe(&tools, path).unwrap();
            let mut j = job(info.duration, info.width, info.height, false);
            j.clips[0].info = info;
            j.clips[0].input = path.clone();
            // About 1.5 MB for half a minute: 400 kbit/s for the picture, as for your 5-minute screen recording at 20 MB.
            choose_height(&j, &ctx(name), 400).unwrap()
        };
        assert_eq!(pick(&easy, "easy"), None, "a still picture fits at full size");
        let busy_height = pick(&busy, "busy").expect("busy video has to shrink");
        assert!(busy_height <= 540, "{busy_height}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `VIDEO_TEST_FILE=clip.mov [VIDEO_TEST_OUT=out.mp4] cargo test real_file -- --ignored --nocapture`: converts a real
    /// video to under 20 MB (and keeps the result if an output path is given).
    #[test]
    #[ignore]
    fn real_file_under_20mb() {
        let tools = ffmpeg::locate().expect("ffmpeg installed");
        let input = PathBuf::from(std::env::var_os("VIDEO_TEST_FILE").expect("set VIDEO_TEST_FILE"));
        let info = ffmpeg::probe(&tools, &input).unwrap();
        let j = Job {
            clips: vec![ClipJob { input, spans: vec![(0.0, info.duration)], info, crop: None }],
            output: std::env::var_os("VIDEO_TEST_OUT").map_or_else(|| std::env::temp_dir().join("video-real-test.mp4"), PathBuf::from),
            format: Format::Mp4,
            target_bytes: Some(20_000_000),
        };
        let started = std::time::Instant::now();
        let done = run(&tools, &j, &AtomicBool::new(false), &|e| {
            if let Event::Status(s) = e {
                println!("{s}");
            }
        })
        .unwrap();
        println!("{:.2} MB in {:.0}s: {}", done.bytes as f64 / 1e6, started.elapsed().as_secs_f64(), done.detail);
        assert!(done.bytes <= 20_000_000);
        if std::env::var_os("VIDEO_TEST_OUT").is_none() {
            let _ = std::fs::remove_file(&j.output);
        }
    }
}
