//! The About window: version and the build number stamped by `build.rs`.

use eframe::egui;

#[derive(Default)]
pub struct AboutWindow {
    pub open: bool,
}

impl AboutWindow {
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new("About gitGoing")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.heading("gitGoing");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                let build = format!("Build {}", env!("GITGOING_BUILD_NUMBER"));
                let commit = env!("GITGOING_BUILD_COMMIT");
                ui.horizontal(|ui| {
                    ui.strong(&build);
                    if !commit.is_empty() {
                        ui.weak(format!("({commit})"));
                    }
                    if ui.small_button("Copy").on_hover_text("Copy the build number").clicked() {
                        let text = if commit.is_empty() { build.clone() } else { format!("{build} ({commit})") };
                        ui.ctx().copy_text(text);
                    }
                });
                ui.weak(format!("Built {}", env!("GITGOING_BUILD_TIME")));
                ui.add_space(4.0);
                ui.label("A fast, cross-platform desktop git client.");
            });
        self.open = open;
    }
}
