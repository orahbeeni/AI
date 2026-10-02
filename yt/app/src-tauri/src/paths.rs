//! Where the app keeps its data. In *portable mode* everything (settings,
//! queue, archives and the downloaded yt-dlp/ffmpeg) lives in a folder next to
//! the app, so the whole thing can be copied to a flash drive.

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub const PORTABLE_MARKER: &str = "portable";
pub const PORTABLE_DIR: &str = "ytgrab-data";
const REL_PREFIX: &str = "portable:";

#[derive(Clone, Debug)]
pub struct Paths {
    /// Folder that contains the app (portable base). Only meaningful when `portable`.
    pub base: PathBuf,
    pub data_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub archive_dir: PathBuf,
    pub default_download_dir: PathBuf,
    pub portable: bool,
}

/// The folder a user thinks of as "where the app is": the directory holding the
/// `.AppImage`, the folder next to `Foo.app`, or the `.exe`'s folder.
pub fn app_base_dir() -> Option<PathBuf> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Path::new(&appimage).parent().map(Path::to_path_buf);
    }
    let exe = std::env::current_exe().ok()?;
    for anc in exe.ancestors() {
        if anc.extension().is_some_and(|e| e == "app") {
            return anc.parent().map(Path::to_path_buf);
        }
    }
    exe.parent().map(Path::to_path_buf)
}

pub fn is_portable_base(base: &Path) -> bool {
    base.join(PORTABLE_MARKER).exists()
        || base.join(PORTABLE_DIR).is_dir()
        || std::env::var_os("YTGRAB_PORTABLE").is_some()
}

impl Paths {
    pub fn resolve(app: &AppHandle) -> Paths {
        if let Some(base) = app_base_dir() {
            if is_portable_base(&base) {
                return Paths::portable_at(base);
            }
        }
        let data_dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("ytgrab"));
        let default_download_dir = app
            .path()
            .download_dir()
            .unwrap_or_else(|_| data_dir.join("downloads"));
        Paths::installed_at(data_dir, default_download_dir)
    }

    pub fn portable_at(base: PathBuf) -> Paths {
        let data_dir = base.join(PORTABLE_DIR);
        Paths {
            bin_dir: data_dir.join("bin"),
            archive_dir: data_dir.join("archives"),
            default_download_dir: base.join("downloads"),
            data_dir,
            base,
            portable: true,
        }
    }

    pub fn installed_at(data_dir: PathBuf, default_download_dir: PathBuf) -> Paths {
        Paths {
            bin_dir: data_dir.join("bin"),
            archive_dir: data_dir.join("archives"),
            default_download_dir,
            base: data_dir.clone(),
            data_dir,
            portable: false,
        }
    }

    pub fn ensure_dirs(&self) {
        for d in [&self.data_dir, &self.bin_dir, &self.archive_dir] {
            let _ = std::fs::create_dir_all(d);
        }
    }

    /// Turn a path into what we save to disk. In portable mode, anything inside
    /// the app folder is stored relative so the folder survives being moved to
    /// another drive letter or mount point.
    pub fn encode(&self, path: &str) -> String {
        if !self.portable || path.is_empty() {
            return path.to_string();
        }
        match Path::new(path).strip_prefix(&self.base) {
            Ok(rel) => {
                let rel = rel.to_string_lossy().replace('\\', "/");
                format!("{REL_PREFIX}{rel}")
            }
            Err(_) => path.to_string(),
        }
    }

    /// Inverse of `encode`, resolving against the current app folder.
    pub fn decode(&self, stored: &str) -> String {
        match stored.strip_prefix(REL_PREFIX) {
            Some(rel) => {
                let mut p = self.base.clone();
                for part in rel.split('/').filter(|s| !s.is_empty()) {
                    p.push(part);
                }
                p.to_string_lossy().into_owned()
            }
            None => stored.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_paths_roundtrip_and_relocate() {
        let a = Paths::portable_at(PathBuf::from("/media/usb/YTGrab"));
        let stored = a.encode("/media/usb/YTGrab/downloads/music");
        assert_eq!(stored, "portable:downloads/music");
        // Same folder, now mounted somewhere else.
        let b = Paths::portable_at(PathBuf::from("/mnt/other/YTGrab"));
        let expected: PathBuf = ["/mnt/other/YTGrab", "downloads", "music"].iter().collect();
        assert_eq!(PathBuf::from(b.decode(&stored)), expected);
    }

    #[test]
    fn outside_paths_are_kept_absolute() {
        let a = Paths::portable_at(PathBuf::from("/media/usb/YTGrab"));
        assert_eq!(a.encode("/home/me/Videos"), "/home/me/Videos");
        assert_eq!(a.decode("/home/me/Videos"), "/home/me/Videos");
    }

    #[test]
    fn installed_mode_does_not_rewrite() {
        let a = Paths::installed_at(PathBuf::from("/data"), PathBuf::from("/dl"));
        assert_eq!(a.encode("/dl/x"), "/dl/x");
    }
}
