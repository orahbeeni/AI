//! Persistent user settings, stored as JSON in the platform config directory.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cmd::Cmd;
use crate::search::Query;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedMacro {
    pub name: String,
    pub steps: Vec<Cmd>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunEntry {
    pub name: String,
    pub command: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub font_size: f32,
    pub font_name: String,
    pub tab_width: usize,
    pub use_spaces: bool,
    pub auto_indent: bool,
    pub auto_close: bool,
    pub auto_close_tags: bool,
    pub line_numbers: bool,
    pub bookmark_margin: bool,
    pub fold_margin: bool,
    pub show_whitespace: bool,
    pub show_eol: bool,
    pub indent_guides: bool,
    pub word_wrap: bool,
    pub highlight_current_line: bool,
    pub smart_highlight: bool,
    pub match_brackets: bool,
    pub scroll_beyond_last_line: bool,
    pub show_edge: bool,
    pub edge_column: usize,
    pub caret_blink: bool,
    pub status_bar: bool,
    pub tool_bar: bool,
    pub tab_bar: bool,
    pub doc_map: bool,
    pub function_list: bool,
    pub workspace: bool,
    pub workspace_roots: Vec<String>,
    pub default_eol: String,
    pub default_new_encoding: String,
    pub restore_session: bool,
    pub autosave_secs: u64,
    pub watch_files: bool,
    pub trim_on_save: bool,
    pub final_newline_on_save: bool,
    pub hide_vault_menu: bool,
    pub zoom: f32,
    pub recent_files: Vec<String>,
    pub max_recent: usize,
    pub find: Query,
    pub find_history: Vec<String>,
    pub replace_history: Vec<String>,
    pub find_filters: String,
    pub find_dir: String,
    pub find_recursive: bool,
    pub find_hidden: bool,
    pub macros: Vec<SavedMacro>,
    pub run_commands: Vec<RunEntry>,
    pub window_size: Option<[f32; 2]>,
    pub window_pos: Option<[f32; 2]>,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "Default".into(),
            font_size: 14.0,
            font_name: String::new(),
            tab_width: 4,
            use_spaces: false,
            auto_indent: true,
            auto_close: false,
            auto_close_tags: false,
            line_numbers: true,
            bookmark_margin: true,
            fold_margin: true,
            show_whitespace: false,
            show_eol: false,
            indent_guides: true,
            word_wrap: false,
            highlight_current_line: true,
            smart_highlight: true,
            match_brackets: true,
            scroll_beyond_last_line: true,
            show_edge: false,
            edge_column: 80,
            caret_blink: true,
            status_bar: true,
            tool_bar: true,
            tab_bar: true,
            doc_map: false,
            function_list: false,
            workspace: false,
            workspace_roots: Vec::new(),
            default_eol: if cfg!(windows) { "crlf".into() } else { "lf".into() },
            default_new_encoding: "UTF-8".into(),
            restore_session: true,
            autosave_secs: 7,
            watch_files: true,
            trim_on_save: false,
            final_newline_on_save: false,
            hide_vault_menu: false,
            zoom: 0.0,
            recent_files: Vec::new(),
            max_recent: 15,
            find: Query::default(),
            find_history: Vec::new(),
            replace_history: Vec::new(),
            find_filters: "*.*".into(),
            find_dir: String::new(),
            find_recursive: true,
            find_hidden: false,
            macros: Vec::new(),
            run_commands: Vec::new(),
            window_size: None,
            window_pos: None,
            maximized: false,
        }
    }
}

pub fn config_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("rustpad")
}

impl Settings {
    fn path() -> PathBuf {
        config_dir().join("config.json")
    }

    pub fn load() -> Settings {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let dir = config_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(text) = serde_json::to_string_pretty(self) {
            // write-then-rename so a crash can't leave a half-written config
            let tmp = dir.join("config.json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, Self::path());
            }
        }
    }

    pub fn add_recent(&mut self, path: &std::path::Path) {
        let s = path.to_string_lossy().into_owned();
        self.recent_files.retain(|p| *p != s);
        self.recent_files.insert(0, s);
        self.recent_files.truncate(self.max_recent.max(1));
    }

    pub fn push_history(list: &mut Vec<String>, item: &str) {
        if item.is_empty() {
            return;
        }
        list.retain(|x| x != item);
        list.insert(0, item.to_string());
        list.truncate(30);
    }
}
