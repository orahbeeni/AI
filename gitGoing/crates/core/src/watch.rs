//! Noticing changes made outside the app: files edited in an editor, commits,
//! checkouts or fetches made in a terminal.
//!
//! A [`Watch`] follows one repository on a background thread. File-system events
//! are filtered (ignored files, git's own object store and lock files are not
//! interesting), merged over a short quiet period, and reported as two flags the
//! UI polls with [`Watch::take`]:
//!
//! * `work`: the working tree or index changed, so the status should be re-read.
//! * `refs`: a branch, tag or HEAD actually moved, so the history should be reloaded.
//!
//! Reference changes are confirmed by comparing a fingerprint of all references, so
//! the app's own operations (which reload by themselves and call [`Watch::sync_refs`])
//! don't cause a second reload.
//!
//! On Linux every watched directory costs an inotify watch, so directories that
//! git ignores (`node_modules`, `target`, ...) are not watched at all. macOS and
//! Windows watch the whole tree natively and filter the events afterwards.

use git2::Repository;
use notify::{EventKind, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Events closer together than this are one change.
const QUIET: Duration = Duration::from_millis(300);
/// Under a constant stream of events, report at least this often.
const MAX_WAIT: Duration = Duration::from_millis(1500);
/// Most directories watched one by one (Linux); beyond this the focus refresh still works.
#[cfg(target_os = "linux")]
const MAX_DIRS: usize = 20_000;

static ENABLED: AtomicBool = AtomicBool::new(true);

/// Turn automatic refreshing on or off for the whole app.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// What changed since the last [`Watch::take`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// Working tree, index or in-progress operation state.
    pub work: bool,
    /// A reference moved or HEAD switched.
    pub refs: bool,
}

#[derive(Default)]
struct Flags {
    work: AtomicBool,
    refs: AtomicBool,
}

enum Msg {
    Event(notify::Result<notify::Event>),
    /// The caller has just brought itself up to date with the references.
    Sync,
    Stop,
}

/// Follows one repository until dropped.
pub struct Watch {
    flags: Arc<Flags>,
    tx: Sender<Msg>,
}

impl Watch {
    /// Start watching the repository containing `path`. `wake` is called from the
    /// watcher thread whenever there is something new to [`take`](Self::take).
    /// `None` if the repository or the OS watcher can't be set up.
    pub fn start(path: &Path, wake: impl Fn() + Send + 'static) -> Option<Watch> {
        let repo = Repository::discover(path).ok()?;
        let workdir = std::fs::canonicalize(repo.workdir()?).ok()?;
        let gitdir = std::fs::canonicalize(repo.path()).ok()?;
        let flags = Arc::new(Flags::default());
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let event_tx = tx.clone();
        let thread_flags = flags.clone();
        std::thread::Builder::new()
            .name("gitgoing-watch".into())
            .spawn(move || {
                let handler = move |res| {
                    let _ = event_tx.send(Msg::Event(res));
                };
                let mut watcher = match notify::recommended_watcher(handler) {
                    Ok(w) => w,
                    Err(_) => {
                        let _ = ready_tx.send(false);
                        return;
                    }
                };
                let _ = ready_tx.send(true);
                let mut w = Worker { repo, workdir, gitdir, flags: thread_flags, wake: Box::new(wake), known: 0 };
                w.known = fingerprint(&w.repo);
                w.attach(&mut watcher);
                w.run(&rx, &mut watcher);
            })
            .ok()?;
        ready_rx.recv().ok()?.then_some(Watch { flags, tx })
    }

    /// What changed since the last call, clearing it.
    pub fn take(&self) -> Changed {
        Changed { work: self.flags.work.swap(false, Ordering::Relaxed), refs: self.flags.refs.swap(false, Ordering::Relaxed) }
    }

    /// Call when the caller has just reloaded after its own operation: references
    /// that moved so far are not news. Also drops anything pending.
    pub fn sync_refs(&self) {
        self.flags.refs.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Msg::Sync);
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

/// A fingerprint of every reference and of what HEAD points at.
fn fingerprint(repo: &Repository) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut refs: Vec<(String, String)> = repo
        .references()
        .map(|it| {
            it.flatten()
                .filter_map(|r| Some((r.name()?.to_owned(), r.target().map(|o| o.to_string()).or_else(|| r.symbolic_target().map(str::to_owned))?)))
                .collect()
        })
        .unwrap_or_default();
    refs.sort();
    let head = repo.find_reference("HEAD").ok().map(|h| (h.symbolic_target().map(str::to_owned), h.target()));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (refs, head).hash(&mut h);
    h.finish()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Work,
    Refs,
}

struct Worker {
    repo: Repository,
    workdir: PathBuf,
    gitdir: PathBuf,
    flags: Arc<Flags>,
    wake: Box<dyn Fn() + Send>,
    /// Fingerprint of the references the caller knows about.
    known: u64,
}

impl Worker {
    /// Register with the OS watcher.
    fn attach(&self, watcher: &mut notify::RecommendedWatcher) {
        #[cfg(target_os = "linux")]
        {
            let _ = watcher.watch(&self.gitdir, RecursiveMode::NonRecursive);
            let _ = watcher.watch(&self.gitdir.join("refs"), RecursiveMode::Recursive);
            let mut budget = MAX_DIRS;
            self.watch_tree(watcher, &self.workdir, &mut budget);
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = watcher.watch(&self.workdir, RecursiveMode::Recursive);
            if !self.gitdir.starts_with(&self.workdir) {
                let _ = watcher.watch(&self.gitdir, RecursiveMode::Recursive);
            }
        }
    }

    /// Watch `dir` and every directory below it that git doesn't ignore.
    #[cfg(target_os = "linux")]
    fn watch_tree(&self, watcher: &mut notify::RecommendedWatcher, dir: &Path, budget: &mut usize) {
        if *budget == 0 || watcher.watch(dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }
        *budget -= 1;
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = entry.path();
            // `.git` is handled separately; a directory with its own `.git` is another repository.
            if path == self.gitdir || path.join(".git").exists() || self.ignored(&path, true) {
                continue;
            }
            self.watch_tree(watcher, &path, budget);
        }
    }

    fn ignored(&self, path: &Path, is_dir: bool) -> bool {
        let Ok(rel) = path.strip_prefix(&self.workdir) else { return false };
        if rel.as_os_str().is_empty() {
            return false;
        }
        // Directory patterns such as `target/` only match with a trailing slash.
        let probe = if is_dir { rel.join("") } else { rel.to_path_buf() };
        self.repo.status_should_ignore(&probe).unwrap_or(false)
    }

    fn classify(&self, path: &Path) -> Option<Kind> {
        if let Ok(rel) = path.strip_prefix(&self.gitdir) {
            if rel.components().any(|c| c.as_os_str().to_string_lossy().ends_with(".lock")) {
                return None;
            }
            let first = rel.components().next()?.as_os_str().to_string_lossy().into_owned();
            return match first.as_str() {
                "HEAD" | "packed-refs" | "refs" => Some(Kind::Refs),
                "index" | "MERGE_HEAD" | "MERGE_MSG" | "CHERRY_PICK_HEAD" | "REVERT_HEAD" | "rebase-merge" | "rebase-apply"
                | "sequencer" => Some(Kind::Work),
                _ => None,
            };
        }
        let rel = path.strip_prefix(&self.workdir).ok()?;
        if rel.components().any(|c| c.as_os_str() == ".git") {
            return None;
        }
        let is_dir = path.is_dir();
        (!self.ignored(path, is_dir)).then_some(Kind::Work)
    }

    fn run(&mut self, rx: &mpsc::Receiver<Msg>, watcher: &mut notify::RecommendedWatcher) {
        let (mut work, mut refs) = (false, false);
        let mut first: Option<Instant> = None;
        let mut last = Instant::now();
        loop {
            let msg = match first {
                None => match rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => return,
                },
                Some(f) => {
                    let wait = QUIET.saturating_sub(last.elapsed()).min(MAX_WAIT.saturating_sub(f.elapsed()));
                    match rx.recv_timeout(wait) {
                        Ok(m) => Some(m),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            };
            match msg {
                Some(Msg::Stop) => return,
                Some(Msg::Sync) => {
                    self.known = fingerprint(&self.repo);
                    refs = false;
                    if !work {
                        first = None;
                    }
                }
                Some(Msg::Event(Ok(ev))) => {
                    if matches!(ev.kind, EventKind::Access(_)) {
                        continue;
                    }
                    #[cfg(target_os = "linux")]
                    if matches!(ev.kind, EventKind::Create(_)) {
                        // A new directory: watch it, and treat what is already inside as new.
                        for p in ev.paths.iter().filter(|p| p.is_dir() && p.starts_with(&self.workdir)) {
                            if !p.components().any(|c| c.as_os_str() == ".git") && !self.ignored(p, true) {
                                let mut budget = MAX_DIRS;
                                self.watch_tree(watcher, p, &mut budget);
                            }
                        }
                    }
                    let mut hit = false;
                    if ev.paths.is_empty() {
                        (work, refs, hit) = (true, true, true);
                    }
                    for p in &ev.paths {
                        // Once the tree is known to be dirty only reference changes add information.
                        if work && !p.starts_with(&self.gitdir) {
                            hit = true;
                            continue;
                        }
                        match self.classify(p) {
                            Some(Kind::Work) => (work, hit) = (true, true),
                            Some(Kind::Refs) => (work, refs, hit) = (true, true, true),
                            None => {}
                        }
                    }
                    if hit {
                        last = Instant::now();
                        first.get_or_insert(last);
                    }
                }
                Some(Msg::Event(Err(_))) => {
                    // Events were lost (e.g. the OS queue overflowed): assume anything changed.
                    (work, refs) = (true, true);
                    last = Instant::now();
                    first.get_or_insert(last);
                }
                None => {
                    // Quiet for long enough: report.
                    let mut report = work;
                    if refs {
                        let now = fingerprint(&self.repo);
                        if now != self.known {
                            self.known = now;
                            self.flags.refs.store(true, Ordering::Relaxed);
                            report = true;
                        }
                    }
                    if report {
                        self.flags.work.store(true, Ordering::Relaxed);
                        (self.wake)();
                    }
                    (work, refs, first) = (false, false, None);
                }
            }
            // Under constant churn the quiet period never arrives; report anyway.
            if first.is_some_and(|f| f.elapsed() >= MAX_WAIT) {
                last = Instant::now().checked_sub(QUIET).unwrap_or(last);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Fixture {
        dir: tempfile::TempDir,
        woken: Arc<AtomicUsize>,
        watch: Watch,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "T").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        cfg.set_bool("commit.gpgsign", false).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "a\n").unwrap();
        crate::status::stage(dir.path(), &[".gitignore".into(), "src/a.rs".into()]).unwrap();
        crate::status::commit(dir.path(), "base").unwrap();
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        let watch = Watch::start(dir.path(), move || {
            w.fetch_add(1, Ordering::SeqCst);
        })
        .expect("watcher starts");
        std::thread::sleep(Duration::from_millis(300)); // let the initial walk finish
        Fixture { dir, woken, watch }
    }

    /// Wait up to two seconds for a report.
    fn wait_for_change(f: &Fixture) -> Option<Changed> {
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            let c = f.watch.take();
            if c != Changed::default() {
                return Some(c);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    #[test]
    fn edits_new_files_and_new_directories_are_noticed() {
        let f = fixture();
        std::fs::write(f.dir.path().join("src/a.rs"), "changed\n").unwrap();
        assert_eq!(wait_for_change(&f), Some(Changed { work: true, refs: false }));

        std::fs::write(f.dir.path().join("new.txt"), "n\n").unwrap();
        assert!(wait_for_change(&f).is_some_and(|c| c.work));

        // A directory created later is watched too.
        std::fs::create_dir_all(f.dir.path().join("deep/er")).unwrap();
        wait_for_change(&f);
        std::thread::sleep(Duration::from_millis(500));
        f.watch.take();
        std::fs::write(f.dir.path().join("deep/er/file"), "x\n").unwrap();
        assert!(wait_for_change(&f).is_some_and(|c| c.work));
    }

    #[test]
    fn ignored_files_and_directories_are_not_reported() {
        let f = fixture();
        std::fs::write(f.dir.path().join("debug.log"), "noise\n").unwrap();
        std::fs::write(f.dir.path().join("target/debug/out.o"), "noise\n").unwrap();
        std::fs::create_dir_all(f.dir.path().join("target/new")).unwrap();
        std::fs::write(f.dir.path().join("target/new/x"), "noise\n").unwrap();
        assert_eq!(wait_for_change(&f), None);
        assert_eq!(f.woken.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn git_internals_that_do_not_matter_are_not_reported() {
        let f = fixture();
        // Reading state must not look like a change either.
        crate::status::status(f.dir.path()).unwrap();
        let repo = Repository::open(f.dir.path()).unwrap();
        let _ = repo.statuses(None).unwrap().len();
        let _ = crate::remote::refs_fingerprint(f.dir.path());
        std::fs::write(f.dir.path().join(".git/objects/stray"), "x").unwrap();
        std::fs::write(f.dir.path().join(".git/index.lock"), "x").unwrap();
        assert_eq!(wait_for_change(&f), None, "status reads and object-store noise are not changes");
    }

    #[test]
    fn a_commit_from_outside_reports_moved_references_once() {
        let f = fixture();
        std::fs::write(f.dir.path().join("b.txt"), "b\n").unwrap();
        wait_for_change(&f);
        let run = |args: &[&str]| {
            assert!(std::process::Command::new("git").arg("-C").arg(f.dir.path()).args(args).status().unwrap().success());
        };
        run(&["add", "b.txt"]);
        run(&["commit", "-q", "-m", "from a terminal"]);
        let c = wait_for_change(&f).expect("noticed");
        assert!(c.refs && c.work, "{c:?}");
        // Nothing further is pending.
        assert_eq!(wait_for_change(&f), None);

        // The app's own operations call sync_refs after reloading, so they don't echo back.
        run(&["branch", "mine"]);
        f.watch.sync_refs();
        assert!(!wait_for_change(&f).is_some_and(|c| c.refs), "own changes are not news");
    }
}
