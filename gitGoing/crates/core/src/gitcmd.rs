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
/// Single quotes, so `$`, backticks and `\` in a path are never interpreted.
pub fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Reject a user-supplied name (branch, remote, tag) that git could mistake for
/// an option, e.g. `--upload-pack=...`.
pub fn plain_name(name: &str) -> Result<&str, String> {
    if name.is_empty() || name.starts_with('-') {
        return Err(format!("'{name}' is not a valid name"));
    }
    Ok(name)
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

/// The command line as the user would type it (without our `-C <repo>`), and
/// the short name of the repository it runs in.
fn describe(cmd: &Command) -> (String, String) {
    let mut args = cmd.get_args().map(|a| a.to_string_lossy().into_owned());
    let mut repo = String::new();
    let mut shown = vec![cmd.get_program().to_string_lossy().into_owned()];
    while let Some(a) = args.next() {
        if a == "-C" {
            if let Some(dir) = args.next() {
                repo = crate::gitlog::repo_name(Path::new(&dir));
            }
            continue;
        }
        shown.push(crate::gitlog::quote(&a));
    }
    (repo, shown.join(" "))
}

/// Run a prepared command and turn failure into a readable message.
pub fn run_cmd(cmd: &mut Command, what: &str) -> Result<String, String> {
    let (repo, display) = describe(cmd);
    crate::gitlog::push(&repo, crate::gitlog::Kind::Cmd, display);
    let started = std::time::Instant::now();
    let result = cmd.output();
    let ms = started.elapsed().as_millis();
    match &result {
        Ok(o) => crate::gitlog::finished(
            &repo,
            &String::from_utf8_lossy(&o.stdout),
            &String::from_utf8_lossy(&o.stderr),
            o.status.success(),
            &o.status.code().map_or_else(|| "terminated".to_owned(), |c| format!("exit {c}")),
            ms,
        ),
        Err(e) => crate::gitlog::finished(&repo, "", &e.to_string(), false, "could not start", ms),
    }
    let out = result.map_err(|e| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlog::{since, Kind};

    #[test]
    fn shell_quote_neutralises_metacharacters() {
        assert_eq!(shell_quote(Path::new("/a b/c")), "'/a b/c'");
        assert_eq!(shell_quote(Path::new("/x/$HOME/`id`")), "'/x/$HOME/`id`'");
        assert_eq!(shell_quote(Path::new("/it's")), "'/it'\\''s'");
        // Run through a real shell, the path comes back untouched.
        if cfg!(unix) {
            let p = "/tmp/we ird/$HOME/`x`/it's";
            let out = Command::new("sh").arg("-c").arg(format!("printf %s {}", shell_quote(Path::new(p)))).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), p);
        }
    }

    #[test]
    fn option_lookalikes_are_rejected() {
        assert!(plain_name("main").is_ok());
        assert!(plain_name("feature/x").is_ok());
        assert!(plain_name("--upload-pack=evil").is_err());
        assert!(plain_name("-x").is_err());
        assert!(plain_name("").is_err());
    }

    #[test]
    fn commands_are_logged_with_output_and_result() {
        let Ok(out) = run(None, &["--version"]) else { return }; // git not installed
        assert!(out.starts_with("git version"));
        let lines = since(0);
        let cmd = lines.iter().rev().find(|l| l.kind == Kind::Cmd && l.text == "git --version").expect("command logged");
        let after: Vec<_> = lines.iter().filter(|l| l.id > cmd.id).collect();
        assert!(after.iter().any(|l| l.kind == Kind::Out && l.text.starts_with("git version")));
        assert!(after.iter().any(|l| l.kind == Kind::Done && l.text.starts_with("exit 0")));
    }

    #[test]
    fn repo_flag_becomes_the_prompt_not_part_of_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, shown) = describe(command(Some(dir.path())).args(["log", "--format=%H", "a b"]));
        assert_eq!(repo, crate::gitlog::repo_name(dir.path()));
        assert_eq!(shown, "git log --format=%H 'a b'");
    }

    #[test]
    fn failures_are_logged_as_errors() {
        let dir = tempfile::tempdir().unwrap();
        if run(None, &["--version"]).is_err() {
            return;
        }
        let _ = run(Some(dir.path()), &["log"]); // not a repository
        let repo = crate::gitlog::repo_name(dir.path());
        let mine: Vec<_> = since(0).into_iter().filter(|l| l.repo == repo).collect();
        assert!(mine.iter().any(|l| l.kind == Kind::Err));
    }
}
