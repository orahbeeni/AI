//! The right-click menu shared by every file row (commit files and working-tree changes).

use eframe::egui;
use gitgoing_core::Oid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    History,
    Blame,
    DiffTool,
    Editor,
    DefaultProgram,
    Reveal,
    Edit,
    Delete,
}

/// A menu choice for one file: `commit` is set for a file of a commit,
/// `staged` for a staged working-tree change.
#[derive(Clone, Debug)]
pub struct FileRequest {
    pub action: FileAction,
    pub path: String,
    pub commit: Option<Oid>,
    pub staged: bool,
}

#[cfg(target_os = "macos")]
const REVEAL: &str = "Show in Finder";
#[cfg(target_os = "windows")]
const REVEAL: &str = "Show in Explorer";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const REVEAL: &str = "Show in file manager";

/// The items below the working-tree specific ones. Returns the chosen action;
/// "Copy file path" is handled here.
pub fn items(ui: &mut egui::Ui, path: &str) -> Option<FileAction> {
    let mut pick = None;
    let mut item = |ui: &mut egui::Ui, label: &str, a: FileAction| {
        if ui.button(label).clicked() {
            pick = Some(a);
            ui.close_menu();
        }
    };
    item(ui, "File History", FileAction::History);
    item(ui, "File Blame", FileAction::Blame);
    ui.separator();
    item(ui, "Open in external diff tool", FileAction::DiffTool);
    item(ui, "Open in external editor", FileAction::Editor);
    item(ui, "Open file in default program", FileAction::DefaultProgram);
    item(ui, REVEAL, FileAction::Reveal);
    ui.separator();
    if ui.button("Copy file path").clicked() {
        ui.ctx().copy_text(path.to_owned());
        ui.close_menu();
    }
    ui.separator();
    item(ui, "Edit file", FileAction::Edit);
    item(ui, "Delete file", FileAction::Delete);
    pick
}
