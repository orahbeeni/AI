//! One-time pairing. Both machines show a 6-digit code derived from the Noise handshake; if
//! the codes match, nobody is in the middle, and each side pins the other's key.

use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::sync::{mpsc, watch};

use crate::config::{Config, Identity, Paths, Peers, Role, fingerprint};
use crate::discovery::{self, Beacon};
use crate::netwatch;
use crate::proto::{Hello, Msg, Os, PROTOCOL_VERSION};
use crate::server::Shared;
use crate::status::{Approval, Hub, PairPrompt};
use crate::wire::{self, Purpose, Secured};

const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

pub fn code(hash: &[u8]) -> String {
    let n = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
}

/// Asks a yes/no question on the terminal without blocking the async runtime.
async fn confirm(question: String) -> bool {
    let ask = tokio::task::spawn_blocking(move || {
        print!("{question} [y/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    });
    matches!(tokio::time::timeout(ANSWER_TIMEOUT, ask).await, Ok(Ok(true)))
}

async fn exchange_hellos(s: &mut Secured, name: &str) -> Result<Hello> {
    s.writer.send(&Msg::Hello(Hello { version: PROTOCOL_VERSION, name: name.into(), os: Os::current(), screen: None })).await?;
    match tokio::time::timeout(Duration::from_secs(10), s.reader.recv::<Msg>()).await.context("peer went quiet")?? {
        Msg::Hello(h) => Ok(h),
        other => bail!("unexpected {other:?}"),
    }
}

async fn finish(s: &mut Secured, ours: bool) -> Result<bool> {
    s.writer.send(&Msg::PairConfirm(ours)).await?;
    match tokio::time::timeout(ANSWER_TIMEOUT, s.reader.recv::<Msg>()).await.context("no answer from the other machine")?? {
        Msg::PairConfirm(theirs) => Ok(ours && theirs),
        other => bail!("unexpected {other:?}"),
    }
}

/// Asks the user (dialog or terminal) whether the code matches; false if nobody can answer.
async fn ask(hub: &Hub, prompt: PairPrompt, question: &str) -> bool {
    match hub.approval {
        Approval::Gui => {
            let answer = hub.ask_pair(prompt);
            matches!(tokio::time::timeout(ANSWER_TIMEOUT, answer).await, Ok(Ok(true)))
        }
        Approval::Terminal => {
            if !std::io::stdin().is_terminal() {
                log::warn!("pairing with {} refused: approve it from a terminal or the wifishare window", prompt.peer);
                return false;
            }
            println!("\n  {} '{}' ({:?}, {})", if prompt.incoming { "Pairing request from" } else { "Server" }, prompt.peer, prompt.os, prompt.addr);
            println!("  Code: {}", prompt.code);
            confirm(question.to_string()).await
        }
    }
}

/// Server side of a pairing request.
pub async fn serve(sh: &Shared, mut s: Secured) -> Result<()> {
    let hello = exchange_hellos(&mut s, &sh.name()).await?;
    let _turn = sh.pair_lock.lock().await;
    log::info!("pairing request from {} ({}), key {}", hello.name, s.peer_addr.ip(), fingerprint(&s.remote_key));
    let prompt = PairPrompt { id: 0, peer: hello.name.clone(), os: hello.os, addr: s.peer_addr.ip(), code: code(&s.hash), incoming: true };
    let ok = ask(&sh.hub, prompt, "  The other machine shows a code too. Allow it to connect if they match?").await;
    if finish(&mut s, ok).await? {
        let mut peers = Peers::load(&sh.paths)?;
        peers.pin(&hello.name, &s.remote_key, Role::Client);
        peers.save(&sh.paths)?;
        log::info!("paired with {}", hello.name);
    } else {
        log::info!("pairing with {} cancelled", hello.name);
    }
    Ok(())
}

/// Client side: `wifishare pair [server]`.
pub async fn run(cfg: Config, paths: Paths, id: Identity, target: Option<String>) -> Result<()> {
    let addr = match target {
        Some(t) => resolve(&t, cfg.port).await?,
        None => discover(cfg.port).await?,
    };
    println!("Connecting to {addr}...");
    let name = pair_with(&cfg.name, &paths, &id, addr, &Hub::new(Approval::Terminal)).await?;
    println!("\nPaired with '{name}'. Start the client with: wifishare client");
    Ok(())
}

/// Pairs this machine, as a client, with the server at `addr`. Returns the server's name.
pub async fn pair_with(my_name: &str, paths: &Paths, id: &Identity, addr: SocketAddr, hub: &Hub) -> Result<String> {
    let mut s = wire::connect(addr, id, Purpose::Pair, Duration::from_secs(5)).await.context("is wifishare running as a server there?")?;
    let hello = exchange_hellos(&mut s, my_name).await?;
    log::info!("pairing with {} ({addr}), key {}", hello.name, fingerprint(&s.remote_key));
    let prompt = PairPrompt { id: 0, peer: hello.name.clone(), os: hello.os, addr: addr.ip(), code: code(&s.hash), incoming: false };
    let ok = ask(hub, prompt, "  The server shows a code as well. Is it the same?").await;
    if hub.approval == Approval::Terminal && ok {
        println!("  Waiting for the server to approve...");
    }
    if !finish(&mut s, ok).await? {
        bail!("pairing was not approved on both machines");
    }
    let mut peers = Peers::load(paths)?;
    peers.pin(&hello.name, &s.remote_key, Role::Server);
    if let Some(p) = peers.peers.last_mut() {
        p.last_addr = Some(addr.to_string());
    }
    peers.save(paths)?;
    log::info!("paired with {}", hello.name);
    Ok(hello.name)
}

pub async fn resolve(target: &str, port: u16) -> Result<SocketAddr> {
    let with_port = if target.contains(':') { target.to_string() } else { format!("{target}:{port}") };
    tokio::net::lookup_host(&with_port)
        .await
        .with_context(|| format!("resolving {with_port}"))?
        .find(|a| a.is_ipv4())
        .with_context(|| format!("{with_port} has no IPv4 address"))
}

/// Listens for server beacons for a few seconds and lets the user pick one.
async fn discover(port: u16) -> Result<SocketAddr> {
    println!("Looking for wifishare servers on this network...");
    let (tx, mut rx) = mpsc::unbounded_channel::<(Beacon, SocketAddr)>();
    let (_probing_tx, probing_rx) = watch::channel(true);
    let task = tokio::spawn(discovery::listen(port, tx, probing_rx, netwatch::spawn()));
    let mut found: HashMap<Vec<u8>, (Beacon, SocketAddr)> = HashMap::new();
    let until = Instant::now() + Duration::from_secs(3);
    while let Ok(Some((b, addr))) = tokio::time::timeout_at(until.into(), rx.recv()).await {
        found.entry(b.key.clone()).or_insert((b, addr));
    }
    task.abort();
    let mut list: Vec<_> = found.into_values().collect();
    list.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    match list.len() {
        0 => bail!("no server answered. Is `wifishare server` running? You can also pass its address: wifishare pair 192.168.1.25"),
        1 => {
            println!("Found '{}' at {}", list[0].0.name, list[0].1.ip());
            Ok(list[0].1)
        }
        _ => {
            for (i, (b, a)) in list.iter().enumerate() {
                println!("  {}) {} at {}", i + 1, b.name, a.ip());
            }
            let pick = tokio::task::spawn_blocking(|| {
                print!("Which one? ");
                let _ = std::io::stdout().flush();
                let mut line = String::new();
                let _ = std::io::stdin().lock().read_line(&mut line);
                line.trim().parse::<usize>().ok()
            })
            .await?;
            let i = pick.filter(|i| (1..=list.len()).contains(i)).context("no server chosen")?;
            Ok(list[i - 1].1)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::server;

    fn temp_paths(tag: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("wifishare-pair-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Paths::new(Some(dir)).unwrap()
    }

    /// Both sides approve through dialogs (the hub), as the windows do.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dialog_approval_pairs_both_sides() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (sp, cp) = (temp_paths("server"), temp_paths("client"));
        let mut cfg = Config { name: "the-server".into(), port, ..Config::default() };
        cfg.clipboard.enabled = false;
        let (server_hub, client_hub) = (Hub::new(Approval::Gui), Hub::new(Approval::Gui));
        let (stop_tx, stop) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(server::run(
            cfg,
            sp.clone(),
            Identity::load_or_create(&sp).unwrap(),
            server::Options { side: None, no_input: true },
            server_hub.clone(),
            stop,
        ));
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Answer every dialog with "yes", remembering the codes each side showed.
        let codes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let answerer = {
            let (s, c, codes) = (server_hub.clone(), client_hub.clone(), codes.clone());
            tokio::spawn(async move {
                loop {
                    for hub in [&s, &c] {
                        for p in hub.pair_prompts() {
                            codes.lock().unwrap().push((p.incoming, p.code.clone()));
                            hub.answer_pair(p.id, true);
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
        };
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let name = pair_with("the-client", &cp, &Identity::load_or_create(&cp).unwrap(), addr, &client_hub).await.unwrap();
        answerer.abort();
        assert_eq!(name, "the-server");
        let codes = codes.lock().unwrap().clone();
        assert_eq!(codes.len(), 2, "one dialog on each side: {codes:?}");
        assert_eq!(codes[0].1, codes[1].1, "both sides showed the same code");
        assert!(Peers::load(&cp).unwrap().servers().any(|p| p.name == "the-server"));
        // The server pins the client right after the exchange.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(Peers::load(&sp).unwrap().peers.iter().any(|p| p.name == "the-client" && p.role == Role::Client));
        let _ = stop_tx.send(());
        server.await.unwrap().unwrap();
    }
}
