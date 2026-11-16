//! Frames for the preview window, on their own thread so dragging the playhead never stutters.

use crate::media::grab_frame;
use crate::plan::{fit_filter, Fit};
use crate::tools::{quiet_command, Tools};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread;

const PLAY_FPS: f64 = 30.0;

/// A clip on the timeline, as the preview needs it.
#[derive(Clone, Debug)]
pub struct TimelineClip {
    pub path: PathBuf,
    /// Where it starts on the timeline.
    pub start: f64,
    /// Where it starts in the source file (later than 0 when the beginning was cut).
    pub source_start: f64,
    pub duration: f64,
}

#[derive(Clone, Debug)]
pub struct PreviewShape {
    pub width: u32,
    pub height: u32,
    pub fit: Fit,
}

impl PreviewShape {
    /// The final video's shape, small enough for a quick preview.
    pub fn for_output(width: u32, height: u32, fit: Fit) -> Self {
        let scale = (854.0 / width as f64).min(480.0 / height as f64).min(1.0);
        let even = |value: f64| ((value.round() as u32).max(2) / 2) * 2;
        Self { width: even(width as f64 * scale), height: even(height as f64 * scale), fit }
    }
}

enum Request {
    Show { clips: Arc<Vec<TimelineClip>>, at: f64, shape: PreviewShape },
    Play { clips: Arc<Vec<TimelineClip>>, at: f64, shape: PreviewShape },
    Stop,
}

pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    pub at: f64,
}

pub enum Update {
    Frame(Frame),
    /// Playback reached the end of the timeline.
    Ended,
}

pub struct Preview {
    requests: Sender<Request>,
    pub updates: Receiver<Update>,
}

impl Preview {
    pub fn new(tools: Tools, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (requests, request_rx) = channel();
        let (update_tx, updates) = sync_channel(4);
        thread::spawn(move || worker(tools, request_rx, update_tx, wake));
        Self { requests, updates }
    }

    pub fn show(&self, clips: Arc<Vec<TimelineClip>>, at: f64, shape: PreviewShape) {
        let _ = self.requests.send(Request::Show { clips, at, shape });
    }

    pub fn play(&self, clips: Arc<Vec<TimelineClip>>, at: f64, shape: PreviewShape) {
        let _ = self.requests.send(Request::Play { clips, at, shape });
    }

    pub fn stop(&self) {
        let _ = self.requests.send(Request::Stop);
    }
}

struct Worker {
    tools: Tools,
    updates: SyncSender<Update>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

fn worker(tools: Tools, requests: Receiver<Request>, updates: SyncSender<Update>, wake: Arc<dyn Fn() + Send + Sync>) {
    let worker = Worker { tools, updates, wake };
    let mut next = None;
    loop {
        let request = match next.take() {
            Some(request) => request,
            None => match requests.recv() {
                Ok(request) => request,
                Err(_) => return,
            },
        };
        // While scrubbing, only the newest position matters.
        let mut request = request;
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }
        match request {
            Request::Show { clips, at, shape } => worker.show(&clips, at, &shape),
            Request::Play { clips, at, shape } => next = worker.play(&clips, at, &shape, &requests),
            Request::Stop => {}
        }
    }
}

impl Worker {
    fn send(&self, update: Update) {
        match self.updates.try_send(update) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            // The window is behind; skip this frame rather than wait for it.
            Err(TrySendError::Full(_)) => {}
        }
        (self.wake)();
    }

    fn show(&self, clips: &[TimelineClip], at: f64, shape: &PreviewShape) {
        let Some(clip) = clip_at(clips, at) else { return };
        let within = clip.source_start + (at - clip.start).clamp(0.0, (clip.duration - 0.05).max(0.0));
        let filter = fit_filter(shape.width, shape.height, shape.fit);
        if let Some(rgba) = grab_frame(&self.tools, &clip.path, within, &filter, shape.width, shape.height) {
            self.send(Update::Frame(Frame { width: shape.width as usize, height: shape.height as usize, rgba, at }));
        }
    }

    /// Play from `at` until the end or until another request arrives, which is returned.
    fn play(
        &self,
        clips: &[TimelineClip],
        at: f64,
        shape: &PreviewShape,
        requests: &Receiver<Request>,
    ) -> Option<Request> {
        let first = clips.iter().position(|clip| at < clip.start + clip.duration)?;
        let mut from = at;
        for clip in &clips[first..] {
            let within = (from - clip.start).max(0.0);
            let spawned = self.spawn_player(clip, within, shape);
            let Some(mut child) = spawned else {
                from = clip.start + clip.duration;
                continue;
            };
            let frame_size = shape.width as usize * shape.height as usize * 4;
            let mut number = 0_u64;
            let mut buffer = vec![0_u8; frame_size];
            let mut reader = child.stdout.take()?;
            loop {
                match requests.try_recv() {
                    Ok(request) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Some(request);
                    }
                    Err(TryRecvError::Disconnected) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return None;
                    }
                    Err(TryRecvError::Empty) => {}
                }
                if reader.read_exact(&mut buffer).is_err() {
                    break;
                }
                let time = clip.start + within + number as f64 / PLAY_FPS;
                number += 1;
                self.send(Update::Frame(Frame {
                    width: shape.width as usize,
                    height: shape.height as usize,
                    rgba: buffer.clone(),
                    at: time,
                }));
            }
            let _ = child.wait();
            from = clip.start + clip.duration;
        }
        self.send(Update::Ended);
        None
    }

    fn spawn_player(&self, clip: &TimelineClip, within: f64, shape: &PreviewShape) -> Option<Child> {
        let filter = format!("{},fps={PLAY_FPS}", fit_filter(shape.width, shape.height, shape.fit));
        quiet_command(self.tools.ffmpeg.as_os_str())
            .args(["-hide_banner", "-loglevel", "error", "-re", "-ss"])
            .arg(format!("{:.3}", clip.source_start + within))
            .arg("-t")
            .arg(format!("{:.3}", (clip.duration - within).max(0.05)))
            .arg("-i")
            .arg(&clip.path)
            .args(["-an", "-vf", &filter, "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    }
}

fn clip_at(clips: &[TimelineClip], at: f64) -> Option<&TimelineClip> {
    clips.iter().find(|clip| at < clip.start + clip.duration).or_else(|| clips.last())
}
