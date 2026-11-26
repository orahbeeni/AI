//! Organise desktop app: Tauri commands over the core crate, plus a background thread that keeps links
//! pointing at their files (watcher events → engine → UI events).

use organise_core::backup;
use organise_core::engine::{Engine, Outcome};
use organise_core::links::{self, Link, LinkKind, LinkStatus};
use organise_core::model::Doc;
use organise_core::scan::{self, Found, Target};
use organise_core::store::{MoveRecord, Store};
use organise_core::tracker::{FsEvent, Targets, Tracker, TrackerStats};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

enum Msg {
    Fs(FsEvent),
    Retarget,
    Verify,
    Found(HashMap<String, Found>),
}

struct AppState {
    store: Arc<Mutex<Store>>,
    tx: Mutex<Sender<Msg>>,
    stats: Arc<Mutex<TrackerStats>>,
    selftest_file: Option<PathBuf>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LoadResult {
    doc: Doc,
    links: Vec<Link>,
    moves: Vec<MoveRecord>,
    data_dir: String,
    host: String,
    home: String,
    start_view: Option<String>,
    selftest_file: Option<String>,
    /** test hook: ORGANISE_ITEM_DRAG=system|relay picks how items are dragged out of card windows */
    item_drag: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LinksUpdated {
    links: Vec<Link>,
    moves: Vec<MoveRecord>,
}

/// A window saved the document; the other windows (cards popped out of the main window) show it too.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DocChanged {
    doc: Doc,
    from: String,
}

fn several_windows(app: &AppHandle) -> bool {
    app.webview_windows().len() > 1
}

/// New or changed links made in one window are known to the others straight away.
fn share_links(app: &AppHandle, links: Vec<Link>) {
    if several_windows(app) {
        let _ = app.emit("links-updated", LinksUpdated { links, moves: vec![] });
    }
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn expand(p: &str) -> PathBuf {
    if p == "~" {
        home()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(p)
    }
}

/// Folders the watcher should cover: every folder holding a linked file or folder, plus the chosen ones.
fn targets_for(st: &Store) -> Targets {
    let mut dirs: Vec<PathBuf> = vec![];
    let mut seen = HashSet::new();
    for l in st.links.values().filter(|l| l.is_fs() && l.status == LinkStatus::Ok) {
        if let Some(parent) = l.path.as_deref().map(Path::new).and_then(Path::parent) {
            if seen.insert(parent.to_path_buf()) {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    Targets { dirs, roots: st.doc.settings.watched.iter().map(|w| expand(w)).collect(), skip_hidden: st.doc.settings.skip_hidden }
}

/// Where to look for missing files: home, chosen folders elsewhere, and removable drives.
fn search_roots(st: &Store) -> Vec<PathBuf> {
    let h = home();
    let mut roots = vec![h.clone()];
    for w in &st.doc.settings.watched {
        let p = expand(w);
        if !p.starts_with(&h) {
            roots.push(p);
        }
    }
    let user = std::env::var("USER").unwrap_or_default();
    for p in [format!("/media/{user}"), format!("/run/media/{user}"), "/mnt".into()] {
        let p = PathBuf::from(p);
        if p.is_dir() {
            roots.push(p);
        }
    }
    roots
}

fn engine_thread(app: AppHandle, store: Arc<Mutex<Store>>, rx: Receiver<Msg>, tx: Sender<Msg>, tracker: Tracker) {
    let mut engine = Engine::default();
    let mut queue: Vec<String> = vec![];
    let mut running = false;
    loop {
        let mut out = Outcome::default();
        let mut retarget = false;
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Msg::Fs(ev)) => {
                out = engine.on_event(&ev, &mut store.lock().unwrap().links);
                if organise_core::tracker::debug_on() {
                    eprintln!("engine: {ev:?} -> {out:?}");
                }
            }
            Ok(Msg::Retarget) => retarget = true,
            Ok(Msg::Verify) => {
                out = engine.verify(&mut store.lock().unwrap().links);
                retarget = true;
            }
            Ok(Msg::Found(found)) => {
                running = false;
                out = engine.apply_found(found, &mut store.lock().unwrap().links);
                let _ = app.emit("search-state", false);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if engine.has_pending() {
            let o = engine.tick(&mut store.lock().unwrap().links);
            if organise_core::tracker::debug_on() && !o.is_empty() {
                eprintln!("engine: tick -> {o:?}");
            }
            out.merge(o);
        }
        for id in out.search.drain(..) {
            if !queue.contains(&id) {
                queue.push(id);
            }
        }
        if !out.changed.is_empty() || !out.moves.is_empty() {
            let mut st = store.lock().unwrap();
            retarget |= !out.moves.is_empty();
            for m in out.moves.iter().cloned() {
                st.record_move(m);
            }
            if let Err(e) = st.save_links() {
                eprintln!("organise: saving links: {e}");
            }
            let changed: Vec<Link> = out.changed.iter().filter_map(|id| st.links.get(id).cloned()).collect();
            let _ = app.emit("links-updated", LinksUpdated { links: changed, moves: out.moves.clone() });
        }
        if retarget {
            tracker.set_targets(targets_for(&store.lock().unwrap()));
        }
        if !running && !queue.is_empty() {
            let st = store.lock().unwrap();
            let targets: Vec<Target> = queue.drain(..).filter_map(|id| st.links.get(&id)).filter(|l| l.status != LinkStatus::Ok).map(Target::from_link).collect();
            if !targets.is_empty() && st.doc.settings.auto_search {
                running = true;
                let roots = search_roots(&st);
                let skip_hidden = st.doc.settings.skip_hidden;
                let tx = tx.clone();
                let _ = app.emit("search-state", true);
                std::thread::spawn(move || {
                    let found = scan::search(&targets, &roots, skip_hidden, &AtomicBool::new(false));
                    let _ = tx.send(Msg::Found(found));
                });
            } else if !targets.is_empty() {
                // not searching automatically: mark as missing straight away
                let found = targets.iter().map(|t| (t.id.clone(), Found::NotFound)).collect();
                let _ = tx.send(Msg::Found(found));
            }
        }
    }
}

fn send(state: &State<AppState>, m: Msg) {
    let _ = state.tx.lock().unwrap().send(m);
}

fn load_result(st: &Store, selftest: &Option<PathBuf>) -> LoadResult {
    LoadResult {
        doc: st.doc.clone(),
        links: st.links.values().cloned().collect(),
        moves: st.moves.clone(),
        data_dir: st.dir.display().to_string(),
        host: st.host.clone(),
        home: home().display().to_string(),
        start_view: std::env::var("ORGANISE_VIEW").ok(),
        selftest_file: selftest.as_ref().map(|p| p.display().to_string()),
        item_drag: std::env::var("ORGANISE_ITEM_DRAG").ok(),
    }
}

#[tauri::command]
fn load(state: State<AppState>) -> LoadResult {
    load_result(&state.store.lock().unwrap(), &state.selftest_file)
}

#[tauri::command]
fn save_doc(app: AppHandle, window: tauri::Window, state: State<AppState>, doc: Doc) -> Result<(), String> {
    let shared = several_windows(&app).then(|| DocChanged { doc: doc.clone(), from: window.label().to_string() });
    let mut st = state.store.lock().unwrap();
    let watched_changed = st.doc.settings != doc.settings;
    st.doc = doc;
    st.save_doc().map_err(err)?;
    if st.gc_links() {
        st.save_links().map_err(err)?;
        drop(st);
        send(&state, Msg::Retarget);
    } else if watched_changed {
        drop(st);
        send(&state, Msg::Retarget);
    }
    if let Some(d) = shared {
        let _ = app.emit("doc-changed", d);
    }
    Ok(())
}

#[tauri::command]
fn add_paths(app: AppHandle, state: State<AppState>, paths: Vec<String>) -> Result<Vec<Link>, String> {
    let mut st = state.store.lock().unwrap();
    let mut out = vec![];
    for p in paths {
        let p = p.strip_prefix("file://").map(str::to_string).unwrap_or(p);
        let abs = std::path::absolute(&p).map_err(err)?.display().to_string();
        if let Some(l) = st.link_for_path(&abs).cloned() {
            out.push(l);
            continue;
        }
        let l = Link::from_path(Path::new(&abs)).map_err(|e| format!("{p}: {e}"))?;
        out.push(st.add_link(l));
    }
    st.save_links().map_err(err)?;
    drop(st);
    send(&state, Msg::Retarget);
    share_links(&app, out.clone());
    Ok(out)
}

#[tauri::command]
fn add_url(app: AppHandle, state: State<AppState>, url: String, title: Option<String>) -> Result<Link, String> {
    let mut st = state.store.lock().unwrap();
    let l = st.add_link(Link::url(url.trim(), title.as_deref()));
    st.save_links().map_err(err)?;
    drop(st);
    share_links(&app, vec![l.clone()]);
    Ok(l)
}

fn link_of(state: &State<AppState>, id: &str) -> Result<Link, String> {
    state.store.lock().unwrap().links.get(id).cloned().ok_or_else(|| "That link no longer exists".to_string())
}

#[tauri::command]
fn open_link(app: AppHandle, state: State<AppState>, id: String) -> Result<(), String> {
    let l = link_of(&state, &id)?;
    if l.kind == LinkKind::Url {
        return app.opener().open_url(l.url.unwrap_or_default(), None::<&str>).map_err(err);
    }
    let path = l.path.filter(|p| Path::new(p).exists()).ok_or_else(|| format!("{} can't be found", l.name))?;
    app.opener().open_path(path, None::<&str>).map_err(err)
}

#[tauri::command]
fn reveal_link(app: AppHandle, state: State<AppState>, id: String) -> Result<(), String> {
    let l = link_of(&state, &id)?;
    let path = l.path.filter(|p| Path::new(p).exists()).ok_or_else(|| format!("{} can't be found", l.name))?;
    app.opener().reveal_item_in_dir(path).map_err(err)
}

#[tauri::command]
fn relink(app: AppHandle, state: State<AppState>, id: String, path: String) -> Result<Link, String> {
    let mut st = state.store.lock().unwrap();
    let new = PathBuf::from(&path);
    let info = links::stat(&new).map_err(|e| format!("{path}: {e}"))?;
    let l = st.links.get_mut(&id).ok_or("That link no longer exists")?;
    let rec = MoveRecord {
        link_id: id.clone(),
        name: links::file_name(&new),
        from: l.path.clone().unwrap_or_default(),
        to: path.clone(),
        at: links::now_secs(),
        how: "you".into(),
    };
    l.refresh_from(&new, &info);
    let l = l.clone();
    st.record_move(rec);
    st.save_links().map_err(err)?;
    drop(st);
    send(&state, Msg::Retarget);
    share_links(&app, vec![l.clone()]);
    Ok(l)
}

#[tauri::command]
fn search_again(state: State<AppState>) {
    send(&state, Msg::Verify);
}

#[tauri::command]
fn tracker_stats(state: State<AppState>) -> TrackerStats {
    state.stats.lock().unwrap().clone()
}

#[tauri::command]
fn export_backup(state: State<AppState>, dest: String) -> Result<usize, String> {
    let dir = state.store.lock().unwrap().dir.clone();
    backup::export(&dir, Path::new(&dest)).map_err(err)
}

#[tauri::command]
fn restore_backup(app: AppHandle, window: tauri::Window, state: State<AppState>, src: String) -> Result<LoadResult, String> {
    let mut st = state.store.lock().unwrap();
    backup::restore(&st.dir.clone(), Path::new(&src)).map_err(err)?;
    st.load().map_err(err)?;
    let r = load_result(&st, &state.selftest_file);
    drop(st);
    send(&state, Msg::Verify);
    if several_windows(&app) {
        let _ = app.emit("doc-changed", DocChanged { doc: r.doc.clone(), from: window.label().to_string() });
        share_links(&app, r.links.clone());
    }
    Ok(r)
}

/// Moves a window (logical pixels) so it's fully on the screen that holds its top middle, or the main screen.
fn fit_on_screen(app: &AppHandle, x: f64, y: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
    let area = |m: &tauri::Monitor| {
        let s = m.scale_factor();
        let a = m.work_area();
        (a.position.x as f64 / s, a.position.y as f64 / s, a.size.width as f64 / s, a.size.height as f64 / s)
    };
    let (cx, cy) = (x + w / 2.0, y + 20.0);
    let pick = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(area)
        .find(|&(ax, ay, aw, ah)| cx >= ax && cx < ax + aw && cy >= ay && cy < ay + ah)
        .or_else(|| app.primary_monitor().ok().flatten().as_ref().map(area));
    match pick {
        Some((ax, ay, aw, ah)) => {
            let (w, h) = (w.min(aw), h.min(ah));
            (x.min(ax + aw - w).max(ax), y.min(ay + ah - h).max(ay), w, h)
        }
        None => (x.max(0.0), y.max(0.0), w, h),
    }
}

/// Opens a board card in its own window (or brings that window forward if it's already open).
/// `x`/`y` place it on screen where the pop-up was dropped; Wayland ignores them and places it itself.
#[tauri::command]
async fn open_card_window(app: AppHandle, id: String, title: String, width: f64, height: f64, x: Option<f64>, y: Option<f64>) -> Result<(), String> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("not a card id".into());
    }
    let label = format!("card-{id}");
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.unminimize();
        return w.set_focus().map_err(err);
    }
    let title = if title.trim().is_empty() { "Untitled".to_string() } else { title };
    let (mut w, mut h) = (width.clamp(420.0, 1400.0), height.clamp(320.0, 1100.0));
    let mut b = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html".into()))
        .title(title)
        .min_inner_size(380.0, 260.0)
        .initialization_script(format!("window.__ORGANISE_CARD__ = {};", serde_json::to_string(&id).map_err(err)?));
    if let (Some(x), Some(y)) = (x, y) {
        let (fx, fy, fw, fh) = fit_on_screen(&app, x, y, w, h);
        (w, h) = (fw, fh);
        b = b.position(fx, fy);
    }
    b = b.inner_size(w, h);
    b.build().map_err(err)?;
    Ok(())
}

#[derive(Serialize, Clone)]
struct CardAction {
    action: String,
    id: String,
}

#[derive(Serialize, Clone)]
struct ItemDrag {
    id: String,
    phase: String,
    x: f64,
    y: f64,
}

/// From a card window (Windows, macOS): an item is dragged outside it. `x`/`y` are the pointer's screen position
/// in CSS pixels; the main window gets them as a position inside itself, to show where the item would land
/// ("over"), stop showing it ("leave") or put it there ("drop").
#[tauri::command]
fn relay_drag(app: AppHandle, id: String, phase: String, x: f64, y: f64) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("The main window is closed")?;
    let scale = main.scale_factor().map_err(err)?;
    let at = main.inner_position().map_err(err)?;
    let (x, y) = (x - at.x as f64 / scale, y - at.y as f64 / scale);
    app.emit_to("main", "item-drag", ItemDrag { id, phase, x, y }).map_err(err)
}

/// From a card window: "full" makes the card the main window's view, "dock" puts it back as a pop-up there.
#[tauri::command]
fn card_to_main(app: AppHandle, action: String, id: String) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("The main window is closed")?;
    app.emit_to("main", "card-action", CardAction { action, id }).map_err(err)?;
    let _ = main.unminimize();
    let _ = main.set_focus();
    Ok(())
}

#[tauri::command]
fn open_data_dir(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let dir = state.store.lock().unwrap().dir.display().to_string();
    app.opener().open_path(dir, None::<&str>).map_err(err)
}

/// Test support (only with ORGANISE_SELFTEST=1): read a list file, move a file, report and quit.
#[tauri::command]
fn selftest_read_list(state: State<AppState>, title_slug: String) -> Result<String, String> {
    if state.selftest_file.is_none() {
        return Err("not in self-test mode".into());
    }
    let dir = state.store.lock().unwrap().dir.join("lists").join(format!("{title_slug}.md"));
    std::fs::read_to_string(&dir).map_err(|e| format!("{}: {e}", dir.display()))
}

#[tauri::command]
fn selftest_move(state: State<AppState>, from: String, to: String) -> Result<(), String> {
    if state.selftest_file.is_none() {
        return Err("not in self-test mode".into());
    }
    if let Some(p) = Path::new(&to).parent() {
        std::fs::create_dir_all(p).map_err(err)?;
    }
    std::fs::rename(&from, &to).map_err(err)
}

#[tauri::command]
fn selftest_windows(app: AppHandle) -> Vec<String> {
    let mut v: Vec<String> = app.webview_windows().into_keys().collect();
    v.sort();
    v
}

#[tauri::command]
fn selftest_done(app: AppHandle, report: String, ok: bool) {
    println!("{report}");
    println!("SELFTEST {}", if ok { "PASSED" } else { "FAILED" });
    app.exit(if ok { 0 } else { 1 });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // avoids a blank window with some GPU drivers
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    let data_dir = std::env::var("ORGANISE_DIR").map(PathBuf::from).unwrap_or_else(|_| home().join("Organise"));
    let store = match Store::open(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("organise: can't open {}: {e:#}", data_dir.display());
            std::process::exit(1);
        }
    };
    let selftest_file = std::env::var("ORGANISE_SELFTEST").ok().map(|_| {
        let p = data_dir.parent().unwrap_or(&data_dir).join("selftest-area").join("Quarterly report.txt");
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(&p, "self-test file");
        p
    });
    let store = Arc::new(Mutex::new(store));
    let (tx, rx) = channel::<Msg>();
    let ev_tx = tx.clone();
    let tracker = Tracker::start(move |e| {
        let _ = ev_tx.send(Msg::Fs(e));
    });
    let stats = tracker.stats.clone();
    let state = AppState { store: store.clone(), tx: Mutex::new(tx.clone()), stats, selftest_file };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(state)
        // closing the main window quits, card windows included
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let engine_tx = tx.clone();
            std::thread::Builder::new()
                .name("organise-engine".into())
                .spawn(move || engine_thread(handle, store, rx, engine_tx, tracker))?;
            // check every link against the disk once at startup
            let _ = tx.send(Msg::Verify);
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title("Organise");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load,
            save_doc,
            add_paths,
            add_url,
            open_link,
            reveal_link,
            relink,
            search_again,
            tracker_stats,
            export_backup,
            restore_backup,
            open_data_dir,
            open_card_window,
            card_to_main,
            relay_drag,
            selftest_read_list,
            selftest_move,
            selftest_windows,
            selftest_done
        ])
        .run(tauri::generate_context!())
        .expect("error while running Organise");
}
