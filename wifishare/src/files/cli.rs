//! `wifishare files ...`: the same shared folders from the command line.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use super::{Files, JobState, join, split};
use crate::clipnet::human;
use crate::config::{Config, Identity, Paths, ShareCfg};
use crate::status::Hub;

#[derive(Subcommand)]
pub enum FilesCmd {
    /// Let the other computer browse a folder of this one.
    Share {
        path: PathBuf,
        /// The name the other computer sees (default: the folder's name).
        #[arg(long)]
        name: Option<String>,
        /// Also let it add files and folders (it can never overwrite or delete).
        #[arg(long)]
        write: bool,
    },
    /// Stop sharing a folder.
    Unshare { name: String },
    /// Client: list the server's shared folders, or the contents of one (`share/sub/folder`).
    Ls { path: Option<String> },
    /// Client: copy a file or folder (`share/sub/item`) from the server to here.
    Get {
        path: String,
        /// Where to put it (default: the current folder).
        dest: Option<PathBuf>,
    },
    /// Client: copy local files and folders into a shared folder of the server (`share/sub`).
    Put {
        to: String,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

pub async fn run(cfg: Config, paths: Paths, id: Identity, action: FilesCmd) -> Result<()> {
    match action {
        FilesCmd::Share { path, name, write } => share(cfg, &paths, path, name, write),
        FilesCmd::Unshare { name } => {
            let mut cfg = cfg;
            let before = cfg.files.shares.len();
            cfg.files.shares.retain(|s| s.name != name);
            if cfg.files.shares.len() == before {
                bail!("no shared folder named \"{name}\"");
            }
            cfg.save(&paths)?;
            println!("no longer sharing \"{name}\"");
            Ok(())
        }
        remote => {
            let (files, server) = connect(&cfg, &paths, id).await?;
            match remote {
                FilesCmd::Ls { path } => ls(&files, &server, path.as_deref().unwrap_or("")).await,
                FilesCmd::Get { path, dest } => {
                    let (share, rest) = path
                        .trim_matches('/')
                        .split_once('/')
                        .context("name a file or folder inside a share, like Documents/report.pdf")?;
                    let (dir, name) = split(rest);
                    let dest = dest.unwrap_or_else(|| PathBuf::from("."));
                    wait(files.download(&server, share, dir, vec![name.to_string()], dest)).await
                }
                FilesCmd::Put { to, files: sources } => {
                    let (share, dir) = to.trim_matches('/').split_once('/').unwrap_or((to.trim_matches('/'), ""));
                    wait(files.upload(&server, share, dir, sources)).await
                }
                _ => unreachable!(),
            }
        }
    }
}

fn share(mut cfg: Config, paths: &Paths, path: PathBuf, name: Option<String>, write: bool) -> Result<()> {
    let path = std::fs::canonicalize(&path).with_context(|| format!("{} does not exist", path.display()))?;
    if !path.is_dir() {
        bail!("{} is not a folder", path.display());
    }
    let name = name.unwrap_or_else(|| path.file_name().map_or_else(|| "files".into(), |n| n.to_string_lossy().into_owned()));
    if cfg.files.shares.iter().any(|s| s.name == name) {
        bail!("a folder is already shared as \"{name}\"; pick another name with --name");
    }
    println!("sharing {} as \"{name}\" ({})", path.display(), if write { "the other computer may add files" } else { "read-only" });
    cfg.files.shares.push(ShareCfg { name, path, write });
    cfg.save(paths)
}

/// A client's view: its paired server, found by trying a files connection.
async fn connect(cfg: &Config, paths: &Paths, id: Identity) -> Result<(std::sync::Arc<Files>, String)> {
    let (server, addr) = crate::client::locate(cfg, paths, &id).await?;
    let key = hex::decode(&server.key)?;
    let files = Files::client(paths.clone(), id, Hub::new(crate::status::Approval::Terminal), key);
    files.set_link(Some(addr));
    Ok((files, server.name))
}

async fn ls(files: &std::sync::Arc<Files>, server: &str, path: &str) -> Result<()> {
    let path = path.trim_matches('/');
    let task = async {
        if path.is_empty() {
            for s in files.open(server).await?.shares().await? {
                println!("{}{}", s.name, if s.write { "   (you may add files)" } else { "" });
            }
        } else {
            let (share, rest) = path.split_once('/').unwrap_or((path, ""));
            for e in files.open(server).await?.list(share, rest).await? {
                println!(
                    "{:>10}  {}{}",
                    if e.dir { "folder".into() } else { human(e.size) },
                    join("", &e.name),
                    if e.dir { "/" } else { "" }
                );
            }
        }
        Ok(())
    };
    tokio::time::timeout(Duration::from_secs(20), task).await.context("timed out")?
}

async fn wait(job: std::sync::Arc<super::Job>) -> Result<()> {
    let mut shown = String::new();
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let line = match job.state() {
            JobState::Done => {
                println!("done: {} ({})", job.what, human(job.total.load(std::sync::atomic::Ordering::Relaxed)));
                return Ok(());
            }
            JobState::Failed(why) => bail!("{why}"),
            JobState::Cancelled => bail!("cancelled"),
            JobState::Waiting(why) => format!("waiting: {why}"),
            JobState::Running => {
                let (done, total) =
                    (job.done.load(std::sync::atomic::Ordering::Relaxed), job.total.load(std::sync::atomic::Ordering::Relaxed));
                format!("{} of {}", human(done), human(total))
            }
        };
        // A new line only when the kind of status changes, so logs stay short.
        let kind = line.split(':').next().unwrap_or("").chars().take(7).collect::<String>();
        if kind != shown {
            eprintln!("{line}");
            shown = kind;
        }
    }
}
