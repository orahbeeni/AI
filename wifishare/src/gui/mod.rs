//! The window (egui), with the engine running in the same process. Where a tray exists,
//! closing the window keeps sharing running and the tray brings the window back.

mod app;
mod files;
mod layout;
#[cfg(test)]
mod tests;
mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::config::{Config, Paths};
use crate::engine::Engine;
use crate::icon;
use crate::instance;
use crate::status::{Approval, Hub, LinkState, Status};

#[derive(Clone, Copy, Debug)]
pub enum Command {
    /// Bring the window up (tray click, or wifishare launched again).
    Show,
    /// Only the tray offers quitting while the window is closed (Linux for now).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Quit,
}

type Wake = Arc<Mutex<Option<egui::Context>>>;

pub fn run(paths: Paths, cfg: Config, background: bool) -> Result<()> {
    if instance::signal_existing(cfg.port) {
        log::info!("wifishare is already running; brought its window forward");
        return Ok(());
    }
    let hub = Hub::new(Approval::Gui);
    let engine = Arc::new(Mutex::new(Engine::new(paths.clone(), hub.clone())));
    if cfg.gui.start_sharing
        && let Some(mode) = cfg.mode
    {
        engine.lock().unwrap().start(mode);
    }

    let (tx, rx) = mpsc::channel();
    let rx = Arc::new(Mutex::new(rx));
    let wake: Wake = Arc::default();
    let waker: Arc<dyn Fn() + Send + Sync> = {
        let wake = wake.clone();
        Arc::new(move || {
            if let Some(ctx) = &*wake.lock().unwrap() {
                ctx.request_repaint();
            }
        })
    };
    let w = waker.clone();
    instance::listen(cfg.port, tx.clone(), Command::Show, move || w());
    let tray = tray::spawn(tx, hub.clone(), waker);

    let keep_running = |paths: &Paths| tray.is_some() && Config::load(paths).map(|c| c.gui.keep_running).unwrap_or(true);
    let mut show = !(background && keep_running(&paths));
    loop {
        if show {
            let quit = window(&paths, &engine, &hub, &rx, &wake, tray.is_some(), background && !keep_running(&paths))?;
            if quit || !keep_running(&paths) {
                break;
            }
        }
        // Window closed: sharing keeps running until the tray (or another launch) says otherwise.
        match rx.lock().unwrap().recv() {
            Ok(Command::Show) => show = true,
            Ok(Command::Quit) | Err(_) => break,
        }
    }
    engine.lock().unwrap().stop();
    Ok(())
}

/// Runs the window until it closes; true if the user chose to quit.
fn window(
    paths: &Paths,
    engine: &Arc<Mutex<Engine>>,
    hub: &Arc<Hub>,
    rx: &Arc<Mutex<mpsc::Receiver<Command>>>,
    wake: &Wake,
    has_tray: bool,
    start_minimized: bool,
) -> Result<bool> {
    let quit = Arc::new(AtomicBool::new(false));
    let size = 128;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("wifishare")
            .with_app_id("wifishare")
            .with_inner_size([880.0, 680.0])
            .with_min_inner_size([620.0, 480.0])
            .with_icon(Arc::new(egui::IconData { rgba: icon::rgba(size), width: size, height: size })),
        run_and_return: true,
        ..Default::default()
    };
    let ctx = app::AppCtx {
        engine: engine.clone(),
        hub: hub.clone(),
        paths: paths.clone(),
        commands: Some(rx.clone()),
        has_tray,
        quit: quit.clone(),
        start_minimized,
    };
    let (hub2, wake2) = (hub.clone(), wake.clone());
    let result = eframe::run_native(
        "wifishare",
        options,
        Box::new(move |cc| {
            let egui_ctx = cc.egui_ctx.clone();
            *wake2.lock().unwrap() = Some(egui_ctx.clone());
            hub2.set_listener(Some(Arc::new(move || egui_ctx.request_repaint())));
            Ok(Box::new(app::App::new(cc, ctx)))
        }),
    );
    hub.set_listener(None);
    *wake.lock().unwrap() = None;
    log::info!("window closed");
    result.map_err(|e| anyhow::anyhow!("could not open the window: {e}"))?;
    Ok(quit.load(Ordering::SeqCst))
}

/// One line describing the state, for the tray.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn summary(st: &Status) -> String {
    if !st.running {
        return "Stopped".into();
    }
    match &st.link {
        LinkState::Connected { server, .. } => format!("Connected to {server}"),
        LinkState::Searching { server, .. } => format!("Looking for {server}…"),
        LinkState::Unpaired => "Not paired".into(),
        LinkState::Idle => match (st.clients.len(), &st.active) {
            (_, Some(active)) => format!("Controlling {active}"),
            (0, _) => "Waiting for computers".into(),
            (1, _) => format!("Sharing with {}", st.clients[0].name),
            (n, _) => format!("Sharing with {n} computers"),
        },
    }
}
