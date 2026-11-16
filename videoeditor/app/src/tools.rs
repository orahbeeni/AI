//! Finding FFmpeg on this computer and starting it.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A working FFmpeg and what it can encode.
#[derive(Clone, Debug)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub version: String,
    pub encoders: HashSet<String>,
}

/// A command that never pops up a terminal window on Windows.
pub fn quiet_command(program: &OsStr) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// Places FFmpeg usually lives that a program started from a desktop icon may not have on its PATH.
fn common_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from));
    } else if cfg!(windows) {
        dirs.extend([r"C:\ffmpeg\bin", r"C:\Program Files\ffmpeg\bin", r"C:\ProgramData\chocolatey\bin"].map(PathBuf::from));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join(r"Microsoft\WinGet\Links"));
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            dirs.push(PathBuf::from(home).join(r"scoop\shims"));
        }
    } else {
        dirs.extend(["/usr/bin", "/usr/local/bin", "/snap/bin"].map(PathBuf::from));
    }
    dirs
}

/// Find FFmpeg: the location the person picked, beside this program, on the PATH, then the usual places.
pub fn locate(preferred: Option<&Path>) -> Option<Tools> {
    let name = exe_name("ffmpeg");
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = preferred {
        candidates.push(if path.is_dir() { path.join(&name) } else { path.to_path_buf() });
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join(&name));
    }
    candidates.push(PathBuf::from(&name));
    candidates.extend(common_dirs().into_iter().map(|dir| dir.join(&name)));
    candidates.into_iter().find_map(|path| inspect(&path))
}

fn inspect(ffmpeg: &Path) -> Option<Tools> {
    let output = quiet_command(ffmpeg.as_os_str()).arg("-version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let version = text.lines().next().unwrap_or("ffmpeg").to_string();
    let probe_name = exe_name("ffprobe");
    let ffprobe = match ffmpeg.parent() {
        Some(dir) if !dir.as_os_str().is_empty() && dir.join(&probe_name).exists() => dir.join(&probe_name),
        _ => PathBuf::from(&probe_name),
    };
    quiet_command(ffprobe.as_os_str()).arg("-version").output().ok()?;
    Some(Tools { ffmpeg: ffmpeg.to_path_buf(), ffprobe, version, encoders: list_encoders(ffmpeg) })
}

fn list_encoders(ffmpeg: &Path) -> HashSet<String> {
    let Ok(output) = quiet_command(ffmpeg.as_os_str()).args(["-hide_banner", "-encoders"]).output() else {
        return HashSet::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip_while(|line| !line.trim_start().starts_with("---"))
        .skip(1)
        .filter_map(|line| line.split_whitespace().nth(1).map(str::to_string))
        .collect()
}

/// A command as it could be typed into a terminal, for the console.
pub fn display_command(program: &OsStr, args: &[OsString]) -> String {
    let name = Path::new(program).file_stem().unwrap_or(program).to_string_lossy().into_owned();
    let mut line = name;
    for arg in args {
        line.push(' ');
        line.push_str(&quote(&arg.to_string_lossy()));
    }
    line
}

fn quote(text: &str) -> String {
    let plain = !text.is_empty()
        && text.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=+,@%".contains(c));
    if plain {
        text.to_string()
    } else if cfg!(windows) {
        format!("\"{}\"", text.replace('"', "\\\""))
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_only_what_needs_it() {
        let args: Vec<OsString> = ["-i", "my clip.mp4", "out.mp4"].iter().map(OsString::from).collect();
        let shown = display_command(OsStr::new("/usr/bin/ffmpeg"), &args);
        if cfg!(windows) {
            assert_eq!(shown, "ffmpeg -i \"my clip.mp4\" out.mp4");
        } else {
            assert_eq!(shown, "ffmpeg -i 'my clip.mp4' out.mp4");
        }
    }
}
