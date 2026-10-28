//! Playback on macOS and Windows through cpal, resampled to the output device's own rate.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::{CHANNELS, Jitter, Resampler};

/// Starts playback on the default output; the stream lives on its own thread (it is not Send).
pub fn play(jitter: Arc<Mutex<Jitter>>, stop: Arc<AtomicBool>) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("audio-play".into()).spawn(move || {
        let stream = match open(jitter) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let _ = tx.send(Ok(()));
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
        drop(stream);
    })?;
    rx.recv().context("audio thread ended")?
}

fn open(jitter: Arc<Mutex<Jitter>>) -> Result<cpal::Stream> {
    let device = cpal::default_host().default_output_device().context("no sound output device")?;
    let supported = device.default_output_config().map_err(|e| anyhow::anyhow!("{e}"))?;
    let config = supported.config();
    let out_channels = config.channels as usize;
    let mut feed = Feed { jitter, resampler: Resampler::new(config.sample_rate), pending: VecDeque::new(), scratch: Vec::new() };
    let on_error = |e| log::warn!("audio: output error: {e}");
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(config, move |out: &mut [f32], _: &_| feed.fill(out, out_channels, |v| v), on_error, None),
        cpal::SampleFormat::I16 => {
            device.build_output_stream(config, move |out: &mut [i16], _: &_| feed.fill(out, out_channels, |v| (v * 32767.0) as i16), on_error, None)
        }
        other => bail!("unsupported output sample format {other:?}"),
    }
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    stream.play().map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(stream)
}

struct Feed {
    jitter: Arc<Mutex<Jitter>>,
    resampler: Resampler,
    /// Input samples fetched but not yet consumed by the resampler, kept for the next callback.
    pending: VecDeque<i16>,
    scratch: Vec<i16>,
}

impl Feed {
    fn fill<T>(&mut self, out: &mut [T], channels: usize, convert: impl Fn(f32) -> T) {
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
