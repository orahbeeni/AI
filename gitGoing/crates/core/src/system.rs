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
