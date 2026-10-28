//! Tray icon (Linux, via StatusNotifierItem: GNOME's AppIndicator extension, KDE, ...).
//! macOS and Windows have none yet, so there closing the window quits.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use super::Command;
use crate::status::Hub;

pub struct Tray {
    #[cfg(target_os = "linux")]
    _handle: ksni::blocking::Handle<linux::WsTray>,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    pub struct WsTray {
        pub tx: Sender<Command>,
        pub wake: Arc<dyn Fn() + Send + Sync>,
        pub status: String,
        pub icon: ksni::Icon,
    }

    impl WsTray {
        fn send(&self, cmd: Command) {
            let _ = self.tx.send(cmd);
            (self.wake)();
        }
    }

    impl ksni::Tray for WsTray {
        fn id(&self) -> String {
            "wifishare".into()
        }
        fn title(&self) -> String {
            "wifishare".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            vec![self.icon.clone()]
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip { title: "wifishare".into(), description: self.status.clone(), ..Default::default() }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            self.send(Command::Show);
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::*;
            vec![
                StandardItem { label: self.status.clone(), enabled: false, ..Default::default() }.into(),
                MenuItem::Separator,
                StandardItem { label: "Open wifishare".into(), activate: Box::new(|t: &mut Self| t.send(Command::Show)), ..Default::default() }.into(),
                StandardItem { label: "Quit".into(), activate: Box::new(|t: &mut Self| t.send(Command::Quit)), ..Default::default() }.into(),
            ]
        }
    }

    /// ARGB32, network byte order.
    pub fn icon() -> ksni::Icon {
        let size = 64;
        let data = crate::icon::rgba(size).chunks(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
        ksni::Icon { width: size as i32, height: size as i32, data }
    }
}

/// Shows the tray icon, or returns None where there is no tray to show it in.
pub fn spawn(tx: Sender<Command>, hub: Arc<Hub>, wake: Arc<dyn Fn() + Send + Sync>) -> Option<Tray> {
    #[cfg(target_os = "linux")]
    {
        use ksni::blocking::TrayMethods;
        let tray = linux::WsTray { tx, wake, status: "Starting".into(), icon: linux::icon() };
        let handle = match tray.spawn() {
            Ok(h) => h,
            Err(e) => {
                log::info!("no tray available ({e}); closing the window will quit");
                return None;
            }
        };
        // Keep the menu's status line current.
        let h = handle.clone();
        std::thread::spawn(move || {
            let mut last = String::new();
            loop {
                let text = super::summary(&hub.snapshot());
                if text != last && h.update(|t| t.status = text.clone()).is_none() {
                    return;
                }
                last = text;
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
        Some(Tray { _handle: handle })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (tx, hub, wake);
        None
    }
}
