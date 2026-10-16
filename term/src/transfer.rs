//! Export and import of everything the user customised: settings and saved layouts, in one file.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::Settings;
use crate::session::{self, LayoutFile};

const FORMAT: &str = "term-settings";

#[derive(Serialize, Deserialize)]
pub struct Bundle {
    /// Always "term-settings", so other JSON files are recognised as not ours.
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub settings: Option<Settings>,
    #[serde(default)]
    pub layouts: Vec<LayoutFile>,
}

pub fn export(path: &Path, settings: &Settings) -> Result<usize, String> {
    let layouts = session::all_layouts();
    let n = layouts.len();
    let bundle = Bundle { format: FORMAT.into(), version: 1, settings: Some(settings.clone()), layouts };
    let text = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(n)
}

pub fn read(path: &Path) -> Result<Bundle, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let mut b: Bundle = serde_json::from_str(&text).map_err(|_| "this is not a term settings file".to_string())?;
    if b.format != FORMAT {
        return Err("this is not a term settings file".into());
    }
    if let Some(s) = &mut b.settings {
        s.normalize();
    }
    b.layouts.retain(|l| !l.name.trim().is_empty());
    Ok(b)
}

/// Save the bundle's layouts. Layouts whose name already exists are replaced only when
/// `replace` is set. Returns (saved, skipped).
pub fn import_layouts(b: &Bundle, replace: bool) -> Result<(usize, usize), String> {
    let existing = session::list_layouts();
    let (mut saved, mut skipped) = (0, 0);
    for l in &b.layouts {
        if existing.contains(&l.name) && !replace {
            skipped += 1;
            continue;
        }
        session::save_layout(l)?;
        saved += 1;
    }
    Ok((saved, skipped))
}
