//! Free-space lookup for the download folder.

use std::path::Path;

pub fn free_space(path: &Path) -> Option<u64> {
    fs4::available_space(path).ok()
}
