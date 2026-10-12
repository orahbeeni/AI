//! Running the system `git` for operations that need the user's credential
//! helpers, SSH setup and hooks (network operations, rebase).
//!
//! The app registers its own executable as a helper ([`set_helper_exe`]). Git
//! and ssh then call it back to ask the user for passwords and passphrases
//! (`GIT_ASKPASS` / `SSH_ASKPASS`) and to supply rebase todo lists, so nothing
//! ever waits on a terminal nobody can see.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

static HELPER: OnceLock<PathBuf> = OnceLock::new();

/// Env var that tells our executable it was started by git as an askpass helper.
pub const ASKPASS_ENV: &str = "GITGOING_ASKPASS";

/// Register the executable git should call back (normally `current_exe()`).
pub fn set_helper_exe(path: PathBuf) {
    let _ = HELPER.set(path);
}

pub fn helper_exe() -> Option<&'static Path> {
    HELPER.get().map(PathBuf::as_path)
}

/// Quote for a POSIX shell (git runs askpass/editor commands through `sh`).
pub fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// A `git` command with a safe, non-interactive environment.
pub fn command(cwd: Option<&Path>) -> Command {
    let mut cmd = Command::new("git");
    if let Some(dir) = cwd {
        cmd.arg("-C").arg(dir);
    }
    // Never wait for a terminal prompt we can't show; never open an editor.
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", ":")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .stdin(Stdio::null());
    if let Some(exe) = helper_exe() {
        cmd.env("GIT_ASKPASS", shell_quote(exe))
            .env("SSH_ASKPASS", exe)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(ASKPASS_ENV, "1");
    } else if !has_custom_ssh(cwd) {
        // No way to ask: fail fast instead of hanging on a passphrase prompt.
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn has_custom_ssh(cwd: Option<&Path>) -> bool {
    if std::env::var_os("GIT_SSH_COMMAND").is_some() || std::env::var_os("GIT_SSH").is_some() {
        return true;
    }
    let mut cmd = Command::new("git");
    if let Some(dir) = cwd {
        cmd.arg("-C").arg(dir);
    }
    cmd.args(["config", "--get", "core.sshCommand"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Run `git <args>`; returns stdout, or git's own error text.
pub fn run(cwd: Option<&Path>, args: &[&str]) -> Result<String, String> {
    run_cmd(command(cwd).args(args), args.first().copied().unwrap_or("git"))
}

/// Run a prepared command and turn failure into a readable message.
pub fn run_cmd(cmd: &mut Command, what: &str) -> Result<String, String> {
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "git executable not found; install git to use this feature".to_owned()
        } else {
            e.to_string()
        }
    })?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let text = if stderr.trim().is_empty() { stdout } else { stderr };
    let text = text.trim();
    Err(if text.is_empty() { format!("git {what} failed ({})", out.status) } else { text.to_owned() })
}
