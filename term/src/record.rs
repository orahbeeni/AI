//! Writing a terminal's output to a text file, and the date and time stamps its name uses.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{Settings, config_dir};

pub struct Recorder {
    out: BufWriter<File>,
    pub path: PathBuf,
}

impl Recorder {
    pub fn create(dir: &Path, name: &str) -> std::io::Result<Recorder> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}_{}.log", timestamp(), file_safe(name)));
        let file = File::create(&path)?;
        Ok(Recorder { out: BufWriter::new(file), path })
    }

    pub fn write_lines(&mut self, lines: &[String]) {
        for l in lines {
            let _ = self.out.write_all(l.as_bytes());
            let _ = self.out.write_all(b"\n");
        }
        let _ = self.out.flush();
    }
}

/// Where recordings go.
pub fn record_dir(s: &Settings) -> PathBuf {
    let d = s.record_dir.trim();
    if d.is_empty() { config_dir().join("logs") } else { PathBuf::from(d) }
}

pub fn file_safe(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        let c = if c.is_alphanumeric() || matches!(c, '-' | '.') { c } else { '_' };
        if c != '_' || !s.ends_with('_') {
            s.push(c);
        }
    }
    let s = s.trim_matches('_');
    if s.is_empty() { "terminal".into() } else { s.chars().take(60).collect() }
}

/// Local date and time as `2026-10-16_143005`.
pub fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, mo, d, h, mi, s) = civil(secs + local_offset(secs));
    format!("{y:04}-{mo:02}-{d:02}_{h:02}{mi:02}{s:02}")
}

/// Seconds east of UTC at `secs`.
#[cfg(unix)]
fn local_offset(secs: i64) -> i64 {
    // SAFETY: localtime_r only reads `t` and writes `tm`, both valid for the call.
    unsafe {
        let t: libc::time_t = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() { 0 } else { tm.tm_gmtoff as i64 }
    }
}

#[cfg(not(unix))]
fn local_offset(_secs: i64) -> i64 {
    0
}

/// Calendar date and time of a Unix timestamp (proleptic Gregorian, UTC).
fn civil(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
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
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_000_000_000), (2001, 9, 9, 1, 46, 40));
        assert_eq!(civil(1_709_164_800 + 86_399), (2024, 2, 29, 23, 59, 59));
        assert_eq!(civil(-86_400), (1969, 12, 31, 0, 0, 0));
    }

    #[test]
    fn names() {
        assert_eq!(file_safe("hybris · core/custom"), "hybris_core_custom");
        assert_eq!(file_safe("///"), "terminal");
    }
}
