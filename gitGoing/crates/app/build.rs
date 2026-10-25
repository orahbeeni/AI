//! Stamps the binary with when it was built and from which commit, for the About window.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// `(20261025.1432, "2026-10-25 14:32 SAST")`: the build number and a readable time, in local time where `date` is available.
fn build_time() -> (String, String) {
    if !cfg!(windows) {
        if let Ok(o) = Command::new("date").arg("+%Y%m%d.%H%M|%Y-%m-%d %H:%M %Z").output() {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if let Some((number, readable)) = text.split_once('|') {
                    return (number.to_string(), readable.to_string());
                }
            }
        }
    }
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, mo, d, h, mi) = civil(secs);
    (format!("{y:04}{mo:02}{d:02}.{h:02}{mi:02}"), format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02} UTC"))
}

/// Calendar date and time of a Unix timestamp (UTC).
fn civil(secs: i64) -> (i64, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d, rem / 3600, rem % 3600 / 60)
}

fn git(args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn main() {
    let (number, readable) = build_time();
    let commit = git(&["rev-parse", "--short", "HEAD"]).map(|c| {
        let dirty = git(&["status", "--porcelain", "--", "."]).is_some_and(|s| !s.is_empty());
        if dirty { format!("{c}+changes") } else { c }
    });
    println!("cargo:rustc-env=GITGOING_BUILD_NUMBER={number}");
    println!("cargo:rustc-env=GITGOING_BUILD_TIME={readable}");
    println!("cargo:rustc-env=GITGOING_BUILD_COMMIT={}", commit.unwrap_or_default());
}
