//! Small OS integrations.

use std::path::Path;
use std::process::{Command, Stdio};

/// Open a file for editing, without running it.
///
/// Handing a path to the generic "open" command would *execute* scripts and
/// programs (a `.command`, `.bat` or `.app` in a cloned repository), so each
/// platform opens it with a text editor instead.
pub fn open_file(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg("-t").arg(path); // default text editor
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("notepad.exe");
        c.arg(path).creation_flags(0x0800_0000);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

/// Open a file with the program the OS associates with it.
///
/// Unlike [`open_file`] this may *run* the file (scripts, executables), so only
/// call it for something the user explicitly asked to open that way.
pub fn open_default(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).creation_flags(0x0800_0000);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = Command::new("xdg-open");
    cmd.arg(path);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

/// Show a file in the platform's file manager (Finder / Explorer / the folder).
pub fn reveal(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("explorer.exe");
        c.arg(format!("/select,{}", path.display())).creation_flags(0x0800_0000);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}
