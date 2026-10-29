//! Playback on macOS and Windows through cpal, resampled to the output device's own rate.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::{CHANNELS, Jitter, Resampler};

/// How long the output may go without asking for sound before it is reopened (after sleep).
const STALL: Duration = Duration::from_secs(2);

/// Starts playback on the default output; the stream lives on its own thread (it is not Send).
/// It is reopened when it fails, stalls, or the default output changes, so sound follows the
/// device this computer plays on.
pub fn play(jitter: Arc<Mutex<Jitter>>, stop: Arc<AtomicBool>) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("audio-play".into()).spawn(move || {
        let mut output = match Output::open(jitter.clone()) {
            Ok(o) => Some(o),
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let _ = tx.send(Ok(()));
        let mut checked = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
            if checked.elapsed() < Duration::from_secs(1) {
                continue;
            }
            checked = Instant::now();
            let trouble = match &mut output {
                Some(o) => o.trouble(),
                None => Some("no sound output yet"),
            };
            let Some(why) = trouble else { continue };
            let was_playing = output.take().is_some();
            match Output::open(jitter.clone()) {
                Ok(o) => {
                    log::info!("audio: reopened the sound output ({why})");
                    output = Some(o);
                }
                Err(e) if was_playing => log::warn!("audio: cannot reopen the sound output ({why}): {e:#}"),
                Err(_) => {}
            }
        }
    })?;
    rx.recv().context("audio thread ended")?
}

/// An open output stream and what tells whether it still works.
struct Output {
    _stream: cpal::Stream,
    device: Option<cpal::DeviceId>,
    failed: Arc<AtomicBool>,
    calls: Arc<AtomicU64>,
    seen_calls: u64,
    quiet_since: Instant,
}

impl Output {
    fn open(jitter: Arc<Mutex<Jitter>>) -> Result<Output> {
        let device = cpal::default_host().default_output_device().context("no sound output device")?;
        let supported = device.default_output_config().map_err(|e| anyhow::anyhow!("{e}"))?;
        let config = supported.config();
        let out_channels = config.channels as usize;
        let (failed, calls) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)));
        let mut feed = Feed { jitter, resampler: Resampler::new(config.sample_rate), pending: VecDeque::new(), scratch: Vec::new(), calls: calls.clone() };
        let failed2 = failed.clone();
        let on_error = move |e| {
            log::warn!("audio: output error: {e}");
            failed2.store(true, Ordering::Relaxed);
        };
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(config, move |out: &mut [f32], _: &_| feed.fill(out, out_channels, |v| v), on_error, None),
            cpal::SampleFormat::I16 => {
                device.build_output_stream(config, move |out: &mut [i16], _: &_| feed.fill(out, out_channels, |v| (v * 32767.0) as i16), on_error, None)
            }
            other => bail!("unsupported output sample format {other:?}"),
        }
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        stream.play().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Output { _stream: stream, device: device.id().ok(), failed, calls, seen_calls: 0, quiet_since: Instant::now() })
    }

    /// Why the stream should be reopened, if it should.
    fn trouble(&mut self) -> Option<&'static str> {
        if self.failed.load(Ordering::Relaxed) {
            return Some("the output reported an error");
        }
        if cpal::default_host().default_output_device().and_then(|d| d.id().ok()) != self.device {
            return Some("the default output changed");
        }
        let calls = self.calls.load(Ordering::Relaxed);
        if calls != self.seen_calls {
            self.seen_calls = calls;
            self.quiet_since = Instant::now();
        } else if self.quiet_since.elapsed() > STALL {
            return Some("the output stopped asking for sound");
        }
        None
    }
}

struct Feed {
    jitter: Arc<Mutex<Jitter>>,
    resampler: Resampler,
    /// Input samples fetched but not yet consumed by the resampler, kept for the next callback.
    pending: VecDeque<i16>,
    scratch: Vec<i16>,
    calls: Arc<AtomicU64>,
}

impl Feed {
    fn fill<T>(&mut self, out: &mut [T], channels: usize, convert: impl Fn(f32) -> T) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let frames = out.len() / channels.max(1);
        // Top up what this callback needs from the jitter buffer in one lock.
        let need = self.resampler.input_needed(frames) * CHANNELS;
        if self.pending.len() < need {
            self.scratch.clear();
            self.scratch.resize(need - self.pending.len(), 0);
            self.jitter.lock().unwrap().pull(&mut self.scratch);
            self.pending.extend(&self.scratch);
        }
        let pending = &mut self.pending;
        let mut src = || match (pending.pop_front(), pending.pop_front()) {
            (Some(l), Some(r)) => [l as f32 / 32768.0, r as f32 / 32768.0],
            _ => [0.0, 0.0],
        };
        for frame in out.chunks_mut(channels.max(1)) {
            let [l, r] = self.resampler.frame(&mut src);
            match frame.len() {
                1 => frame[0] = convert((l + r) / 2.0),
                _ => {
                    frame[0] = convert(l);
                    frame[1] = convert(r);
                    for extra in &mut frame[2..] {
                        *extra = convert(0.0);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays silence on the real default output and checks the watchdog leaves a healthy stream alone.
    #[test]
    #[ignore]
    fn healthy_output_is_not_reopened() {
        let mut out = Output::open(Arc::new(Mutex::new(Jitter::new(40, Arc::default())))).unwrap();
        for _ in 0..4 {
            std::thread::sleep(Duration::from_secs(1));
            assert_eq!(out.trouble(), None);
        }
        assert!(out.calls.load(Ordering::Relaxed) > 0, "the output asked for sound");
    }
}
