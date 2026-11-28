//! The side panel: settings, file sharing and the activity log.

use egui::{Align, Layout, Sense, Stroke, Ui, pos2, vec2};

use super::icons::{self, Icon};
use super::theme::*;
use super::widgets::{self, Kind};
use super::{Drawer, ReceiverApp, open_folder};
use crate::config::{Decoder, MaxVideo, PointerStyle};
use crate::engine::EngineCmd;
use crate::share::{download_folder, human_size};

pub const WIDTH: f32 = 368.0;

impl ReceiverApp {
    pub(super) fn drawer_contents(&mut self, ui: &mut Ui, drawer: Drawer) {
        let title = match drawer {
            Drawer::Settings => "Settings",
            Drawer::Files => "File sharing",
            Drawer::Activity => "Activity",
        };
        egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 12, top: 12, bottom: 8 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).font(semibold(16.0)).color(TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::Close, "Close", false).clicked() {
                        self.drawer = None;
                    }
                });
            });
        });
        widgets::separator(ui);
        egui::ScrollArea::vertical().id_salt(title).auto_shrink(false).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 18, top: 14, bottom: 20 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 10.0;
                match drawer {
                    Drawer::Settings => self.settings_panel(ui),
                    Drawer::Files => self.files_panel(ui),
                    Drawer::Activity => self.activity_panel(ui),
                }
            });
        });
    }

    fn settings_panel(&mut self, ui: &mut Ui) {
        let before = self.settings.clone();

        widgets::section(ui, "This receiver");
        widgets::caption(ui, "Name shown to devices");
        ui.add(egui::TextEdit::singleline(&mut self.settings.name).margin(egui::Margin::symmetric(11, 8)));
        if self.restart_needed {
            widgets::banner(ui, WARN, Icon::Info, "Restart discovery to apply name and connection changes.");
            if widgets::button(Kind::Primary, "Restart discovery now").icon(Icon::Refresh).width(ui.available_width()).show(ui).clicked() {
                self.engine.send(EngineCmd::RestartTransports);
                self.restart_needed = false;
            }
        }

        group(ui, "Connections");
        let s = &mut self.settings;
        widgets::switch_row(ui, "Wi-Fi Direct", "Phones, Samsung DeX and Windows", &mut s.wifi_direct);
        widgets::switch_row(ui, "Same-network casting", "Windows PCs on your network (MS-MICE)", &mut s.infrastructure);
        widgets::switch_row(ui, "Allow without asking", "Skip the prompt when a device connects", &mut s.auto_accept);

        group(ui, "Picture and sound");
        widgets::caption(ui, "Maximum quality");
        egui::ComboBox::from_id_salt("maxv").width(ui.available_width()).selected_text(s.max_video.label()).show_ui(ui, |ui| {
            for m in [MaxVideo::P1080p60, MaxVideo::P1080p30, MaxVideo::P720p60, MaxVideo::P720p30] {
                ui.selectable_value(&mut s.max_video, m, m.label());
            }
        });
        widgets::caption(ui, "Video decoder");
        widgets::segmented(
            ui,
            &mut s.decoder,
            &[(Decoder::Auto, "Automatic"), (Decoder::Hardware, "Hardware"), (Decoder::Software, "Software")],
        );
        widgets::caption(ui, "Playback");
        widgets::segmented(ui, &mut s.av_sync, &[(false, "Lowest latency"), (true, "Smooth (A/V sync)")]);
        ui.horizontal(|ui| {
            widgets::caption(ui, "Network buffer");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("{} ms", s.jitter_ms)).font(medium(12.5)).color(TEXT_DIM));
            });
        });
        let mut jitter = s.jitter_ms as f32;
        if widgets::slider(ui, &mut jitter, 20.0..=300.0, ui.available_width(), "Network buffer").changed() {
            s.jitter_ms = ((jitter / 5.0).round() * 5.0) as u32;
        }
        widgets::hint(ui, "Quality, decoder and playback apply to the next connection.");

        group(ui, "Mouse, keyboard and files");
        widgets::switch_row(ui, "Control the device", "Send mouse and keyboard input (UIBC)", &mut s.uibc);
        widgets::switch_row(ui, "Lock the mouse on click", "For devices that accept a relative mouse", &mut s.capture_on_click);
        widgets::caption(ui, "Mouse pointer over the picture");
        egui::ComboBox::from_id_salt("pointer").width(ui.available_width()).selected_text(s.pointer.label()).show_ui(ui, |ui| {
            for p in PointerStyle::ALL {
                ui.selectable_value(&mut s.pointer, p, p.label());
            }
        });
        widgets::switch_row(ui, "File sharing", "Exchange files with the connected device", &mut s.file_share);

        group(ui, "About");
        widgets::hint(ui, &format!("Miracast Receiver {}", env!("CARGO_PKG_VERSION")));
        widgets::hint(ui, &format!("Decoders found: {}", wfd_media::available_decoders().join(", ")));
        if let Some(n) = &self.renderer_note {
            widgets::hint(ui, &format!("Video drawing: CPU fallback ({n})"));
        }

        let s = &self.settings;
        if s.name != before.name
            || s.wifi_direct != before.wifi_direct
            || s.infrastructure != before.infrastructure
            || s.uibc != before.uibc
        {
            self.restart_needed = true;
        }
        if s.name != before.name
            || s.wifi_direct != before.wifi_direct
            || s.infrastructure != before.infrastructure
            || s.auto_accept != before.auto_accept
            || s.uibc != before.uibc
            || s.capture_on_click != before.capture_on_click
            || s.pointer != before.pointer
            || s.file_share != before.file_share
            || s.max_video != before.max_video
            || s.decoder != before.decoder
            || s.av_sync != before.av_sync
            || s.jitter_ms != before.jitter_ms
        {
            self.settings.save();
            self.engine.send(EngineCmd::UpdateSettings(self.settings.clone()));
        }
    }

    fn files_panel(&mut self, ui: &mut Ui) {
        if let Some(p) = &self.share_problem {
            widgets::banner(ui, WARN, Icon::Alert, p);
        }
        let Some(url) = self.snapshot.share_url.clone() else {
            empty(ui, Icon::Folder, "Connect a device to exchange files with it.");
            return;
        };

        widgets::card().inner_margin(egui::Margin::same(14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            widgets::hint(ui, "Open this address in the device's web browser");
            ui.add(egui::Label::new(egui::RichText::new(&url).font(mono(15.0)).color(ACCENT_HOVER)).wrap().selectable(true));
            if widgets::button(Kind::Secondary, "Copy address").icon(Icon::Copy).width(ui.available_width()).show(ui).clicked() {
                ui.ctx().copy_text(url.clone());
            }
        });

        ui.add_space(6.0);
        widgets::section(ui, "Shared with the device");
        drop_zone(ui);
        let files = self.snapshot.shared_files.lock().unwrap().files().to_vec();
        for f in files {
            let removed = file_row(ui, &f.name, &human_size(f.size), true);
            if removed {
                self.snapshot.shared_files.lock().unwrap().remove(f.id);
            }
        }

        ui.add_space(8.0);
        widgets::section(ui, "Received from the device");
        if self.snapshot.received.is_empty() {
            widgets::hint(ui, "Files the device sends are saved in Downloads/Miracast.");
        }
        for p in self.snapshot.received.iter().rev() {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            file_row(ui, &name, "Saved to Downloads/Miracast", false);
        }
        if widgets::button(Kind::Secondary, "Open folder").icon(Icon::Folder).width(ui.available_width()).show(ui).clicked() {
            open_folder(&download_folder());
        }
    }

    fn activity_panel(&mut self, ui: &mut Ui) {
        let log = &self.snapshot.log;
        if widgets::button(Kind::Secondary, "Copy log")
            .icon(Icon::Copy)
            .width(ui.available_width())
            .enabled(!log.is_empty())
            .show(ui)
            .clicked()
        {
            let text: String = log.iter().map(|(t, m)| format!("{:>5}s ago  {m}\n", t.elapsed().as_secs())).collect();
            ui.ctx().copy_text(text);
        }
        if log.is_empty() {
            empty(ui, Icon::List, "Nothing has happened yet.");
        }
        for (i, (t, m)) in log.iter().rev().take(100).enumerate() {
            if i > 0 {
                widgets::separator(ui);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(egui::RichText::new(m).font(regular(12.5)).color(TEXT_DIM)).wrap().selectable(true));
                ui.label(egui::RichText::new(ago(t.elapsed().as_secs())).font(regular(11.0)).color(TEXT_FAINT));
            });
        }
    }
}

fn group(ui: &mut Ui, title: &str) {
    ui.add_space(4.0);
    widgets::separator(ui);
    ui.add_space(4.0);
    widgets::section(ui, title);
}

fn ago(secs: u64) -> String {
    match secs {
        0..=4 => "just now".into(),
        5..=59 => format!("{secs} s ago"),
        60..=3599 => format!("{} min ago", secs / 60),
        _ => format!("{} h ago", secs / 3600),
    }
}

fn empty(ui: &mut Ui, icon: Icon, text: &str) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        icons::show_tile(ui, icon, 48.0, IDLE);
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(text).color(TEXT_DIM)).wrap());
    });
}

fn drop_zone(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 76.0), Sense::hover());
    let r = rect.shrink(1.0);
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    ui.painter().extend(egui::Shape::dashed_line(&corners, Stroke::new(1.0, BORDER_STRONG), 5.0, 4.0));
    let icon = egui::Rect::from_center_size(pos2(r.center().x, r.center().y - 11.0), vec2(22.0, 22.0));
    icons::paint(ui.painter(), icon, Icon::Upload, TEXT_DIM);
    ui.painter().text(
        pos2(r.center().x, r.center().y + 15.0),
        egui::Align2::CENTER_CENTER,
        "Drag files onto this window",
        regular(12.5),
        TEXT_DIM,
    );
}

/// One file; returns true when its remove button was pressed.
fn file_row(ui: &mut Ui, name: &str, detail: &str, removable: bool) -> bool {
    let mut removed = false;
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(10.0)
        .inner_margin(egui::Margin { left: 11, right: 6, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 36.0), Layout::left_to_right(Align::Center), |ui| {
                ui.set_height(36.0);
                ui.spacing_mut().item_spacing.x = 10.0;
                icons::show(ui, if removable { Icon::File } else { Icon::Download }, 18.0, ACCENT_HOVER);
                let width = ui.available_width() - if removable { 44.0 } else { 6.0 };
                widgets::title_block(ui, name, detail, width).on_hover_text(name);
                if removable {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        removed = widgets::icon_button(ui, Icon::Close, "Stop sharing", false).clicked();
                    });
                }
            });
        });
    removed
}
