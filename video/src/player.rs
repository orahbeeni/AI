//! Playing a video inside the window: ffmpeg decodes the picture and the sound, the window shows the frames
//! on time and the sound card plays the audio. Nothing is written to disk.

use crate::ffmpeg::{self, Info, Tools};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use eframe::egui::ColorImage;
use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Playback is decoded at a reduced size and rate; this is for choosing cut points, not for the final picture.
const MAX_WIDTH: u32 = 960;
const MAX_FPS: f64 = 30.0;
/// How much decoded audio is kept ready, in seconds.
const AUDIO_AHEAD: usize = 2;

struct Session {
    stop: Arc<AtomicBool>,
    /// Lets the audio start once the first picture is on screen, so the two begin together.
    go: Arc<AtomicBool>,
    frames: Receiver<Vec<u8>>,
    /// A picture that arrived before its time: (its number, its pixels).
    waiting: Option<(u64, Vec<u8>)>,
    received: u64,
    size: [usize; 2],
    fps: f64,
    from: f64,
    until: Option<f64>,
    duration: f64,
    /// When picture 0 was shown; later pictures are timed from here.
    clock: Option<Instant>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub struct Tick {
    /// The newest picture that is due, if there is one.
    pub frame: Option<ColorImage>,
    /// Where playback is now. `None` when nothing was playing.
    pub position: Option<f64>,
}

pub struct Player {
    session: Option<Session>,
    gain: Arc<AtomicU32>,
    pub volume: f32,
    pub muted: bool,
}

impl Default for Player {
    fn default() -> Self {
        let player = Player { session: None, gain: Arc::new(AtomicU32::new(1.0f32.to_bits())), volume: 0.8, muted: false };
        player.apply_volume();
        player
    }
}

impl Player {
    pub fn is_playing(&self) -> bool {
        self.session.is_some()
    }

    /// The point playback will stop at by itself, if it was started with one.
    pub fn until(&self) -> Option<f64> {
        self.session.as_ref().and_then(|s| s.until)
    }

    pub fn apply_volume(&self) {
        let gain = if self.muted { 0.0 } else { self.volume * self.volume };
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    pub fn stop(&mut self) {
        self.session = None;
    }

    /// Starts playing `path` at `from` seconds, stopping at `until` if given.
    pub fn play(&mut self, tools: &Tools, path: &Path, info: &Info, from: f64, until: Option<f64>) {
        self.stop();
        let fps = info.fps.clamp(1.0, MAX_FPS);
        let width = info.width.min(MAX_WIDTH);
        let height = ((width as f64 * info.height as f64 / info.width as f64).round() as u32).max(2);
        let stop = Arc::new(AtomicBool::new(false));
        let go = Arc::new(AtomicBool::new(false));
        let (tx, frames) = mpsc::sync_channel(6);

        let (tools_v, path_v, stop_v) = (tools.clone(), path.to_path_buf(), stop.clone());
        thread::spawn(move || decode_video(&tools_v, &path_v, from, fps, (width, height), tx, &stop_v));
        if info.has_audio {
            let (tools_a, path_a) = (tools.clone(), path.to_path_buf());
            let (stop_a, go_a, gain) = (stop.clone(), go.clone(), self.gain.clone());
            thread::spawn(move || play_audio(&tools_a, &path_a, from, &stop_a, &go_a, &gain));
        }
        self.session = Some(Session {
            stop,
            go,
            frames,
            waiting: None,
            received: 0,
            size: [width as usize, height as usize],
            fps,
            from,
            until,
            duration: info.duration,
            clock: None,
        });
    }

    /// Call every frame of the window. Picks the picture that is due and says where playback is.
    pub fn tick(&mut self) -> Tick {
        let Some(s) = self.session.as_mut() else { return Tick { frame: None, position: None } };
        let now = Instant::now();
        let mut due = None;
        let mut ended = false;
        loop {
            let (index, pixels) = match s.waiting.take() {
                Some(w) => w,
                None => match s.frames.try_recv() {
                    Ok(pixels) => {
                        s.received += 1;
                        (s.received - 1, pixels)
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        ended = true;
                        break;
                    }
                },
            };
            if s.clock.is_none() {
                s.clock = Some(now);
                s.go.store(true, Ordering::Relaxed);
            }
            let elapsed = now.duration_since(s.clock.unwrap_or(now)).as_secs_f64();
            if index as f64 / s.fps <= elapsed {
                due = Some(pixels); // late pictures are skipped, keeping the newest
            } else {
                s.waiting = Some((index, pixels));
                break;
            }
        }

        let mut position = s.from + s.clock.map_or(0.0, |c| now.duration_since(c).as_secs_f64());
        let mut finished = ended && s.waiting.is_none();
        if finished {
            position = (s.from + s.received as f64 / s.fps).min(s.duration);
        }
        if let Some(until) = s.until.filter(|u| position >= *u) {
            position = until;
            finished = true;
        }
        let frame = due.map(|pixels| ColorImage::from_rgba_unmultiplied(s.size, &pixels));
        let position = position.min(s.duration);
        if finished {
            self.session = None;
        }
        Tick { frame, position: Some(position) }
    }
}

/// Reads decoded pictures from ffmpeg and hands them over, until told to stop or the video ends.
fn decode_video(tools: &Tools, path: &Path, from: f64, fps: f64, (w, h): (u32, u32), tx: SyncSender<Vec<u8>>, stop: &AtomicBool) {
    let child = ffmpeg::command(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-ss", &format!("{from:.3}"), "-i"])
        .arg(path)
        .args(["-an", "-vf", &format!("fps={fps:.3},scale={w}:{h}"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return };
    if let Some(mut out) = child.stdout.take() {
        let mut buf = vec![0u8; w as usize * h as usize * 4];
        while !stop.load(Ordering::Relaxed) && out.read_exact(&mut buf).is_ok() {
            // Blocks while the window has enough pictures, which also pauses ffmpeg. Fails once the window lets go.
            if tx.send(buf.clone()).is_err() {
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Plays the sound of `path` from `from` seconds on the default output device, until told to stop.
/// Any failure (no device, no sound in the file) just means silent playback.
fn play_audio(tools: &Tools, path: &Path, from: f64, stop: &Arc<AtomicBool>, go: &Arc<AtomicBool>, gain: &Arc<AtomicU32>) {
    let Some(device) = cpal::default_host().default_output_device() else { return };
    let Ok(supported) = device.default_output_config() else { return };
    let (rate, channels) = (supported.sample_rate(), supported.channels());
    let queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
    let shared = (queue.clone(), stop.clone(), go.clone(), gain.clone());
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => output_stream::<f32>(&device, supported.config(), shared),
        cpal::SampleFormat::I16 => output_stream::<i16>(&device, supported.config(), shared),
        cpal::SampleFormat::U16 => output_stream::<u16>(&device, supported.config(), shared),
        cpal::SampleFormat::I32 => output_stream::<i32>(&device, supported.config(), shared),
        _ => return,
    };
    let Some(stream) = stream else { return };
    if stream.play().is_err() {
        return;
    }

    let child = ffmpeg::command(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-ss", &format!("{from:.3}"), "-i"])
        .arg(path)
        .args(["-vn", "-ac", &channels.to_string(), "-ar", &rate.to_string(), "-f", "f32le", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return };
    let Some(mut out) = child.stdout.take() else { return };

    let ahead = rate as usize * channels as usize * AUDIO_AHEAD;
    let mut chunk = [0u8; 16 * 1024];
    let mut leftover: Vec<u8> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        if queue.lock().map_or(0, |q| q.len()) > ahead {
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        let n = match out.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        leftover.extend_from_slice(&chunk[..n]);
        let whole = leftover.len() / 4 * 4;
        if let Ok(mut q) = queue.lock() {
            q.extend(leftover[..whole].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        }
        leftover.drain(..whole);
    }
    // The file's sound is finished; let what is queued play out unless playback was stopped.
    while !stop.load(Ordering::Relaxed) && queue.lock().is_ok_and(|q| !q.is_empty()) {
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    drop(stream);
}

type Shared = (Arc<Mutex<VecDeque<f32>>>, Arc<AtomicBool>, Arc<AtomicBool>, Arc<AtomicU32>);

fn output_stream<T>(device: &cpal::Device, config: cpal::StreamConfig, (queue, stop, go, gain): Shared) -> Option<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let live = go.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed);
                let gain = f32::from_bits(gain.load(Ordering::Relaxed));
                let mut queue = queue.lock().expect("audio queue");
                for sample in data.iter_mut() {
                    let v = if live { queue.pop_front().unwrap_or(0.0) } else { 0.0 };
                    *sample = T::from_sample(v * gain);
                }
            },
            |_| {},
            None,
        )
        .ok()
}
