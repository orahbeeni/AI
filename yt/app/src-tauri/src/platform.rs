//! Every OS-specific behaviour lives in this file. The rest of the app never
//! checks the operating system directly.

use std::path::Path;
use tokio::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X64,
    Arm64,
}

pub fn os() -> Os {
    if cfg!(target_os = "windows") {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::MacOs
    } else {
        Os::Linux
    }
}

pub fn arch() -> Arch {
    if cfg!(target_arch = "aarch64") {
        Arch::Arm64
    } else {
        Arch::X64
    }
}

/// `yt-dlp` -> `yt-dlp.exe` on Windows.
pub fn exe_name(base: &str) -> String {
    exe_name_for(os(), base)
}

pub fn exe_name_for(os: Os, base: &str) -> String {
    if os == Os::Windows {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

/// The yt-dlp release asset to download for a platform.
pub fn ytdlp_asset(os: Os, arch: Arch) -> &'static str {
    match (os, arch) {
        (Os::Windows, Arch::X64) => "yt-dlp.exe",
        (Os::Windows, Arch::Arm64) => "yt-dlp_arm64.exe",
        (Os::MacOs, _) => "yt-dlp_macos",
        (Os::Linux, Arch::X64) => "yt-dlp_linux",
        (Os::Linux, Arch::Arm64) => "yt-dlp_linux_aarch64",
    }
}

/// Platform suffix used by the ffmpeg-static release assets.
pub fn ffmpeg_suffix(os: Os, arch: Arch) -> &'static str {
    match (os, arch) {
        (Os::Windows, _) => "win32-x64", // arm64 Windows runs x64 binaries via emulation
        (Os::MacOs, Arch::X64) => "darwin-x64",
        (Os::MacOs, Arch::Arm64) => "darwin-arm64",
        (Os::Linux, Arch::X64) => "linux-x64",
        (Os::Linux, Arch::Arm64) => "linux-arm64",
    }
}

/// Target triple used by the deno release assets.
pub fn deno_triple(os: Os, arch: Arch) -> &'static str {
    match (os, arch) {
        (Os::Windows, Arch::X64) => "x86_64-pc-windows-msvc",
        (Os::Windows, Arch::Arm64) => "aarch64-pc-windows-msvc",
        (Os::MacOs, Arch::X64) => "x86_64-apple-darwin",
        (Os::MacOs, Arch::Arm64) => "aarch64-apple-darwin",
        (Os::Linux, Arch::X64) => "x86_64-unknown-linux-gnu",
        (Os::Linux, Arch::Arm64) => "aarch64-unknown-linux-gnu",
    }
}

/// Prepare a child process: no flashing console window on Windows, and its own
/// process group on Unix so the whole tree can be signalled together.
pub fn configure_command(cmd: &mut Command) {
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
}

/// Ask a running download to stop gracefully so yt-dlp can finish writing
/// (important for live recordings). Returns false if graceful stop is not
/// possible on this platform.
pub fn request_stop(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Negative pid = the whole process group (yt-dlp + ffmpeg).
        unsafe { libc::kill(-(pid as i32), libc::SIGINT) == 0 }
    }
    #[cfg(windows)]
    {
        // A console-less child cannot receive Ctrl+C; the caller falls back to kill_tree.
        let _ = pid;
        false
    }
}

/// Forcefully terminate a process and all of its children.
pub fn kill_tree(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .status();
    }
}

/// Make a downloaded binary runnable and, on macOS, clear the Gatekeeper
/// quarantine flag so it is not blocked.
pub fn prepare_binary(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
    }
    if os() == Os::MacOs {
        let _ = std::process::Command::new("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(path)
            .status();
    }
    Ok(())
}

/// Power the machine off (used by the "when the queue is finished" action).
pub fn shutdown_machine() {
    let _ = match os() {
        Os::Windows => std::process::Command::new("shutdown").args(["/s", "/t", "60"]).spawn(),
        Os::MacOs => std::process::Command::new("osascript")
            .args(["-e", "tell app \"System Events\" to shut down"])
            .spawn(),
        Os::Linux => std::process::Command::new("systemctl").arg("poweroff").spawn(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_names() {
        assert_eq!(exe_name_for(Os::Windows, "yt-dlp"), "yt-dlp.exe");
        assert_eq!(exe_name_for(Os::Linux, "yt-dlp"), "yt-dlp");
        assert_eq!(exe_name_for(Os::MacOs, "ffmpeg"), "ffmpeg");
    }

    #[test]
    fn every_platform_has_assets() {
        for os in [Os::Linux, Os::MacOs, Os::Windows] {
            for arch in [Arch::X64, Arch::Arm64] {
                assert!(!ytdlp_asset(os, arch).is_empty());
                assert!(!ffmpeg_suffix(os, arch).is_empty());
                assert!(!deno_triple(os, arch).is_empty());
            }
        }
        assert_eq!(ytdlp_asset(Os::Windows, Arch::X64), "yt-dlp.exe");
        assert_eq!(ffmpeg_suffix(Os::MacOs, Arch::Arm64), "darwin-arm64");
    }
}
