//! Saved layouts: tabs, their split trees, and each terminal's folder and commands.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::{config_dir, layouts_dir};
use crate::layout::Node;
use crate::pane::SavedCommand;

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct PaneSpec {
    pub cwd: Option<String>,
    pub name: Option<String>,
    pub commands: Vec<SavedCommand>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TabSpec {
    #[serde(default)]
    pub title: Option<String>,
    pub root: Node<PaneSpec>,
    /// Index (in leaf order) of the terminal that had focus.
    #[serde(default)]
    pub focused: usize,
    /// Name of the saved layout this tab belongs to, shown in the tab when enabled.
    #[serde(default)]
    pub layout: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct LayoutFile {
    pub name: String,
    pub tabs: Vec<TabSpec>,
    pub active_tab: usize,
    pub window_size: Option<[f32; 2]>,
}

fn file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.') { c } else { '_' }).collect();
    format!("{}.json", s.trim())
}

/// Every saved layout, sorted by name.
pub fn all_layouts() -> Vec<LayoutFile> {
    let mut v: Vec<LayoutFile> = std::fs::read_dir(layouts_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| read(&e.path()))
        .collect();
    v.sort_by_key(|l| l.name.to_lowercase());
    v
}

pub fn list_layouts() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(layouts_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| read(&e.path()).map(|l| l.name))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

fn read(path: &PathBuf) -> Option<LayoutFile> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write(path: &PathBuf, l: &LayoutFile) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(l).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

/// What the layout list shows about a saved layout.
pub struct Summary {
    pub name: String,
    pub tabs: usize,
    pub terminals: usize,
    pub auto_run: usize,
    /// Distinct folders of its terminals, with the home folder shown as ~.
    pub folders: Vec<String>,
}

pub fn summaries() -> Vec<Summary> {
    let home = dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_default();
    let mut out: Vec<Summary> = std::fs::read_dir(layouts_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| read(&e.path()))
        .map(|l| {
            let panes: Vec<PaneSpec> = l.tabs.iter().flat_map(|t| t.root.leaves()).collect();
            let mut folders: Vec<String> = Vec::new();
            for p in &panes {
                if let Some(c) = &p.cwd {
                    let c = if !home.is_empty() && c.starts_with(&home) { format!("~{}", &c[home.len()..]) } else { c.clone() };
                    if !folders.contains(&c) {
                        folders.push(c);
                    }
                }
            }
            Summary {
                name: l.name.clone(),
                tabs: l.tabs.len(),
                terminals: panes.len(),
                auto_run: panes.iter().flat_map(|p| &p.commands).filter(|c| c.auto_run).count(),
                folders,
            }
        })
        .collect();
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

pub fn rename_layout(old: &str, new: &str) -> Result<(), String> {
    let mut l = load_layout(old).ok_or_else(|| format!("could not read layout \"{old}\""))?;
    if layout_path(new) != layout_path(old) && layout_path(new).exists() {
        return Err(format!("a layout called \"{new}\" already exists"));
    }
    l.name = new.to_string();
    save_layout(&l)?;
    if layout_path(new) != layout_path(old) {
        delete_layout(old)?;
    }
    Ok(())
}

pub fn layout_path(name: &str) -> PathBuf {
    layouts_dir().join(file_name(name))
}

pub fn load_layout(name: &str) -> Option<LayoutFile> {
    read(&find_path(name)?)
}

pub fn save_layout(l: &LayoutFile) -> Result<(), String> {
    write(&layout_path(&l.name), l)
}

/// The file holding layout `name`: normally `<name>.json`, but a file renamed or copied by hand
/// is found by the name stored inside it.
fn find_path(name: &str) -> Option<PathBuf> {
    let direct = layout_path(name);
    if direct.exists() {
        return Some(direct);
    }
    std::fs::read_dir(layouts_dir())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "json") && read(p).is_some_and(|l| l.name == name))
}

pub fn delete_layout(name: &str) -> Result<(), String> {
    let path = find_path(name).ok_or_else(|| format!("there is no layout called \"{name}\""))?;
    std::fs::remove_file(&path).map_err(|e| format!("could not delete {}: {e}", path.display()))
}

fn session_path() -> PathBuf {
    config_dir().join("session.json")
}

pub fn load_session() -> Option<LayoutFile> {
    read(&session_path())
}

pub fn save_session(l: &LayoutFile) {
    let _ = write(&session_path(), l);
}
