//! The phone app's engine (wrapped for Android in `android/wifishare/rust`). It pairs with
//! computers, follows one of them at a time over a control link, browses that computer's shared
//! folders, sends files into its inbox, and answers the computer's requests for this phone's
//! storage (shared as one folder, "Phone"). It never listens: every connection is dialled from
//! here, to the computer's phone port (`phones::port`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::config::{Config, Identity, Paths, Peer, Peers, ShareCfg};
use crate::discovery::{self, Beacon};
use crate::files::{Files, INBOX, Job};
use crate::netwatch;
use crate::pair;
use crate::phones;
use crate::proto::{DirEntry, Hello, Msg, Os, PROTOCOL_VERSION, Share};
use crate::status::{Approval, Hub, LinkState, Status};
use crate::wire::{self, Link, Purpose};

/// What the computer sees this phone's storage as.
pub const STORAGE_SHARE: &str = "Phone";
const MAX_BACKOFF: Duration = Duration::from_secs(2);
/// Browsing and folder making give up after this.
const ASK: Duration = Duration::from_secs(20);

/// How the phone app starts the engine.
pub struct Setup {
    /// The app's private folder (keys, paired computers, settings).
    pub dir: PathBuf,
    /// What computers see this phone as.
    pub name: String,
    /// The storage computers may browse; `None` until the app has the permission for it.
    pub storage: Option<PathBuf>,
    /// Let computers add files to the storage (never overwrite or delete).
    pub allow_add: bool,
}

/// A paired computer.
#[derive(Clone, Debug)]
pub struct Computer {
    pub name: String,
    pub key: String,
    pub last_addr: Option<String>,
    /// The one this phone follows.
    pub current: bool,
}

/// Every computer heard on the LAN: key -> where and when.
type Heard = Arc<Mutex<HashMap<Vec<u8>, HashMap<SocketAddr, Instant>>>>;

struct Following {
    peer: Peer,
    files: Arc<Files>,
    task: JoinHandle<()>,
}

pub struct Mobile {
    paths: Paths,
    id: Identity,
    pub hub: Arc<Hub>,
    rt: tokio::runtime::Runtime,
    /// The computers' phone port.
    port: u16,
    heard: Heard,
    following: Mutex<Option<Following>>,
    /// Earlier computers' copies, kept on the list until cleared.
    retired: Mutex<Vec<Arc<Files>>>,
}

impl Mobile {
    pub fn start(setup: Setup) -> Result<Mobile> {
        let paths = Paths::new(Some(setup.dir))?;
        let mut cfg = Config::load(&paths)?;
        cfg.name = setup.name;
        cfg.files.shares =
            setup.storage.into_iter().map(|path| ShareCfg { name: STORAGE_SHARE.into(), path, write: setup.allow_add }).collect();
        cfg.save(&paths)?;
        let id = Identity::load_or_create(&paths)?;
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().thread_name("wifishare-net").build()?;
        let hub = Hub::new(Approval::Gui);
        let heard: Heard = Arc::default();
        let port = phones::port(&cfg);
        {
            let (hub, heard) = (hub.clone(), heard.clone());
            rt.spawn(async move {
                let (tx, mut rx) = mpsc::unbounded_channel::<(Beacon, SocketAddr)>();
                // Probing stops when this sender goes, so it lives as long as the task.
                let (_probing, probing) = watch::channel(true);
                tokio::spawn(discovery::listen(port, tx, probing, netwatch::spawn()));
                while let Some((b, addr)) = rx.recv().await {
                    hub.heard(&b, addr);
                    let mut heard = heard.lock().unwrap();
                    let seen = heard.entry(b.key.clone()).or_default();
                    seen.retain(|_, t| t.elapsed() < Duration::from_secs(30));
                    seen.insert(addr, Instant::now());
                }
            });
        }
        let me = Mobile { paths, id, hub, rt, port, heard, following: Mutex::new(None), retired: Mutex::default() };
        // The computer chosen last time, or else the one paired last.
        let chosen = std::fs::read_to_string(me.chosen_file()).ok();
        let computers = me.computers();
        let pick = computers.iter().find(|c| Some(c.name.as_str()) == chosen.as_deref()).or(computers.last());
        if let Some(c) = pick {
            me.follow(Some(&c.name))?;
        } else {
            me.hub.update(|s| s.link = LinkState::Unpaired);
        }
        Ok(me)
    }

    fn chosen_file(&self) -> PathBuf {
        self.paths.config_dir.join("following")
    }

    /// Changes what computers see: this phone's name and whether they may add files.
    pub fn configure(&self, name: &str, storage: Option<PathBuf>, allow_add: bool) -> Result<()> {
        let mut cfg = Config::load(&self.paths)?;
        cfg.name = name.into();
        cfg.files.shares = storage.into_iter().map(|path| ShareCfg { name: STORAGE_SHARE.into(), path, write: allow_add }).collect();
        cfg.save(&self.paths)
    }

    pub fn status(&self) -> Status {
        self.hub.snapshot()
    }

    pub fn computers(&self) -> Vec<Computer> {
        let current = self.following.lock().unwrap().as_ref().map(|f| f.peer.key.clone());
        Peers::load(&self.paths)
            .map(|p| p.servers().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .map(|p| Computer { current: current.as_deref() == Some(p.key.as_str()), name: p.name, key: p.key, last_addr: p.last_addr })
            .collect()
    }

    /// Pairs with the computer at `target` (an address, or `host:port`); the outcome shows up as
    /// a notice, and the questions in between as pairing prompts.
    pub fn pair(self: &Arc<Self>, target: String) {
        let me = self.clone();
        self.rt.spawn(async move {
            let result = async {
                let cfg = Config::load(&me.paths)?;
                let addr = pair::resolve(&target, me.port).await?;
                pair::pair_with(&cfg.name, &me.paths, &me.id, addr, &me.hub).await
            }
            .await;
            let text = match result {
                Ok(name) => {
                    if let Err(e) = me.follow(Some(&name)) {
                        log::warn!("following {name}: {e:#}");
                    }
                    format!("Paired with {name}")
                }
                Err(e) => format!("Pairing failed: {e:#}"),
            };
            me.hub.update(|s| s.notice = Some((text, Instant::now())));
        });
    }

    /// Follows the paired computer `name` (stopping any other), or nobody. Copies still running
    /// with the computer left behind are cancelled.
    pub fn follow(&self, name: Option<&str>) -> Result<()> {
        let peer = match name {
            Some(n) => {
                Some(Peers::load(&self.paths)?.servers().find(|p| p.name == n).cloned().with_context(|| format!("{n} is not paired"))?)
            }
            None => None,
        };
        let mut following = self.following.lock().unwrap();
        if let (Some(f), Some(p)) = (following.as_ref(), &peer)
            && f.peer.key == p.key
        {
            return Ok(());
        }
        if let Some(old) = following.take() {
            old.task.abort();
            old.files.set_link(None);
            for job in old.files.jobs() {
                job.cancel();
            }
            self.retired.lock().unwrap().push(old.files);
        }
        let Some(peer) = peer else {
            self.hub.update(|s| s.link = LinkState::Idle);
            return Ok(());
        };
        let key = hex::decode(&peer.key)?;
        let _enter = self.rt.enter();
        let files = Files::client(self.paths.clone(), self.id.clone(), self.hub.clone(), key);
        let task = self.rt.spawn(follow(
            self.paths.clone(),
            self.id.clone(),
            self.hub.clone(),
            files.clone(),
            peer.clone(),
            self.port,
            self.heard.clone(),
        ));
        log::info!("following {}", peer.name);
        let _ = std::fs::write(self.chosen_file(), &peer.name);
        *following = Some(Following { peer, files, task });
        Ok(())
    }

    /// Unpairs a computer (it must pair again to be used).
    pub fn forget(&self, name: &str) -> Result<()> {
        if self.following.lock().unwrap().as_ref().is_some_and(|f| f.peer.name == name) {
            self.follow(None)?;
        }
        let mut peers = Peers::load(&self.paths)?;
        peers.peers.retain(|p| p.name != name);
        peers.save(&self.paths)?;
        if peers.servers().next().is_none() {
            self.hub.update(|s| s.link = LinkState::Unpaired);
        }
        Ok(())
    }

    fn current(&self) -> Result<(String, Arc<Files>)> {
        let following = self.following.lock().unwrap();
        let f = following.as_ref().context("no computer chosen")?;
        Ok((f.peer.name.clone(), f.files.clone()))
    }

    /// Blocks the calling thread (never the runtime's) until `fut` is done or `ASK` runs out.
    fn ask<T>(&self, fut: impl Future<Output = Result<T>>) -> Result<T> {
        self.rt.block_on(async { tokio::time::timeout(ASK, fut).await.map_err(|_| anyhow::anyhow!("the computer did not answer"))? })
    }

    pub fn shares(&self) -> Result<Vec<Share>> {
        let (peer, files) = self.current()?;
        self.ask(async { files.open(&peer).await?.shares().await })
    }

    pub fn list(&self, share: &str, path: &str) -> Result<Vec<DirEntry>> {
        let (peer, files) = self.current()?;
        self.ask(async { files.open(&peer).await?.list(share, path).await })
    }

    pub fn mkdir(&self, share: &str, path: &str) -> Result<()> {
        let (peer, files) = self.current()?;
        self.ask(async { files.open(&peer).await?.mkdir(share, path).await })
    }

    /// Copies `names` from the folder `dir` of a share on the computer into `dest` here.
    pub fn download(&self, share: &str, dir: &str, names: Vec<String>, dest: PathBuf) -> Result<Arc<Job>> {
        let (peer, files) = self.current()?;
        let _enter = self.rt.enter();
        Ok(files.download(&peer, share, dir, names, dest))
    }

    /// Copies local files and folders into the folder `dir` of a share on the computer.
    pub fn upload(&self, share: &str, dir: &str, sources: Vec<PathBuf>) -> Result<Arc<Job>> {
        let (peer, files) = self.current()?;
        let _enter = self.rt.enter();
        Ok(files.upload(&peer, share, dir, sources))
    }

    /// Sends local files to the computer's inbox (its Downloads folder unless set otherwise).
    pub fn send(&self, sources: Vec<PathBuf>) -> Result<Arc<Job>> {
        self.upload(INBOX, "", sources)
    }

    /// Every copy, oldest first.
    pub fn jobs(&self) -> Vec<Arc<Job>> {
        let mut all: Vec<Arc<Job>> = self.retired.lock().unwrap().iter().flat_map(|f| f.jobs()).collect();
        if let Ok((_, files)) = self.current() {
            all.extend(files.jobs());
        }
        all
    }

    pub fn clear_finished(&self) {
        let mut retired = self.retired.lock().unwrap();
        for f in retired.iter() {
            f.clear_finished();
        }
        retired.retain(|f| !f.jobs().is_empty());
        drop(retired);
        if let Ok((_, files)) = self.current() {
            files.clear_finished();
        }
    }
}

/// Keeps a control link to `peer` up, for as long as this phone follows it.
async fn follow(paths: Paths, id: Identity, hub: Arc<Hub>, files: Arc<Files>, peer: Peer, port: u16, heard: Heard) {
    let Ok(key) = hex::decode(&peer.key) else { return };
    let mut last: Option<SocketAddr> = peer.last_addr.as_deref().and_then(|a| a.parse().ok());
    let mut backoff = Duration::from_millis(250);
    let mut since = Instant::now();
    let mut error: Option<String> = None;
    loop {
        hub.update(|s| s.link = LinkState::Searching { server: peer.name.clone(), since, error: error.clone() });
        let beacons = heard.lock().unwrap().get(&key).cloned().unwrap_or_default();
        let candidates = discovery::candidates(None, port, last, &beacons).await;
        let attempt = if candidates.is_empty() {
            Err(anyhow::anyhow!("not heard on this network yet"))
        } else {
            wire::connect_any(&candidates, &id, &key, Purpose::Control).await
        };
        match attempt {
            Ok(s) => {
                let addr = s.peer_addr;
                if last != Some(addr) {
                    last = Some(addr);
                    Peers::remember_addr(&paths, &peer.key, addr);
                }
                let started = Instant::now();
                error = Some(match session(&paths, s, &files, &hub, &peer.name).await {
                    Ok(()) => "the link dropped".into(),
                    Err(e) => format!("{e:#}"),
                });
                files.set_link(None);
                since = Instant::now();
                backoff = if started.elapsed() < Duration::from_secs(2) { MAX_BACKOFF } else { Duration::from_millis(100) };
            }
            Err(e) => error = Some(format!("{e:#}")),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// One control link: answers the computer's requests for dial-backs until the link drops.
async fn session(paths: &Paths, s: wire::Secured, files: &Arc<Files>, hub: &Hub, name: &str) -> Result<()> {
    let addr = s.peer_addr;
    let cfg = Config::load(paths)?;
    let mut link = Link::spawn(
        s.reader,
        s.writer,
        Duration::from_millis(cfg.network.heartbeat_ms),
        Duration::from_millis(cfg.network.timeout_ms),
        format!("computer {name} ({})", addr.ip()),
    );
    let _ = link.tx.send(Msg::Hello(Hello { version: PROTOCOL_VERSION, name: cfg.name.clone(), os: Os::current(), screen: None }));
    let hello = match tokio::time::timeout(Duration::from_secs(5), link.rx.recv()).await {
        Ok(Some(Msg::Hello(h))) => h,
        Ok(Some(Msg::Bye(reason))) => bail!("{reason}"),
        _ => bail!("{name} did not answer"),
    };
    if hello.version != PROTOCOL_VERSION {
        bail!("{name} runs protocol {} but this phone runs {PROTOCOL_VERSION}; update both", hello.version);
    }
    log::info!("connected to {name} at {addr}");
    files.set_link(Some(addr));
    hub.update(|s| s.link = LinkState::Connected { server: hello.name.clone(), addr, since: Instant::now(), rtt: link.rtt.clone() });
    while let Some(msg) = link.rx.recv().await {
        match msg {
            Msg::FilesDial { token } => files.on_dial(token),
            Msg::Bye(reason) => bail!("{reason}"),
            _ => {}
        }
    }
    log::info!("disconnected from {name}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::files::JobState;
    use crate::files::tests::scratch;

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let start = Instant::now();
        while !ok() {
            assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn finish(job: &Job) -> JobState {
        wait_for("the copy", || job.finished());
        job.state()
    }

    /// Says yes to every pairing question on `hubs` until the returned flag is cleared.
    fn approve_all(hubs: Vec<Arc<Hub>>) -> Arc<std::sync::atomic::AtomicBool> {
        let on = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let flag = on.clone();
        std::thread::spawn(move || {
            while flag.load(Ordering::Relaxed) {
                for hub in &hubs {
                    for p in hub.pair_prompts() {
                        hub.answer_pair(p.id, true);
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        on
    }

    /// A computer running the phone service on loopback, a phone pairing with it, and files in
    /// both directions.
    #[test]
    fn a_phone_pairs_sends_browses_and_is_browsed() {
        let dir = scratch("mobile");
        let (shared, inbox, storage) = (dir.join("shared"), dir.join("inbox"), dir.join("storage"));
        std::fs::create_dir_all(shared.join("photos")).unwrap();
        std::fs::write(shared.join("photos/beach.jpg"), b"sand").unwrap();
        std::fs::create_dir_all(storage.join("DCIM")).unwrap();
        std::fs::write(storage.join("DCIM/selfie.jpg"), b"me").unwrap();

        // The computer: a share, an inbox, and the phone service on port + 1.
        // Phones use port + 1, which must be free too: below the ephemeral range, so other tests'
        // sockets do not take it meanwhile.
        let port = (31000..40000)
            .step_by(7)
            .map(|p| p + std::process::id() as u16 % 7)
            .find(|p| std::net::TcpListener::bind(("0.0.0.0", *p + 1)).is_ok())
            .unwrap();
        let desk_paths = Paths::new(Some(dir.join("desk"))).unwrap();
        let mut cfg = Config { name: "the-mac".into(), port, ..Config::default() };
        cfg.files.shares.push(ShareCfg { name: "Shared".into(), path: shared.clone(), write: false });
        cfg.phone.inbox = Some(inbox.clone());
        cfg.save(&desk_paths).unwrap();
        let desk_rt = tokio::runtime::Runtime::new().unwrap();
        let desk_hub = Hub::new(Approval::Gui);
        let desk_files = {
            let _enter = desk_rt.enter();
            let id = Identity::load_or_create(&desk_paths).unwrap();
            let files = Files::server(desk_paths.clone(), id.clone(), desk_hub.clone());
            desk_rt.spawn(phones::run(cfg.clone(), desk_paths.clone(), id, desk_hub.clone(), files.clone()));
            files
        };
        std::thread::sleep(Duration::from_millis(200));

        let phone = Arc::new(
            Mobile::start(Setup { dir: dir.join("phone"), name: "pixel".into(), storage: Some(storage.clone()), allow_add: true }).unwrap(),
        );
        assert!(matches!(phone.status().link, LinkState::Unpaired));
        let approving = approve_all(vec![desk_hub.clone(), phone.hub.clone()]);
        phone.pair(format!("127.0.0.1:{}", port + 1));
        wait_for("the phone to connect", || matches!(phone.status().link, LinkState::Connected { .. }));
        approving.store(false, Ordering::Relaxed);
        assert!(Peers::load(&desk_paths).unwrap().phones().any(|p| p.name == "pixel"));
        let computers = phone.computers();
        assert_eq!(computers.len(), 1);
        assert!(computers[0].current && computers[0].name == "the-mac");
        wait_for("the window to list the phone", || desk_hub.snapshot().phones.iter().any(|p| p.name == "pixel"));

        // The phone browses the computer; the inbox is not on the list and cannot be listed.
        assert_eq!(phone.shares().unwrap(), [Share { name: "Shared".into(), write: false }]);
        assert_eq!(phone.list("Shared", "photos").unwrap()[0].name, "beach.jpg");
        assert!(phone.list(INBOX, "").unwrap_err().to_string().contains("cannot be browsed"));
        let got = dir.join("phone-downloads");
        let job = phone.download("Shared", "photos", vec!["beach.jpg".into()], got.clone()).unwrap();
        assert_eq!(finish(&job), JobState::Done);
        assert_eq!(std::fs::read(got.join("beach.jpg")).unwrap(), b"sand");

        // Sharing from the phone lands in the inbox, twice without overwriting.
        let note = dir.join("note.txt");
        std::fs::write(&note, "hi").unwrap();
        for _ in 0..2 {
            assert_eq!(finish(&phone.send(vec![note.clone()]).unwrap()), JobState::Done);
        }
        assert_eq!(std::fs::read_to_string(inbox.join("note.txt")).unwrap(), "hi");
        assert!(inbox.join("note (2).txt").exists());

        // The computer browses the phone through a dial-back and copies both ways.
        let pending = desk_files.shares("pixel");
        wait_for("the phone's shares", || {
            if let Some(r) = pending.take() {
                assert_eq!(r.unwrap(), [Share { name: STORAGE_SHARE.into(), write: true }]);
                true
            } else {
                false
            }
        });
        let from_phone = dir.join("from-phone");
        let job = desk_files.download("pixel", STORAGE_SHARE, "DCIM", vec!["selfie.jpg".into()], from_phone.clone());
        assert_eq!(finish(&job), JobState::Done);
        assert_eq!(std::fs::read(from_phone.join("selfie.jpg")).unwrap(), b"me");
        let job = desk_files.upload("pixel", STORAGE_SHARE, "", vec![shared.join("photos")]);
        assert_eq!(finish(&job), JobState::Done);
        assert_eq!(std::fs::read(storage.join("photos/beach.jpg")).unwrap(), b"sand");
        assert_eq!(job.done.load(Ordering::Relaxed), 4);

        // A restarted app goes back to the computer it was using.
        let again = Mobile::start(Setup { dir: dir.join("phone"), name: "pixel".into(), storage: None, allow_add: false }).unwrap();
        assert!(again.computers().iter().any(|c| c.current && c.name == "the-mac"));
        drop(again);

        // Forgotten on the computer: the phone is turned away and says why.
        let mut peers = Peers::load(&desk_paths).unwrap();
        peers.peers.retain(|p| p.role != crate::config::Role::Phone);
        peers.save(&desk_paths).unwrap();
        phone.follow(None).unwrap();
        phone.follow(Some("the-mac")).unwrap();
        wait_for("the refusal", || matches!(&phone.status().link, LinkState::Searching { error: Some(e), .. } if e.contains("not paired")));
        drop(desk_rt);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
