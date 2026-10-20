//! Desktop notifications, sent with what each OS already has, so nothing extra has to be installed.

use std::process::{Command, Stdio};

/// Show a notification without blocking; failures are ignored (the in-app alert still shows).
pub fn send(title: &str, body: &str) {
    let title = clean(title, 80);
    let body = clean(body, 240);
    let _ = std::thread::Builder::new().name("notify".into()).spawn(move || {
        let _ = deliver(&title, &body);
    });
}

/// Send a notification after `delay` and report, in words, what happened. Desktops tend to hide
/// notifications from the window you are using, so the delay leaves time to switch away.
pub fn test(delay: std::time::Duration, done: impl FnOnce(String) + Send + 'static) {
    let _ = std::thread::Builder::new().name("notify-test".into()).spawn(move || {
        std::thread::sleep(delay);
        done(match deliver("term", "Notifications are working.") {
            Ok(how) => format!(
                "Sent ({how}). If nothing appeared, your desktop is hiding notifications from the window you are using, \
                 or they are switched off for term in its notification settings."
            ),
            Err(e) => format!("Could not send it: {e}"),
        });
    });
}

fn clean(s: &str, max: usize) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max { format!("{}…", flat.chars().take(max - 1).collect::<String>()) } else { flat }
}

fn quiet(cmd: &mut Command) -> &mut Command {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
}

/// Run a command to completion; Ok(stderr-free success) or the reason it failed.
fn run(cmd: &mut Command, name: &str) -> Result<(), String> {
    match quiet(cmd).stderr(Stdio::piped()).output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Err(if err.is_empty() { format!("{name} exited with {}", o.status) } else { format!("{name}: {err}") })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!("{name} is not installed")),
        Err(e) => Err(format!("{name}: {e}")),
    }
}

#[cfg(target_os = "linux")]
fn deliver(title: &str, body: &str) -> Result<String, String> {
    let first = run(Command::new("notify-send").args(["--app-name=term", "--icon=utilities-terminal", "--", title, body]), "notify-send");
    if first.is_ok() {
        return Ok("notify-send".into());
    }
    // No notify-send: talk to the notification service directly.
    let via_dbus = run(
        Command::new("gdbus").args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.Notifications",
            "--object-path",
            "/org/freedesktop/Notifications",
            "--method",
            "org.freedesktop.Notifications.Notify",
            "term",
            "0",
            "utilities-terminal",
            title,
            body,
            "[]",
            "{}",
            "8000",
        ]),
        "gdbus",
    );
    match via_dbus {
        Ok(()) => Ok("gdbus".into()),
        Err(e) => Err(format!("{}; {e}. Installing libnotify-bin (notify-send) usually fixes this", first.unwrap_err())),
    }
}

#[cfg(target_os = "macos")]
fn deliver(title: &str, body: &str) -> Result<String, String> {
    let script = format!("display notification \"{}\" with title \"{}\"", applescript(body), applescript(title));
    run(Command::new("osascript").args(["-e", &script]), "osascript").map(|()| "osascript".into())
}

#[cfg(windows)]
fn deliver(title: &str, body: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let script = format!(
        "$ErrorActionPreference='Stop';\
         [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null;\
         $x=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02);\
         $n=$x.GetElementsByTagName('text');\
         $n.Item(0).AppendChild($x.CreateTextNode('{}')) | Out-Null;\
         $n.Item(1).AppendChild($x.CreateTextNode('{}')) | Out-Null;\
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x))",
        powershell(title),
        powershell(body)
    );
    // CREATE_NO_WINDOW: no console flashes up.
    run(Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).creation_flags(0x0800_0000), "powershell")
        .map(|()| "PowerShell".into())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn deliver(_title: &str, _body: &str) -> Result<String, String> {
    Err("notifications are not supported on this system".into())
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg_attr(not(windows), allow(dead_code))]
fn powershell(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_made_safe_and_short() {
        assert_eq!(clean("a\nb\t c\x1b[0m", 80), "a b c [0m");
        assert_eq!(clean(&"x".repeat(300), 10).chars().count(), 10);
        assert_eq!(applescript(r#"say "hi" \ now"#), r#"say \"hi\" \\ now"#);
        assert_eq!(powershell("it's"), "it''s");
    }
}
