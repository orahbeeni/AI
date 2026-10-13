//! Things that must be known before the window exists: whether to play the
//! intro (which runs in a borderless window) and where the window was last.
//!
//! They live in small files next to eframe's own storage, because that storage
//! is only available once the window has been created.

use std::path::PathBuf;

const APP_ID: &str = "gitgoing";

/// The borderless window the intro plays in (16:9, like the video). The frames
/// are stored at twice this size, so they are sharp on high-density screens.
pub const SPLASH_SIZE: [f32; 2] = [576.0, 324.0];
pub const DEFAULT_SIZE: [f32; 2] = [1100.0, 700.0];

fn dir() -> Option<PathBuf> {
    let d = eframe::storage_dir(APP_ID)?;
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

/// Where the main window was last: position and inner size (logical points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowGeom {
    pub pos: Option<[f32; 2]>,
    pub size: [f32; 2],
    pub maximized: bool,
}

impl WindowGeom {
    pub fn serialise(&self) -> String {
        let pos = self.pos.map_or_else(|| "none none".to_owned(), |p| format!("{} {}", p[0], p[1]));
        format!("{pos} {} {} {}", self.size[0], self.size[1], self.maximized as u8)
    }

    /// Parse the saved text; anything implausible (a window the size of a pixel,
    /// or miles off-screen) is rejected so a corrupt file can't hide the window.
    pub fn parse(text: &str) -> Option<WindowGeom> {
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.len() != 5 {
            return None;
        }
        let num = |s: &str| s.parse::<f32>().ok().filter(|v| v.is_finite());
        let pos = match (num(parts[0]), num(parts[1])) {
            (Some(x), Some(y)) if x.abs() < 20_000.0 && y.abs() < 20_000.0 => Some([x, y]),
            _ => None,
        };
        let (w, h) = (num(parts[2])?, num(parts[3])?);
        if !(400.0..=10_000.0).contains(&w) || !(300.0..=10_000.0).contains(&h) {
            return None;
        }
        Some(WindowGeom { pos, size: [w, h], maximized: parts[4] == "1" })
    }
}

pub fn load_geom() -> Option<WindowGeom> {
    WindowGeom::parse(&std::fs::read_to_string(dir()?.join("window.txt")).ok()?)
}

pub fn save_geom(g: &WindowGeom) {
    if let Some(d) = dir() {
        let _ = std::fs::write(d.join("window.txt"), g.serialise());
    }
}

/// `text` without eframe's saved `"window"` entry.
///
/// eframe restores the window's last size from its storage whether or not we ask
/// it to save it, which would override the intro's own window size. We keep the
/// window's place ourselves (see [`WindowGeom`]), so the entry is dropped. The
/// storage is a RON map with one `"key": "value",` entry per line.
fn without_window_entry(text: &str) -> String {
    let kept: Vec<&str> = text.lines().filter(|l| !l.trim_start().starts_with("\"window\":")).collect();
    let mut out = kept.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Remove eframe's saved window entry from its storage file (see above).
pub fn forget_eframe_window() {
    let Some(path) = dir().map(|d| d.join("app.ron")) else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let cleaned = without_window_entry(&text);
    if cleaned != text {
        let tmp = path.with_extension("ron.tmp");
        if std::fs::write(&tmp, cleaned).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Is the intro enabled? (A marker file turns it off.)
pub fn intro_enabled() -> bool {
    dir().is_none_or(|d| !d.join("no_intro").exists())
}

pub fn set_intro_enabled(on: bool) {
    if let Some(d) = dir() {
        let marker = d.join("no_intro");
        if on {
            let _ = std::fs::remove_file(marker);
        } else {
            let _ = std::fs::write(marker, "");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_round_trips() {
        let g = WindowGeom { pos: Some([-120.5, 40.0]), size: [1100.0, 700.0], maximized: true };
        assert_eq!(WindowGeom::parse(&g.serialise()), Some(g));
        let centred = WindowGeom { pos: None, size: [900.0, 600.0], maximized: false };
        assert_eq!(WindowGeom::parse(&centred.serialise()), Some(centred));
    }

    #[test]
    fn eframes_saved_window_is_dropped_and_the_rest_kept() {
        let text = "{\n    \"intro\": \"1\",\n    \"window\": \"(outer_position_points:Some((x:1.0,y:2.0)))\",\n    \"zoom\": \"1\",\n}";
        let out = without_window_entry(text);
        assert!(!out.contains("window"));
        assert!(out.contains("\"intro\"") && out.contains("\"zoom\""));
        assert!(out.starts_with('{') && out.ends_with('}'));
        // Nothing to drop: unchanged.
        let plain = "{\n    \"zoom\": \"1\",\n}\n";
        assert_eq!(without_window_entry(plain), plain);
    }

    #[test]
    fn implausible_geometry_is_rejected() {
        assert!(WindowGeom::parse("").is_none());
        assert!(WindowGeom::parse("1 2 3").is_none());
        assert!(WindowGeom::parse("0 0 1 1 0").is_none(), "a one-pixel window");
        assert!(WindowGeom::parse("0 0 99999 700 0").is_none());
        assert!(WindowGeom::parse("NaN 0 800 600 0").is_none_or(|g| g.pos.is_none()));
        // Wildly off-screen positions fall back to "no saved position".
        assert_eq!(WindowGeom::parse("99999 0 800 600 0").unwrap().pos, None);
    }
}
