use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// "5 minutes ago" style string, relative to `now`.
pub fn relative(time: i64, now: i64) -> String {
    let d = (now - time).max(0);
    let (n, unit) = match d {
        0..=59 => return "just now".into(),
        60..=3599 => (d / 60, "minute"),
        3600..=86_399 => (d / 3600, "hour"),
        86_400..=2_591_999 => (d / 86_400, "day"),
        2_592_000..=31_535_999 => (d / 2_592_000, "month"),
        _ => (d / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::relative;

    #[test]
    fn formats() {
        assert_eq!(relative(100, 100), "just now");
        assert_eq!(relative(0, 60), "1 minute ago");
        assert_eq!(relative(0, 3 * 86_400), "3 days ago");
        assert_eq!(relative(0, 800 * 86_400), "2 years ago");
    }
}

/// "YYYY-MM-DD HH:MM" in the timezone given by `offset_minutes`.
pub fn absolute(time: i64, offset_minutes: i32) -> String {
    let t = time + offset_minutes as i64 * 60;
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    // Civil-from-days (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

#[cfg(test)]
mod abs_tests {
    use super::absolute;

    #[test]
    fn formats_absolute() {
        assert_eq!(absolute(0, 0), "1970-01-01 00:00");
        assert_eq!(absolute(1_700_000_000, 0), "2023-11-14 22:13");
        assert_eq!(absolute(1_700_000_000, 120), "2023-11-15 00:13");
    }
}
