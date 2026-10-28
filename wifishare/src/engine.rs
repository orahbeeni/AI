//! Runs the server or client inside the GUI process, on its own tokio runtime, so it can be
//! stopped and restarted (mode or settings changes) without restarting the app.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::client;
use crate::config::{Config, Identity, Mode, Paths};
use crate::pair;
use crate::server;
use crate::status::{Hub, LinkState};

struct Running {
    rt: tokio::runtime::Runtime,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

pub struct Engine {
    paths: Paths,
    hub: Arc<Hub>,
    run: Option<Running>,
}

impl Engine {
    pub fn new(paths: Paths, hub: Arc<Hub>) -> Engine {
        Engine { paths, hub, run: None }
    }

    pub fn start(&mut self, mode: Mode) {
        self.stop();
        self.hub.reset();
        let fail = |hub: &Hub, e: anyhow::Error| {
            log::error!("{e:#}");
            hub.update(|s| s.error = Some(format!("{e:#}")));
        };
        let cfg = match Config::load(&self.paths) {
            Ok(c) => c,
            Err(e) => return fail(&self.hub, e),
        };
        let id = match Identity::load_or_create(&self.paths) {
            Ok(id) => id,
            Err(e) => return fail(&self.hub, e),
        };
        let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().thread_name("wifishare-net").build() {
            Ok(rt) => rt,
            Err(e) => return fail(&self.hub, e.into()),
        };
        let (tx, rx) = oneshot::channel();
        let (hub, paths) = (self.hub.clone(), self.paths.clone());
        let task = rt.spawn(async move {
            let result = match mode {
                Mode::Server => server::run(cfg, paths, id, server::Options { side: None, no_input: false }, hub.clone(), rx).await,
                Mode::Client => client::run(cfg, paths, id, None, hub.clone(), rx).await,
            };
            if let Err(e) = result {
                log::error!("{e:#}");
                hub.update(|s| s.error = Some(format!("{e:#}")));
            }
            hub.update(|s| s.running = false);
        });
        log::info!("started as {}", if mode == Mode::Server { "server" } else { "client" });
        self.run = Some(Running { rt, stop: Some(tx), task });
    }

    /// Stops sharing, giving the server a moment to hand the cursor back.
    pub fn stop(&mut self) {
        let Some(mut r) = self.run.take() else { return };
        if let Some(tx) = r.stop.take() {
            let _ = tx.send(());
        }
        let task = &mut r.task;
        let _ = r.rt.block_on(async { tokio::time::timeout(Duration::from_secs(2), task).await });
        r.rt.shutdown_timeout(Duration::from_secs(1));
        self.hub.update(|s| {
            s.running = false;
            s.clients.clear();
            s.active = None;
            s.link = LinkState::Idle;
            s.transfers.clear();
        });
        log::info!("stopped");
    }

    /// Applies config changes that need no restart (the server's screen layout).
    pub fn reload(&self) {
        self.hub.reload.notify_waiters();
    }

    /// Pairs with the server at `target` (host or IP); the outcome shows up as a notice.
    pub fn pair(&self, target: String) {
        let (hub, paths) = (self.hub.clone(), self.paths.clone());
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => return hub.update(|s| s.notice = Some((format!("Pairing failed: {e}"), Instant::now()))),
            };
            let result = rt.block_on(async {
                let cfg = Config::load(&paths)?;
                let id = Identity::load_or_create(&paths)?;
                let addr = pair::resolve(&target, cfg.port).await?;
                pair::pair_with(&cfg.name, &paths, &id, addr, &hub).await
            });
            let text = match result {
                Ok(name) => format!("Paired with {name}"),
                Err(e) => format!("Pairing failed: {e:#}"),
            };
            hub.update(|s| s.notice = Some((text, Instant::now())));
        });
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}
