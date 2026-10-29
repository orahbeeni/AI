//! Sound sharing: one computer streams what it plays to the other, which plays it.
//!
//! Audio travels as raw 48 kHz stereo 16-bit PCM in 5 ms packets over UDP (about 1.5 Mbit/s,
//! nothing for a LAN), each packet sealed with ChaCha20-Poly1305 under a per-session key agreed
//! over the already encrypted control link. Silent stretches are not sent. The receiver keeps a
//! small jitter buffer (`audio.buffer_ms`), fills short gaps with silence, and trims the buffer
//! when it grows, so latency stays bounded after a Wi-Fi stall.

#[cfg(any(target_os = "macos", windows))]
mod cpal_out;
#[cfg(target_os = "linux")]
pub mod linux;

use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

use crate::status::{AudioLink, Hub};

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
const FRAME_MS: usize = 5;
/// Interleaved samples in one packet.
pub const FRAME_SAMPLES: usize = RATE as usize / 1000 * FRAME_MS * CHANNELS;
const MAGIC: &[u8; 2] = b"WA";
const HEADER: usize = 2 + 4 + 8;

fn samples_for(ms: usize) -> usize {
    RATE as usize / 1000 * ms * CHANNELS
}

/// Whether this machine can send its sound (capturing system audio is Linux-only so far).
pub fn can_send() -> bool {
    if std::env::var_os("WIFISHARE_FAKE_AUDIO").is_some() {
        return true;
    }
    #[cfg(target_os = "linux")]
    return linux::available();
    #[cfg(not(target_os = "linux"))]
    false
}

#[derive(Clone)]
pub struct SessionKey {
    pub session: u32,
    pub key: [u8; 32],
}

impl SessionKey {
    pub fn random() -> Result<SessionKey> {
        let mut key = [0u8; 32];
        let mut session = [0u8; 4];
        getrandom::fill(&mut key).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
        getrandom::fill(&mut session).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
        Ok(SessionKey { session: u32::from_be_bytes(session), key })
    }
}

struct Seal {
    cipher: ChaCha20Poly1305,
    session: u32,
}

impl Seal {
    fn new(k: &SessionKey) -> Seal {
        Seal { cipher: ChaCha20Poly1305::new(Key::from_slice(&k.key)), session: k.session }
    }

    fn header(&self, seq: u64) -> [u8; HEADER] {
        let mut h = [0u8; HEADER];
        h[..2].copy_from_slice(MAGIC);
        h[2..6].copy_from_slice(&self.session.to_be_bytes());
        h[6..].copy_from_slice(&seq.to_be_bytes());
        h
    }

    fn seal(&self, seq: u64, pcm: &[i16]) -> Vec<u8> {
        let header = self.header(seq);
        let plain: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        let sealed = self.cipher.encrypt(Nonce::from_slice(&header[2..]), Payload { msg: &plain, aad: &header }).unwrap_or_default();
        let mut out = header.to_vec();
        out.extend_from_slice(&sealed);
        out
    }

    fn open(&self, packet: &[u8]) -> Option<(u64, Vec<i16>)> {
        let header = packet.get(..HEADER)?;
        if &header[..2] != MAGIC || header[2..6] != self.session.to_be_bytes() {
            return None;
        }
        let plain = self.cipher.decrypt(Nonce::from_slice(&header[2..]), Payload { msg: &packet[HEADER..], aad: header }).ok()?;
        let seq = u64::from_be_bytes(header[6..].try_into().ok()?);
        Some((seq, plain.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()))
    }
}

/// Reorders nothing, but keeps time: short gaps become silence, late packets are dropped, an
/// overfull buffer is trimmed back to the target, and an empty one re-buffers before playing.
pub struct Jitter {
    buf: VecDeque<i16>,
    next_seq: Option<u64>,
    target: usize,
    max: usize,
    playing: bool,
    pub lost: Arc<AtomicU64>,
}

impl Jitter {
    pub fn new(buffer_ms: usize, lost: Arc<AtomicU64>) -> Jitter {
        let target = samples_for(buffer_ms.clamp(10, 500));
        Jitter { buf: VecDeque::new(), next_seq: None, target, max: target + samples_for(60), playing: false, lost }
    }

    pub fn push(&mut self, seq: u64, pcm: &[i16]) {
        if let Some(next) = self.next_seq {
            if seq < next {
                return;
            }
            let missing = (seq - next) as usize;
            // A short hole is lost packets: keep time with silence. A long one is the sender
            // skipping silence (or a stall), so just start again from here.
            if missing > 0 && missing * FRAME_SAMPLES <= samples_for(100) && self.playing {
                self.buf.extend(std::iter::repeat_n(0, missing * FRAME_SAMPLES));
                self.lost.fetch_add(missing as u64, Ordering::Relaxed);
            }
        }
        self.next_seq = Some(seq + 1);
        self.buf.extend(pcm);
        if self.buf.len() > self.max {
            let excess = (self.buf.len() - self.target) / CHANNELS * CHANNELS;
            self.buf.drain(..excess);
        }
    }

    /// Fills `out` (interleaved stereo) with what is due now, silence where there is nothing.
    pub fn pull(&mut self, out: &mut [i16]) {
        if !self.playing {
            if self.buf.len() < self.target {
                out.fill(0);
                return;
            }
            self.playing = true;
        }
        let n = out.len().min(self.buf.len());
        for (o, s) in out.iter_mut().zip(self.buf.drain(..n)) {
            *o = s;
        }
        out[n..].fill(0);
        if self.buf.is_empty() {
            self.playing = false;
        }
    }

    #[cfg(test)]
    fn buffered(&self) -> usize {
        self.buf.len()
    }
}

/// Registers an audio stream in the status shown by the window; removed on drop.
struct Listed {
    hub: Arc<Hub>,
    link: AudioLink,
}

impl Listed {
    fn new(hub: &Arc<Hub>, peer: &str, sending: bool) -> Listed {
        let link = AudioLink { peer: peer.into(), sending, since: Instant::now(), packets: Arc::default(), lost: Arc::default() };
        hub.update(|s| s.audio.push(link.clone()));
        Listed { hub: hub.clone(), link }
    }
}

impl Drop for Listed {
    fn drop(&mut self) {
        let p = self.link.packets.clone();
        self.hub.update(|s| s.audio.retain(|a| !Arc::ptr_eq(&a.packets, &p)));
    }
}

/// Source of this machine's sound, 5 ms at a time.
trait Capture: Send {
    fn read(&mut self, frame: &mut [i16]) -> Result<()>;
    /// Starts over after a failed read (the sound server restarted, say).
    fn reopen(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A test tone instead of the real sound (`WIFISHARE_FAKE_AUDIO`).
struct Tone {
    phase: f64,
    next: Instant,
}

impl Capture for Tone {
    fn read(&mut self, frame: &mut [i16]) -> Result<()> {
        let now = Instant::now();
        if self.next > now {
            std::thread::sleep(self.next - now);
        }
        self.next += Duration::from_millis(FRAME_MS as u64);
        for pair in frame.chunks_mut(CHANNELS) {
            let v = (self.phase.sin() * 8000.0) as i16;
            pair.fill(v);
            self.phase += std::f64::consts::TAU * 440.0 / RATE as f64;
        }
        Ok(())
    }
}

/// Streams this machine's sound to `dest` until dropped.
pub struct Sender {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    #[cfg(target_os = "linux")]
    _route: Option<linux::Route>,
    _listed: Listed,
}

impl Sender {
    pub fn start(key: &SessionKey, dest: SocketAddr, play_here: bool, peer: &str, hub: &Arc<Hub>, state_dir: &std::path::Path) -> Result<Sender> {
        let fake = std::env::var_os("WIFISHARE_FAKE_AUDIO").is_some();
        #[cfg(target_os = "linux")]
        let (route, mut capture): (Option<linux::Route>, Box<dyn Capture>) = if fake {
            (None, Box::new(Tone { phase: 0.0, next: Instant::now() }))
        } else {
            let route = if play_here { None } else { Some(linux::Route::to_virtual_sink(state_dir)?) };
            let device = route.as_ref().map(|r| r.monitor()).unwrap_or_else(|| "@DEFAULT_MONITOR@".to_string());
            (route, Box::new(linux::Recorder::open(device)?))
        };
        #[cfg(not(target_os = "linux"))]
        let mut capture: Box<dyn Capture> = {
            let _ = (play_here, state_dir);
            if !fake {
                anyhow::bail!("sending this computer's sound is only supported on Linux so far");
            }
            Box::new(Tone { phase: 0.0, next: Instant::now() })
        };

        let sock = UdpSocket::bind("0.0.0.0:0").context("opening an audio socket")?;
        let _ = socket2::SockRef::from(&sock).set_tos_v4(0xB8);
        sock.connect(dest)?;
        let seal = Seal::new(key);
        let stop = Arc::new(AtomicBool::new(false));
        let listed = Listed::new(hub, peer, true);
        let (stop2, packets) = (stop.clone(), listed.link.packets.clone());
        let thread = std::thread::Builder::new().name("audio-send".into()).spawn(move || {
            let mut frame = vec![0i16; FRAME_SAMPLES];
            let mut seq = 0u64;
            while !stop2.load(Ordering::Relaxed) {
                if let Err(e) = capture.read(&mut frame) {
                    log::warn!("audio: capture failed, retrying: {e:#}");
                    while !stop2.load(Ordering::Relaxed) && capture.reopen().is_err() {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    continue;
                }
                seq += 1;
                if frame.iter().all(|s| *s == 0) {
                    continue;
                }
                if sock.send(&seal.seal(seq, &frame)).is_ok() {
                    packets.fetch_add(1, Ordering::Relaxed);
                }
            }
        })?;
        log::info!("audio: sending this computer's sound to {peer} ({dest})");
        Ok(Sender {
            stop,
            thread: Some(thread),
            #[cfg(target_os = "linux")]
            _route: route,
            _listed: listed,
        })
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The capture read returns within one 5 ms frame; do not hang if the sound server stalls.
        if let Some(t) = self.thread.take() {
            let deadline = Instant::now() + Duration::from_millis(300);
            while !t.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        log::info!("audio: stopped sending");
    }
}

/// Plays sound streamed by the peer until dropped.
pub struct Receiver {
    stop: Arc<AtomicBool>,
    port: u16,
    _listed: Listed,
}

impl Receiver {
    pub fn start(key: &SessionKey, peer: &str, buffer_ms: usize, hub: &Arc<Hub>) -> Result<Receiver> {
        let sock = UdpSocket::bind("0.0.0.0:0").context("opening an audio socket")?;
        sock.set_read_timeout(Some(Duration::from_millis(200)))?;
        let port = sock.local_addr()?.port();
        let listed = Listed::new(hub, peer, false);
        let jitter = Arc::new(Mutex::new(Jitter::new(buffer_ms, listed.link.lost.clone())));
        let stop = Arc::new(AtomicBool::new(false));
        play(jitter.clone(), stop.clone())?;

        let seal = Seal::new(key);
        let (stop2, packets) = (stop.clone(), listed.link.packets.clone());
        std::thread::Builder::new().name("audio-recv".into()).spawn(move || {
            let mut buf = vec![0u8; 4096];
            while !stop2.load(Ordering::Relaxed) {
                let Ok(n) = sock.recv(&mut buf) else { continue };
                if let Some((seq, pcm)) = seal.open(&buf[..n]) {
                    packets.fetch_add(1, Ordering::Relaxed);
                    jitter.lock().unwrap().push(seq, &pcm);
                }
            }
        })?;
        log::info!("audio: playing sound from {peer}");
        Ok(Receiver { stop, port, _listed: listed })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        log::info!("audio: stopped playing");
    }
}

/// Starts playing from `jitter` on this machine's output until `stop` is set.
fn play(jitter: Arc<Mutex<Jitter>>, stop: Arc<AtomicBool>) -> Result<()> {
    if let Some(dir) = std::env::var_os("WIFISHARE_FAKE_AUDIO") {
        return record_to_file(jitter, stop, std::path::Path::new(&dir).join("received.raw"));
    }
    #[cfg(target_os = "linux")]
    return linux::play(jitter, stop);
    #[cfg(any(target_os = "macos", windows))]
    return cpal_out::play(jitter, stop);
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (jitter, stop);
        anyhow::bail!("no audio output support on this platform")
    }
}

/// Test sink: appends what would be played to a raw s16le stereo file, in real time.
fn record_to_file(jitter: Arc<Mutex<Jitter>>, stop: Arc<AtomicBool>, path: std::path::PathBuf) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    std::thread::spawn(move || {
        let mut frame = vec![0i16; FRAME_SAMPLES];
        let mut next = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            jitter.lock().unwrap().pull(&mut frame);
            let bytes: Vec<u8> = frame.iter().flat_map(|s| s.to_le_bytes()).collect();
            let _ = file.write_all(&bytes);
            next += Duration::from_millis(FRAME_MS as u64);
            if let Some(d) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
        }
    });
    Ok(())
}

/// Linear resampling from 48 kHz stereo to the output device's rate.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub struct Resampler {
    step: f64,
    pos: f64,
    prev: [f32; 2],
    next: [f32; 2],
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
impl Resampler {
    pub fn new(out_rate: u32) -> Resampler {
        Resampler { step: RATE as f64 / out_rate.max(1) as f64, pos: 1.0, prev: [0.0; 2], next: [0.0; 2] }
    }

    /// Input frames needed for `out_frames` output frames (a little over, never under).
    pub fn input_needed(&self, out_frames: usize) -> usize {
        ((self.pos + out_frames as f64 * self.step).floor() as usize).saturating_add(1)
    }

    /// One output frame; `src` yields input frames as needed.
    pub fn frame(&mut self, src: &mut impl FnMut() -> [f32; 2]) -> [f32; 2] {
        while self.pos >= 1.0 {
            self.prev = self.next;
            self.next = src();
            self.pos -= 1.0;
        }
        let t = self.pos as f32;
        self.pos += self.step;
        [self.prev[0] + (self.next[0] - self.prev[0]) * t, self.prev[1] + (self.next[1] - self.prev[1]) * t]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_round_trip_and_reject_tampering() {
        let k = SessionKey::random().unwrap();
        let seal = Seal::new(&k);
        let pcm: Vec<i16> = (0..FRAME_SAMPLES as i16).collect();
        let mut packet = seal.seal(7, &pcm);
        assert_eq!(seal.open(&packet), Some((7, pcm)));
        let last = packet.len() - 1;
        packet[last] ^= 1;
        assert_eq!(seal.open(&packet), None);
        let other = Seal::new(&SessionKey::random().unwrap());
        assert_eq!(other.open(&seal.seal(1, &[0; 4])), None);
    }

    #[test]
    fn jitter_buffers_fills_gaps_and_bounds_latency() {
        let mut j = Jitter::new(20, Arc::default());
        let frame = vec![100i16; FRAME_SAMPLES];
        let mut out = vec![1i16; FRAME_SAMPLES];
        j.push(1, &frame);
        j.pull(&mut out);
        assert!(out.iter().all(|s| *s == 0), "silence until 20 ms are buffered");
        for seq in 2..=4 {
            j.push(seq, &frame);
        }
        j.pull(&mut out);
        assert!(out.iter().all(|s| *s == 100));
        // Packet 5 lost: its 5 ms become silence so later audio keeps its timing.
        j.push(6, &frame);
        assert_eq!(j.lost.load(Ordering::Relaxed), 1);
        assert_eq!(j.buffered(), 5 * FRAME_SAMPLES);
        j.push(3, &frame); // late: dropped
        assert_eq!(j.buffered(), 5 * FRAME_SAMPLES);
        // A burst after a stall is trimmed back to the target instead of adding latency.
        for seq in 7..60 {
            j.push(seq, &frame);
        }
        assert!(j.buffered() <= samples_for(20 + 60));
    }

    /// One side of a link: its sound state, settings and outgoing messages.
    struct End {
        audio: LinkAudio,
        cfg: crate::config::AudioCfg,
        tx: tokio::sync::mpsc::UnboundedSender<crate::proto::Msg>,
        rx: tokio::sync::mpsc::UnboundedReceiver<crate::proto::Msg>,
    }

    impl End {
        fn new(peer: &str, cfg: crate::config::AudioCfg, hub: &Arc<Hub>, dir: &std::path::Path) -> End {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            End { audio: LinkAudio::new(peer, std::net::IpAddr::from([127, 0, 0, 1]), hub.clone(), dir.into()), cfg, tx, rx }
        }

        fn apply(&mut self, changed: bool) {
            self.audio.apply(&self.cfg, &self.tx, changed);
        }
    }

    /// Delivers messages both ways until neither side has more to say.
    fn pump(a: &mut End, b: &mut End) {
        while deliver(a, b) | deliver(b, a) {}
    }

    /// Hands `to` what `from` sent; true if there was anything.
    fn deliver(from: &mut End, to: &mut End) -> bool {
        let mut any = false;
        while let Ok(m) = from.rx.try_recv() {
            any = true;
            to.audio.handle(&m, &to.cfg, &to.tx);
        }
        any
    }

    #[test]
    fn receiving_again_after_turning_it_off_asks_for_the_sound() {
        let dir = std::env::temp_dir().join(format!("wifishare-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: no other test reads or writes the environment.
        unsafe { std::env::set_var("WIFISHARE_FAKE_AUDIO", &dir) };
        let hub = Hub::new(crate::status::Approval::Gui);
        let mut sender = End::new("player", crate::config::AudioCfg { send: true, ..Default::default() }, &hub, &dir);
        let mut player = End::new("sender", Default::default(), &hub, &dir);
        let streaming = |s: &End, p: &End| s.audio.sender.is_some() && p.audio.receiver.is_some();

        sender.apply(false);
        player.apply(false);
        pump(&mut sender, &mut player);
        assert!(streaming(&sender, &player), "streaming on link-up");

        player.cfg.receive = false;
        player.apply(true);
        pump(&mut sender, &mut player);
        assert!(sender.audio.sender.is_none() && player.audio.receiver.is_none(), "stopped when turned off");

        player.cfg.receive = true;
        player.apply(true);
        pump(&mut sender, &mut player);
        assert!(streaming(&sender, &player), "streaming again when turned back on");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resampler_keeps_the_rate() {
        let mut r = Resampler::new(44_100);
        let mut taken = 0usize;
        let mut src = || {
            taken += 1;
            [0.0, 0.0]
        };
        for _ in 0..44_100 {
            r.frame(&mut src);
        }
        assert!((47_990..=48_010).contains(&taken), "{taken}");
    }
}

/// Sound on one link: what this machine sends to the peer and what it plays from the peer.
pub struct LinkAudio {
    peer: String,
    peer_ip: std::net::IpAddr,
    hub: Arc<Hub>,
    state_dir: std::path::PathBuf,
    offer: Option<(SessionKey, bool)>,
    sender: Option<(u32, bool, Sender)>,
    receiver: Option<(u32, Receiver)>,
    /// The peer turned our offer down; do not ask again until the settings change.
    refused: bool,
}

impl LinkAudio {
    pub fn new(peer: &str, peer_ip: std::net::IpAddr, hub: Arc<Hub>, state_dir: std::path::PathBuf) -> LinkAudio {
        LinkAudio { peer: peer.into(), peer_ip, hub, state_dir, offer: None, sender: None, receiver: None, refused: false }
    }

    fn notice(&self, text: String) {
        self.hub.update(|s| s.notice = Some((text, Instant::now())));
    }

    /// Starts or stops streams to match the settings (on link-up and after edits).
    pub fn apply(&mut self, cfg: &crate::config::AudioCfg, tx: &tokio::sync::mpsc::UnboundedSender<crate::proto::Msg>, changed: bool) {
        use crate::proto::Msg;
        if changed {
            self.refused = false;
        }
        let stale = self.sender.as_ref().is_some_and(|(_, play_here, _)| *play_here != cfg.play_here);
        if !cfg.send || stale {
            if let Some((session, _, _)) = self.sender.take() {
                let _ = tx.send(Msg::AudioStop { session });
            }
            if let Some((k, _)) = self.offer.take() {
                let _ = tx.send(Msg::AudioStop { session: k.session });
            }
        }
        if cfg.send && self.sender.is_none() && self.offer.is_none() && !self.refused {
            if !can_send() {
                if changed {
                    self.notice("Sending sound is only supported on Linux so far".into());
                }
            } else if let Ok(key) = SessionKey::random() {
                let _ = tx.send(Msg::AudioOffer { session: key.session, key: key.key });
                self.offer = Some((key, cfg.play_here));
            }
        }
        if !cfg.receive
            && let Some((session, _)) = self.receiver.take()
        {
            let _ = tx.send(Msg::AudioRefuse { session, reason: "sound from other computers was turned off there".into() });
        }
        // The peer stops offering once refused, so say when sound is welcome again.
        if changed && cfg.receive && self.receiver.is_none() {
            let _ = tx.send(Msg::AudioWanted);
        }
    }

    /// Handles an audio message from the peer; returns false for anything else.
    pub fn handle(&mut self, msg: &crate::proto::Msg, cfg: &crate::config::AudioCfg, tx: &tokio::sync::mpsc::UnboundedSender<crate::proto::Msg>) -> bool {
        use crate::proto::Msg;
        match msg {
            Msg::AudioOffer { session, key } => {
                let session = *session;
                if !cfg.receive {
                    let _ = tx.send(Msg::AudioRefuse { session, reason: "sound from other computers is turned off there".into() });
                    return true;
                }
                self.receiver = None;
                match Receiver::start(&SessionKey { session, key: *key }, &self.peer, cfg.buffer_ms as usize, &self.hub) {
                    Ok(r) => {
                        let _ = tx.send(Msg::AudioAccept { session, port: r.port() });
                        self.receiver = Some((session, r));
                    }
                    Err(e) => {
                        log::warn!("audio: cannot play {}'s sound: {e:#}", self.peer);
                        let _ = tx.send(Msg::AudioRefuse { session, reason: format!("{e:#}") });
                    }
                }
            }
            Msg::AudioAccept { session, port } => {
                let Some((key, play_here)) = self.offer.take_if(|(k, _)| k.session == *session) else { return true };
                match Sender::start(&key, SocketAddr::new(self.peer_ip, *port), play_here, &self.peer, &self.hub, &self.state_dir) {
                    Ok(s) => self.sender = Some((key.session, play_here, s)),
                    Err(e) => {
                        log::warn!("audio: cannot send sound: {e:#}");
                        self.notice(format!("Cannot send sound: {e:#}"));
                        self.refused = true;
                        let _ = tx.send(Msg::AudioStop { session: key.session });
                    }
                }
            }
            Msg::AudioRefuse { session, reason } => {
                let ours = self.offer.as_ref().is_some_and(|(k, _)| k.session == *session) || self.sender.as_ref().is_some_and(|(s, _, _)| s == session);
                if ours {
                    self.offer = None;
                    self.sender = None;
                    self.refused = true;
                    log::info!("audio: {} does not take sound: {reason}", self.peer);
                    self.notice(format!("{} does not take sound: {reason}", self.peer));
                }
            }
            Msg::AudioStop { session } => {
                if self.receiver.as_ref().is_some_and(|(s, _)| s == session) {
                    self.receiver = None;
                }
            }
            Msg::AudioWanted => {
                self.refused = false;
                self.apply(cfg, tx, false);
            }
            _ => return false,
        }
        true
    }
}
