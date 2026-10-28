//! Linux sound through the PulseAudio API (PipeWire provides it too), loaded at run time so the
//! build needs no audio development packages.
//!
//! Sending normally moves this machine's sound to a virtual "wifishare" output and records that,
//! so the local speakers go quiet. The previous output is restored when sending stops, and a
//! small guardian process restores it as well should wifishare die without cleaning up.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, bail};

use super::{Capture, FRAME_SAMPLES, Jitter, RATE};

const SINK: &str = "wifishare_out";
const PA_SAMPLE_S16LE: u32 = 3;
const PA_STREAM_PLAYBACK: c_int = 1;
const PA_STREAM_RECORD: c_int = 2;

#[repr(C)]
struct SampleSpec {
    format: u32,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

type NewFn = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    c_int,
    *const c_char,
    *const c_char,
    *const SampleSpec,
    *const c_void,
    *const BufferAttr,
    *mut c_int,
) -> *mut c_void;
type ReadFn = unsafe extern "C" fn(*mut c_void, *mut c_void, usize, *mut c_int) -> c_int;
type WriteFn = unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut c_int) -> c_int;
type FreeFn = unsafe extern "C" fn(*mut c_void);
type StrErrorFn = unsafe extern "C" fn(c_int) -> *const c_char;

struct Pulse {
    new: NewFn,
    read: ReadFn,
    write: WriteFn,
    free: FreeFn,
    strerror: StrErrorFn,
}

fn pulse() -> Result<&'static Pulse> {
    static PULSE: OnceLock<std::result::Result<Pulse, String>> = OnceLock::new();
    PULSE
        .get_or_init(|| {
            // SAFETY: loads the system library and looks up functions with matching signatures.
            unsafe {
                let lib = libc::dlopen(c"libpulse-simple.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                if lib.is_null() {
                    return Err("libpulse-simple.so.0 not found (install libpulse0)".into());
                }
                let sym = |name: &CStr| {
                    let p = libc::dlsym(lib, name.as_ptr());
                    if p.is_null() { Err(format!("{} missing from libpulse", name.to_string_lossy())) } else { Ok(p) }
                };
                Ok(Pulse {
                    new: std::mem::transmute::<*mut c_void, NewFn>(sym(c"pa_simple_new")?),
                    read: std::mem::transmute::<*mut c_void, ReadFn>(sym(c"pa_simple_read")?),
                    write: std::mem::transmute::<*mut c_void, WriteFn>(sym(c"pa_simple_write")?),
                    free: std::mem::transmute::<*mut c_void, FreeFn>(sym(c"pa_simple_free")?),
                    strerror: std::mem::transmute::<*mut c_void, StrErrorFn>(sym(c"pa_strerror")?),
                })
            }
        })
        .as_ref()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

pub fn available() -> bool {
    pulse().is_ok()
}

fn error_text(pa: &Pulse, code: c_int) -> String {
    // SAFETY: pa_strerror returns a static string.
    unsafe { CStr::from_ptr((pa.strerror)(code)).to_string_lossy().into_owned() }
}

/// A blocking PulseAudio stream of interleaved 48 kHz stereo s16.
pub struct Stream {
    pa: &'static Pulse,
    s: *mut c_void,
}

// SAFETY: a pa_simple handle is used by one thread at a time.
unsafe impl Send for Stream {}

impl Stream {
    fn open(direction: c_int, device: Option<&str>, attr: BufferAttr) -> Result<Stream> {
        let pa = pulse()?;
        let spec = SampleSpec { format: PA_SAMPLE_S16LE, rate: RATE, channels: 2 };
        let device = device.map(CString::new).transpose()?;
        let mut err = 0;
        // SAFETY: all pointers are valid for the call; the device string outlives it.
        let s = unsafe {
            (pa.new)(
                std::ptr::null(),
                c"wifishare".as_ptr(),
                direction,
                device.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                if direction == PA_STREAM_RECORD { c"sound for the other computer".as_ptr() } else { c"sound from the other computer".as_ptr() },
                &spec,
                std::ptr::null(),
                &attr,
                &mut err,
            )
        };
        if s.is_null() {
            bail!("opening a sound stream: {}", error_text(pa, err));
        }
        Ok(Stream { pa, s })
    }

    /// Records from `device` (a monitor source) in chunks of `samples`.
    pub fn record(device: &str, samples: usize) -> Result<Stream> {
        let frag = (samples * 2) as u32;
        Stream::open(PA_STREAM_RECORD, Some(device), BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: frag })
    }

    /// Plays on the default output with about 20 ms of buffering.
    pub fn playback() -> Result<Stream> {
        let target = (RATE / 1000 * 20 * 2 * 2) as u32;
        Stream::open(PA_STREAM_PLAYBACK, None, BufferAttr { maxlength: u32::MAX, tlength: target, prebuf: u32::MAX, minreq: u32::MAX, fragsize: u32::MAX })
    }

    pub fn read(&mut self, buf: &mut [i16]) -> Result<()> {
        let mut err = 0;
        // SAFETY: `buf` is valid for its full length in bytes.
        if unsafe { (self.pa.read)(self.s, buf.as_mut_ptr().cast(), std::mem::size_of_val(buf), &mut err) } < 0 {
            bail!("reading sound: {}", error_text(self.pa, err));
        }
        Ok(())
    }

    pub fn write(&mut self, buf: &[i16]) -> Result<()> {
        let mut err = 0;
        // SAFETY: `buf` is valid for its full length in bytes.
        if unsafe { (self.pa.write)(self.s, buf.as_ptr().cast(), std::mem::size_of_val(buf), &mut err) } < 0 {
            bail!("playing sound: {}", error_text(self.pa, err));
        }
        Ok(())
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: frees the handle created in `open`.
        unsafe { (self.pa.free)(self.s) };
    }
}

pub struct Recorder(pub Stream);

impl Capture for Recorder {
    fn read(&mut self, frame: &mut [i16]) -> Result<()> {
        self.0.read(frame)
    }
}

pub fn play(jitter: Arc<Mutex<Jitter>>, stop: Arc<AtomicBool>) -> Result<()> {
    let mut stream = Stream::playback()?;
    std::thread::Builder::new().name("audio-play".into()).spawn(move || {
        let mut frame = vec![0i16; FRAME_SAMPLES];
        while !stop.load(Ordering::Relaxed) {
            jitter.lock().unwrap().pull(&mut frame);
            // Blocks while the server's buffer is full, which paces this loop.
            if let Err(e) = stream.write(&frame) {
                log::warn!("audio: {e:#}");
                return;
            }
        }
    })?;
    Ok(())
}

fn pactl(args: &[&str]) -> Result<String> {
    let out = Command::new("pactl").args(args).output().context("running pactl (install pulseaudio-utils)")?;
    if !out.status.success() {
        bail!("pactl {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// This machine's sound redirected into the virtual output while it lives.
pub struct Route {
    module: String,
    previous: String,
    /// PulseAudio proper does not move playing streams when the default changes; PipeWire does.
    move_streams: bool,
    state: PathBuf,
    guardian: Option<Child>,
}

impl Route {
    pub fn to_virtual_sink(state_dir: &Path) -> Result<Route> {
        std::fs::create_dir_all(state_dir)?;
        let state = state_dir.join("audio-route");
        restore_leftover(&state);
        let move_streams = !pactl(&["info"])?.contains("PipeWire");
        let previous = pactl(&["get-default-sink"])?;
        if previous.is_empty() || previous == SINK {
            bail!("cannot tell which output to restore later");
        }
        let module = pactl(&["load-module", "module-null-sink", &format!("sink_name={SINK}"), "sink_properties=device.description=wifishare"])?;
        std::fs::write(&state, format!("{module}\n{previous}\n"))?;
        let guardian = spawn_guardian(&module, &previous, &state);
        let route = Route { module, previous, move_streams, state, guardian };
        pactl(&["set-default-sink", SINK])?;
        if route.move_streams {
            move_all(SINK);
        }
        log::info!("audio: this computer's sound now goes to the other computer (was {})", route.previous);
        Ok(route)
    }

    pub fn monitor(&self) -> String {
        format!("{SINK}.monitor")
    }
}

impl Drop for Route {
    fn drop(&mut self) {
        let _ = pactl(&["set-default-sink", &self.previous]);
        if self.move_streams {
            move_all(&self.previous);
        }
        let _ = pactl(&["unload-module", &self.module]);
        let _ = std::fs::remove_file(&self.state);
        if let Some(mut g) = self.guardian.take() {
            let _ = g.kill();
            let _ = g.wait();
        }
        log::info!("audio: sound plays here again ({})", self.previous);
    }
}

fn move_all(sink: &str) {
    let Ok(list) = pactl(&["list", "short", "sink-inputs"]) else { return };
    for id in list.lines().filter_map(|l| l.split_whitespace().next()) {
        let _ = pactl(&["move-sink-input", id, sink]);
    }
}

/// Undoes a route left behind by a crash (if the guardian did not get to it first).
fn restore_leftover(state: &Path) {
    let Ok(text) = std::fs::read_to_string(state) else { return };
    let mut lines = text.lines();
    if let (Some(module), Some(previous)) = (lines.next(), lines.next()) {
        log::info!("audio: restoring the sound output a previous run left redirected");
        let _ = pactl(&["set-default-sink", previous]);
        let _ = pactl(&["unload-module", module]);
    }
    let _ = std::fs::remove_file(state);
}

/// A detached shell that waits for this process to exit and then restores the output.
fn spawn_guardian(module: &str, previous: &str, state: &Path) -> Option<Child> {
    use std::os::unix::process::CommandExt;
    let script = r#"while kill -0 "$1" 2>/dev/null; do sleep 1; done
pactl set-default-sink "$2"; pactl unload-module "$3"; rm -f "$4""#;
    Command::new("sh")
        .args(["-c", script, "wifishare-audio-guard", &std::process::id().to_string(), previous, module])
        .arg(state)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own process group, so a Ctrl+C aimed at wifishare does not take it down too.
        .process_group(0)
        .spawn()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records a second from `WIFISHARE_TEST_MONITOR` (play a 440 Hz tone into its sink first).
    #[test]
    #[ignore]
    fn records_a_monitor() {
        let device = std::env::var("WIFISHARE_TEST_MONITOR").expect("WIFISHARE_TEST_MONITOR");
        let mut s = Stream::record(&device, FRAME_SAMPLES).unwrap();
        let mut all = Vec::new();
        let mut frame = vec![0i16; FRAME_SAMPLES];
        for _ in 0..200 {
            s.read(&mut frame).unwrap();
            all.extend_from_slice(&frame);
        }
        let left: Vec<i16> = all.iter().step_by(2).copied().collect();
        let crossings = left.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
        let loud = left.iter().filter(|v| v.abs() > 1000).count();
        println!("{} frames, {} Hz, {}% loud", left.len(), crossings / 2, loud * 100 / left.len());
        assert!((420..=460).contains(&(crossings / 2)) && loud * 2 > left.len());
    }
}
