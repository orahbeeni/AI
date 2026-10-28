//! Minimal timestamped logger on stderr, keeping recent lines for the window's log view.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const KEEP: usize = 2000;
static RECENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// The most recent log lines, oldest first.
pub fn recent() -> Vec<String> {
    RECENT.lock().map(|r| r.iter().cloned().collect()).unwrap_or_default()
}

struct Logger {
    level: log::LevelFilter,
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level && metadata.target().starts_with("wifishare")
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let (h, m, s) = clock((ms / 1000) as i64);
        let level = match record.level() {
            log::Level::Error => "ERROR ",
            log::Level::Warn => "WARN  ",
            log::Level::Info => "",
            log::Level::Debug => "debug ",
            log::Level::Trace => "trace ",
        };
        let line = format!("{h:02}:{m:02}:{s:02}.{:03} {level}{}", ms % 1000, record.args());
        let _ = writeln!(std::io::stderr(), "{line}");
        if let Ok(mut r) = RECENT.lock() {
            if r.len() == KEEP {
                r.pop_front();
            }
            r.push_back(line);
        }
    }

    fn flush(&self) {}
}

/// Local wall-clock time of day (UTC where the C library offers no conversion).
fn clock(secs: i64) -> (i64, i64, i64) {
    #[cfg(unix)]
    {
        let t = secs as libc::time_t;
        // SAFETY: localtime_r only writes into `tm`.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return (tm.tm_hour as i64, tm.tm_min as i64, tm.tm_sec as i64);
        }
    }
    ((secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

/// `verbosity`: 0 = info, 1 = debug, 2+ = trace.
pub fn init(verbosity: u8) {
    let level = match verbosity {
        0 => log::LevelFilter::Info,
        1 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };
    let _ = log::set_boxed_logger(Box::new(Logger { level }));
    log::set_max_level(level);
}
