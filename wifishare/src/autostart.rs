//! Starting wifishare at login, and (Linux) listing it in the applications menu.

use std::path::PathBuf;

use anyhow::{Context, Result};

fn exe() -> Result<PathBuf> {
    std::env::current_exe().context("cannot tell where the wifishare binary is")
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;

    fn autostart_file() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("autostart/wifishare.desktop"))
    }

    fn launcher_file() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("applications/wifishare.desktop"))
    }

    fn icon_file() -> Option<PathBuf> {
        Some(dirs::data_dir()?.join("icons/hicolor/256x256/apps/wifishare.png"))
    }

    fn desktop_entry(args: &str) -> Result<String> {
        let icon = icon_file().context("no data directory")?;
        Ok(format!(
            "[Desktop Entry]\nType=Application\nName=wifishare\nComment=Share keyboard, mouse and clipboard between computers\n\
             Exec=\"{}\" {args}\nIcon={}\nTerminal=false\nCategories=Utility;Network;\nStartupWMClass=wifishare\n\
             X-GNOME-Autostart-enabled=true\n",
            exe()?.display(),
            icon.display()
        ))
    }

    fn write_icon() -> Result<()> {
        let icon = icon_file().context("no data directory")?;
        std::fs::create_dir_all(icon.parent().unwrap())?;
        std::fs::write(&icon, crate::icon::png(256))?;
        Ok(())
    }

    pub fn is_enabled() -> bool {
        autostart_file().is_some_and(|f| f.exists())
    }

    pub fn set_enabled(on: bool) -> Result<()> {
        let file = autostart_file().context("no config directory")?;
        if on {
            write_icon()?;
            std::fs::create_dir_all(file.parent().unwrap())?;
            std::fs::write(&file, desktop_entry("gui --background")?)?;
        } else if file.exists() {
            std::fs::remove_file(&file)?;
        }
        Ok(())
    }

    pub fn launcher_installed() -> bool {
        launcher_file().is_some_and(|f| f.exists())
    }

    /// Adds wifishare to the applications menu, pointing at this binary.
    pub fn install_launcher() -> Result<()> {
        write_icon()?;
        let file = launcher_file().context("no data directory")?;
        std::fs::create_dir_all(file.parent().unwrap())?;
        std::fs::write(&file, desktop_entry("gui")?)?;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    fn agent_file() -> Option<PathBuf> {
        Some(dirs::home_dir()?.join("Library/LaunchAgents/com.wifishare.gui.plist"))
    }

    pub fn is_enabled() -> bool {
        agent_file().is_some_and(|f| f.exists())
    }

    pub fn set_enabled(on: bool) -> Result<()> {
        let file = agent_file().context("no home directory")?;
        if on {
            let plist = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>com.wifishare.gui</string>
    <key>ProgramArguments</key><array><string>{}</string><string>gui</string><string>--background</string></array>
    <key>RunAtLoad</key><true/>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#,
                exe()?.display()
            );
            std::fs::create_dir_all(file.parent().unwrap())?;
            std::fs::write(&file, plist)?;
        } else if file.exists() {
            std::fs::remove_file(&file)?;
        }
        Ok(())
    }

    pub fn launcher_installed() -> bool {
        true
    }

    pub fn install_launcher() -> Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use std::process::Command;

    use super::*;

    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn is_enabled() -> bool {
        Command::new("reg").args(["query", KEY, "/v", "wifishare"]).output().is_ok_and(|o| o.status.success())
    }

    pub fn set_enabled(on: bool) -> Result<()> {
        let status = if on {
            let value = format!("\"{}\" gui --background", exe()?.display());
            Command::new("reg").args(["add", KEY, "/v", "wifishare", "/t", "REG_SZ", "/d", &value, "/f"]).status()?
        } else {
            Command::new("reg").args(["delete", KEY, "/v", "wifishare", "/f"]).status()?
        };
        anyhow::ensure!(status.success(), "reg.exe failed");
        Ok(())
    }

    pub fn launcher_installed() -> bool {
        true
    }

    pub fn install_launcher() -> Result<()> {
        Ok(())
    }
}

pub use imp::*;
