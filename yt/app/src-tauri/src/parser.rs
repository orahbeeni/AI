//! Understands what yt-dlp prints. Progress and file paths come from the
//! machine-readable templates set in `args::progress_flags`; the rest is
//! recognised from yt-dlp's human-readable messages.

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Progress(Progress),
    PostProcess { name: String, status: String },
    /// Final path of a finished file.
    File(String),
    /// A new stream started downloading (video, audio, subtitles...).
    Destination(String),
    /// "Waiting for video" or a scheduled-start message.
    Waiting(String),
    /// "[download] Downloading item 3 of 10".
    PlaylistItem { index: u32, total: u32 },
    Error(String),
    Warning(String),
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Progress {
    pub status: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    pub fragment_index: Option<u32>,
    pub fragment_count: Option<u32>,
    pub elapsed: Option<f64>,
}

impl Progress {
    /// 0..=100, or None when the size is unknown (typical for live streams).
    pub fn percent(&self) -> Option<f64> {
        if let Some(t) = self.total.filter(|t| *t > 0) {
            return Some((self.downloaded as f64 / t as f64 * 100.0).min(100.0));
        }
        match (self.fragment_index, self.fragment_count) {
            (Some(i), Some(n)) if n > 0 => Some((i as f64 / n as f64 * 100.0).min(100.0)),
            _ => None,
        }
    }
}

/// Remove terminal escape sequences (yt-dlp emits `\x1b[K` on wait lines even with --no-colors).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn num<T: std::str::FromStr>(s: Option<&&str>) -> Option<T> {
    let s = s?.trim();
    if s.is_empty() || s == "NA" || s == "None" {
        None
    } else {
        // Values like "1234.0" appear for integer fields on some versions.
        s.parse().ok()
    }
}

fn int(s: Option<&&str>) -> Option<u64> {
    num::<u64>(s).or_else(|| num::<f64>(s).filter(|f| *f >= 0.0).map(|f| f as u64))
}

pub fn parse_line(raw: &str) -> Line {
    let line = strip_ansi(raw);
    let line = line.trim();

    if let Some(rest) = line.strip_prefix("PRG|") {
        let f: Vec<&str> = rest.split('|').collect();
        return Line::Progress(Progress {
            status: f.first().map(|s| s.to_string()).unwrap_or_default(),
            downloaded: int(f.get(1)).unwrap_or(0),
            total: int(f.get(2)).or_else(|| int(f.get(3))),
            speed: num::<f64>(f.get(4)),
            eta: int(f.get(5)),
            fragment_index: int(f.get(6)).map(|v| v as u32),
            fragment_count: int(f.get(7)).map(|v| v as u32),
            elapsed: num::<f64>(f.get(8)),
        });
    }
    if let Some(rest) = line.strip_prefix("PP|") {
        let mut it = rest.split('|');
        return Line::PostProcess {
            name: it.next().unwrap_or("").to_string(),
            status: it.next().unwrap_or("").to_string(),
        };
    }
    if let Some(rest) = line.strip_prefix("FILE|") {
        return Line::File(rest.to_string());
    }
    if let Some(rest) = line.strip_prefix("[download] Destination: ") {
        return Line::Destination(rest.to_string());
    }
    if let Some(rest) = line.strip_prefix("[download] Downloading item ") {
        // "3 of 10"
        let mut it = rest.split(" of ");
        if let (Some(a), Some(b)) = (it.next(), it.next()) {
            if let (Ok(index), Ok(total)) = (a.trim().parse(), b.trim().parse()) {
                return Line::PlaylistItem { index, total };
            }
        }
    }
    if let Some(rest) = line.strip_prefix("ERROR:") {
        return Line::Error(rest.trim().to_string());
    }
    if let Some(rest) = line.strip_prefix("WARNING:") {
        return Line::Warning(rest.trim().to_string());
    }
    let lower = line.to_lowercase();
    if lower.starts_with("[wait]")
        || lower.contains("will begin in")
        || lower.contains("will premiere in")
        || lower.contains("premieres in")
        || lower.contains("waiting for video")
        || lower.contains("waiting for ")
    {
        return Line::Waiting(line.trim_start_matches("[wait]").trim().to_string());
    }
    Line::Other(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_progress_line() {
        let l = parse_line("PRG|downloading|64512|223779|NA|10026046.377945753|0|NA|NA|0.019");
        let Line::Progress(p) = l else { panic!("not progress") };
        assert_eq!(p.status, "downloading");
        assert_eq!(p.downloaded, 64512);
        assert_eq!(p.total, Some(223779));
        assert_eq!(p.eta, Some(0));
        assert!((p.percent().unwrap() - 28.83).abs() < 0.1);
    }

    #[test]
    fn uses_estimate_when_total_is_missing() {
        let Line::Progress(p) = parse_line("PRG|downloading|500|NA|1000|NA|NA|NA|NA|1.0") else { panic!() };
        assert_eq!(p.total, Some(1000));
        assert_eq!(p.percent(), Some(50.0));
    }

    #[test]
    fn live_stream_has_no_percent_but_fragments() {
        let Line::Progress(p) = parse_line("PRG|downloading|9000|NA|NA|120000|NA|5|NA|30.5") else { panic!() };
        assert_eq!(p.percent(), None);
        assert_eq!(p.elapsed, Some(30.5));
        let Line::Progress(p) = parse_line("PRG|downloading|9000|NA|NA|NA|NA|5|10|3.0") else { panic!() };
        assert_eq!(p.percent(), Some(50.0));
    }

    #[test]
    fn parses_files_and_post_processing() {
        assert_eq!(parse_line("FILE|/tmp/a b.mp4"), Line::File("/tmp/a b.mp4".into()));
        assert_eq!(
            parse_line("PP|Merger|started"),
            Line::PostProcess { name: "Merger".into(), status: "started".into() }
        );
        assert_eq!(parse_line("[download] Destination: x.f140.m4a"), Line::Destination("x.f140.m4a".into()));
        assert_eq!(parse_line("[download] Downloading item 3 of 10"), Line::PlaylistItem { index: 3, total: 10 });
    }

    #[test]
    fn recognises_waiting_messages_with_ansi() {
        let l = parse_line("[wait] Waiting for 00:00:32 - Press Ctrl+C to try now\u{1b}[K");
        assert_eq!(l, Line::Waiting("Waiting for 00:00:32 - Press Ctrl+C to try now".into()));
        assert!(matches!(parse_line("[youtube] abc: This live event will begin in 2 hours."), Line::Waiting(_)));
        assert!(matches!(parse_line("[youtube] abc: Premieres in 3 days"), Line::Waiting(_)));
    }

    #[test]
    fn errors_and_warnings() {
        assert_eq!(parse_line("ERROR: [youtube] x: Video unavailable"), Line::Error("[youtube] x: Video unavailable".into()));
        assert!(matches!(parse_line("WARNING: something"), Line::Warning(_)));
        assert!(matches!(parse_line("[info] x: Downloading 1 format(s): 18"), Line::Other(_)));
    }
}
