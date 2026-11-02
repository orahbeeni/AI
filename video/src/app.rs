//! The window: open videos, pick the parts to keep, crop, join, choose a format and a size, convert.

use crate::encode::{self, ClipJob, Crop, Format, Job, Span};
use crate::ffmpeg::{self, Frame, Info, Tools};
use crate::player::Player;
use crate::timeline::{self, Handle};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, pos2,
    vec2,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;

const SIZE_PRESETS: [f64; 5] = [8.0, 10.0, 20.0, 25.0, 50.0];
const DEFAULT_TARGET_MB: f64 = 20.0;
const VIDEO_EXTENSIONS: [&str; 15] =
    ["mov", "mp4", "m4v", "mkv", "avi", "webm", "wmv", "flv", "mpg", "mpeg", "mts", "m2ts", "ts", "3gp", "ogv"];
/// Smallest crop, in pixels of the source.
const MIN_CROP: f32 = 16.0;
const ACCENT: Color32 = Color32::from_rgb(70, 140, 230);
const BAD: Color32 = Color32::from_rgb(235, 100, 90);

enum ToolState {
    Ready(Tools),
    Installing { status: String, frac: Option<f32> },
    Failed(String),
}

enum InstallMsg {
    Progress(String, Option<f32>),
    Done(Result<Tools, String>),
}

enum RunState {
    Idle,
    Running { status: String, progress: f32, cancel: Arc<AtomicBool> },
    Done { path: PathBuf, bytes: u64, detail: String },
    Failed(String),
}

enum RunMsg {
    Event(encode::Event),
    Finished(Result<encode::Done, encode::Error>),
}

/// Where a newly opened video goes.
#[derive(Clone, Copy, PartialEq)]
enum AddMode {
    /// Start over with just this video.
    Replace,
    Before,
    After,
}

/// What the row of videos asked for.
enum ClipAction {
    Select(usize),
    Move(isize),
    Remove,
    Add(AddMode),
}

/// The crop rectangle, in pixels of the source picture.
#[derive(Clone, Copy)]
struct CropBox {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl CropBox {
    /// A box covering the middle 80 % of the picture.
    fn inset(w: u32, h: u32) -> Self {
        let (w, h) = (w as f32, h as f32);
        CropBox { x0: w * 0.1, y0: h * 0.1, x1: w * 0.9, y1: h * 0.9 }
    }

    fn size(&self) -> (f32, f32) {
        (self.x1 - self.x0, self.y1 - self.y0)
    }

    /// The largest box of this shape that fits the picture, centred.
    fn fit_aspect(&mut self, ratio_w: f32, ratio_h: f32, frame: (u32, u32)) {
        let (fw, fh) = (frame.0 as f32, frame.1 as f32);
        let (w, h) = if fw / fh > ratio_w / ratio_h { (fh * ratio_w / ratio_h, fh) } else { (fw, fw * ratio_h / ratio_w) };
        *self = CropBox { x0: (fw - w) / 2.0, y0: (fh - h) / 2.0, x1: (fw + w) / 2.0, y1: (fh + h) / 2.0 };
    }

    fn to_crop(self) -> Crop {
        Crop { x: self.x0.round() as u32, y: self.y0.round() as u32, w: (self.x1 - self.x0).round() as u32, h: (self.y1 - self.y0).round() as u32 }
    }
}

#[derive(Clone, Copy)]
enum CropDrag {
    New(Pos2),
    Move(Pos2),
    Edges { l: bool, r: bool, t: bool, b: bool },
}

/// Things the play controls and the keyboard can ask for.
#[derive(Clone, Copy)]
enum Transport {
    PlayPause,
    PlaySelection,
    /// Jump by this many seconds.
    Jump(f64),
    /// Move by this many pictures, paused.
    Step(i32),
    SetStart,
    SetEnd,
}

/// Edits of the parts to keep.
#[derive(Clone, Copy)]
enum Edit {
    KeepOnly,
    Remove,
    Reset,
}

/// The picture shown above the timeline, decoded off the UI thread; only the newest request is kept.
#[derive(Default)]
struct Preview {
    rx: Option<Receiver<Result<Frame, String>>>,
    pending: Option<f64>,
    texture: Option<TextureHandle>,
    error: Option<String>,
}

impl Preview {
    fn request(&mut self, tools: &Tools, path: &Path, t: f64, ctx: &egui::Context) {
        if self.rx.is_some() {
            self.pending = Some(t);
            return;
        }
        let (tools, path, ctx) = (tools.clone(), path.to_path_buf(), ctx.clone());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(ffmpeg::frame_at(&tools, &path, t, 1280));
            ctx.request_repaint();
        });
        self.rx = Some(rx);
    }

    /// Shows a picture straight away (playback frames).
    fn set_image(&mut self, ctx: &egui::Context, image: egui::ColorImage) {
        match &mut self.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => self.texture = Some(ctx.load_texture("preview", image, egui::TextureOptions::LINEAR)),
        }
        self.error = None;
    }

    /// Forgets a picture that is still being fetched, so it cannot replace what is playing.
    fn cancel(&mut self) {
        self.rx = None;
        self.pending = None;
    }

    /// Takes a finished picture, if there is one. Returns the time of a request that came in meanwhile.
    fn poll(&mut self, ctx: &egui::Context) -> Option<f64> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(Ok(frame)) => {
                let image = egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
                self.texture = Some(ctx.load_texture("preview", image, egui::TextureOptions::LINEAR));
                self.error = None;
            }
            Ok(Err(e)) => self.error = Some(e),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {}
        }
        self.rx = None;
        self.pending.take()
    }
}

/// One video of the project, with what to keep of it.
struct Clip {
    path: PathBuf,
    info: Info,
    /// The selection: two handles on the timeline.
    start: f64,
    end: f64,
    /// The parts that are kept once the selection has been used to cut (see [`Edit`]). While `None` the selection
    /// itself is what is kept, so plain trimming needs no extra step.
    kept: Option<Vec<Span>>,
    /// The position: the time of the picture on screen, and where playback is.
    scrub: f64,
    player: Player,
    /// A drag on the position slider paused playback; play again when it ends (up to the inner time, if any).
    resume_after_seek: Option<Option<f64>>,
    /// Playback has been inside a kept part; leaving it means a removed part starts, which is skipped.
    inside_kept: bool,
    start_text: String,
    end_text: String,
    crop_on: bool,
    crop: CropBox,
    preview: Preview,
}

fn in_spans(spans: &[Span], t: f64) -> bool {
    spans.iter().any(|&(a, b)| t >= a - 1e-3 && t < b)
}

impl Clip {
    fn new(path: PathBuf, info: Info) -> Self {
        Clip {
            start: 0.0,
            end: info.duration,
            kept: None,
            scrub: 0.0,
            player: Player::default(),
            resume_after_seek: None,
            inside_kept: false,
            start_text: String::new(),
            end_text: String::new(),
            crop_on: false,
            crop: CropBox::inset(info.width, info.height),
            preview: Preview::default(),
            path,
            info,
        }
    }

    fn name(&self) -> String {
        self.path.file_name().unwrap_or_default().to_string_lossy().into_owned()
    }

    /// What will be kept of this video.
    fn spans(&self) -> Vec<Span> {
        self.kept.clone().unwrap_or_else(|| vec![(self.start, self.end)])
    }

    fn kept_total(&self) -> f64 {
        self.spans().iter().map(|(a, b)| b - a).sum()
    }

    /// What the selection cuts from: the kept parts, or the whole video if nothing was cut yet.
    fn base(&self) -> Vec<Span> {
        self.kept.clone().unwrap_or_else(|| vec![(0.0, self.info.duration)])
    }

    fn can_remove_selection(&self) -> bool {
        !encode::subtract(&self.base(), (self.start, self.end)).is_empty()
    }

    fn edit(&mut self, edit: Edit) {
        let duration = self.info.duration;
        match edit {
            Edit::KeepOnly => {
                let kept = encode::intersect(&self.base(), (self.start, self.end));
                if !kept.is_empty() {
                    self.kept = Some(kept);
                }
            }
            Edit::Remove => {
                let kept = encode::subtract(&self.base(), (self.start, self.end));
                if !kept.is_empty() {
                    self.kept = Some(kept);
                    // Start the next selection from scratch.
                    self.start = 0.0;
                    self.end = duration;
                }
            }
            Edit::Reset => {
                self.kept = None;
                self.start = 0.0;
                self.end = duration;
            }
        }
    }

    /// Pauses and shows the picture at `t`.
    fn show_frame(&mut self, tools: &Tools, ctx: &egui::Context, t: f64) {
        self.player.stop();
        self.scrub = t;
        // Seeking to the very end finds no picture.
        self.preview.request(tools, &self.path, t.min(self.info.duration - 0.05).max(0.0), ctx);
    }

    fn play(&mut self, tools: &Tools, from: f64, until: Option<f64>) {
        self.preview.cancel();
        self.scrub = from;
        self.inside_kept = false;
        self.player.play(tools, &self.path, &self.info, from, until);
    }

    /// Moves to `t`, carrying on playing if it was playing.
    fn seek(&mut self, tools: &Tools, ctx: &egui::Context, t: f64) {
        let t = t.clamp(0.0, self.info.duration);
        match (self.player.is_playing(), self.player.until()) {
            (true, until) => self.play(tools, t, until.filter(|u| *u > t)),
            _ => self.show_frame(tools, ctx, t),
        }
    }

    fn job(&self) -> ClipJob {
        ClipJob { input: self.path.clone(), info: self.info.clone(), spans: self.spans(), crop: self.crop_on.then(|| self.crop.to_crop()) }
    }
}

fn make_job(clips: &[Clip], output: &Path, format: Format, target_mb: Option<f64>) -> Job {
    Job {
        clips: clips.iter().map(Clip::job).collect(),
        output: output.to_path_buf(),
        format,
        target_bytes: target_mb.map(|mb| (mb * 1_000_000.0) as u64),
    }
}

/// `20`, `12.5`, `500kb`, `1.5 GB`: a size in megabytes (1 MB is 1,000,000 bytes).
fn parse_size(text: &str) -> Option<f64> {
    let t = text.trim().to_lowercase();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(t.len());
    let (number, unit) = t.split_at(split);
    let value: f64 = number.replace(',', ".").parse().ok()?;
    let factor = match unit.trim() {
        "" | "mb" | "m" => 1.0,
        "kb" | "k" => 0.001,
        "gb" | "g" => 1000.0,
        _ => return None,
    };
    let mb = value * factor;
    (mb.is_finite() && mb >= 0.05).then_some(mb)
}

pub struct App {
    tools: ToolState,
    install_rx: Option<Receiver<InstallMsg>>,
    clips: Vec<Clip>,
    /// The clip shown in the preview and edited.
    current: usize,
    out_path: PathBuf,
    format: Format,
    /// A file asked for before ffmpeg was ready.
    waiting: Option<(PathBuf, AddMode)>,
    error: Option<String>,
    limit_size: bool,
    target_mb: f64,
    /// What is typed in the size box (it may be half-typed, so it is kept apart from `target_mb`).
    size_text: String,
    run: RunState,
    run_rx: Option<Receiver<RunMsg>>,
    last_dir: Option<PathBuf>,
    frames: u32,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>) -> Self {
        let mut app = App {
            tools: ToolState::Failed(String::new()),
            install_rx: None,
            clips: Vec::new(),
            current: 0,
            out_path: PathBuf::new(),
            format: Format::Mp4,
            waiting: file.map(|f| (f, AddMode::Replace)),
            error: None,
            limit_size: true,
            target_mb: DEFAULT_TARGET_MB,
            size_text: DEFAULT_TARGET_MB.to_string(),
            run: RunState::Idle,
            run_rx: None,
            last_dir: None,
            frames: 0,
        };
        match ffmpeg::locate() {
            Some(tools) => app.tools = ToolState::Ready(tools),
            None => app.start_install(&cc.egui_ctx),
        }
        app.open_waiting(&cc.egui_ctx);
        app
    }

    fn tools(&self) -> Option<&Tools> {
        self.tools.as_tools()
    }

    fn start_install(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let report = |status: String, frac: Option<f32>| {
                let _ = tx.send(InstallMsg::Progress(status, frac));
                ctx.request_repaint();
            };
            let result = ffmpeg::install(&report);
            let _ = tx.send(InstallMsg::Done(result));
            ctx.request_repaint();
        });
        self.install_rx = Some(rx);
        self.tools = ToolState::Installing { status: "ffmpeg is not installed; installing it…".into(), frac: None };
    }

    fn open_waiting(&mut self, ctx: &egui::Context) {
        if let (Some((path, mode)), Some(tools)) = (self.waiting.clone(), self.tools().cloned()) {
            self.waiting = None;
            self.open(path, mode, &tools, ctx);
        }
    }

    fn open(&mut self, path: PathBuf, mode: AddMode, tools: &Tools, ctx: &egui::Context) {
        let info = match ffmpeg::probe(tools, &path) {
            Ok(info) => info,
            Err(e) => {
                self.error = Some(format!("{}: {e}", path.display()));
                return;
            }
        };
        self.last_dir = path.parent().map(Path::to_path_buf);
        for clip in &mut self.clips {
            clip.player.stop();
        }
        let clip = Clip::new(path.clone(), info);
        if self.clips.is_empty() || mode == AddMode::Replace {
            self.clips = vec![clip];
            self.current = 0;
            self.out_path = default_output(&path, self.format);
        } else {
            self.current = if mode == AddMode::After { self.current + 1 } else { self.current };
            self.clips.insert(self.current, clip);
        }
        self.clips[self.current].show_frame(tools, ctx, 0.0);
        self.error = None;
        self.run = RunState::Idle;
    }

    fn pick_file(&mut self, mode: AddMode, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new().add_filter("Video", &VIDEO_EXTENSIONS);
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        if let (Some(path), Some(tools)) = (dialog.pick_file(), self.tools().cloned()) {
            self.open(path, mode, &tools, ctx);
        }
    }

    /// The output file follows the chosen format, and never lands on a file that is already there.
    fn format_changed(&mut self) {
        let dir = self.out_path.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = stem_of(&self.out_path);
        self.out_path = unique_path(&dir, &stem, self.format.extension());
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let msgs: Vec<InstallMsg> = self.install_rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for msg in msgs {
            match msg {
                InstallMsg::Progress(status, frac) => self.tools = ToolState::Installing { status, frac },
                InstallMsg::Done(Ok(tools)) => {
                    self.tools = ToolState::Ready(tools);
                    self.install_rx = None;
                }
                InstallMsg::Done(Err(e)) => {
                    self.tools = ToolState::Failed(e);
                    self.install_rx = None;
                }
            }
        }

        let msgs: Vec<RunMsg> = self.run_rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for msg in msgs {
            match msg {
                RunMsg::Event(event) => {
                    if let RunState::Running { status, progress, .. } = &mut self.run {
                        match event {
                            encode::Event::Status(s) => *status = s,
                            encode::Event::Progress(p) => *progress = p,
                        }
                    }
                }
                RunMsg::Finished(result) => {
                    self.run_rx = None;
                    self.run = match result {
                        Ok(done) => {
                            let path = self.out_path.clone();
                            // The next conversion must not overwrite this one.
                            self.out_path = unique_path(path.parent().unwrap_or(Path::new("")), &stem_of(&path), self.format.extension());
                            RunState::Done { path, bytes: done.bytes, detail: done.detail }
                        }
                        Err(encode::Error::Cancelled) => RunState::Idle,
                        Err(encode::Error::Failed(msg)) => RunState::Failed(msg),
                    };
                }
            }
        }

        if let (Some(v), Some(tools)) = (self.clips.get_mut(self.current), self.tools.as_tools()) {
            if let Some(t) = v.preview.poll(ctx) {
                v.preview.request(tools, &v.path, t, ctx);
            }
            let tick = v.player.tick();
            if let Some(image) = tick.frame {
                v.preview.set_image(ctx, image);
            }
            if let Some(position) = tick.position {
                v.scrub = position;
                // Playing the result: jump over a removed part, and stop where the last kept part ends.
                if let (true, Some(kept)) = (v.player.is_playing(), v.kept.clone()) {
                    if in_spans(&kept, position) {
                        v.inside_kept = true;
                    } else if v.inside_kept {
                        match kept.iter().find(|s| s.0 >= position) {
                            Some(&(next, _)) => {
                                let until = v.player.until();
                                v.play(tools, next, until.filter(|u| *u > next));
                            }
                            None => v.player.stop(),
                        }
                    }
                }
            }
            if v.player.is_playing() {
                ctx.request_repaint();
            }
        }
        self.open_waiting(ctx);

        // Dropping a video on the window starts a project, or joins it to the end of the current one.
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            let mode = if self.clips.is_empty() { AddMode::Replace } else { AddMode::After };
            self.waiting = Some((path, mode));
            self.open_waiting(ctx);
        }
    }

    fn transport(&mut self, action: Transport, ctx: &egui::Context) {
        let (Some(tools), Some(v)) = (self.tools().cloned(), self.clips.get_mut(self.current)) else { return };
        let duration = v.info.duration;
        let gap = timeline::min_length(duration);
        match action {
            Transport::PlayPause if v.player.is_playing() => v.player.stop(),
            Transport::PlayPause => {
                let from = if v.scrub >= duration - 0.05 { 0.0 } else { v.scrub };
                v.play(&tools, from, None);
            }
            Transport::PlaySelection => v.play(&tools, v.start, Some(v.end)),
            Transport::Jump(seconds) => {
                let t = v.scrub + seconds;
                v.seek(&tools, ctx, t);
            }
            Transport::Step(n) => {
                let t = (v.scrub + n as f64 / v.info.fps.max(1.0)).clamp(0.0, duration);
                v.show_frame(&tools, ctx, t);
            }
            Transport::SetStart => v.start = v.scrub.min(v.end - gap).max(0.0),
            Transport::SetEnd => v.end = v.scrub.max(v.start + gap).min(duration),
        }
    }

    /// Space plays and pauses, arrows step (with Shift: jump 5 s), I and O set the start and end.
    fn keys(&mut self, ctx: &egui::Context) {
        if self.clips.is_empty() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let action = ctx.input_mut(|i| {
            let mut pressed = |mods, key| i.consume_key(mods, key);
            if pressed(Modifiers::NONE, Key::Space) {
                Some(Transport::PlayPause)
            } else if pressed(Modifiers::SHIFT, Key::ArrowLeft) {
                Some(Transport::Jump(-5.0))
            } else if pressed(Modifiers::SHIFT, Key::ArrowRight) {
                Some(Transport::Jump(5.0))
            } else if pressed(Modifiers::NONE, Key::ArrowLeft) {
                Some(Transport::Step(-1))
            } else if pressed(Modifiers::NONE, Key::ArrowRight) {
                Some(Transport::Step(1))
            } else if pressed(Modifiers::NONE, Key::I) {
                Some(Transport::SetStart)
            } else if pressed(Modifiers::NONE, Key::O) {
                Some(Transport::SetEnd)
            } else {
                None
            }
        });
        if let Some(action) = action {
            self.transport(action, ctx);
        }
    }

    fn start_convert(&mut self, ctx: &egui::Context) {
        let Some(tools) = self.tools().cloned() else { return };
        let job = make_job(&self.clips, &self.out_path, self.format, self.limit_size.then_some(self.target_mb));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (flag, ctx) = (cancel.clone(), ctx.clone());
        thread::spawn(move || {
            let report = |event| {
                let _ = tx.send(RunMsg::Event(event));
                ctx.request_repaint();
            };
            let result = encode::run(&tools, &job, &flag, &report);
            let _ = tx.send(RunMsg::Finished(result));
            ctx.request_repaint();
        });
        self.run = RunState::Running { status: "Starting…".into(), progress: 0.0, cancel };
        self.run_rx = Some(rx);
    }

    // -----------------------------------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut retry = false;
        let mut action = None;
        let ready = self.tools().is_some();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(ready, egui::Button::new("Open video…")).on_hover_text("Start over with another video").clicked() {
                action = Some(ClipAction::Add(AddMode::Replace));
            }
            match self.clips.get(self.current) {
                Some(v) => {
                    let i = &v.info;
                    ui.label(RichText::new(v.name()).strong());
                    ui.label(
                        RichText::new(format!(
                            "{}×{} · {:.0} fps · {} · {} · {}",
                            i.width,
                            i.height,
                            i.fps,
                            timeline::format_time(i.duration),
                            format_size(i.bytes),
                            i.codec
                        ))
                        .weak(),
                    );
                }
                None => {
                    ui.label(RichText::new("Open a video, or drop one on this window").weak());
                }
            }
        });
        if !self.clips.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Videos:").weak());
                for (i, clip) in self.clips.iter().enumerate() {
                    let mut name = clip.name();
                    if name.chars().count() > 22 {
                        name = format!("{}…", name.chars().take(21).collect::<String>());
                    }
                    let label = format!("{}  {} ({})", i + 1, name, timeline::format_time(clip.kept_total()));
                    if ui.selectable_label(i == self.current, label).clicked() && i != self.current {
                        action = Some(ClipAction::Select(i));
                    }
                }
                ui.separator();
                let (many, last) = (self.clips.len() > 1, self.clips.len().saturating_sub(1));
                if ui.add_enabled(self.current > 0, egui::Button::new("◀").small()).on_hover_text("Move this video earlier").clicked() {
                    action = Some(ClipAction::Move(-1));
                }
                if ui.add_enabled(self.current < last, egui::Button::new("▶").small()).on_hover_text("Move this video later").clicked() {
                    action = Some(ClipAction::Move(1));
                }
                if ui.add_enabled(many, egui::Button::new("Remove").small()).on_hover_text("Take this video out of the result").clicked() {
                    action = Some(ClipAction::Remove);
                }
                ui.separator();
                if ui.add_enabled(ready, egui::Button::new("+ Add video before…").small()).clicked() {
                    action = Some(ClipAction::Add(AddMode::Before));
                }
                if ui.add_enabled(ready, egui::Button::new("+ Add video after…").small()).clicked() {
                    action = Some(ClipAction::Add(AddMode::After));
                }
            });
        }
        match &self.tools {
            ToolState::Installing { status, frac } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(status);
                });
                match frac {
                    Some(f) => ui.add(egui::ProgressBar::new(*f).show_percentage()),
                    None => ui.add(egui::ProgressBar::new(0.0).animate(true)),
                };
            }
            ToolState::Failed(e) if !e.is_empty() => {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(BAD, format!("Could not install ffmpeg: {e}"));
                    retry = ui.button("Try again").clicked();
                });
            }
            _ => {}
        }
        if let Some(e) = &self.error {
            ui.colored_label(BAD, e);
        }
        ui.add_space(4.0);

        if retry {
            self.start_install(&ctx);
        }
        match action {
            Some(ClipAction::Select(i)) => {
                for clip in &mut self.clips {
                    clip.player.stop();
                }
                self.current = i;
                if let Some(tools) = self.tools().cloned() {
                    let t = self.clips[i].scrub;
                    self.clips[i].show_frame(&tools, &ctx, t);
                }
            }
            Some(ClipAction::Move(delta)) => {
                let to = self.current as isize + delta;
                if (0..self.clips.len() as isize).contains(&to) {
                    self.clips.swap(self.current, to as usize);
                    self.current = to as usize;
                }
            }
            Some(ClipAction::Remove) if self.clips.len() > 1 => {
                self.clips.remove(self.current);
                self.current = self.current.min(self.clips.len() - 1);
                if let Some(tools) = self.tools().cloned() {
                    let t = self.clips[self.current].scrub;
                    self.clips[self.current].show_frame(&tools, &ctx, t);
                }
            }
            Some(ClipAction::Add(mode)) => self.pick_file(mode, &ctx),
            _ => {}
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let tools = self.tools().cloned();
        let mut transport: Option<Transport> = None;

        // --- this video: play, position, selection, cuts, crop ------------------------------------------------
        {
            let Some(v) = self.clips.get_mut(self.current) else { return };
            let duration = v.info.duration;
            let frame = (v.info.width, v.info.height);
            let mut show: Option<f64> = None;
            let mut edit: Option<Edit> = None;

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let playing = v.player.is_playing();
                let label = if playing { "⏸ Pause" } else { "▶ Play" };
                if ui.add(egui::Button::new(label).min_size(vec2(84.0, 0.0))).on_hover_text("Space").clicked() {
                    transport = Some(Transport::PlayPause);
                }
                for (label, tip, action) in [
                    ("◀ 5s", "Back 5 seconds (Shift+←)", Transport::Jump(-5.0)),
                    ("◀|", "Previous picture (←)", Transport::Step(-1)),
                    ("|▶", "Next picture (→)", Transport::Step(1)),
                    ("5s ▶", "Forward 5 seconds (Shift+→)", Transport::Jump(5.0)),
                    ("▶ Selection", "Play only the selected part", Transport::PlaySelection),
                ] {
                    if ui.button(label).on_hover_text(tip).clicked() {
                        transport = Some(action);
                    }
                }
                ui.separator();
                if ui.button("Set start").on_hover_text("Selection starts here (I)").clicked() {
                    transport = Some(Transport::SetStart);
                }
                if ui.button("Set end").on_hover_text("Selection ends here (O)").clicked() {
                    transport = Some(Transport::SetEnd);
                }
                ui.separator();
                let muted = ui.checkbox(&mut v.player.muted, "Mute").changed();
                let volume = ui.add(egui::Slider::new(&mut v.player.volume, 0.0..=1.0).show_value(false)).changed();
                if muted || volume {
                    v.player.apply_volume();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} / {}", timeline::format_time(v.scrub), timeline::format_time(duration))).monospace());
                });
            });
            let mut pos = v.scrub;
            let seek = ui
                .scope(|ui| {
                    ui.spacing_mut().slider_width = (ui.available_width() - 12.0).max(50.0);
                    ui.add(egui::Slider::new(&mut pos, 0.0..=duration).show_value(false))
                })
                .inner;
            if seek.changed() {
                if v.player.is_playing() && v.resume_after_seek.is_none() {
                    v.resume_after_seek = Some(v.player.until());
                }
                show = Some(pos);
            }
            let seeking = seek.dragged() || seek.is_pointer_button_down_on();
            let resume = if seeking { None } else { v.resume_after_seek.take() };

            ui.add_space(4.0);
            let spans = v.spans();
            if let Some(handle) = timeline::range_slider(ui, duration, &mut v.start, &mut v.end, v.scrub, &spans) {
                show = Some(if handle == Handle::Start { v.start } else { v.end });
            }
            ui.horizontal_wrapped(|ui| {
                let gap = timeline::min_length(duration);
                ui.label("Start");
                if timeline::time_field(ui, &mut v.start, &mut v.start_text, (0.0, (v.end - gap).max(0.0))) {
                    show = Some(v.start);
                }
                ui.label("End");
                if timeline::time_field(ui, &mut v.end, &mut v.end_text, (v.start + gap, duration)) {
                    show = Some(v.end);
                }
                ui.label(RichText::new(format!("Selected {}", timeline::format_time(v.end - v.start))).strong());
                if ui.button("Keep only selection").on_hover_text("Throw away everything outside the selection").clicked() {
                    edit = Some(Edit::KeepOnly);
                }
                if ui.add_enabled(v.can_remove_selection(), egui::Button::new("Remove selection")).on_hover_text("Cut the selection out; the parts either side are joined").clicked() {
                    edit = Some(Edit::Remove);
                }
                if ui.button("Reset").on_hover_text("Keep the whole video again").clicked() {
                    edit = Some(Edit::Reset);
                }
            });
            let parts = spans.len();
            ui.label(
                RichText::new(format!(
                    "Keeping {} of {} from this video, in {parts} part{}{}",
                    timeline::format_time(v.kept_total()),
                    timeline::format_time(duration),
                    if parts == 1 { "" } else { "s" },
                    if v.kept.is_some() { " (playing skips the removed parts)" } else { "" }
                ))
                .weak(),
            );
            if let Some(edit) = edit {
                v.edit(edit);
                show = Some(v.scrub);
            }
            if let (Some(t), Some(tools)) = (show, &tools) {
                v.show_frame(tools, &ctx, t);
            }
            if let (Some(until), Some(tools)) = (resume, &tools) {
                let from = v.scrub;
                v.play(tools, from, until.filter(|u| *u > from));
            }

            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut v.crop_on, "Crop this video").on_hover_text("Drag on the picture to draw the area to keep");
                if v.crop_on {
                    for (label, rw, rh) in [("16:9", 16.0, 9.0), ("9:16", 9.0, 16.0), ("1:1", 1.0, 1.0), ("4:3", 4.0, 3.0), ("4:5", 4.0, 5.0)] {
                        if ui.small_button(label).clicked() {
                            v.crop.fit_aspect(rw, rh, frame);
                        }
                    }
                    let (fw, fh) = (frame.0 as f32, frame.1 as f32);
                    let (mut x, mut y) = (v.crop.x0.round(), v.crop.y0.round());
                    let (mut w, mut h) = (v.crop.size().0.round(), v.crop.size().1.round());
                    let field = |ui: &mut egui::Ui, name: &str, value: &mut f32, max: f32| {
                        ui.label(name);
                        ui.add(egui::DragValue::new(value).range(0.0..=max).speed(2.0).max_decimals(0));
                    };
                    field(ui, "x", &mut x, fw - MIN_CROP);
                    field(ui, "y", &mut y, fh - MIN_CROP);
                    field(ui, "width", &mut w, fw);
                    field(ui, "height", &mut h, fh);
                    let w = w.max(MIN_CROP).min(fw - x);
                    let h = h.max(MIN_CROP).min(fh - y);
                    v.crop = CropBox { x0: x, y0: y, x1: x + w, y1: y + h };
                } else {
                    ui.label(RichText::new("off").weak());
                }
            });
        }

        // --- the result: format, size, where it goes ----------------------------------------------------------
        ui.separator();
        let running = matches!(self.run, RunState::Running { .. });
        let before = self.format;
        ui.horizontal_wrapped(|ui| {
            ui.label("Convert to");
            egui::ComboBox::from_id_salt("format").selected_text(self.format.label()).width(200.0).show_ui(ui, |ui| {
                for format in Format::ALL {
                    ui.selectable_value(&mut self.format, format, format.label());
                }
            });
            ui.separator();
            ui.checkbox(&mut self.limit_size, "Keep the file under");
            ui.add_enabled_ui(self.limit_size, |ui| {
                let edit = ui
                    .add(egui::TextEdit::singleline(&mut self.size_text).desired_width(64.0).hint_text("20"))
                    .on_hover_text("Any size: 12.5, 500kb, 1.5gb (a plain number is megabytes)");
                if edit.changed() {
                    if let Some(mb) = parse_size(&self.size_text) {
                        self.target_mb = mb;
                    }
                }
                ui.label("MB");
                for preset in SIZE_PRESETS {
                    if ui.small_button(format!("{preset}")).clicked() {
                        self.target_mb = preset;
                        self.size_text = format!("{preset}");
                    }
                }
            });
        });
        if self.format != before {
            self.format_changed();
        }

        let size_problem = self.limit_size && parse_size(&self.size_text).is_none();
        let job = make_job(&self.clips, &self.out_path, self.format, self.limit_size.then_some(self.target_mb));
        let mut plan_problem = size_problem.then(|| "type a size such as 20, 12.5, 500kb or 1.5gb".to_string());
        if size_problem {
            ui.colored_label(BAD, plan_problem.as_deref().unwrap_or_default());
        } else if self.limit_size {
            match encode::plan(&job, job.target_bytes.unwrap_or(0), 0.96) {
                Ok(plan) => {
                    let text = match self.format {
                        Format::Gif => "Plan: the GIF is made smaller (size and frame rate) until it fits".to_string(),
                        Format::Mp3 => format!("Plan: {} kbit/s", plan.audio_kbps),
                        _ => {
                            let audio = if plan.audio_kbps > 0 { format!(" + {} kbit/s audio", plan.audio_kbps) } else { String::new() };
                            format!(
                                "Plan: about {:.2} Mbit/s video{audio}, at most 30 fps. The picture size is chosen by testing samples of your video: it keeps the full size when the size allows and shrinks only as much as needed.",
                                plan.video_kbps as f32 / 1000.0
                            )
                        }
                    };
                    ui.label(RichText::new(text).weak());
                }
                Err(e) => {
                    ui.colored_label(BAD, &e);
                    plan_problem = Some(e);
                }
            }
        } else {
            let text = match self.format {
                Format::Mp3 => "High quality (VBR); the file is as big as it needs to be",
                Format::Gif => "GIF at up to 640 px wide; this can get big quickly",
                Format::WebM => "High quality (CRF 32); the file is as big as it needs to be",
                _ => "High quality (CRF 20); the file is as big as it needs to be",
            };
            ui.label(RichText::new(text).weak());
        }
        let joined = if self.clips.len() > 1 { format!("{} videos joined, ", self.clips.len()) } else { String::new() };
        ui.label(RichText::new(format!("Result: {joined}{} long, {}×{}", timeline::format_time(job.duration()), job.canvas().0, job.canvas().1)).weak());

        ui.separator();
        ui.horizontal(|ui| {
            if ui.add_enabled(!running, egui::Button::new("Save as…")).clicked() {
                let name = self.out_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let mut dialog = rfd::FileDialog::new().add_filter(self.format.label(), &[self.format.extension()]).set_file_name(name);
                if let Some(dir) = self.out_path.parent().filter(|d| d.is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(mut path) = dialog.save_file() {
                    if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case(self.format.extension())) {
                        path.set_extension(self.format.extension());
                    }
                    self.out_path = path;
                }
            }
            ui.add(egui::Label::new(self.out_path.display().to_string()).truncate());
        });
        let same_file = self.clips.iter().any(|c| c.path == self.out_path);
        if same_file {
            ui.colored_label(BAD, "That would overwrite an original video; choose another name.");
        }

        ui.add_space(2.0);
        let (mut convert, mut cancel, mut reveal, mut play) = (false, false, false, false);
        match &self.run {
            RunState::Running { status, progress, .. } => {
                ui.horizontal(|ui| {
                    cancel = ui.button("Cancel").clicked();
                    ui.label(status);
                });
                ui.add(egui::ProgressBar::new(*progress).show_percentage());
            }
            other => {
                ui.horizontal(|ui| {
                    let ready = tools.is_some() && plan_problem.is_none() && !same_file;
                    let button = egui::Button::new(RichText::new("Convert").strong().color(Color32::WHITE)).fill(ACCENT).min_size(vec2(110.0, 28.0));
                    convert = ui.add_enabled(ready, button).clicked();
                    match other {
                        RunState::Done { path, bytes, detail } => {
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            ui.label(RichText::new(format!("Saved {name}: {} ({detail})", format_size(*bytes))).color(Color32::from_rgb(110, 200, 130)));
                            reveal = ui.button("Show in folder").clicked();
                            play = ui.button("Open").clicked();
                        }
                        RunState::Failed(msg) => {
                            ui.colored_label(BAD, msg);
                        }
                        _ => {}
                    }
                });
            }
        }
        ui.add_space(6.0);

        let finished = match &self.run {
            RunState::Done { path, .. } => Some(path.clone()),
            _ => None,
        };
        if let RunState::Running { cancel: flag, .. } = &self.run {
            if cancel {
                flag.store(true, Ordering::Relaxed);
            }
        }
        if let Some(path) = finished {
            if reveal {
                reveal_in_folder(&path);
            }
            if play {
                open_with_default_app(&path);
            }
        }
        if let Some(action) = transport {
            self.transport(action, &ctx);
        }
        if convert {
            self.start_convert(&ctx);
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let Some(v) = self.clips.get_mut(self.current) else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Open a video to start\n\nConvert it, cut parts out, crop it, join other videos to it,\nand keep the file under a size.")
                        .weak(),
                );
            });
            return;
        };
        let (area, resp) = ui.allocate_exact_size(ui.available_size(), Sense::drag());
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0, Color32::BLACK);
        if let Some(texture) = &v.preview.texture {
            let size = texture.size_vec2();
            let scale = (area.width() / size.x).min(area.height() / size.y);
            let image = Rect::from_center_size(area.center(), size * scale);
            painter.image(texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            if v.crop_on {
                crop_overlay(ui, &resp, &painter, image, (v.info.width, v.info.height), &mut v.crop);
            }
        }
        let note = match (&v.preview.error, v.preview.rx.is_some()) {
            (Some(e), _) => Some((e.clone(), BAD)),
            (None, true) if v.preview.texture.is_none() => Some(("Loading picture…".to_string(), Color32::GRAY)),
            _ => None,
        };
        if let Some((text, color)) = note {
            painter.text(area.center(), Align2::CENTER_CENTER, text, FontId::proportional(14.0), color);
        }
        painter.text(
            area.left_top() + vec2(8.0, 6.0),
            Align2::LEFT_TOP,
            timeline::format_time(v.scrub),
            FontId::monospace(13.0),
            Color32::from_white_alpha(200),
        );
    }
}

impl App {
    /// Testing hook: `VIDEO_SCREENSHOT=out.png` saves the window to a PNG a moment after start and quits.
    /// `VIDEO_DEMO=1` turns the crop box on and selects part of the video; `play` starts playback at 10 s;
    /// `edit` cuts a piece out of the middle and joins a second copy of the video on the end.
    fn screenshot_hook(&mut self, ctx: &egui::Context) {
        let Some(path) = std::env::var_os("VIDEO_SCREENSHOT") else { return };
        self.frames += 1;
        ctx.request_repaint();
        if self.frames == 5 && std::env::var_os("VIDEO_DEMO").is_some() {
            let demo = std::env::var("VIDEO_DEMO").unwrap_or_default();
            if let (Some(v), Some(tools)) = (self.clips.get_mut(self.current), self.tools.as_tools().cloned()) {
                let duration = v.info.duration;
                match demo.as_str() {
                    "play" => {
                        v.play(&tools, 10.0, None);
                        return;
                    }
                    "edit" => {
                        v.start = duration * 0.3;
                        v.end = duration * 0.5;
                        v.edit(Edit::Remove);
                        v.start = duration * 0.6;
                        v.end = duration * 0.7;
                        v.show_frame(&tools, ctx, duration * 0.6);
                        let path = v.path.clone();
                        self.open(path, AddMode::After, &tools, ctx);
                        self.current = 0;
                        return;
                    }
                    _ => {}
                }
                v.crop_on = true;
                v.start = duration * 0.2;
                v.end = duration * 0.7;
                v.show_frame(&tools, ctx, v.end);
            }
        }
        if self.frames == 300 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            if let Ok(file) = std::fs::File::create(&path) {
                let mut enc = png::Encoder::new(file, image.width() as u32, image.height() as u32);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                if let Ok(mut w) = enc.write_header() {
                    let _ = w.write_image_data(image.as_raw());
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl ToolState {
    fn as_tools(&self) -> Option<&Tools> {
        match self {
            ToolState::Ready(t) => Some(t),
            _ => None,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.keys(&ctx);
        self.screenshot_hook(&ctx);
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        if !self.clips.is_empty() {
            egui::Panel::bottom("controls").show(ui, |ui| self.controls(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.preview(ui));
    }
}

/// Draws the crop box over the picture and lets the mouse draw, move and resize it.
fn crop_overlay(ui: &egui::Ui, resp: &egui::Response, painter: &egui::Painter, image: Rect, frame: (u32, u32), crop: &mut CropBox) {
    const GRAB: f32 = 9.0;
    let (fw, fh) = (frame.0 as f32, frame.1 as f32);
    let to_screen = |x: f32, y: f32| pos2(image.left() + x / fw * image.width(), image.top() + y / fh * image.height());
    let to_source = |p: Pos2| {
        pos2(((p.x - image.left()) / image.width() * fw).clamp(0.0, fw), ((p.y - image.top()) / image.height() * fh).clamp(0.0, fh))
    };
    let screen = Rect::from_min_max(to_screen(crop.x0, crop.y0), to_screen(crop.x1, crop.y1));

    let hit = |p: Pos2| {
        let across_y = p.y >= screen.top() - GRAB && p.y <= screen.bottom() + GRAB;
        let across_x = p.x >= screen.left() - GRAB && p.x <= screen.right() + GRAB;
        let l = (p.x - screen.left()).abs() <= GRAB && across_y;
        let r = (p.x - screen.right()).abs() <= GRAB && across_y;
        let t = (p.y - screen.top()).abs() <= GRAB && across_x;
        let b = (p.y - screen.bottom()).abs() <= GRAB && across_x;
        if l || r || t || b {
            CropDrag::Edges { l, r, t, b }
        } else if screen.contains(p) {
            let s = to_source(p);
            CropDrag::Move(pos2(s.x - crop.x0, s.y - crop.y0))
        } else {
            CropDrag::New(to_source(p))
        }
    };

    let id = Id::new("crop-drag");
    if let Some(p) = resp.hover_pos().filter(|p| image.contains(*p)) {
        use egui::CursorIcon::*;
        ui.ctx().set_cursor_icon(match hit(p) {
            CropDrag::Edges { l, r, t, b } => match ((l || r), (t || b)) {
                (true, true) if (l && t) || (r && b) => ResizeNwSe,
                (true, true) => ResizeNeSw,
                (true, false) => ResizeHorizontal,
                _ => ResizeVertical,
            },
            CropDrag::Move(_) => Move,
            CropDrag::New(_) => Crosshair,
        });
    }
    if resp.drag_started() {
        let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        if let Some(p) = press.filter(|p| image.contains(*p)) {
            let mode = hit(p);
            if let CropDrag::New(a) = mode {
                *crop = CropBox { x0: a.x, y0: a.y, x1: a.x, y1: a.y };
            }
            ui.data_mut(|d| d.insert_temp(id, Some(mode)));
        } else {
            ui.data_mut(|d| d.insert_temp(id, None::<CropDrag>));
        }
    }
    if resp.dragged() {
        if let (Some(p), Some(mode)) = (resp.interact_pointer_pos(), ui.data(|d| d.get_temp::<Option<CropDrag>>(id)).flatten()) {
            let s = to_source(p);
            match mode {
                CropDrag::New(a) => *crop = CropBox { x0: a.x.min(s.x), y0: a.y.min(s.y), x1: a.x.max(s.x), y1: a.y.max(s.y) },
                CropDrag::Move(off) => {
                    let (w, h) = crop.size();
                    let x0 = (s.x - off.x).clamp(0.0, fw - w);
                    let y0 = (s.y - off.y).clamp(0.0, fh - h);
                    *crop = CropBox { x0, y0, x1: x0 + w, y1: y0 + h };
                }
                CropDrag::Edges { l, r, t, b } => {
                    if l {
                        crop.x0 = s.x.min(crop.x1 - MIN_CROP).max(0.0);
                    }
                    if r {
                        crop.x1 = s.x.max(crop.x0 + MIN_CROP).min(fw);
                    }
                    if t {
                        crop.y0 = s.y.min(crop.y1 - MIN_CROP).max(0.0);
                    }
                    if b {
                        crop.y1 = s.y.max(crop.y0 + MIN_CROP).min(fh);
                    }
                }
            }
        }
    }

    let screen = Rect::from_min_max(to_screen(crop.x0, crop.y0), to_screen(crop.x1, crop.y1));
    let dim = Color32::from_black_alpha(150);
    painter.rect_filled(Rect::from_min_max(image.left_top(), pos2(image.right(), screen.top())), 0, dim);
    painter.rect_filled(Rect::from_min_max(pos2(image.left(), screen.bottom()), image.right_bottom()), 0, dim);
    painter.rect_filled(Rect::from_min_max(pos2(image.left(), screen.top()), pos2(screen.left(), screen.bottom())), 0, dim);
    painter.rect_filled(Rect::from_min_max(pos2(screen.right(), screen.top()), pos2(image.right(), screen.bottom())), 0, dim);
    let faint = Stroke::new(1.0, Color32::from_white_alpha(60));
    for i in 1..3 {
        let f = i as f32 / 3.0;
        let x = screen.left() + screen.width() * f;
        let y = screen.top() + screen.height() * f;
        painter.line_segment([pos2(x, screen.top()), pos2(x, screen.bottom())], faint);
        painter.line_segment([pos2(screen.left(), y), pos2(screen.right(), y)], faint);
    }
    painter.rect_stroke(screen, CornerRadius::ZERO, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
    for corner in [screen.left_top(), screen.right_top(), screen.left_bottom(), screen.right_bottom()] {
        painter.rect_filled(Rect::from_center_size(corner, vec2(8.0, 8.0)), 1, Color32::WHITE);
    }
    let (w, h) = crop.size();
    painter.text(
        screen.left_top() + vec2(6.0, 4.0),
        Align2::LEFT_TOP,
        format!("{}×{}", w.round(), h.round()),
        FontId::monospace(12.0),
        Color32::WHITE,
    );
}

fn format_size(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb >= 1000.0 {
        format!("{:.2} GB", mb / 1000.0)
    } else if mb < 1.0 {
        format!("{:.0} KB", bytes as f64 / 1e3)
    } else {
        format!("{mb:.1} MB")
    }
}

fn stem_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into())
}

/// `dir/stem.ext`, or `dir/stem (2).ext` and so on if that is taken.
fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| dir.join(format!("{stem} ({n}).{ext}"))).find(|p| !p.exists()).expect("endless range")
}

fn default_output(input: &Path, format: Format) -> PathBuf {
    let stem = stem_of(input);
    let same_type = input.extension().is_some_and(|e| e.eq_ignore_ascii_case(format.extension()));
    let stem = if same_type { format!("{stem}_edited") } else { stem };
    unique_path(input.parent().unwrap_or(Path::new("")), &stem, format.extension())
}

fn reveal_in_folder(path: &Path) {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(windows)]
    let _ = Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(dir) = path.parent() {
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
}

fn open_with_default_app(path: &Path) {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(path).spawn();
    #[cfg(windows)]
    let _ = Command::new("explorer").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = Command::new("xdg-open").arg(path).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(duration: f64) -> Clip {
        let info = Info { duration, width: 1920, height: 1080, fps: 30.0, has_audio: true, bytes: 0, bitrate_kbps: None, codec: "h264".into() };
        Clip::new("a.mov".into(), info)
    }

    #[test]
    fn aspect_box_is_centred_and_fits() {
        let mut b = CropBox::inset(1920, 1080);
        b.fit_aspect(1.0, 1.0, (1920, 1080));
        assert_eq!((b.size().0, b.size().1), (1080.0, 1080.0));
        assert_eq!(b.x0, 420.0);
        b.fit_aspect(9.0, 16.0, (1920, 1080));
        assert_eq!(b.size().1, 1080.0);
        assert!((b.size().0 - 607.5).abs() < 0.01);
        b.fit_aspect(16.0, 9.0, (1080, 1920));
        assert_eq!(b.size().0, 1080.0);
    }

    #[test]
    fn output_never_clobbers_anything() {
        let dir = std::env::temp_dir().join(format!("video-name-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("clip.mp4");
        std::fs::write(&input, b"x").unwrap();
        assert_eq!(default_output(&input, Format::Mp4), dir.join("clip_edited.mp4"));
        assert_eq!(default_output(&input, Format::WebM), dir.join("clip.webm"));
        // `clip.mp4` exists (it is the input above), so a .mov of the same name gets the next free name.
        assert_eq!(default_output(&dir.join("clip.mov"), Format::Mp4), dir.join("clip (2).mp4"));
        assert_eq!(default_output(&dir.join("other.mov"), Format::Mp4), dir.join("other.mp4"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_are_typed_freely() {
        assert_eq!(parse_size("20"), Some(20.0));
        assert_eq!(parse_size(" 12.5 "), Some(12.5));
        assert_eq!(parse_size("12,5"), Some(12.5));
        assert_eq!(parse_size("500kb"), Some(0.5));
        assert_eq!(parse_size("1.5 GB"), Some(1500.0));
        assert_eq!(parse_size("8MB"), Some(8.0));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size("5 parsecs"), None);
        assert_eq!(parse_size("0"), None);
        assert_eq!(parse_size("10kb"), None);
    }

    #[test]
    fn the_selection_is_the_trim_until_it_is_used_to_cut() {
        let mut c = clip(60.0);
        c.start = 10.0;
        c.end = 50.0;
        assert_eq!(c.spans(), vec![(10.0, 50.0)]);
        // Cutting a piece out of the middle of what is selected:
        c.edit(Edit::KeepOnly);
        assert_eq!(c.kept, Some(vec![(10.0, 50.0)]));
        c.start = 20.0;
        c.end = 30.0;
        c.edit(Edit::Remove);
        assert_eq!(c.spans(), vec![(10.0, 20.0), (30.0, 50.0)]);
        assert_eq!(c.kept_total(), 30.0);
        // The selection starts over after a cut, and cutting everything is refused.
        assert_eq!((c.start, c.end), (0.0, 60.0));
        assert!(!c.can_remove_selection());
        c.edit(Edit::Remove);
        assert_eq!(c.spans(), vec![(10.0, 20.0), (30.0, 50.0)]);
        c.edit(Edit::Reset);
        assert_eq!(c.spans(), vec![(0.0, 60.0)]);
    }

    #[test]
    fn removing_without_a_trim_cuts_from_the_whole_video() {
        let mut c = clip(60.0);
        c.start = 20.0;
        c.end = 30.0;
        c.edit(Edit::Remove);
        assert_eq!(c.spans(), vec![(0.0, 20.0), (30.0, 60.0)]);
    }

    #[test]
    fn playback_position_is_inside_kept_parts() {
        let kept = vec![(0.0, 20.0), (30.0, 60.0)];
        assert!(in_spans(&kept, 5.0) && in_spans(&kept, 30.0));
        assert!(!in_spans(&kept, 20.0) && !in_spans(&kept, 25.0) && !in_spans(&kept, 60.0));
    }
}
