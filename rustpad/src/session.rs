//! Session persistence ("hot exit"): open tabs, caret positions and unsaved
//! buffers are written on exit / periodically and restored on the next start.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::doc::{Doc, Eol};
use crate::encoding::Enc;
use crate::lang;
use crate::settings::config_dir;

#[derive(Serialize, Deserialize, Default)]
struct SessionDoc {
    path: Option<String>,
    backup: Option<String>,
    untitled_no: u32,
    caret: usize,
    scroll_row: f32,
    lang: String,
    enc: String,
    eol: String,
    modified: bool,
    bookmarks: Vec<usize>,
}

#[derive(Serialize, Deserialize, Default)]
struct Session {
    docs: Vec<SessionDoc>,
    active: usize,
}

fn session_path() -> PathBuf {
    config_dir().join("session.json")
}

fn backup_dir() -> PathBuf {
    config_dir().join("backup")
}

fn eol_name(e: Eol) -> &'static str {
    match e {
        Eol::Lf => "lf",
        Eol::CrLf => "crlf",
        Eol::Cr => "cr",
    }
}

pub fn save(app: &mut App, wait: bool) {
    let dir = backup_dir();
    let _ = std::fs::create_dir_all(&dir);
    // a previous snapshot is still being written: skip this round
    if let Some(h) = &app.snapshot_thread {
        if !h.is_finished() {
            if wait {
                if let Some(h) = app.snapshot_thread.take() {
                    let _ = h.join();
                }
            } else {
                return;
            }
        }
    }
    let mut session = Session { active: app.active, docs: Vec::new() };
    let mut keep: Vec<String> = Vec::new();
    let mut writes: Vec<(String, ropey::Rope)> = Vec::new();
    for (i, d) in app.tabs.iter_mut().enumerate() {
        let needs_backup = d.is_modified() || (d.path.is_none() && d.len_chars() > 0);
        let mut backup = None;
        if needs_backup {
            let name = format!("doc{}-{}.bak", i, d.id);
            if d.backup_ver != d.version || !dir.join(&name).exists() {
                // ropes clone in O(1); the file is written off the UI thread
                writes.push((name.clone(), d.rope.clone()));
                d.backup_ver = d.version;
            }
            keep.push(name.clone());
            backup = Some(name);
        } else if d.path.is_none() {
            // empty untitled buffers are not worth restoring
            continue;
        }
        session.docs.push(SessionDoc {
            path: d.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            backup,
            untitled_no: d.untitled_no,
            caret: d.caret(),
            scroll_row: d.scroll_row,
            lang: lang::name(d.lang).to_string(),
            enc: d.enc.label(),
            eol: eol_name(d.eol).to_string(),
            modified: d.is_modified(),
            bookmarks: d.bookmarks.iter().copied().collect(),
        });
    }
    session.active = session.active.min(session.docs.len().saturating_sub(1));
    let json = serde_json::to_string(&session).unwrap_or_default();
    let handle = std::thread::spawn(move || {
        use std::io::Write;
        for (name, rope) in writes {
            if let Ok(f) = std::fs::File::create(dir.join(&name)) {
                let mut w = std::io::BufWriter::new(f);
                let _ = rope.write_to(&mut w);
                let _ = w.flush();
            }
        }
        let tmp = config_dir().join("session.json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, session_path());
        }
        // drop stale backups
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !keep.contains(&name) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    });
    if wait {
        let _ = handle.join();
    } else {
        app.snapshot_thread = Some(handle);
    }
}

pub fn restore(app: &mut App) {
    let Ok(text) = std::fs::read_to_string(session_path()) else { return };
    let Ok(session) = serde_json::from_str::<Session>(&text) else { return };
    let dir = backup_dir();
    let mut max_untitled = 0;
    for sd in &session.docs {
        let backup_text = sd.backup.as_ref().and_then(|b| std::fs::read(dir.join(b)).ok()).map(|b| String::from_utf8_lossy(&b).into_owned());
        let doc: Option<Doc> = match (&sd.path, backup_text) {
            (Some(p), Some(text)) => {
                let path = PathBuf::from(p);
                let enc = Enc::from_label(&sd.enc).unwrap_or(Enc::Utf8);
                let eol = Eol::detect(&text).unwrap_or(Eol::Lf);
                let mut d = Doc::from_text(&text, enc, eol);
                d.path = Some(path.clone());
                d.disk_mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
                d.mark_modified();
                Some(d)
            }
            (Some(p), None) => {
                let path = PathBuf::from(p);
                if path.exists() { app.load_file(&path).ok() } else { None }
            }
            (None, Some(text)) => {
                let enc = Enc::from_label(&sd.enc).unwrap_or(Enc::Utf8);
                let eol = Eol::detect(&text).unwrap_or(Eol::Lf);
                let mut d = Doc::from_text(&text, enc, eol);
                d.untitled_no = sd.untitled_no;
                d.mark_modified();
                Some(d)
            }
            (None, None) => None,
        };
        let Some(mut d) = doc else { continue };
        if d.untitled_no > max_untitled {
            max_untitled = d.untitled_no;
        }
        if let Some(l) = lang::find_by_name(&sd.lang) {
            d.lang = l;
        }
        if d.path.is_none() && d.untitled_no == 0 {
            d.untitled_no = max_untitled + 1;
            max_untitled += 1;
        }
        let p = sd.caret.min(d.len_chars());
        d.set_caret(p);
        d.scroll_row = sd.scroll_row;
        d.bookmarks = sd.bookmarks.iter().copied().filter(|l| *l < d.len_lines()).collect();
        app.apply_doc_opts(&mut d);
        app.tabs.push(d);
    }
    app.next_untitled = max_untitled + 1;
    if !app.tabs.is_empty() {
        app.active = session.active.min(app.tabs.len() - 1);
    }
}
