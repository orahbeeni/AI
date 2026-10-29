//! wifishare: share one keyboard and mouse (and the clipboard) between computers on a LAN.

// Linux cannot be a server yet, so the input-capture half (switching core, key tables) is only
// exercised by tests there; the macOS and Windows builds use all of it.
#![cfg_attr(target_os = "linux", allow(dead_code))]

mod audio;
mod autostart;
mod client;
mod clipboard;
mod clipnet;
mod config;
mod discovery;
mod engine;
mod files;
mod geom;
mod gui;
mod icon;
mod instance;
mod keymap;
mod logger;
mod netwatch;
mod pair;
mod platform;
mod proto;
mod server;
mod status;
mod wire;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::{Config, Identity, Paths, Peers, Role};
use crate::geom::Side;
use crate::status::{Approval, Hub};

#[derive(Parser)]
#[command(version, about = "Share a keyboard, mouse and clipboard between computers over Wi-Fi/LAN")]
struct Cli {
    /// Keep config, keys and cache in this folder instead of the usual places.
    #[arg(long, global = true, value_name = "DIR")]
    config_dir: Option<PathBuf>,
    /// More logging (-v debug, -vv trace).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Without a command, the window opens.
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the window (the default).
    Gui {
        /// Start without showing the window (in the tray, or minimized).
        #[arg(long)]
        background: bool,
    },
    /// Share this machine's keyboard and mouse (run it where they are plugged in).
    Server {
        /// Edge of this desktop where clients without a layout entry sit.
        #[arg(long)]
        side: Option<Side>,
        /// Only share the clipboard, not the keyboard and mouse.
        #[arg(long)]
        no_input: bool,
    },
    /// Be controlled by the paired server; reconnects on its own forever.
    Client {
        /// Server host name or IP (optional: paired servers are discovered on the LAN).
        server: Option<String>,
    },
    /// Pair this machine with a server. Run it while `wifishare server` runs on the other one.
    Pair {
        /// Server host name or IP; without it, servers on the LAN are listed.
        server: Option<String>,
    },
    /// Shared folders: choose what to share, or copy files to and from the server.
    Files {
        #[command(subcommand)]
        action: files::cli::FilesCmd,
    },
    /// Show this machine's key, paired machines, detected monitors and permission problems.
    Info,
    /// Remove a paired machine.
    Forget { name: String },
    /// Write a commented config.toml to edit.
    Init,
    /// Move the mouse in a small square to check that input injection works here.
    TestInject,
    /// List this binary in the applications menu, and point start-at-login at it if that is on
    /// (used by install_ubuntu.sh).
    #[command(hide = true)]
    InstallLauncher,
}

fn main() {
    let cli = Cli::parse();
    logger::init(cli.verbose);
    if let Err(e) = run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let paths = Paths::new(cli.config_dir)?;
    let cfg = Config::load(&paths)?;
    match cli.cmd.unwrap_or(Cmd::Gui { background: false }) {
        Cmd::Gui { background } => gui::run(paths, cfg, background),
        Cmd::Init => init(&paths),
        Cmd::Info => info(&cfg, &paths),
        Cmd::Forget { name } => forget(&paths, &name),
        Cmd::TestInject => test_inject(&cfg),
        Cmd::InstallLauncher => install_launcher(),
        cmd => {
            let id = Identity::load_or_create(&paths)?;
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build()?;
            rt.block_on(async move {
                let hub = Hub::new(Approval::Terminal);
                let (stop_tx, stop) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    let _ = tokio::signal::ctrl_c().await;
                    let _ = stop_tx.send(());
                });
                match cmd {
                    Cmd::Server { side, no_input } => server::run(cfg, paths, id, server::Options { side, no_input }, hub, stop).await,
                    Cmd::Client { server } => client::run(cfg, paths, id, server, hub, stop).await,
                    Cmd::Pair { server } => pair::run(cfg, paths, id, server).await,
                    Cmd::Files { action } => files::cli::run(cfg, paths, id, action).await,
                    _ => unreachable!(),
                }
            })
        }
    }
}

fn install_launcher() -> Result<()> {
    autostart::install_launcher()?;
    if autostart::is_enabled() {
        autostart::set_enabled(true)?;
    }
    Ok(())
}

fn init(paths: &Paths) -> Result<()> {
    let file = paths.config_file();
    if file.exists() {
        println!("{} already exists; not overwriting it", file.display());
    } else {
        std::fs::write(&file, config::SAMPLE_CONFIG)?;
        println!("wrote {}", file.display());
    }
    Ok(())
}

fn info(cfg: &Config, paths: &Paths) -> Result<()> {
    let id = Identity::load_or_create(paths)?;
    println!("name:        {}", cfg.name);
    println!("key:         {}", id.fingerprint());
    println!("config:      {}", paths.config_file().display());
    println!("addresses:   {}", netwatch::fmt_addrs(&netwatch::current_addrs()));
    match platform::screen() {
        Ok(s) => {
            println!("monitors:    {}", server::describe_screen(&s));
            for m in &s.monitors {
                println!("             {}x{} at {},{}", m.w, m.h, m.x, m.y);
            }
        }
        Err(e) => println!("monitors:    detection failed: {e:#}"),
    }
    let peers = Peers::load(paths)?;
    if peers.peers.is_empty() {
        println!("paired:      nobody yet (run `wifishare pair` on the client)");
    }
    for p in &peers.peers {
        let role = if p.role == Role::Server { "server" } else { "client" };
        let key = hex::decode(&p.key).map(|k| config::fingerprint(&k)).unwrap_or_default();
        let addr = p.last_addr.as_deref().map(|a| format!(", last seen at {a}")).unwrap_or_default();
        println!("paired:      {} ({role}, key {key}{addr})", p.name);
    }
    for (role, server) in [("as a server", true), ("as a client", false)] {
        for problem in platform::permission_problems(server) {
            println!("problem {role}: {problem}");
        }
    }
    Ok(())
}

fn forget(paths: &Paths, name: &str) -> Result<()> {
    let mut peers = Peers::load(paths)?;
    let before = peers.peers.len();
    peers.peers.retain(|p| !p.name.eq_ignore_ascii_case(name));
    if peers.peers.len() == before {
        anyhow::bail!("no paired machine named '{name}'");
    }
    peers.save(paths)?;
    println!("forgot {name}");
    Ok(())
}

fn test_inject(cfg: &Config) -> Result<()> {
    let screen = client::detect_screen(cfg);
    println!("monitors: {}", server::describe_screen(&screen));
    let mut inj = platform::injector(1.0)?;
    inj.set_screen(&screen);
    let (cx, cy) = screen.center();
    println!("moving the pointer around a 200px square centred on {cx},{cy}...");
    for (dx, dy) in [(-100.0, -100.0), (100.0, -100.0), (100.0, 100.0), (-100.0, 100.0), (-100.0, -100.0), (0.0, 0.0)] {
        inj.motion(cx + dx, cy + dy);
        std::thread::sleep(Duration::from_millis(400));
    }
    println!("done. The pointer should have traced a square and ended in the middle of the first monitor.");
    Ok(())
}
