//! The local clipboard: notices copies on this machine and applies copies from the peer.
//!
//! One thread owns the `arboard` handle (on X11 it must stay alive to keep serving what we put
//! on the clipboard). A cheap change counter decides when to read; a content hash stops
//! clipboard contents we applied from echoing back to the peer.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc::UnboundedSender;

use crate::config::ClipboardCfg;
use crate::platform;
use crate::proto::ClipContent;

/// Something copied on this machine.
#[derive(Clone, Debug)]
pub enum Local {
    Content(ClipContent),
    Files(Vec<PathBuf>),
}

#[derive(Clone, Debug)]
pub struct Copied {
    pub local: Local,
    /// Unix milliseconds.
    pub at: u64,
}

enum Cmd {
    Content(ClipContent),
    Files(Vec<PathBuf>),
}

#[derive(Clone)]
pub struct ClipHandle {
    cmd: mpsc::Sender<Cmd>,
}

impl ClipHandle {
    pub fn apply(&self, content: ClipContent) {
        let _ = self.cmd.send(Cmd::Content(content));
    }

    pub fn apply_files(&self, paths: Vec<PathBuf>) {
        let _ = self.cmd.send(Cmd::Files(paths));
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Starts the clipboard thread; local copies are reported on `copied`.
pub fn start(cfg: ClipboardCfg, copied: UnboundedSender<Copied>) -> ClipHandle {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new().name("clipboard".into()).spawn(move || run(cfg, rx, copied)).expect("spawning clipboard thread");
    ClipHandle { cmd: tx }
}

fn hash_of<T: Hash + ?Sized>(v: &T) -> u64 {
    let mut h = DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

fn content_hash(c: &ClipContent) -> u64 {
    match c {
        ClipContent::Text { text, .. } => hash_of(text.as_str()),
        ClipContent::Image { width, height, png } => hash_of(&(width, height, png)),
    }
}

/// Where clipboard contents are read from and written to.
trait Board {
    /// Reads the clipboard, preferring files, then text, then an image.
    fn read(&mut self, cfg: &ClipboardCfg) -> Option<Local>;
    fn write(&mut self, cmd: Cmd) -> Result<(), String>;
    /// See `ClipboardWatch::change_count`.
    fn change_count(&mut self) -> Option<u64>;
}

fn open_board() -> Box<dyn Board> {
    if let Some(dir) = std::env::var_os("WIFISHARE_FAKE_CLIPBOARD") {
        log::info!("clipboard: using the test clipboard in {}", PathBuf::from(&dir).display());
        return Box::new(FakeBoard { dir: dir.into() });
    }
    loop {
        match arboard::Clipboard::new() {
            Ok(cb) => return Box::new(SystemBoard { cb, watch: platform::clipboard_watch() }),
            Err(e) => {
                log::warn!("clipboard unavailable ({e}); retrying");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

fn run(cfg: ClipboardCfg, rx: mpsc::Receiver<Cmd>, copied: UnboundedSender<Copied>) {
    let mut board = open_board();
    let mut last_count = board.change_count();
    // Whatever is on the clipboard at startup is not news.
    let mut last_hash = board.read(&cfg).map(|l| local_hash(&l));
    let mut last_poll = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(cmd) => {
                let hash = match &cmd {
                    Cmd::Content(c) => content_hash(c),
                    Cmd::Files(paths) => hash_of(paths),
                };
                if Some(hash) == last_hash {
                    continue;
                }
                match board.write(cmd) {
                    Ok(()) => {
                        last_hash = Some(hash);
                        // macOS and Windows bump the counter synchronously: skip our own change.
                        last_count = board.change_count();
                    }
                    Err(e) => log::warn!("clipboard: could not set the clipboard: {e}"),
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        let count = board.change_count();
        if count.is_some() {
            if count == last_count {
                continue;
            }
            last_count = count;
        } else if last_poll.elapsed() < Duration::from_secs(1) {
            continue;
        }
        last_poll = Instant::now();

        let Some(local) = board.read(&cfg) else { continue };
        let hash = local_hash(&local);
        if Some(hash) == last_hash {
            continue;
        }
        last_hash = Some(hash);
        log::info!("clipboard: copied {}", describe(&local));
        if copied.send(Copied { local, at: now_ms() }).is_err() {
            return;
        }
    }
}

fn local_hash(l: &Local) -> u64 {
    match l {
        Local::Content(c) => content_hash(c),
        Local::Files(p) => hash_of(p),
    }
}

pub fn describe(l: &Local) -> String {
    match l {
        Local::Content(ClipContent::Text { text, .. }) => format!("text ({} chars)", text.chars().count()),
        Local::Content(ClipContent::Image { width, height, .. }) => format!("image ({width}x{height})"),
        Local::Files(p) if p.len() == 1 => format!("file {}", p[0].display()),
        Local::Files(p) => format!("{} files", p.len()),
    }
}

struct SystemBoard {
    cb: arboard::Clipboard,
    watch: Box<dyn platform::ClipboardWatch>,
}

impl Board for SystemBoard {
    fn read(&mut self, cfg: &ClipboardCfg) -> Option<Local> {
        let cb = &mut self.cb;
        if let Ok(files) = cb.get().file_list()
            && !files.is_empty()
        {
            return Some(Local::Files(files));
        }
        if let Ok(text) = cb.get_text()
            && !text.is_empty()
        {
            if text.len() as u64 > cfg.max_text_mb * 1024 * 1024 {
                log::info!("clipboard: text larger than max_text_mb, not shared");
                return None;
            }
            let html = cb.get().html().ok().filter(|h| !h.is_empty());
            return Some(Local::Content(ClipContent::Text { text, html }));
        }
        if let Ok(img) = cb.get_image() {
            if img.bytes.len() as u64 > cfg.max_image_mb * 1024 * 1024 {
                log::info!("clipboard: image larger than max_image_mb, not shared");
                return None;
            }
            match encode_png(img.width as u32, img.height as u32, &img.bytes) {
                Ok(png) => return Some(Local::Content(ClipContent::Image { width: img.width as u32, height: img.height as u32, png })),
                Err(e) => log::warn!("clipboard: could not encode image: {e}"),
            }
        }
        None
    }

    fn write(&mut self, cmd: Cmd) -> Result<(), String> {
        match cmd {
            Cmd::Content(ClipContent::Text { text, html: Some(html) }) => self.cb.set_html(html, Some(text)),
            Cmd::Content(ClipContent::Text { text, html: None }) => self.cb.set_text(text),
            Cmd::Content(ClipContent::Image { png, .. }) => {
                let (width, height, rgba) = decode_png(&png).map_err(|e| e.to_string())?;
                self.cb.set_image(arboard::ImageData { width: width as usize, height: height as usize, bytes: rgba.into() })
            }
            Cmd::Files(paths) => self.cb.set().file_list(&paths),
        }
        .map_err(|e| e.to_string())
    }

    fn change_count(&mut self) -> Option<u64> {
        self.watch.change_count()
    }
}

/// A clipboard made of files, for tests: `text` holds copied text, `files` one copied path per
/// line; whichever was written last is the clipboard.
struct FakeBoard {
    dir: PathBuf,
}

impl FakeBoard {
    fn stamp(&self, name: &str) -> Option<(SystemTime, u64)> {
        let m = std::fs::metadata(self.dir.join(name)).ok()?;
        Some((m.modified().ok()?, m.len()))
    }
}

impl Board for FakeBoard {
    fn read(&mut self, _cfg: &ClipboardCfg) -> Option<Local> {
        let (text, files) = (self.stamp("text"), self.stamp("files"));
        if files.is_some() && (text.is_none() || files > text) {
            let list = std::fs::read_to_string(self.dir.join("files")).ok()?;
            return Some(Local::Files(list.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect()));
        }
        let text = std::fs::read_to_string(self.dir.join("text")).ok()?;
        Some(Local::Content(ClipContent::Text { text, html: None }))
    }

    fn write(&mut self, cmd: Cmd) -> Result<(), String> {
        let (name, other, body) = match cmd {
            Cmd::Content(ClipContent::Text { text, .. }) => ("text", "files", text),
            Cmd::Files(paths) => ("files", "text", paths.iter().map(|p| format!("{}\n", p.display())).collect()),
            Cmd::Content(ClipContent::Image { .. }) => return Err("the test clipboard holds no images".into()),
        };
        let _ = std::fs::remove_file(self.dir.join(other));
        std::fs::write(self.dir.join(name), body).map_err(|e| e.to_string())
    }

    fn change_count(&mut self) -> Option<u64> {
        Some(hash_of(&(self.stamp("text"), self.stamp("files"))))
    }
}

pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(out)
}

pub fn decode_png(data: &[u8]) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => anyhow::bail!("unexpanded palette image"),
    };
    Ok((info.width, info.height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip() {
        let rgba: Vec<u8> = (0..4 * 3 * 2).map(|i| i as u8).collect();
        let png = encode_png(3, 2, &rgba).unwrap();
        assert_eq!(decode_png(&png).unwrap(), (3, 2, rgba));
    }
}
