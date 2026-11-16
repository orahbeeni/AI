//! The application's state and everything that changes it. The drawing is in `ui/`.

use crate::ui::theme;
use cutline::export::{self, Event, ExportJob};
use cutline::log::{LogKind, LogLine};
use cutline::media::{self, natural_cmp, MediaInfo, THUMB_HEIGHT, THUMB_WIDTH};
use cutline::plan::{self, Fit, FormatChoice, Plan, Settings};
use cutline::preview::{Preview, PreviewShape, TimelineClip, Update};
use cutline::tools::{self, quiet_command, Tools};
use eframe::egui::{self, Color32, ColorImage, CornerRadius, Margin, RichText, TextureHandle, TextureOptions};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const THUMBS_PER_CLIP: usize = 5;
const MAX_CONSOLE_LINES: usize = 5000;
const SCREENSHOT_FRAMES: u64 = 120;
const SCREENSHOT_MAX_FRAMES: u64 = 900;
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "webm", "avi", "wmv", "flv", "mpg", "mpeg", "ts", "mts", "m2ts", "3gp", "ogv", "gif", "vob",
];

pub const SIZE_PRESETS: &[(&str, Option<(u32, u32)>)] = &[
    ("Same as my videos", None),
    ("4K  (3840 x 2160)", Some((3840, 2160))),
    ("1440p  (2560 x 1440)", Some((2560, 1440))),
    ("1080p  (1920 x 1080)", Some((1920, 1080))),
    ("720p  (1280 x 720)", Some((1280, 720))),
    ("480p  (854 x 480)", Some((854, 480))),
    ("360p  (640 x 360)", Some((640, 360))),
    ("Vertical  (1080 x 1920)", Some((1080, 1920))),
    ("Square  (1080 x 1080)", Some((1080, 1080))),
    ("Custom size...", None),
];

pub struct ClipItem {
    pub id: u64,
    pub info: MediaInfo,
    pub thumbs: Vec<Option<TextureHandle>>,
    /// Stretches of this clip's source that are cut out, merged and in order.
    pub removed: Vec<(f64, f64)>,
}

impl ClipItem {
    /// The parts that stay, as (start, end) in the source.
    pub fn kept(&self) -> Vec<(f64, f64)> {
        plan::kept_ranges(self.info.duration, &self.removed)
    }

    pub fn kept_duration(&self) -> f64 {
        self.kept().iter().map(|(start, end)| end - start).sum()
    }
}

/// Every clip's removed stretches (by clip id) at one moment, for Undo.
type CutSnapshot = Vec<(u64, Vec<(f64, f64)>)>;

/// A removed stretch, shown on the timeline where it was taken out.
pub struct CutMarker {
    pub at: f64,
    pub clip_id: u64,
    pub range: (f64, f64),
}

#[derive(Clone)]
pub enum PartState {
    Waiting,
    Working { label: String, fraction: f32 },
    Done { path: PathBuf, bytes: u64, over_limit: bool },
    Failed(String),
}

#[derive(Clone)]
pub struct ExportResult {
    pub ok: bool,
    pub title: String,
    pub folder: Option<PathBuf>,
}

enum ProbeEvent {
    Log(LogLine),
    Ready { id: u64, info: Box<MediaInfo> },
    Thumb { id: u64, slot: usize, rgba: Vec<u8> },
    Failed(String),
}

struct Toast {
    text: String,
    shown: Instant,
}

/// What `--screenshot` is waiting for.
struct ScreenshotJob {
    path: PathBuf,
    frames: u64,
    requested: bool,
    saved: Receiver<Result<(), String>>,
    saved_tx: Sender<Result<(), String>>,
}

pub struct App {
    ctx: egui::Context,
    pub tools: Option<Tools>,
    saved_ffmpeg: Option<PathBuf>,

    pub clips: Vec<ClipItem>,
    pub manual_order: bool,
    pub selected: Option<u64>,
    pub list_drag: Option<usize>,
    pub timeline_drag: Option<usize>,
    pub probing: usize,
    next_id: u64,
    probe_tx: Option<Sender<(u64, PathBuf)>>,
    probe_rx: Option<Receiver<ProbeEvent>>,

    pub format: FormatChoice,
    pub other_format: String,
    pub size_preset: usize,
    pub custom_width: String,
    pub custom_height: String,
    pub fit: Fit,
    pub part_limits: Vec<String>,
    pub same_limit: bool,
    pub base_name: String,
    pub name_edited: bool,
    pub out_dir: Option<PathBuf>,

    pub playhead: f64,
    pub playing: bool,
    /// The stretch marked for removal, in timeline time.
    pub mark_in: Option<f64>,
    pub mark_out: Option<f64>,
    /// Each entry: every clip's removed stretches before a cut, for Undo.
    cut_history: Vec<CutSnapshot>,
    preview: Option<Preview>,
    pub preview_texture: Option<TextureHandle>,
    preview_dirty: bool,
    last_shape: Option<(u32, u32, Fit)>,

    job: Option<ExportJob>,
    pub part_states: Vec<PartState>,
    pub export_result: Option<ExportResult>,
    export_started: Option<Instant>,

    pub console: Vec<LogLine>,
    pub show_console: bool,
    toasts: Vec<Toast>,
    screenshot: Option<ScreenshotJob>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>, screenshot: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        let keep_settings = screenshot.is_none();
        let read = |key: &str| cc.storage.filter(|_| keep_settings).and_then(|storage| storage.get_string(key)).filter(|text| !text.is_empty());
        let (saved_tx, saved) = channel();
        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            tools: None,
            saved_ffmpeg: read("ffmpeg").map(PathBuf::from),
            clips: Vec::new(),
            manual_order: false,
            selected: None,
            list_drag: None,
            timeline_drag: None,
            probing: 0,
            next_id: 1,
            probe_tx: None,
            probe_rx: None,
            format: FormatChoice::Listed(0),
            other_format: String::new(),
            size_preset: 0,
            custom_width: "1280".to_string(),
            custom_height: "720".to_string(),
            fit: Fit::Bars,
            part_limits: vec!["20".to_string()],
            same_limit: true,
            base_name: "edited_video".to_string(),
            name_edited: false,
            out_dir: read("out_dir").map(PathBuf::from),
            playhead: 0.0,
            playing: false,
            mark_in: None,
            mark_out: None,
            cut_history: Vec::new(),
            preview: None,
            preview_texture: None,
            preview_dirty: false,
            last_shape: None,
            job: None,
            part_states: Vec::new(),
            export_result: None,
            export_started: None,
            console: Vec::new(),
            show_console: read("show_console").as_deref() == Some("1"),
            toasts: Vec::new(),
            screenshot: screenshot.map(|path| ScreenshotJob { path, frames: 0, requested: false, saved, saved_tx }),
        };
        app.find_tools();
        app.add_files(files);
        app
    }

    fn wake(&self) -> Arc<dyn Fn() + Send + Sync> {
        let ctx = self.ctx.clone();
        Arc::new(move || ctx.request_repaint())
    }

    /// Look for FFmpeg and start the background helpers when it's there.
    pub fn find_tools(&mut self) {
        self.tools = tools::locate(self.saved_ffmpeg.as_deref());
        let Some(tools) = self.tools.clone() else {
            self.preview = None;
            self.probe_tx = None;
            self.probe_rx = None;
            return;
        };
        self.log(LogKind::Info, format!("Using {}", tools.version));
        self.preview = Some(Preview::new(tools.clone(), self.wake()));
        let (job_tx, job_rx) = channel();
        let (event_tx, event_rx) = channel();
        let wake = self.wake();
        std::thread::spawn(move || probe_worker(tools, job_rx, event_tx, wake));
        self.probe_tx = Some(job_tx);
        self.probe_rx = Some(event_rx);
    }

    pub fn pick_ffmpeg(&mut self) {
        if let Some(path) = rfd::FileDialog::new().set_title("Find your FFmpeg program").pick_file() {
            self.saved_ffmpeg = Some(path);
            self.find_tools();
            if self.tools.is_none() {
                self.toast("That file doesn't look like FFmpeg");
            }
        }
    }

    pub fn add_with_dialog(&mut self) {
        let picked = rfd::FileDialog::new().set_title("Add videos").add_filter("Videos", VIDEO_EXTENSIONS).add_filter("All files", &["*"]).pick_files();
        if let Some(files) = picked {
            self.add_files(files);
        }
    }

    pub fn pick_out_dir(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().set_title("Where should the videos go?").pick_folder() {
            self.out_dir = Some(dir);
        }
    }

    /// Queue files (or the videos in dropped folders) for reading.
    pub fn add_files(&mut self, files: Vec<PathBuf>) {
        let Some(sender) = self.probe_tx.clone() else {
            if !files.is_empty() {
                self.toast("Cutline needs FFmpeg before it can read videos");
            }
            return;
        };
        let mut paths = Vec::new();
        for path in files {
            if path.is_dir() {
                let mut inside: Vec<PathBuf> = std::fs::read_dir(&path)
                    .map(|entries| entries.flatten().map(|entry| entry.path()).filter(|file| is_video_name(file)).collect())
                    .unwrap_or_default();
                inside.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
                paths.extend(inside);
            } else {
                paths.push(path);
            }
        }
        for path in paths {
            let id = self.next_id;
            self.next_id += 1;
            self.probing += 1;
            let _ = sender.send((id, path));
        }
    }

    pub fn total_duration(&self) -> f64 {
        self.clips.iter().map(ClipItem::kept_duration).sum()
    }

    fn clip_start(&self, index: usize) -> f64 {
        self.clips[..index].iter().map(ClipItem::kept_duration).sum()
    }

    /// The marked stretch, once both ends are set.
    pub fn selection(&self) -> Option<(f64, f64)> {
        match (self.mark_in, self.mark_out) {
            (Some(start), Some(end)) if end - start >= 0.05 => Some((start, end)),
            _ => None,
        }
    }

    pub fn mark_start(&mut self) {
        self.mark_in = Some(self.playhead);
        if self.mark_out.is_some_and(|end| end <= self.playhead) {
            self.mark_out = None;
        }
    }

    pub fn mark_end(&mut self) {
        // The end of the video is a natural end even though the playhead stops just short of it.
        let total = self.total_duration();
        let at = if total - self.playhead < 0.1 { total } else { self.playhead };
        self.mark_out = Some(at);
        if self.mark_in.is_some_and(|start| start >= at) {
            self.mark_in = None;
        }
    }

    pub fn clear_marks(&mut self) {
        self.mark_in = None;
        self.mark_out = None;
    }

    /// Cut the marked stretch out of whichever clips it covers.
    pub fn remove_selection(&mut self) {
        let Some((start, end)) = self.selection() else { return };
        self.cut_history.push(self.clips.iter().map(|clip| (clip.id, clip.removed.clone())).collect());
        let mut timeline = 0.0;
        for clip in &mut self.clips {
            let mut taken = Vec::new();
            for (piece_start, piece_end) in clip.kept() {
                let length = piece_end - piece_start;
                let (from, to) = (start.max(timeline), end.min(timeline + length));
                if to > from {
                    taken.push((piece_start + from - timeline, piece_start + to - timeline));
                }
                timeline += length;
            }
            if !taken.is_empty() {
                taken.extend(clip.removed.iter().copied());
                clip.removed = plan::merge_ranges(&taken);
            }
        }
        let removed = end - start;
        self.after_clip_change();
        self.seek(start);
        self.toast(format!("Removed {}. Ctrl+Z puts it back.", media::fine_clock(removed)));
    }

    pub fn can_undo_cut(&self) -> bool {
        !self.cut_history.is_empty()
    }

    pub fn undo_cut(&mut self) {
        let Some(before) = self.cut_history.pop() else { return };
        for (id, removed) in before {
            if let Some(clip) = self.clips.iter_mut().find(|clip| clip.id == id) {
                clip.removed = removed;
            }
        }
        self.after_clip_change();
    }

    /// Put one removed stretch back.
    pub fn restore_cut(&mut self, clip_id: u64, range: (f64, f64)) {
        self.cut_history.push(self.clips.iter().map(|clip| (clip.id, clip.removed.clone())).collect());
        if let Some(clip) = self.clips.iter_mut().find(|clip| clip.id == clip_id) {
            clip.removed.retain(|removed| *removed != range);
        }
        self.after_clip_change();
    }

    /// Where each removed stretch was taken out, on the timeline.
    pub fn cut_markers(&self) -> Vec<CutMarker> {
        let mut markers = Vec::new();
        let mut clip_start = 0.0;
        for clip in &self.clips {
            for &range in &clip.removed {
                let kept_before: f64 = clip.kept().iter().map(|(start, end)| (end.min(range.0) - start).max(0.0)).sum();
                markers.push(CutMarker { at: clip_start + kept_before, clip_id: clip.id, range });
            }
            clip_start += clip.kept_duration();
        }
        markers
    }

    fn after_clip_change(&mut self) {
        self.clear_marks();
        self.stop_playing();
        self.playhead = self.playhead.min(self.total_duration());
        self.preview_dirty = true;
        self.clear_results();
    }

    pub fn move_clip(&mut self, from: usize, target: usize) {
        if from >= self.clips.len() || target > self.clips.len() {
            return;
        }
        let clip = self.clips.remove(from);
        let insert_at = if target > from { target - 1 } else { target };
        self.clips.insert(insert_at, clip);
        self.manual_order = true;
        self.after_clip_change();
    }

    pub fn remove_clip(&mut self, index: usize) {
        if index < self.clips.len() {
            self.clips.remove(index);
            if self.clips.is_empty() {
                self.preview_texture = None;
                self.manual_order = false;
            }
            self.after_clip_change();
        }
    }

    pub fn sort_alphabetically(&mut self) {
        self.clips.sort_by(|a, b| natural_cmp(&a.info.name, &b.info.name));
        self.manual_order = false;
        self.after_clip_change();
    }

    pub fn select_clip(&mut self, index: usize) {
        self.selected = self.clips.get(index).map(|clip| clip.id);
        let start = self.clip_start(index);
        self.seek(start);
    }

    pub fn seek(&mut self, at: f64) {
        self.stop_playing();
        let total = self.total_duration();
        self.playhead = at.clamp(0.0, (total - 0.05).max(0.0));
        self.preview_dirty = true;
    }

    pub fn skip_clip(&mut self, forward: bool) {
        let mut start = 0.0;
        let mut starts = Vec::with_capacity(self.clips.len());
        for clip in &self.clips {
            starts.push(start);
            start += clip.kept_duration();
        }
        let target = if forward {
            starts.iter().copied().find(|time| *time > self.playhead + 0.01)
        } else {
            starts.iter().copied().rev().find(|time| *time < self.playhead - 0.5).or(Some(0.0))
        };
        if let Some(target) = target {
            self.seek(target);
        }
    }

    fn timeline_clips(&self) -> Arc<Vec<TimelineClip>> {
        let mut start = 0.0;
        let mut pieces = Vec::new();
        for clip in &self.clips {
            for (source_start, source_end) in clip.kept() {
                let duration = source_end - source_start;
                pieces.push(TimelineClip { path: clip.info.path.clone(), start, source_start, duration });
                start += duration;
            }
        }
        Arc::new(pieces)
    }

    fn preview_shape(&self) -> PreviewShape {
        let (width, height) = self.output_dims();
        PreviewShape::for_output(width, height, self.fit)
    }

    pub fn toggle_play(&mut self) {
        if self.playing {
            self.stop_playing();
            self.preview_dirty = true;
        } else if !self.clips.is_empty() {
            if self.playhead >= self.total_duration() - 0.1 {
                self.playhead = 0.0;
            }
            if let Some(preview) = &self.preview {
                preview.play(self.timeline_clips(), self.playhead, self.preview_shape());
                self.playing = true;
            }
        }
    }

    fn stop_playing(&mut self) {
        if self.playing {
            self.playing = false;
            if let Some(preview) = &self.preview {
                preview.stop();
            }
        }
    }

    pub fn exporting(&self) -> bool {
        self.job.is_some()
    }

    pub fn set_part_count(&mut self, count: usize) {
        if count == self.part_limits.len() || count == 0 {
            return;
        }
        let last = self.part_limits.last().cloned().unwrap_or_else(|| "20".to_string());
        self.part_limits.resize(count, last);
    }

    /// A size field: `Ok(None)` when empty (no limit), `Err` when it isn't a usable number.
    pub fn parse_limit(&self, index: usize) -> Result<Option<f64>, ()> {
        let text = self.part_limits[index].trim().replace(',', ".");
        if text.is_empty() {
            return Ok(None);
        }
        match text.parse::<f64>() {
            Ok(value) if value > 0.0 && value.is_finite() => Ok(Some(value)),
            _ => Err(()),
        }
    }

    pub fn limits_differ(&self) -> bool {
        let first = self.part_limits.first();
        !self.same_limit && self.part_limits.iter().any(|limit| Some(limit) != first)
    }

    fn custom_frame(&self) -> Option<(u32, u32)> {
        let width = self.custom_width.trim().parse::<u32>().ok().filter(|v| (16..=8192).contains(v))?;
        let height = self.custom_height.trim().parse::<u32>().ok().filter(|v| (16..=8192).contains(v))?;
        Some((width, height))
    }

    fn chosen_frame(&self) -> Option<(u32, u32)> {
        if self.size_preset + 1 == SIZE_PRESETS.len() {
            self.custom_frame()
        } else {
            SIZE_PRESETS[self.size_preset].1
        }
    }

    pub fn output_dims(&self) -> (u32, u32) {
        let infos: Vec<MediaInfo> = self.clips.iter().map(|clip| clip.info.clone()).collect();
        self.chosen_frame().unwrap_or_else(|| plan::source_frame(&infos))
    }

    /// What's wrong with the typed-in settings, in words.
    pub fn input_problem(&self) -> Option<String> {
        if self.size_preset + 1 == SIZE_PRESETS.len() && self.custom_frame().is_none() {
            return Some("Enter a width and height between 16 and 8192.".to_string());
        }
        (0..self.part_limits.len())
            .find(|index| self.parse_limit(*index).is_err())
            .map(|index| format!("Part {}'s size isn't a number.", index + 1))
    }

    fn effective_out_dir(&self) -> PathBuf {
        if let Some(dir) = &self.out_dir {
            return dir.clone();
        }
        self.clips
            .first()
            .and_then(|clip| clip.info.path.parent().map(Path::to_path_buf))
            .or_else(|| std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from))
            .unwrap_or_else(std::env::temp_dir)
    }

    fn current_plan(&self) -> Plan {
        let infos: Vec<MediaInfo> = self.clips.iter().map(|clip| clip.info.clone()).collect();
        let settings = Settings {
            format: self.format.clone(),
            frame: self.chosen_frame(),
            fit: self.fit,
            limits_mb: (0..self.part_limits.len()).map(|index| self.parse_limit(index).unwrap_or(None)).collect(),
            out_dir: self.effective_out_dir(),
            base_name: self.base_name.trim().to_string(),
            removed: self.clips.iter().map(|clip| clip.removed.clone()).collect(),
        };
        let encoders = self.tools.as_ref().map(|tools| tools.encoders.clone()).unwrap_or_default();
        plan::build(&infos, &settings, &encoders, &|path| path.exists())
    }

    pub fn start_export(&mut self, plan: Plan) {
        let Some(tools) = self.tools.clone() else { return };
        self.stop_playing();
        self.part_states = plan.parts.iter().map(|_| PartState::Waiting).collect();
        self.export_result = None;
        self.export_started = Some(Instant::now());
        self.job = Some(export::start(plan, tools, self.wake()));
    }

    pub fn cancel_export(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }

    pub fn clear_results(&mut self) {
        if self.job.is_none() {
            self.part_states.clear();
            self.export_result = None;
        }
    }

    pub fn overall_fraction(&self) -> f32 {
        if self.part_states.is_empty() {
            return 0.0;
        }
        let sum: f32 = self
            .part_states
            .iter()
            .map(|state| match state {
                PartState::Done { .. } | PartState::Failed(_) => 1.0,
                PartState::Working { fraction, .. } => *fraction,
                PartState::Waiting => 0.0,
            })
            .sum();
        sum / self.part_states.len() as f32
    }

    pub fn export_eta(&self) -> Option<f64> {
        let started = self.export_started?;
        export::eta(started, self.overall_fraction()).map(|left| left.as_secs_f64())
    }

    pub fn log(&mut self, kind: LogKind, text: impl Into<String>) {
        self.push_log(LogLine::new(kind, text));
    }

    fn push_log(&mut self, line: LogLine) {
        if line.kind == LogKind::Live && self.console.last().is_some_and(|last| last.kind == LogKind::Live) {
            self.console.pop();
        }
        self.console.push(line);
        if self.console.len() > MAX_CONSOLE_LINES {
            let extra = self.console.len() - MAX_CONSOLE_LINES;
            self.console.drain(..extra);
        }
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), shown: Instant::now() });
    }

    fn poll_probes(&mut self, ctx: &egui::Context) {
        let events: Vec<ProbeEvent> = self.probe_rx.as_ref().map_or(Vec::new(), |rx| rx.try_iter().collect());
        for event in events {
            match event {
                ProbeEvent::Log(line) => self.push_log(line),
                ProbeEvent::Ready { id, info } => {
                    self.probing = self.probing.saturating_sub(1);
                    self.clips.push(ClipItem { id, info: *info, thumbs: vec![None; THUMBS_PER_CLIP], removed: Vec::new() });
                    if !self.manual_order {
                        self.clips.sort_by(|a, b| natural_cmp(&a.info.name, &b.info.name));
                    }
                    if !self.name_edited {
                        if let Some(first) = self.clips.first() {
                            let stem = Path::new(&first.info.name).file_stem().unwrap_or_default().to_string_lossy();
                            self.base_name = format!("{stem}_edited");
                        }
                    }
                    self.after_clip_change();
                }
                ProbeEvent::Thumb { id, slot, rgba } => {
                    if let Some(clip) = self.clips.iter_mut().find(|clip| clip.id == id) {
                        let image = ColorImage::from_rgba_unmultiplied([THUMB_WIDTH as usize, THUMB_HEIGHT as usize], &rgba);
                        clip.thumbs[slot] = Some(ctx.load_texture(format!("thumb-{id}-{slot}"), image, TextureOptions::LINEAR));
                    }
                }
                ProbeEvent::Failed(message) => {
                    self.probing = self.probing.saturating_sub(1);
                    self.log(LogKind::Warning, message.clone());
                    self.toast(message);
                }
            }
        }
    }

    fn poll_preview(&mut self, ctx: &egui::Context) {
        let updates: Vec<Update> = self.preview.as_ref().map_or(Vec::new(), |preview| preview.updates.try_iter().collect());
        for update in updates {
            match update {
                Update::Frame(frame) => {
                    if self.playing {
                        self.playhead = frame.at;
                    }
                    let image = ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
                    match &mut self.preview_texture {
                        Some(texture) => texture.set(image, TextureOptions::LINEAR),
                        None => self.preview_texture = Some(ctx.load_texture("preview", image, TextureOptions::LINEAR)),
                    }
                }
                Update::Ended => {
                    self.playing = false;
                    self.preview_dirty = true;
                }
            }
        }
    }

    fn poll_export(&mut self) {
        let events: Vec<Event> = self.job.as_ref().map_or(Vec::new(), |job| job.events.try_iter().collect());
        for event in events {
            match event {
                Event::Log(line) => self.push_log(line),
                Event::Stage { part, label } => {
                    let fraction = match &self.part_states[part] {
                        PartState::Working { fraction, .. } => *fraction,
                        _ => 0.0,
                    };
                    self.part_states[part] = PartState::Working { label, fraction };
                }
                Event::Progress { part, fraction, .. } => {
                    if let PartState::Working { label, .. } = &self.part_states[part] {
                        self.part_states[part] = PartState::Working { label: label.clone(), fraction };
                    }
                }
                Event::PartDone { part, path, bytes, over_limit } => {
                    self.part_states[part] = PartState::Done { path, bytes, over_limit };
                }
                Event::PartFailed { part, message } => {
                    self.part_states[part] = PartState::Failed(message);
                }
                Event::Finished { cancelled } => self.finish_export(cancelled),
            }
        }
    }

    fn finish_export(&mut self, cancelled: bool) {
        self.job = None;
        let total = self.part_states.len();
        let done = self.part_states.iter().filter(|state| matches!(state, PartState::Done { .. })).count();
        let over = self.part_states.iter().filter(|state| matches!(state, PartState::Done { over_limit: true, .. })).count();
        let failed = self.part_states.iter().filter(|state| matches!(state, PartState::Failed(_))).count();
        let folder = self
            .part_states
            .iter()
            .find_map(|state| match state {
                PartState::Done { path, .. } => path.parent().map(Path::to_path_buf),
                _ => None,
            });
        let (ok, title) = if cancelled {
            (false, format!("Stopped. {done} of {total} finished."))
        } else if failed > 0 {
            (false, format!("{failed} of {total} failed. The console has the details."))
        } else if over > 0 {
            (false, format!("Saved, but {over} went over its size limit."))
        } else if total == 1 {
            (true, "Your video is saved".to_string())
        } else {
            (true, format!("All {total} videos are saved"))
        };
        self.export_result = Some(ExportResult { ok, title, folder });
        if !cancelled && failed > 0 {
            self.show_console = true;
        }
    }

    fn refresh_preview(&mut self) {
        if self.clips.is_empty() {
            return;
        }
        let (width, height) = self.output_dims();
        let shape = (width, height, self.fit);
        if self.last_shape != Some(shape) {
            self.last_shape = Some(shape);
            self.preview_dirty = true;
        }
        if self.preview_dirty && !self.playing {
            if let Some(preview) = &self.preview {
                preview.show(self.timeline_clips(), self.playhead, self.preview_shape());
            }
            self.preview_dirty = false;
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        if !typing && ctx.input(|input| input.key_pressed(egui::Key::Space)) {
            self.toggle_play();
        }
        let editing = !typing && !self.exporting() && !self.clips.is_empty();
        let pressed = |key| ctx.input(|input| input.key_pressed(key));
        if editing && pressed(egui::Key::I) {
            self.mark_start();
        }
        if editing && pressed(egui::Key::O) {
            self.mark_end();
        }
        if editing && pressed(egui::Key::Escape) {
            self.clear_marks();
        }
        if editing && ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Z)) {
            self.undo_cut();
        }
        if editing && (pressed(egui::Key::X) || pressed(egui::Key::Delete) || pressed(egui::Key::Backspace)) {
            if self.selection().is_some() {
                self.remove_selection();
            } else if pressed(egui::Key::Delete) {
                if let Some(index) = self.clips.iter().position(|clip| Some(clip.id) == self.selected) {
                    self.remove_clip(index);
                }
            }
        }
        let dropped: Vec<PathBuf> = ctx.input(|input| input.raw.dropped_files.iter().map(|file| file.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            if self.exporting() {
                self.toast("Wait for the export to finish before adding videos");
            } else {
                self.add_files(dropped);
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            let (mark, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(mark, CornerRadius::same(7), theme::ACCENT);
            theme::triangle(painter, mark.center() + egui::vec2(1.0, 0.0), 6.5, 1.0, Color32::WHITE);
            ui.label(RichText::new("Cutline").size(17.0).strong());
            if !self.clips.is_empty() {
                let count = self.clips.len();
                let noun = if count == 1 { "video" } else { "videos" };
                ui.add_space(10.0);
                ui.label(RichText::new(format!("{count} {noun} · {} in total", media::clock(self.total_duration()))).size(12.5).color(theme::MUTED));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                theme::toggle(ui, &mut self.show_console);
                ui.label(RichText::new("View console").size(13.0));
                if self.exporting() && !self.show_console {
                    ui.spinner();
                }
            });
        });
    }

    fn drop_overlay(&self, ctx: &egui::Context) {
        if ctx.input(|input| input.raw.hovered_files.is_empty()) {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop overlay")));
        let screen = ctx.content_rect();
        painter.rect_filled(screen, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(15, 17, 21, 225));
        theme::dashed_rect(&painter, screen.shrink(18.0), egui::Stroke::new(3.0, theme::ACCENT_HOVER));
        painter.text(screen.center() - egui::vec2(0.0, 12.0), egui::Align2::CENTER_CENTER, "Drop to add", egui::FontId::proportional(30.0), theme::TEXT);
        painter.text(
            screen.center() + egui::vec2(0.0, 24.0),
            egui::Align2::CENTER_CENTER,
            "They'll be added in alphabetical order",
            egui::FontId::proportional(15.0),
            theme::MUTED,
        );
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|toast| toast.shown.elapsed() < Duration::from_secs(4));
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::unique("toasts"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 62.0))
            .show(ctx, |ui| {
                for toast in &self.toasts {
                    egui::Frame::new()
                        .fill(theme::RAISED)
                        .stroke(egui::Stroke::new(1.0, theme::LINE))
                        .corner_radius(CornerRadius::same(9))
                        .inner_margin(Margin::symmetric(14, 9))
                        .show(ui, |ui| ui.label(RichText::new(toast.text.as_str()).size(13.0)));
                }
            });
        ctx.request_repaint_after(Duration::from_millis(300));
    }

    fn run_screenshot(&mut self, ctx: &egui::Context) {
        let settled = self.probing == 0
            && self.clips.iter().all(|clip| clip.thumbs.iter().all(Option::is_some))
            && (self.clips.is_empty() || self.preview_texture.is_some());
        let Some(job) = &mut self.screenshot else { return };
        job.frames += 1;
        ctx.request_repaint();
        if !job.requested && job.frames >= SCREENSHOT_FRAMES && (settled || job.frames >= SCREENSHOT_MAX_FRAMES) {
            job.requested = true;
            let (path, done) = (job.path.clone(), job.saved_tx.clone());
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                let _ = done.send(save_png(&image, &path));
            })));
        }
        if let Ok(result) = job.saved.try_recv() {
            if let Err(message) = result {
                eprintln!("Couldn't save the screenshot: {message}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_probes(&ctx);
        self.poll_preview(&ctx);
        self.poll_export();
        self.handle_shortcuts(&ctx);
        let plan = self.current_plan();
        self.refresh_preview();
        if self.exporting() || self.probing > 0 {
            ctx.request_repaint_after(Duration::from_millis(120));
        }

        let panel_frame = |margin: i8| egui::Frame::new().fill(theme::PANEL).inner_margin(Margin::same(margin));
        egui::Panel::top("top bar")
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(14, 8)))
            .exact_size(50.0)
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("timeline")
            .frame(panel_frame(12))
            .exact_size(176.0)
            .show(ui, |ui| self.timeline_panel(ui, Some(&plan)));
        egui::Panel::left("media")
            .frame(panel_frame(12))
            .resizable(true)
            .default_size(300.0)
            .min_size(240.0)
            .max_size(460.0)
            .show(ui, |ui| self.media_panel(ui));
        egui::Panel::right("output")
            .frame(panel_frame(12))
            .resizable(true)
            .default_size(340.0)
            .min_size(300.0)
            .max_size(460.0)
            .show(ui, |ui| self.output_panel(ui, Some(&plan)));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin::same(12)))
            .show(ui, |ui| {
                if self.show_console {
                    egui::Panel::bottom("console")
                        .frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin { top: 10, ..Margin::ZERO }))
                        .resizable(true)
                        .default_size(240.0)
                        .min_size(120.0)
                        .max_size(480.0)
                        .show(ui, |ui| self.console_panel(ui));
                }
                self.preview_panel(ui);
            });

        self.drop_overlay(&ctx);
        self.draw_toasts(&ctx);
        self.run_screenshot(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.screenshot.is_some() {
            return;
        }
        let path_text = |path: &Option<PathBuf>| path.as_ref().map(|path| path.display().to_string()).unwrap_or_default();
        storage.set_string("ffmpeg", path_text(&self.saved_ffmpeg));
        storage.set_string("out_dir", path_text(&self.out_dir));
        storage.set_string("show_console", if self.show_console { "1" } else { "0" }.to_string());
    }

    fn persist_egui_memory(&self) -> bool {
        self.screenshot.is_none()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(job) = &self.job {
            job.cancel();
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                match job.events.recv_timeout(Duration::from_millis(200)) {
                    Ok(Event::Finished { .. }) => break,
                    _ => continue,
                }
            }
        }
    }
}

fn is_video_name(path: &Path) -> bool {
    path.extension().is_some_and(|ext| VIDEO_EXTENSIONS.contains(&ext.to_string_lossy().to_lowercase().as_str()))
}

fn probe_worker(tools: Tools, jobs: Receiver<(u64, PathBuf)>, events: Sender<ProbeEvent>, wake: Arc<dyn Fn() + Send + Sync>) {
    for (id, path) in jobs {
        let mut log = |line| {
            let _ = events.send(ProbeEvent::Log(line));
        };
        match media::probe(&tools, &path, &mut log) {
            Err(message) => {
                let _ = events.send(ProbeEvent::Failed(message));
                wake();
            }
            Ok(info) => {
                let duration = info.duration;
                let source = info.path.clone();
                let _ = events.send(ProbeEvent::Ready { id, info: Box::new(info) });
                wake();
                for slot in 0..THUMBS_PER_CLIP {
                    let at = duration * (slot as f64 + 0.5) / THUMBS_PER_CLIP as f64;
                    if let Some(rgba) = media::grab_thumbnail(&tools, &source, at) {
                        let _ = events.send(ProbeEvent::Thumb { id, slot, rgba });
                        wake();
                    }
                }
            }
        }
    }
}

fn save_png(image: &ColorImage, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let [width, height] = image.size;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|pixel| [pixel.r(), pixel.g(), pixel.b(), 255]).collect();
    writer.write_image_data(&bytes).map_err(|error| error.to_string())
}

/// Show a folder in the system's file manager.
pub fn open_folder(path: &Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = quiet_command(std::ffi::OsStr::new(program)).arg(path).spawn();
}
