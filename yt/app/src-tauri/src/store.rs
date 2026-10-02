//! Small versioned JSON files: settings, queue/history and subscriptions.
//! Plain JSON so they read the same on every OS and are easy to back up.

use crate::args::JobOptions;
use crate::paths::Paths;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

pub fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write atomically (temp file + rename) so a crash never leaves half a file.
pub fn save<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, path)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub version: u32,
    pub download_dir: String,
    pub max_concurrent: u32,
    pub close_to_tray: bool,
    pub notifications: bool,
    /// "managed" = prefer the copies this app downloads, "system" = prefer PATH.
    pub binary_source: String,
    pub custom_ytdlp_path: String,
    /// "none" | "notify" | "quit" | "shutdown"
    pub after_queue: String,
    /// UI-owned values (theme, presets, last-used options...) preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: 1,
            download_dir: String::new(),
            max_concurrent: 2,
            close_to_tray: true,
            notifications: true,
            binary_source: "managed".into(),
            custom_ytdlp_path: String::new(),
            after_queue: "none".into(),
            extra: Map::new(),
        }
    }
}

impl Settings {
    pub fn load(paths: &Paths) -> Settings {
        let mut s: Settings = load(&paths.data_dir.join("settings.json"));
        s.download_dir = paths.decode(&s.download_dir);
        if s.download_dir.trim().is_empty() {
            s.download_dir = paths.default_download_dir.to_string_lossy().into_owned();
        }
        s.max_concurrent = s.max_concurrent.clamp(1, 8);
        s
    }

    pub fn save(&self, paths: &Paths) -> std::io::Result<()> {
        let mut copy = self.clone();
        copy.download_dir = paths.encode(&copy.download_dir);
        save(&paths.data_dir.join("settings.json"), &copy)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub url: String,
    pub interval_minutes: u32,
    pub enabled: bool,
    pub options: JobOptions,
    pub last_check: Option<i64>,
    pub last_note: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SubFile {
    pub subscriptions: Vec<Subscription>,
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_keep_unknown_ui_fields() {
        let json = r#"{"maxConcurrent":3,"theme":"dark","presets":[{"id":"a"}]}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.max_concurrent, 3);
        let out = serde_json::to_value(&s).unwrap();
        assert_eq!(out["theme"], "dark");
        assert_eq!(out["presets"][0]["id"], "a");
    }

    #[test]
    fn save_and_load_roundtrip_in_temp_dir() {
        let dir = std::env::temp_dir().join(format!("ytgrab-test-{}", uuid::Uuid::new_v4()));
        let p = dir.join("s.json");
        let s = Settings { max_concurrent: 5, ..Default::default() };
        save(&p, &s).unwrap();
        let back: Settings = load(&p);
        assert_eq!(back.max_concurrent, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("ytgrab-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.json");
        std::fs::write(&p, "{not json").unwrap();
        let s: Settings = load(&p);
        assert_eq!(s.max_concurrent, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
