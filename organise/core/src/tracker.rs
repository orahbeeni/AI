//! Live watching of folders, turned into simple events: moved, gone, appeared.
//!
//! Linux uses inotify. A watch covers one folder (not its sub-folders), and the number of watches per user
//! is limited (often 65,536 for the whole session), so we watch:
//! - the folders that hold linked files (always), and
//! - the person's chosen folders, recursively, up to a budget.
//!
//! A rename inside watched folders arrives as a MOVED_FROM / MOVED_TO pair with the same cookie, which gives
//! the old and new path at once. A MOVED_FROM with no partner means the file left the watched folders; it's
//! reported as `Gone` and the engine searches for it.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Clone, Debug, PartialEq)]
pub enum FsEvent {
    Moved { from: PathBuf, to: PathBuf },
    /// Deleted, or moved somewhere that isn't watched.
    Gone(PathBuf),
    /// Created, finished writing, or moved in from somewhere that isn't watched.
    Appeared(PathBuf),
    /// The kernel dropped events; everything should be re-checked.
    Overflow,
}

#[derive(Clone, Debug, Default)]
pub struct Targets {
    /// Watched on their own (the folders holding linked files).
    pub dirs: Vec<PathBuf>,
    /// Watched with all their sub-folders, up to the budget.
    pub roots: Vec<PathBuf>,
    pub skip_hidden: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerStats {
    pub watches: usize,
    pub budget: usize,
    /// The chosen folders had more sub-folders than the budget allows.
    pub limited: bool,
    pub error: Option<String>,
}

enum Cmd {
    Set(Targets),
    Stop,
}

pub struct Tracker {
    tx: Sender<Cmd>,
    handle: Option<JoinHandle<()>>,
    pub stats: Arc<Mutex<TrackerStats>>,
}

impl Tracker {
    pub fn start(on_event: impl Fn(FsEvent) + Send + 'static) -> Tracker {
        let (tx, rx) = channel();
        let stats = Arc::new(Mutex::new(TrackerStats::default()));
        let st = stats.clone();
        let handle = std::thread::Builder::new()
            .name("organise-tracker".into())
            .spawn(move || run(rx, Box::new(on_event), st))
            .ok();
        Tracker { tx, handle, stats }
    }

    pub fn set_targets(&self, t: Targets) {
        let _ = self.tx.send(Cmd::Set(t));
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Prints to stderr when ORGANISE_DEBUG is set.
pub fn debug_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ORGANISE_DEBUG").is_some())
}

macro_rules! dbg_log {
    ($($t:tt)*) => { if debug_on() { eprintln!($($t)*); } };
}

/// How many watches we allow ourselves: a quarter of the system limit, at most 8,192.
pub fn watch_budget() -> usize {
    std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .map(|m| (m / 4).min(8192))
        .unwrap_or(2048)
}

#[cfg(not(target_os = "linux"))]
fn run(rx: Receiver<Cmd>, _on_event: Box<dyn Fn(FsEvent) + Send>, stats: Arc<Mutex<TrackerStats>>) {
    stats.lock().unwrap().error = Some("Live watching isn't available on this system yet".into());
    while let Ok(c) = rx.recv() {
        if matches!(c, Cmd::Stop) {
            return;
        }
    }
}

#[cfg(target_os = "linux")]
fn run(rx: Receiver<Cmd>, on_event: Box<dyn Fn(FsEvent) + Send>, stats: Arc<Mutex<TrackerStats>>) {
    use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::mpsc::TryRecvError;
    use std::time::{Duration, Instant};

    let mut ino = match Inotify::init() {
        Ok(i) => i,
        Err(e) => {
            stats.lock().unwrap().error = Some(format!("inotify: {e}"));
            return;
        }
    };
    let mask = WatchMask::MOVED_FROM
        | WatchMask::MOVED_TO
        | WatchMask::CREATE
        | WatchMask::DELETE
        | WatchMask::CLOSE_WRITE
        | WatchMask::ONLYDIR
        | WatchMask::DONT_FOLLOW;
    let budget = watch_budget();
    stats.lock().unwrap().budget = budget;
    let mut wds: HashMap<WatchDescriptor, PathBuf> = HashMap::new();
    // Watches we removed: events already queued for them still arrive (until IN_IGNORED), so keep their paths.
    let mut retired: HashMap<WatchDescriptor, PathBuf> = HashMap::new();
    let mut targets = Targets::default();
    let mut pending: HashMap<u32, (PathBuf, bool, Instant)> = HashMap::new();
    let mut buf = vec![0u8; 64 * 1024];

    // Adds a watch for each folder in `dirs` (and their sub-folders when `recursive`) until the budget is used.
    let add = |ino: &mut Inotify, wds: &mut HashMap<WatchDescriptor, PathBuf>, start: &Path, recursive: bool, skip_hidden: bool| -> bool {
        let mut stack = vec![start.to_path_buf()];
        while let Some(d) = stack.pop() {
            if wds.len() >= budget {
                return false;
            }
            match ino.watches().add(&d, mask) {
                Ok(wd) => {
                    wds.insert(wd, d.clone());
                }
                Err(e) => {
                    dbg_log!("tracker: can't watch {}: {e}", d.display());
                    continue;
                }
            }
            if recursive {
                if let Ok(rd) = std::fs::read_dir(&d) {
                    for e in rd.flatten() {
                        if e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false) {
                            let p = e.path();
                            if !crate::scan::skip_dir(&e.file_name().to_string_lossy(), &p, skip_hidden) {
                                stack.push(p);
                            }
                        }
                    }
                }
            }
        }
        true
    };
    let in_roots = |t: &Targets, p: &Path| t.roots.iter().any(|r| p.starts_with(r));

    loop {
        // commands
        loop {
            match rx.try_recv() {
                Ok(Cmd::Set(t)) => {
                    // Work out the folders wanted, then change only the difference. (Removing and re-adding a
                    // watch would lose events the kernel has already queued for it.)
                    let mut want: Vec<PathBuf> = vec![];
                    let mut seen = HashSet::new();
                    for d in &t.dirs {
                        if d.is_dir() && seen.insert(d.clone()) {
                            want.push(d.clone());
                        }
                    }
                    let mut complete = true;
                    for r in &t.roots {
                        let mut stack = vec![r.clone()];
                        while let Some(d) = stack.pop() {
                            if want.len() >= budget {
                                complete = false;
                                break;
                            }
                            if !d.is_dir() {
                                continue;
                            }
                            if seen.insert(d.clone()) {
                                want.push(d.clone());
                            }
                            if let Ok(rd) = std::fs::read_dir(&d) {
                                for e in rd.flatten() {
                                    if e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false) {
                                        let p = e.path();
                                        if !crate::scan::skip_dir(&e.file_name().to_string_lossy(), &p, t.skip_hidden) {
                                            stack.push(p);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let have: HashSet<PathBuf> = wds.values().cloned().collect();
                    let gone: Vec<WatchDescriptor> = wds.iter().filter(|(_, p)| !seen.contains(*p)).map(|(w, _)| w.clone()).collect();
                    for wd in gone {
                        let _ = ino.watches().remove(wd.clone());
                        if let Some(p) = wds.remove(&wd) {
                            retired.insert(wd, p);
                        }
                    }
                    for d in want.iter().filter(|d| !have.contains(*d)) {
                        add(&mut ino, &mut wds, d, false, t.skip_hidden);
                    }
                    dbg_log!("tracker: targets {:?} roots {:?} -> {} watches", t.dirs, t.roots, wds.len());
                    let mut s = stats.lock().unwrap();
                    s.watches = wds.len();
                    s.limited = !complete;
                    targets = t;
                }
                Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => break,
            }
        }

        let mut got_any = false;
        let mut new_dirs: Vec<PathBuf> = vec![];
        let mut moved_dirs: Vec<(PathBuf, PathBuf)> = vec![];
        let mut out: Vec<FsEvent> = vec![];
        match ino.read_events(&mut buf) {
            Ok(events) => {
                for ev in events {
                    got_any = true;
                    if ev.mask.contains(EventMask::Q_OVERFLOW) {
                        out.push(FsEvent::Overflow);
                        continue;
                    }
                    if ev.mask.contains(EventMask::IGNORED) {
                        if retired.remove(&ev.wd).is_none() {
                            wds.remove(&ev.wd); // the folder itself was deleted
                        }
                        continue;
                    }
                    let (Some(dir), Some(name)) = (wds.get(&ev.wd).or_else(|| retired.get(&ev.wd)), ev.name) else { continue };
                    let path = dir.join(name);
                    let is_dir = ev.mask.contains(EventMask::ISDIR);
                    if ev.mask.contains(EventMask::MOVED_FROM) {
                        pending.insert(ev.cookie, (path, is_dir, Instant::now()));
                    } else if ev.mask.contains(EventMask::MOVED_TO) {
                        if let Some((from, _, _)) = pending.remove(&ev.cookie) {
                            if is_dir {
                                moved_dirs.push((from.clone(), path.clone()));
                            }
                            out.push(FsEvent::Moved { from, to: path });
                        } else {
                            if is_dir {
                                new_dirs.push(path.clone());
                            }
                            out.push(FsEvent::Appeared(path));
                        }
                    } else if ev.mask.contains(EventMask::CREATE) {
                        if is_dir {
                            new_dirs.push(path.clone());
                            out.push(FsEvent::Appeared(path));
                        }
                    } else if ev.mask.contains(EventMask::CLOSE_WRITE) {
                        out.push(FsEvent::Appeared(path));
                    } else if ev.mask.contains(EventMask::DELETE) {
                        out.push(FsEvent::Gone(path));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => {
                stats.lock().unwrap().error = Some(format!("inotify read: {e}"));
                return;
            }
        }
        // a watched folder that moved: its watches stay, so fix the paths we report for them
        for (from, to) in moved_dirs {
            for p in wds.values_mut().chain(retired.values_mut()) {
                if let Ok(rest) = p.strip_prefix(&from) {
                    *p = to.join(rest);
                }
            }
            if in_roots(&targets, &to) && !wds.values().any(|p| p == &to) {
                new_dirs.push(to);
            }
        }
        for d in new_dirs {
            if in_roots(&targets, &d) {
                add(&mut ino, &mut wds, &d, true, targets.skip_hidden);
            }
        }
        // moves out of the watched folders
        let now = Instant::now();
        let mut expired = vec![];
        pending.retain(|_, (p, is_dir, t)| {
            if now.duration_since(*t) > Duration::from_millis(300) {
                expired.push((p.clone(), *is_dir));
                false
            } else {
                true
            }
        });
        for (p, is_dir) in expired {
            if is_dir {
                // its watches would now report wrong paths
                let gone: Vec<WatchDescriptor> = wds.iter().filter(|(_, d)| d.starts_with(&p)).map(|(w, _)| w.clone()).collect();
                for wd in gone {
                    let _ = ino.watches().remove(wd.clone());
                    if let Some(p) = wds.remove(&wd) {
                        retired.insert(wd, p);
                    }
                }
            }
            out.push(FsEvent::Gone(p));
        }
        if !out.is_empty() {
            stats.lock().unwrap().watches = wds.len();
        }
        for e in out {
            dbg_log!("tracker: {e:?}");
            on_event(e);
        }
        if !got_any {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
