//! Runs a plan with FFmpeg on a background thread and reports what happens.

use crate::log::{LogKind, LogLine};
use crate::media::human_bytes;
use crate::plan::{Pass, PartPlan, Plan};
use crate::tools::{display_command, quiet_command, Tools};
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const MAX_ATTEMPTS: u32 = 4;
const GIF_ATTEMPTS: u32 = 6;
static NEXT_JOB: AtomicU32 = AtomicU32::new(1);

#[derive(Debug)]
pub enum Event {
    Log(LogLine),
    /// A part started a stage, such as the analysis run.
    Stage { part: usize, label: String },
    Progress { part: usize, fraction: f32, speed: Option<f32> },
    PartDone { part: usize, path: PathBuf, bytes: u64, over_limit: bool },
    PartFailed { part: usize, message: String },
    Finished { cancelled: bool },
}

/// A running export.
pub struct ExportJob {
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl ExportJob {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

struct Runner {
    plan: Plan,
    tools: Tools,
    events: Sender<Event>,
    cancel: Arc<AtomicBool>,
    wake: Arc<dyn Fn() + Send + Sync>,
    work_dir: PathBuf,
}

/// Start exporting. `wake` is called whenever there is something new to show.
pub fn start(plan: Plan, tools: Tools, wake: Arc<dyn Fn() + Send + Sync>) -> ExportJob {
    let (events, receiver) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let job_number = NEXT_JOB.fetch_add(1, Ordering::SeqCst);
    let work_dir = std::env::temp_dir().join(format!("cutline-{}-{job_number}", std::process::id()));
    let runner = Runner { plan, tools, events, cancel: cancel.clone(), wake, work_dir };
    thread::spawn(move || runner.run());
    ExportJob { events: receiver, cancel }
}

/// How a single FFmpeg run ended.
enum RunEnd {
    Cancelled,
    Failed(String),
}

impl Runner {
    fn send(&self, event: Event) {
        let _ = self.events.send(event);
        (self.wake)();
    }

    fn log(&self, kind: LogKind, text: impl Into<String>) {
        self.send(Event::Log(LogLine::new(kind, text)));
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn run(self) {
        let _ = std::fs::create_dir_all(&self.work_dir);
        let count = self.plan.parts.len();
        self.log(
            LogKind::Info,
            format!(
                "Exporting {} {}x{} at {:.2} fps into {count} {}",
                self.plan.ext.to_uppercase(),
                self.plan.width,
                self.plan.height,
                self.plan.fps,
                if count == 1 { "video" } else { "equal videos" },
            ),
        );
        let mut was_cancelled = false;
        for part in self.plan.parts.clone() {
            if self.cancelled() {
                was_cancelled = true;
                break;
            }
            match self.export_part(&part) {
                Ok(()) => {}
                Err(RunEnd::Cancelled) => {
                    was_cancelled = true;
                    let _ = std::fs::remove_file(&part.output);
                    self.send(Event::PartFailed { part: part.index, message: "Cancelled".to_string() });
                    break;
                }
                Err(RunEnd::Failed(message)) => {
                    let _ = std::fs::remove_file(&part.output);
                    self.log(LogKind::Error, format!("Part {} failed: {message}", part.index + 1));
                    self.send(Event::PartFailed { part: part.index, message });
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.work_dir);
        self.send(Event::Finished { cancelled: was_cancelled });
    }

    fn export_part(&self, part: &PartPlan) -> Result<(), RunEnd> {
        let number = part.index + 1;
        let passlog = self.work_dir.join(format!("part{number}"));
        let two_pass = self.plan.two_pass(part);
        let (mut kbps, mut scale) = (part.video_kbps, 1.0_f64);
        if two_pass {
            self.send(Event::Stage { part: part.index, label: "Preparing".to_string() });
            self.run_ffmpeg(part, Pass::First, kbps, scale, &passlog, 0.0, 0.5)?;
        }
        let base = if two_pass { 0.5 } else { 0.0 };
        let attempts = if self.plan.is_gif { GIF_ATTEMPTS } else { MAX_ATTEMPTS };
        let mut attempt = 1;
        loop {
            let label = if attempt == 1 { "Encoding".to_string() } else { format!("Fitting to size, try {attempt}") };
            self.send(Event::Stage { part: part.index, label });
            self.run_ffmpeg(part, Pass::Final, kbps, scale, &passlog, base, 1.0)?;
            let bytes = std::fs::metadata(&part.output)
                .map(|meta| meta.len())
                .map_err(|_| RunEnd::Failed("FFmpeg didn't write the file.".to_string()))?;
            if bytes == 0 {
                return Err(RunEnd::Failed("FFmpeg wrote an empty file.".to_string()));
            }
            let over = part.limit_bytes.is_some_and(|limit| bytes > limit);
            if !over || attempt >= attempts {
                let name = part.output.file_name().unwrap_or_default().to_string_lossy().into_owned();
                if over {
                    self.log(
                        LogKind::Warning,
                        format!("{name} is {} after {attempt} tries, still over its limit.", human_bytes(bytes as f64)),
                    );
                } else {
                    self.log(LogKind::Success, format!("Saved {name} ({})", human_bytes(bytes as f64)));
                }
                self.send(Event::PartDone { part: part.index, path: part.output.clone(), bytes, over_limit: over });
                return Ok(());
            }
            let limit = part.limit_bytes.unwrap_or(bytes);
            let ratio = limit as f64 / bytes as f64;
            self.log(
                LogKind::Warning,
                format!(
                    "Part {number} came out at {}, over its {} limit. Trying again smaller.",
                    human_bytes(bytes as f64),
                    human_bytes(limit as f64)
                ),
            );
            if self.plan.is_gif {
                scale *= (ratio * 0.92).sqrt();
            } else {
                kbps = kbps.map(|value| value * ratio * 0.96);
            }
            attempt += 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_ffmpeg(
        &self,
        part: &PartPlan,
        pass: Pass,
        kbps: Option<f64>,
        scale: f64,
        passlog: &Path,
        from: f32,
        to: f32,
    ) -> Result<(), RunEnd> {
        let args: Vec<OsString> = self.plan.args(part, pass, kbps, scale, passlog);
        self.log(LogKind::Command, display_command(self.tools.ffmpeg.as_os_str(), &args));
        let mut child = quiet_command(self.tools.ffmpeg.as_os_str())
            .args(&args)
            .current_dir(&self.work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| RunEnd::Failed(format!("Couldn't start FFmpeg: {error}")))?;

        let stderr = child.stderr.take().map(BufReader::new);
        let stdout = child.stdout.take().map(BufReader::new);
        let events = self.events.clone();
        let wake = self.wake.clone();
        let error_reader = thread::spawn(move || {
            let mut last_errors: Vec<String> = Vec::new();
            let Some(mut reader) = stderr else { return last_errors };
            let mut buffer = Vec::new();
            loop {
                buffer.clear();
                match reader.read_until(b'\n', &mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let text = String::from_utf8_lossy(&buffer);
                let line = classify(text.trim_end());
                if line.kind == LogKind::Error {
                    last_errors.push(line.text.clone());
                    if last_errors.len() > 3 {
                        last_errors.remove(0);
                    }
                }
                if !line.text.is_empty() {
                    let _ = events.send(Event::Log(line));
                    wake();
                }
            }
            last_errors
        });

        let duration = part.duration.max(0.001);
        let events = self.events.clone();
        let wake = self.wake.clone();
        let index = part.index;
        let progress_reader = thread::spawn(move || {
            let Some(reader) = stdout else { return };
            let mut block = ProgressBlock::default();
            for line in reader.lines().map_while(Result::ok) {
                let Some((key, value)) = line.split_once('=') else { continue };
                match key.trim() {
                    "out_time_us" | "out_time_ms" => block.out_us = value.trim().parse().ok(),
                    "frame" => block.frame = value.trim().to_string(),
                    "speed" => block.speed = value.trim().trim_end_matches('x').parse().ok(),
                    "bitrate" => block.bitrate = value.trim().to_string(),
                    "progress" => {
                        if let Some(us) = block.out_us {
                            let done = (us as f64 / 1_000_000.0 / duration).clamp(0.0, 1.0) as f32;
                            let fraction = from + (to - from) * done;
                            let _ = events.send(Event::Progress { part: index, fraction, speed: block.speed });
                            let status = format!(
                                "frame={} time={:.1}s bitrate={} speed={}x",
                                block.frame,
                                us as f64 / 1_000_000.0,
                                block.bitrate,
                                block.speed.map_or("?".to_string(), |s| format!("{s:.2}")),
                            );
                            let _ = events.send(Event::Log(LogLine::new(LogKind::Live, status)));
                            wake();
                        }
                    }
                    _ => {}
                }
            }
        });

        let status = loop {
            if self.cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = error_reader.join();
                let _ = progress_reader.join();
                return Err(RunEnd::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => thread::sleep(Duration::from_millis(40)),
                Err(error) => return Err(RunEnd::Failed(error.to_string())),
            }
        };
        let errors = error_reader.join().unwrap_or_default();
        let _ = progress_reader.join();
        if status.success() {
            Ok(())
        } else if errors.is_empty() {
            Err(RunEnd::Failed(format!("FFmpeg stopped unexpectedly ({status}).")))
        } else {
            Err(RunEnd::Failed(errors.join(" / ")))
        }
    }
}

#[derive(Default)]
struct ProgressBlock {
    out_us: Option<i64>,
    frame: String,
    speed: Option<f32>,
    bitrate: String,
}

/// Sort a line of FFmpeg's output by its `[level]` prefix (from `-loglevel level+info`).
fn classify(text: &str) -> LogLine {
    let mut kind = LogKind::Output;
    let mut rest = text;
    for (tag, level) in [
        ("[fatal] ", LogKind::Error),
        ("[panic] ", LogKind::Error),
        ("[error] ", LogKind::Error),
        ("[warning] ", LogKind::Warning),
        ("[info] ", LogKind::Output),
    ] {
        if let Some(position) = text.find(tag) {
            kind = level;
            rest = &text[position + tag.len()..];
            let prefix = text[..position].trim();
            if !prefix.is_empty() {
                return LogLine::new(kind, format!("{prefix} {rest}"));
            }
            break;
        }
    }
    LogLine::new(kind, rest)
}

/// Time left, from how far the export has come.
pub fn eta(started: Instant, fraction: f32) -> Option<Duration> {
    (fraction > 0.02).then(|| {
        let elapsed = started.elapsed().as_secs_f32();
        Duration::from_secs_f32(((elapsed / fraction) - elapsed).max(0.0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_read_from_the_prefix() {
        assert_eq!(classify("[error] Conversion failed!").kind, LogKind::Error);
        assert_eq!(classify("[error] Conversion failed!").text, "Conversion failed!");
        assert_eq!(classify("[libx264 @ 0x55] [warning] buffer").kind, LogKind::Warning);
        assert_eq!(classify("[libx264 @ 0x55] [warning] buffer").text, "[libx264 @ 0x55] buffer");
        assert_eq!(classify("plain").kind, LogKind::Output);
    }
}
