//! Two machines in one process, talking over loopback.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::config::{Config, FilesCfg, ShareCfg};
use crate::geom::{Screen, Side};
use crate::proto::{FsReq, Os};
use crate::server::core::{ClientSettings, ServerCore};
use crate::status::{Approval, ClientInfo, Rtt};

pub(crate) fn identity() -> Identity {
    let pair = snow::Builder::new(wire::NOISE_PARAMS.parse().unwrap()).generate_keypair().unwrap();
    Identity { private: pair.private, public: pair.public }
}

pub(crate) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wifishare-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A machine's config folder with the given shares.
pub(crate) fn machine(dir: &Path, shares: &[(&str, &Path, bool)]) -> Paths {
    let paths = Paths::new(Some(dir.join("config"))).unwrap();
    let cfg = Config {
        files: FilesCfg {
            shares: shares.iter().map(|(n, p, w)| ShareCfg { name: n.to_string(), path: p.to_path_buf(), write: *w }).collect(),
        },
        ..Config::default()
    };
    cfg.save(&paths).unwrap();
    paths
}

/// A server that answers files connections on loopback, and a client dialling it.
pub(crate) async fn pair(server_paths: Paths, client_paths: Paths) -> (Arc<Files>, Arc<Files>, SocketAddr) {
    let (sid, cid) = (identity(), identity());
    let hub = Hub::new(Approval::Terminal);
    let core = Arc::new(Mutex::new(ServerCore::new(Screen::single(1920, 1080))));
    let server = Files::server(server_paths, sid.clone(), hub.clone(), core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let s2 = server.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let (server, sid) = (s2.clone(), sid.clone());
            tokio::spawn(async move {
                let (purpose, s) = wire::accept(stream, &sid, Duration::from_secs(5)).await.unwrap();
                assert_eq!(purpose, Purpose::Files);
                let _ = server.accept(s).await;
            });
        }
    });
    let client = Files::client(client_paths, cid, Hub::new(Approval::Terminal), server.id.public.clone());
    client.set_link(Some(addr));
    (server, client, addr)
}

async fn finish(job: &Arc<Job>) -> JobState {
    let start = Instant::now();
    while !job.finished() {
        assert!(start.elapsed() < Duration::from_secs(20), "the job did not finish: {:?}", job.state());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    job.state()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn bytes(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

#[tokio::test]
async fn browse_and_download() {
    let dir = scratch("download");
    let share = dir.join("share");
    write(&share.join("notes.txt"), "hello");
    write(&share.join("album/one.bin"), "1");
    std::fs::write(share.join("album/big.bin"), bytes(3 * 1024 * 1024 + 17)).unwrap();
    std::fs::create_dir_all(share.join("album/empty")).unwrap();
    let (_server, client, _) = pair(machine(&dir.join("s"), &[("Docs", &share, false)]), machine(&dir.join("c"), &[])).await;

    let shares = client.shares("").take_wait().await.unwrap();
    assert_eq!(shares, [Share { name: "Docs".into(), write: false }]);
    let listing = client.list("", "Docs", "").take_wait().await.unwrap();
    let names: Vec<_> = listing.iter().map(|e| (e.name.as_str(), e.dir)).collect();
    assert_eq!(names, [("album", true), ("notes.txt", false)], "folders first");
    assert_eq!(listing[1].size, 5);

    let dest = dir.join("got");
    std::fs::create_dir_all(&dest).unwrap();
    write(&dest.join("notes.txt"), "already here");
    let job = client.download("", "Docs", "", vec!["album".into(), "notes.txt".into()], dest.clone());
    assert_eq!(finish(&job).await, JobState::Done);
    assert_eq!(std::fs::read(dest.join("album/big.bin")).unwrap(), bytes(3 * 1024 * 1024 + 17));
    assert_eq!(std::fs::read_to_string(dest.join("album/one.bin")).unwrap(), "1");
    assert!(dest.join("album/empty").is_dir());
    assert_eq!(std::fs::read_to_string(dest.join("notes.txt")).unwrap(), "already here", "nothing is overwritten");
    assert_eq!(std::fs::read_to_string(dest.join("notes (2).txt")).unwrap(), "hello");
    assert_eq!(job.done.load(Ordering::Relaxed), job.total.load(Ordering::Relaxed));
    assert!(std::fs::read_dir(&dest).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(PART)));

    // Asking for something outside the share is refused by the other side.
    let job = client.download("", "Docs", "", vec!["../s".into()], dir.join("got2"));
    assert!(matches!(finish(&job).await, JobState::Failed(_)));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn upload_into_a_writable_share_only() {
    let dir = scratch("upload");
    let (rw, ro) = (dir.join("rw"), dir.join("ro"));
    std::fs::create_dir_all(&rw).unwrap();
    std::fs::create_dir_all(&ro).unwrap();
    write(&rw.join("taken.txt"), "old");
    let (_server, client, _) =
        pair(machine(&dir.join("s"), &[("Inbox", &rw, true), ("Archive", &ro, false)]), machine(&dir.join("c"), &[])).await;

    let src = dir.join("src");
    write(&src.join("taken.txt"), "new");
    write(&src.join("tree/a.txt"), "a");
    write(&src.join("tree/deep/b.bin"), "b");
    std::fs::write(src.join("big.bin"), bytes(2 * 1024 * 1024 + 5)).unwrap();
    let job = client.upload("", "Inbox", "", vec![src.join("taken.txt"), src.join("tree"), src.join("big.bin")]);
    assert_eq!(finish(&job).await, JobState::Done);
    assert_eq!(std::fs::read_to_string(rw.join("taken.txt")).unwrap(), "old", "nothing is overwritten");
    assert_eq!(std::fs::read_to_string(rw.join("taken (2).txt")).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(rw.join("tree/deep/b.bin")).unwrap(), "b");
    assert_eq!(std::fs::read(rw.join("big.bin")).unwrap(), bytes(2 * 1024 * 1024 + 5));

    let job = client.upload("", "Archive", "", vec![src.join("tree")]);
    let JobState::Failed(why) = finish(&job).await else { panic!("expected a refusal") };
    assert!(why.contains("read-only"), "{why}");
    assert!(!ro.join("tree").exists());

    client.mkdir("", "Inbox", "made").take_wait().await.unwrap();
    assert!(rw.join("made").is_dir());
    client.mkdir("", "Inbox", "made").take_wait().await.unwrap();
    assert!(client.mkdir("", "Archive", "nope").take_wait().await.is_err());
    assert!(client.mkdir("", "Inbox", "../escape").take_wait().await.is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn interrupted_copies_carry_on_where_they_stopped() {
    let dir = scratch("resume");
    let share = dir.join("share");
    let content = bytes(1_000_000);
    std::fs::create_dir_all(&share).unwrap();
    std::fs::write(share.join("movie.bin"), &content).unwrap();
    let (_server, client, _) = pair(machine(&dir.join("s"), &[("Docs", &share, true)]), machine(&dir.join("c"), &[])).await;

    // A download that was cut off after 400 000 bytes. The leftover is marked so it can be told
    // from bytes fetched again.
    let dest = dir.join("got");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join(format!("movie.bin{PART}")), vec![b'X'; 400_000]).unwrap();
    // The job picks a name that is free, and "movie.bin" is not taken, so it resumes that leftover.
    let job = client.download("", "Docs", "", vec!["movie.bin".into()], dest.clone());
    assert_eq!(finish(&job).await, JobState::Done);
    let got = std::fs::read(dest.join("movie.bin")).unwrap();
    assert_eq!(got.len(), content.len());
    assert!(got[..400_000].iter().all(|b| *b == b'X'), "the first part was kept, not fetched again");
    assert_eq!(got[400_000..], content[400_000..]);

    // An upload cut off after 250 000 bytes: the connection just goes away mid-file. What the
    // other side received is marked so it can be told from bytes sent again.
    let src = dir.join("src.bin");
    std::fs::write(&src, &content).unwrap();
    let mut cut = client.open("").await.unwrap();
    cut.send(&FsReq::Write { share: "Docs".into(), path: "src.bin".into(), size: content.len() as u64, exec: false }).await.unwrap();
    cut.send(&FsReq::Chunk(vec![b'Y'; 250_000])).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(cut);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!share.join("src.bin").exists(), "an unfinished file does not appear under its real name");
    let done = AtomicU64::new(0);
    let ctl = session::Ctl { done: &done, base: 0, cancel: &Default::default() };
    let mut again = client.open("").await.unwrap();
    again.write_from("Docs", "src.bin", &src, content.len() as u64, false, &ctl).await.unwrap();
    let got = std::fs::read(share.join("src.bin")).unwrap();
    assert_eq!(got.len(), content.len());
    assert!(got[..250_000].iter().all(|b| *b == b'Y'), "the first part was kept, not sent again");
    assert_eq!(got[250_000..], content[250_000..]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_copy_waits_for_the_link_and_then_finishes() {
    let dir = scratch("wait");
    let share = dir.join("share");
    write(&share.join("a.txt"), "payload");
    let (_server, client, addr) = pair(machine(&dir.join("s"), &[("Docs", &share, false)]), machine(&dir.join("c"), &[])).await;

    client.set_link(None);
    let job = client.download("", "Docs", "", vec!["a.txt".into()], dir.join("got"));
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(matches!(job.state(), JobState::Waiting(_)), "{:?}", job.state());
    client.set_link(Some(addr));
    assert_eq!(finish(&job).await, JobState::Done);
    assert_eq!(std::fs::read_to_string(dir.join("got/a.txt")).unwrap(), "payload");

    // Cancelling a waiting job ends it.
    client.set_link(None);
    let job = client.download("", "Docs", "", vec!["a.txt".into()], dir.join("got2"));
    tokio::time::sleep(Duration::from_millis(300)).await;
    job.cancel();
    assert_eq!(finish(&job).await, JobState::Cancelled);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The server asks a client to dial back, the way it must to browse the client's folders.
#[tokio::test]
async fn the_server_browses_a_client_through_a_dial_back() {
    let dir = scratch("dialback");
    let share = dir.join("share");
    write(&share.join("from-client.txt"), "client data");
    let (server, client, _) = pair(machine(&dir.join("s"), &[]), machine(&dir.join("c"), &[("Laptop", &share, true)])).await;

    // A fake control link for client "c": messages for it land in `rx`.
    let Endpoint::Server(core) = &server.endpoint else { unreachable!() };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let conn = core.lock().unwrap().attach(
        "c",
        Screen::single(1920, 1080),
        ClientSettings { side: Side::Right, speed: 1.0, swap_cmd_ctrl: false },
        tx,
    );
    server.hub.update(|s| {
        s.clients.push(ClientInfo {
            conn,
            name: "c".into(),
            os: Os::Linux,
            addr: "127.0.0.1".parse().unwrap(),
            screen: Screen::single(1920, 1080),
            side: Side::Right,
            since: Instant::now(),
            rtt: Rtt::default(),
        })
    });
    let c2 = client.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Msg::FilesDial { token } = msg {
                c2.on_dial(token);
            }
        }
    });

    let shares = server.shares("c").take_wait().await.unwrap();
    assert_eq!(shares, [Share { name: "Laptop".into(), write: true }]);
    let listing = server.list("c", "Laptop", "").take_wait().await.unwrap();
    assert_eq!(listing.len(), 1);
    let dest = dir.join("got");
    let job = server.download("c", "Laptop", "", vec!["from-client.txt".into()], dest.clone());
    assert_eq!(finish(&job).await, JobState::Done);
    assert_eq!(std::fs::read_to_string(dest.join("from-client.txt")).unwrap(), "client data");
    let job = server.upload("c", "Laptop", "", vec![dest.join("from-client.txt")]);
    assert_eq!(finish(&job).await, JobState::Done);
    assert_eq!(std::fs::read_to_string(share.join("from-client (2).txt")).unwrap(), "client data");

    assert!(server.shares("nobody").take_wait().await.unwrap_err().contains("not connected"));
    std::fs::remove_dir_all(&dir).unwrap();
}

impl<T: Send + 'static> Pending<T> {
    async fn take_wait(self) -> Result<T, String> {
        let start = Instant::now();
        loop {
            if let Some(r) = self.take() {
                return r;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "no answer");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
