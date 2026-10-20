//! The alert ping, played with whatever the OS already has for it.

use std::process::{Command, Stdio};

/// Play the ping (or the sound file `custom`, when set) without blocking. Failures are silent.
pub fn ping(custom: &str) {
    let custom = custom.trim().to_string();
    let _ = std::thread::Builder::new().name("ping".into()).spawn(move || {
        let _ = play(&custom);
    });
}

/// Play it and say what was used, or why nothing could be played.
pub fn test(custom: &str, done: impl FnOnce(String) + Send + 'static) {
    let custom = custom.trim().to_string();
    let _ = std::thread::Builder::new().name("ping-test".into()).spawn(move || {
        done(match play(&custom) {
            Ok(how) => format!("Played with {how}."),
            Err(e) => format!("Could not play a sound: {e}"),
        });
    });
}

/// A custom sound must exist (some players report success for a missing file).
fn check_file(custom: &str) -> Result<(), String> {
    if custom.is_empty() || std::path::Path::new(custom).is_file() { Ok(()) } else { Err(format!("{custom} does not exist")) }
}

/// Run the first command that works.
fn first_working(candidates: Vec<Command>) -> Result<String, String> {
    let mut last = "no sound player found".to_string();
    for mut c in candidates {
        let name = c.get_program().to_string_lossy().into_owned();
        match c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status() {
            Ok(s) if s.success() => return Ok(name),
            Ok(s) => last = format!("{name} exited with {s}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => last = format!("{name}: {e}"),
        }
    }
    Err(last)
}

fn cmd(program: &str, args: &[&str]) -> Command {
    let mut c = Command::new(program);
    c.args(args);
    c
}

#[cfg(target_os = "linux")]
fn play(custom: &str) -> Result<String, String> {
    check_file(custom)?;
    if !custom.is_empty() {
        return first_working(vec![
            cmd("paplay", &[custom]),
            cmd("pw-play", &[custom]),
            cmd("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet", custom]),
            cmd("mpv", &["--no-video", "--really-quiet", custom]),
            cmd("aplay", &["-q", custom]),
        ]);
    }
    // The desktop's own message sound first, so it follows the user's sound theme and volume.
    const THEME: &str = "/usr/share/sounds/freedesktop/stereo/message.oga";
    let mut options = vec![cmd("canberra-gtk-play", &["-i", "message-new-instant"])];
    if std::path::Path::new(THEME).exists() {
        options.push(cmd("paplay", &[THEME]));
        options.push(cmd("pw-play", &[THEME]));
    }
    first_working(options)
}

#[cfg(target_os = "macos")]
fn play(custom: &str) -> Result<String, String> {
    check_file(custom)?;
    let file = if custom.is_empty() { "/System/Library/Sounds/Ping.aiff" } else { custom };
    first_working(vec![cmd("afplay", &[file])])
}

#[cfg(windows)]
fn play(custom: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    check_file(custom)?;
    let script = if custom.is_empty() {
        "[System.Media.SystemSounds]::Asterisk.Play(); Start-Sleep -Milliseconds 1000".to_string()
    } else {
        format!("(New-Object System.Media.SoundPlayer '{}').PlaySync()", custom.replace('\'', "''"))
    };
    let mut c = cmd("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script]);
    c.creation_flags(0x0800_0000);
    first_working(vec![c])
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn play(_custom: &str) -> Result<String, String> {
    Err("sounds are not supported on this system".into())
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn a_missing_sound_file_is_reported_not_played() {
        let (tx, rx) = std::sync::mpsc::channel();
        super::test("/nonexistent/ping.wav", move |text| tx.send(text).unwrap());
        let text = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the test reports back");
        assert!(text.starts_with("Could not play a sound"), "{text}");
    }
}
