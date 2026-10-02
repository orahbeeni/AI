//! Turns the options chosen in the UI into a yt-dlp argument list.
//! Pure functions only, so it is fully unit-tested on every OS.

use crate::platform::{self, Os};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WaitRange {
    pub min: u32,
    pub max: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct JobOptions {
    pub url: String,
    pub output_dir: String,
    pub output_template: Option<String>,

    // Quality / format
    pub format: Option<String>,
    pub format_sort: Option<String>,
    pub container: Option<String>,

    // Audio
    pub audio_only: bool,
    pub audio_format: Option<String>,
    pub audio_quality: Option<String>,

    // Live & scheduled
    pub wait_for_video: Option<WaitRange>,
    pub live_from_start: bool,

    // Subtitles
    pub subtitles: Vec<String>,
    pub auto_subs: bool,
    pub embed_subs: bool,
    pub sub_format: Option<String>,

    // Metadata
    pub embed_thumbnail: bool,
    pub embed_metadata: bool,
    pub embed_chapters: bool,
    pub write_thumbnail: bool,
    pub write_info_json: bool,
    pub write_description: bool,

    // Playlist
    pub playlist_items: Option<String>,
    pub no_playlist: bool,
    pub playlist_reverse: bool,
    pub use_archive: bool,
    pub archive_name: Option<String>,

    // SponsorBlock / chapters / clips
    pub sponsorblock_remove: Vec<String>,
    pub sponsorblock_mark: Vec<String>,
    pub split_chapters: bool,
    pub download_sections: Vec<String>,
    pub force_keyframes: bool,

    // Network / auth
    pub cookies_browser: Option<String>,
    pub cookies_file: Option<String>,
    pub rate_limit: Option<String>,
    pub proxy: Option<String>,
    pub retries: Option<u32>,
    pub concurrent_fragments: Option<u32>,

    // Files
    pub restrict_filenames: bool,

    /// Anything else, typed the way you would on the command line.
    pub extra_args: Option<String>,
}

/// Machine-specific inputs that are not part of the user's choices.
#[derive(Clone, Debug, Default)]
pub struct ArgContext {
    pub ffmpeg_dir: Option<PathBuf>,
    pub deno_path: Option<PathBuf>,
    pub archive_dir: PathBuf,
}

fn nonempty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn push_kv(args: &mut Vec<String>, flag: &str, val: impl Into<String>) {
    args.push(flag.to_string());
    args.push(val.into());
}

/// Flags that make yt-dlp's output machine-readable. Shared by downloads.
pub fn progress_flags() -> Vec<String> {
    let mut a: Vec<String> = ["--newline", "--no-colors", "--no-update", "--no-quiet"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    push_kv(
        &mut a,
        "--progress-template",
        "download:PRG|%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(progress.fragment_index)s|%(progress.fragment_count)s|%(progress.elapsed)s",
    );
    push_kv(&mut a, "--progress-template", "postprocess:PP|%(progress.postprocessor)s|%(progress.status)s");
    push_kv(&mut a, "--print", "after_move:FILE|%(filepath)s");
    a
}

/// Flags shared by probing and downloading (auth, network, JS runtime).
pub fn common_flags(o: &JobOptions, ctx: &ArgContext) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(dir) = &ctx.ffmpeg_dir {
        push_kv(&mut a, "--ffmpeg-location", dir.to_string_lossy());
    }
    if let Some(deno) = &ctx.deno_path {
        push_kv(&mut a, "--js-runtimes", format!("deno:{}", deno.to_string_lossy()));
    }
    if let Some(b) = nonempty(&o.cookies_browser) {
        push_kv(&mut a, "--cookies-from-browser", b);
    } else if let Some(f) = nonempty(&o.cookies_file) {
        push_kv(&mut a, "--cookies", f);
    }
    if let Some(p) = nonempty(&o.proxy) {
        push_kv(&mut a, "--proxy", p);
    }
    a
}

pub fn build_args(o: &JobOptions, ctx: &ArgContext) -> Vec<String> {
    let mut a = progress_flags();
    a.extend(common_flags(o, ctx));

    if !o.output_dir.trim().is_empty() {
        push_kv(&mut a, "-P", o.output_dir.trim());
    }
    if let Some(t) = nonempty(&o.output_template) {
        push_kv(&mut a, "-o", t);
    }
    if o.restrict_filenames {
        a.push("--restrict-filenames".into());
    }

    // Live / scheduled
    if let Some(w) = &o.wait_for_video {
        let (min, max) = (w.min.max(1), w.max.max(w.min.max(1)));
        let v = if min == max { min.to_string() } else { format!("{min}-{max}") };
        push_kv(&mut a, "--wait-for-video", v);
    }
    if o.live_from_start {
        a.push("--live-from-start".into());
    }

    // Format
    if o.audio_only {
        push_kv(&mut a, "-f", nonempty(&o.format).unwrap_or("bestaudio/best"));
        a.push("-x".into());
        push_kv(&mut a, "--audio-format", nonempty(&o.audio_format).unwrap_or("mp3"));
        if let Some(q) = nonempty(&o.audio_quality) {
            push_kv(&mut a, "--audio-quality", q);
        }
    } else {
        if let Some(f) = nonempty(&o.format) {
            push_kv(&mut a, "-f", f);
        }
        if let Some(s) = nonempty(&o.format_sort) {
            push_kv(&mut a, "-S", s);
        }
        if let Some(c) = nonempty(&o.container) {
            push_kv(&mut a, "--merge-output-format", c);
        }
    }

    // Subtitles
    if !o.subtitles.is_empty() || o.auto_subs {
        a.push("--write-subs".into());
        if o.auto_subs {
            a.push("--write-auto-subs".into());
        }
        let langs = if o.subtitles.is_empty() { "en".to_string() } else { o.subtitles.join(",") };
        push_kv(&mut a, "--sub-langs", langs);
        if let Some(f) = nonempty(&o.sub_format) {
            push_kv(&mut a, "--convert-subs", f);
        }
        if o.embed_subs {
            a.push("--embed-subs".into());
        }
    }

    // Metadata
    for (on, flag) in [
        (o.embed_thumbnail, "--embed-thumbnail"),
        (o.embed_metadata, "--embed-metadata"),
        (o.embed_chapters, "--embed-chapters"),
        (o.write_thumbnail, "--write-thumbnail"),
        (o.write_info_json, "--write-info-json"),
        (o.write_description, "--write-description"),
        (o.split_chapters, "--split-chapters"),
    ] {
        if on {
            a.push(flag.into());
        }
    }

    // Playlist
    if o.no_playlist {
        a.push("--no-playlist".into());
    } else {
        if let Some(i) = nonempty(&o.playlist_items) {
            push_kv(&mut a, "--playlist-items", i);
        }
        if o.playlist_reverse {
            a.push("--playlist-reverse".into());
        }
    }
    if o.use_archive {
        let name = nonempty(&o.archive_name).unwrap_or("default");
        let file = ctx.archive_dir.join(format!("{}.txt", sanitize_component(name)));
        push_kv(&mut a, "--download-archive", file.to_string_lossy());
    }

    // SponsorBlock / clips
    if !o.sponsorblock_remove.is_empty() {
        push_kv(&mut a, "--sponsorblock-remove", o.sponsorblock_remove.join(","));
    }
    if !o.sponsorblock_mark.is_empty() {
        push_kv(&mut a, "--sponsorblock-mark", o.sponsorblock_mark.join(","));
    }
    for s in o.download_sections.iter().filter(|s| !s.trim().is_empty()) {
        push_kv(&mut a, "--download-sections", s.trim());
    }
    if o.force_keyframes && !o.download_sections.is_empty() {
        a.push("--force-keyframes-at-cuts".into());
    }

    // Network
    if let Some(r) = nonempty(&o.rate_limit) {
        push_kv(&mut a, "--limit-rate", r);
    }
    if let Some(r) = o.retries {
        push_kv(&mut a, "--retries", r.to_string());
    }
    if let Some(n) = o.concurrent_fragments.filter(|n| *n > 1) {
        push_kv(&mut a, "--concurrent-fragments", n.to_string());
    }

    if let Some(extra) = nonempty(&o.extra_args) {
        if let Some(parts) = shlex::split(extra) {
            a.extend(parts);
        }
    }

    a.push("--".into());
    a.push(o.url.trim().to_string());
    a
}

/// A file-name-safe version of a user-supplied label (archive names etc.),
/// valid on Windows, macOS and Linux alike.
pub fn sanitize_component(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "default".into()
    } else {
        cleaned.chars().take(80).collect()
    }
}

fn quote_arg(os: Os, s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars().all(|c| c.is_alphanumeric() || "-_./:=,@%+~".contains(c));
    if safe {
        return s.to_string();
    }
    match os {
        Os::Windows => format!("\"{}\"", s.replace('"', "\\\"")),
        _ => format!("'{}'", s.replace('\'', "'\\''")),
    }
}

/// The command line as a person would type it (for the "show command" box).
pub fn display_command_for(os: Os, program: &str, args: &[String]) -> String {
    let mut parts = vec![program.to_string()];
    parts.extend(args.iter().map(|a| quote_arg(os, a)));
    parts.join(" ")
}

/// Same as the real command, minus the machine-readable output plumbing, so the
/// preview is short and copy-pasteable into a terminal.
pub fn preview_command(o: &JobOptions, ctx: &ArgContext) -> String {
    let all = build_args(o, ctx);
    let mut kept = Vec::new();
    let mut i = 0;
    while i < all.len() {
        let is_noise_flag = matches!(
            all[i].as_str(),
            "--newline" | "--no-colors" | "--no-update" | "--no-quiet"
        );
        let is_noise_kv = matches!(all[i].as_str(), "--progress-template" | "--print");
        if is_noise_flag {
            i += 1;
        } else if is_noise_kv {
            i += 2;
        } else {
            kept.push(all[i].clone());
            i += 1;
        }
    }
    display_command_for(platform::os(), "yt-dlp", &kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn ctx() -> ArgContext {
        ArgContext { archive_dir: PathBuf::from("/data/archives"), ..Default::default() }
    }

    /// The exact command this app was built around.
    #[test]
    fn scheduled_live_preset_matches_the_original_command() {
        let o = JobOptions {
            url: "https://www.youtube.com/watch?v=GxCw7cN44fE".into(),
            wait_for_video: Some(WaitRange { min: 30, max: 60 }),
            live_from_start: true,
            format: Some("bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]".into()),
            ..Default::default()
        };
        let cmd = preview_command(&o, &ctx());
        assert_eq!(
            cmd,
            "yt-dlp --wait-for-video 30-60 --live-from-start -f 'bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]' -- 'https://www.youtube.com/watch?v=GxCw7cN44fE'"
        );
    }

    #[test]
    fn audio_only_defaults() {
        let o = JobOptions { url: "u".into(), audio_only: true, ..Default::default() };
        let a = build_args(&o, &ctx());
        assert!(a.windows(2).any(|w| w == ["-f", "bestaudio/best"]));
        assert!(a.contains(&"-x".to_string()));
        assert!(a.windows(2).any(|w| w == ["--audio-format", "mp3"]));
    }

    #[test]
    fn wait_range_is_normalised() {
        let mk = |min, max| {
            let o = JobOptions { url: "u".into(), wait_for_video: Some(WaitRange { min, max }), ..Default::default() };
            build_args(&o, &ctx())
        };
        assert!(mk(30, 30).windows(2).any(|w| w == ["--wait-for-video", "30"]));
        assert!(mk(60, 10).windows(2).any(|w| w == ["--wait-for-video", "60"]));
        assert!(mk(0, 0).windows(2).any(|w| w == ["--wait-for-video", "1"]));
    }

    #[test]
    fn url_is_last_and_protected_from_flag_injection() {
        let o = JobOptions { url: "--exec evil".into(), ..Default::default() };
        let a = build_args(&o, &ctx());
        let n = a.len();
        assert_eq!(a[n - 2], "--");
        assert_eq!(a[n - 1], "--exec evil");
    }

    #[test]
    fn subtitles_and_sponsorblock() {
        let o = JobOptions {
            url: "u".into(),
            subtitles: vec!["en".into(), "fr".into()],
            embed_subs: true,
            sub_format: Some("srt".into()),
            sponsorblock_remove: vec!["sponsor".into(), "intro".into()],
            ..Default::default()
        };
        let a = build_args(&o, &ctx());
        assert!(a.windows(2).any(|w| w == ["--sub-langs", "en,fr"]));
        assert!(a.contains(&"--embed-subs".to_string()));
        assert!(a.windows(2).any(|w| w == ["--sponsorblock-remove", "sponsor,intro"]));
    }

    #[test]
    fn archive_name_cannot_escape_the_archive_dir() {
        let o = JobOptions { url: "u".into(), use_archive: true, archive_name: Some("../../etc/passwd".into()), ..Default::default() };
        let a = build_args(&o, &ctx());
        let i = a.iter().position(|s| s == "--download-archive").unwrap();
        let p = PathBuf::from(&a[i + 1]);
        assert_eq!(p.parent().unwrap(), Path::new("/data/archives"));
    }

    #[test]
    fn extra_args_are_split_like_a_shell() {
        let o = JobOptions { url: "u".into(), extra_args: Some("--user-agent \"My Agent\" --no-mtime".into()), ..Default::default() };
        let a = build_args(&o, &ctx());
        assert!(a.windows(2).any(|w| w == ["--user-agent", "My Agent"]));
        assert!(a.contains(&"--no-mtime".to_string()));
    }

    #[test]
    fn clip_sections_force_keyframes_only_with_sections() {
        let o = JobOptions { url: "u".into(), force_keyframes: true, ..Default::default() };
        assert!(!build_args(&o, &ctx()).contains(&"--force-keyframes-at-cuts".to_string()));
        let o = JobOptions { url: "u".into(), force_keyframes: true, download_sections: vec!["*1:00-2:00".into()], ..Default::default() };
        let a = build_args(&o, &ctx());
        assert!(a.windows(2).any(|w| w == ["--download-sections", "*1:00-2:00"]));
        assert!(a.contains(&"--force-keyframes-at-cuts".to_string()));
    }

    #[test]
    fn quoting_differs_per_os() {
        let args = vec!["-f".to_string(), "a b".to_string(), "it's".to_string()];
        assert_eq!(display_command_for(Os::Linux, "yt-dlp", &args), "yt-dlp -f 'a b' 'it'\\''s'");
        assert_eq!(display_command_for(Os::Windows, "yt-dlp", &args), "yt-dlp -f \"a b\" \"it's\"");
    }

    #[test]
    fn sanitize_component_is_cross_platform_safe() {
        assert_eq!(sanitize_component("My Channel: Live?"), "My_Channel__Live_");
        assert_eq!(sanitize_component("..."), "default");
        assert_eq!(sanitize_component("a/b\\c"), "a_b_c");
    }
}
