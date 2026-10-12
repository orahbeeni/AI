//! User preferences: appearance, defaults, and the global git identity.

use eframe::egui;
use gitgoing_core::remote::PullMode;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    const ALL: [(ThemeChoice, &'static str); 3] =
        [(ThemeChoice::System, "Follow system"), (ThemeChoice::Dark, "Dark"), (ThemeChoice::Light, "Light")];

    fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
        }
    }

    fn from_key(k: &str) -> Self {
        match k {
            "dark" => ThemeChoice::Dark,
            "light" => ThemeChoice::Light,
            _ => ThemeChoice::System,
        }
    }
}

fn pull_key(m: PullMode) -> &'static str {
    match m {
        PullMode::Merge => "merge",
        PullMode::FastForwardOnly => "ff-only",
        PullMode::Rebase => "rebase",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// UI scale factor.
    pub zoom: f32,
    /// What the Pull button does.
    pub pull_mode: PullMode,
    /// Open diffs side by side.
    pub split_diff: bool,
    /// Open diffs ignoring whitespace changes.
    pub ignore_whitespace: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { theme: ThemeChoice::System, zoom: 1.0, pull_mode: PullMode::Merge, split_diff: false, ignore_whitespace: false }
    }
}

impl Settings {
    pub fn load(get: impl Fn(&str) -> Option<String>) -> Self {
        let d = Settings::default();
        Settings {
            theme: get("theme").map_or(d.theme, |v| ThemeChoice::from_key(&v)),
            zoom: get("zoom").and_then(|v| v.parse().ok()).map_or(d.zoom, |z: f32| z.clamp(0.7, 2.0)),
            pull_mode: get("pull_mode")
                .and_then(|v| PullMode::ALL.into_iter().find(|m| pull_key(*m) == v))
                .unwrap_or(d.pull_mode),
            split_diff: get("split_diff").is_some_and(|v| v == "1"),
            ignore_whitespace: get("ignore_ws").is_some_and(|v| v == "1"),
        }
    }

    pub fn save(&self, mut set: impl FnMut(&str, String)) {
        set("theme", self.theme.key().to_owned());
        set("zoom", self.zoom.to_string());
        set("pull_mode", pull_key(self.pull_mode).to_owned());
        set("split_diff", (self.split_diff as u8).to_string());
        set("ignore_ws", (self.ignore_whitespace as u8).to_string());
    }

    /// Push the settings into egui and the diff view.
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.theme {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::Light => egui::ThemePreference::Light,
        });
        ctx.set_zoom_factor(self.zoom);
        crate::diffview::set_defaults(self.split_diff, self.ignore_whitespace);
    }
}

#[derive(Default)]
pub struct SettingsWindow {
    pub open: bool,
    name: String,
    email: String,
    loaded: bool,
    message: Option<(String, bool)>,
}

impl SettingsWindow {
    /// Show the window; returns true if a setting changed.
    pub fn show(&mut self, ctx: &egui::Context, s: &mut Settings) -> bool {
        if !self.open {
            self.loaded = false;
            return false;
        }
        if !self.loaded {
            (self.name, self.email) = gitgoing_core::config::read_identity();
            self.message = None;
            self.loaded = true;
        }
        let before = s.clone();
        let mut open = true;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.strong("Appearance");
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    egui::ComboBox::from_id_salt("theme")
                        .selected_text(ThemeChoice::ALL.iter().find(|(t, _)| *t == s.theme).map_or("", |(_, l)| *l))
                        .show_ui(ui, |ui| {
                            for (t, label) in ThemeChoice::ALL {
                                ui.selectable_value(&mut s.theme, t, label);
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Interface scale");
                    ui.add(egui::Slider::new(&mut s.zoom, 0.7..=1.6).step_by(0.05).fixed_decimals(2));
                });
                ui.add_space(8.0);
                ui.strong("Git");
                ui.horizontal(|ui| {
                    ui.label("Pull button");
                    egui::ComboBox::from_id_salt("pull_mode")
                        .selected_text(s.pull_mode.label())
                        .show_ui(ui, |ui| {
                            for m in PullMode::ALL {
                                ui.selectable_value(&mut s.pull_mode, m, m.label());
                            }
                        });
                });
                ui.checkbox(&mut s.split_diff, "Show diffs side by side by default");
                ui.checkbox(&mut s.ignore_whitespace, "Ignore whitespace changes in diffs by default");
                ui.add_space(8.0);
                ui.strong("Commit author (global git config)");
                egui::Grid::new("identity").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(300.0));
                    ui.end_row();
                    ui.label("Email");
                    ui.add(egui::TextEdit::singleline(&mut self.email).desired_width(300.0));
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    if ui.button("Save author").clicked() {
                        self.message = Some(match gitgoing_core::config::write_identity(&self.name, &self.email) {
                            Ok(()) => ("Saved to your global git config.".to_owned(), false),
                            Err(e) => (e, true),
                        });
                    }
                    if let Some((m, err)) = &self.message {
                        if *err {
                            ui.colored_label(egui::Color32::LIGHT_RED, m);
                        } else {
                            ui.weak(m);
                        }
                    }
                });
            });
        self.open = open;
        *s != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn round_trips_through_storage_strings() {
        let s = Settings {
            theme: ThemeChoice::Dark,
            zoom: 1.25,
            pull_mode: PullMode::Rebase,
            split_diff: true,
            ignore_whitespace: true,
        };
        let mut store = HashMap::new();
        s.save(|k, v| {
            store.insert(k.to_owned(), v);
        });
        assert_eq!(Settings::load(|k| store.get(k).cloned()), s);
    }

    #[test]
    fn bad_values_fall_back_to_defaults() {
        let mut store = HashMap::new();
        store.insert("zoom".to_owned(), "99".to_owned());
        store.insert("pull_mode".to_owned(), "bogus".to_owned());
        let s = Settings::load(|k| store.get(k).cloned());
        assert_eq!(s.zoom, 2.0);
        assert_eq!(s.pull_mode, PullMode::Merge);
        assert_eq!(Settings::load(|_| None), Settings::default());
    }
}
