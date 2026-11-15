//! Finding videos that are already saved in the download folder.
//!
//! yt-dlp skips a download whose file exists, but still runs every post-processing step on it,
//! and re-tagging an Opus/MP3 file that already carries cover art fails with "Conversion failed".
//! So on a retry, or when the same playlist is added again, the finished videos are handed to
//! yt-dlp as an "already downloaded" list and it never touches them.

use crate::args::JobOptions;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub id: String,
    pub path: PathBuf,
}

fn is_youtube(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("youtube.com/") || u.contains("youtu.be/")
}

/// The file extensions this job will produce, or none when it can't be known.
fn produced_extensions(o: &JobOptions) -> Vec<&'static str> {
    if o.audio_only {
        match o.audio_format.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("mp3") {
            "mp3" => vec!["mp3"],
            "m4a" => vec!["m4a"],
            "opus" => vec!["opus"],
            "flac" => vec!["flac"],
            "wav" => vec!["wav"],
            "aac" => vec!["aac"],
            "vorbis" => vec!["ogg"],
            _ => vec![],
        }
    } else {
        match o.container.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => vec!["mp4", "mkv", "webm"],
            Some("mp4") => vec!["mp4"],
            Some("mkv") => vec!["mkv"],
            Some("webm") => vec!["webm"],
            Some(_) => vec![],
        }
    }
}

/// "Some title [dQw4w9WgXcQ]" -> "dQw4w9WgXcQ"
pub fn video_id(stem: &str) -> Option<&str> {
    let inner = stem.strip_suffix(']')?;
    let id = &inner[inner.rfind('[')? + 1..];
    (id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then_some(id)
}

/// Complete files in `dir` that this job would only be re-creating. Empty when the job is the
/// kind where that isn't safe to assume (live, clips, chapters, custom names, its own archive…).
pub fn saved_in(dir: &Path, o: &JobOptions) -> Vec<Saved> {
    let custom_args = o.extra_args.as_deref().is_some_and(|a| a.to_ascii_lowercase().contains("overwrite") || a.split_whitespace().any(|w| w == "-w"));
    // A name pattern that still ends in "[id].ext" and has no folders keeps the id in the file name,
    // which is all that is needed to recognise the file.
    let template = o.output_template.as_deref().map(str::trim).unwrap_or("");
    let plain = template.is_empty() || (template.ends_with("[%(id)s].%(ext)s") && !template.contains('/') && !template.contains('\\'));
    if !is_youtube(&o.url)
        || !plain
        || custom_args
        || o.use_archive
        || o.wait_for_video.is_some()
        || o.live_from_start
        || !o.download_sections.is_empty()
        || o.split_chapters
    {
        return vec![];
    }
    let exts = produced_extensions(o);
    if exts.is_empty() {
        return vec![];
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<Saved> = vec![];
    for e in rd.flatten() {
        let path = e.path();
        let (Some(stem), Some(ext)) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|s| s.to_str())) else { continue };
        if !exts.contains(&ext.to_ascii_lowercase().as_str()) {
            continue;
        }
        let Some(id) = video_id(stem) else { continue };
        if e.metadata().map(|m| m.is_file() && m.len() > 0).unwrap_or(false) && !out.iter().any(|s| s.id == id) {
            out.push(Saved { id: id.to_string(), path });
        }
    }
    out
}

/// The text of a yt-dlp download archive listing these videos.
pub fn archive_text(saved: &[Saved]) -> String {
    saved.iter().map(|s| format!("youtube {}\n", s.id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("ytgrab-existing-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn job(url: &str) -> JobOptions {
        JobOptions { url: url.into(), audio_only: true, audio_format: Some("opus".into()), ..Default::default() }
    }

    #[test]
    fn ids_come_from_the_bracket_before_the_extension() {
        assert_eq!(video_id("A title [AROz5X7LBqs]"), Some("AROz5X7LBqs"));
        assert_eq!(video_id("A title [AROz5X7LBqs].temp"), None);
        assert_eq!(video_id("Name [short]"), None);
        assert_eq!(video_id("No brackets"), None);
    }

    #[test]
    fn only_complete_files_of_the_wanted_type_count() {
        let d = dir();
        let w = |n: &str, body: &str| std::fs::write(d.join(n), body).unwrap();
        w("Done one [AAAAAAAAAAA].opus", "x");
        w("Other format [BBBBBBBBBBB].mp3", "x"); // a different audio format is a different download
        w("Half [CCCCCCCCCCC].webm.part", "x");
        w("Temp [DDDDDDDDDDD].temp.opus", "x");
        w("Empty [EEEEEEEEEEE].opus", "");
        w("Done two [FFFFFFFFFFF].opus", "x");
        let mut ids: Vec<String> = saved_in(&d, &job("https://www.youtube.com/@x/videos")).into_iter().map(|s| s.id).collect();
        ids.sort();
        assert_eq!(ids, vec!["AAAAAAAAAAA", "FFFFFFFFFFF"]);
        // Asking for MP3 instead finds the MP3 only.
        let mut o = job("https://youtu.be/x");
        o.audio_format = Some("mp3".into());
        assert_eq!(saved_in(&d, &o).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["BBBBBBBBBBB"]);
        assert_eq!(archive_text(&saved_in(&d, &o)), "youtube BBBBBBBBBBB\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn jobs_where_skipping_would_be_wrong_skip_nothing() {
        let d = dir();
        std::fs::write(d.join("T [AAAAAAAAAAA].opus"), "x").unwrap();
        let base = job("https://www.youtube.com/watch?v=x");
        assert_eq!(saved_in(&d, &base).len(), 1);
        assert!(saved_in(&d, &JobOptions { url: "https://vimeo.com/1".into(), ..base.clone() }).is_empty(), "other sites: ids unknown");
        assert!(saved_in(&d, &JobOptions { output_template: Some("%(title)s.%(ext)s".into()), ..base.clone() }).is_empty());
        // Still recognisable: the id stays at the end of the name, whatever comes before it.
        std::fs::write(d.join("2026-01-02 - T [BBBBBBBBBBB].opus"), "x").unwrap();
        let dated = saved_in(&d, &JobOptions { output_template: Some(crate::args::DEFAULT_TEMPLATE.into()), ..base.clone() });
        assert_eq!(dated.len(), 2, "old and dated names are both found");
        assert!(saved_in(&d, &JobOptions { output_template: Some("%(uploader)s/%(title)s [%(id)s].%(ext)s".into()), ..base.clone() }).is_empty(), "folders: unknown place");
        assert!(saved_in(&d, &JobOptions { use_archive: true, ..base.clone() }).is_empty(), "has its own archive");
        assert!(saved_in(&d, &JobOptions { live_from_start: true, ..base.clone() }).is_empty());
        assert!(saved_in(&d, &JobOptions { split_chapters: true, ..base.clone() }).is_empty());
        assert!(saved_in(&d, &JobOptions { audio_format: Some("best".into()), ..base.clone() }).is_empty(), "extension unknown");
        let _ = std::fs::remove_dir_all(&d);
    }
}
