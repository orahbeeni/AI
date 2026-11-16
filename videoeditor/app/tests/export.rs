//! Real exports with the FFmpeg on this computer. Skipped when there is none.

use cutline::export::{self, Event};
use cutline::log::LogKind;
use cutline::media::{self, MediaInfo};
use cutline::plan::{self, Fit, FormatChoice, Settings, FORMATS, MB};
use cutline::tools::{self, quiet_command, Tools};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cutline-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_clip(tools: &Tools, path: &Path, size: &str, seconds: u32, fps: u32, audio: bool) {
    let mut command = quiet_command(tools.ffmpeg.as_os_str());
    command.args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"]);
    command.arg(format!("testsrc2=size={size}:rate={fps}:duration={seconds}"));
    if audio {
        command.args(["-f", "lavfi", "-i"]).arg(format!("sine=frequency=440:duration={seconds}"));
    }
    command.args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"]);
    if audio {
        command.args(["-c:a", "aac", "-shortest"]);
    }
    command.arg(path);
    assert!(command.status().unwrap().success());
}

fn clips(tools: &Tools, dir: &Path) -> Vec<MediaInfo> {
    let mut log = |_| {};
    let wide = dir.join("a clip wide.mp4");
    make_clip(tools, &wide, "1280x720", 8, 30, true);
    let tall = dir.join("b tall silent.mp4");
    make_clip(tools, &tall, "480x854", 4, 24, false);
    let other = dir.join("c other.mkv");
    make_clip(tools, &other, "640x480", 6, 25, true);
    [wide, tall, other].iter().map(|path| media::probe(tools, path, &mut log).unwrap()).collect()
}

fn run(plan: plan::Plan, tools: &Tools) -> (Vec<(usize, PathBuf, u64, bool)>, Vec<String>) {
    let job = export::start(plan, tools.clone(), Arc::new(|| {}));
    let mut done = Vec::new();
    let mut commands = Vec::new();
    while let Ok(event) = job.events.recv() {
        match event {
            Event::Log(line) if line.kind == LogKind::Command => commands.push(line.text),
            Event::PartDone { part, path, bytes, over_limit } => done.push((part, path, bytes, over_limit)),
            Event::PartFailed { part, message } => panic!("part {part} failed: {message}"),
            Event::Finished { .. } => break,
            _ => {}
        }
    }
    (done, commands)
}

fn settings(out_dir: &Path, format: FormatChoice, limits: Vec<Option<f64>>, frame: Option<(u32, u32)>) -> Settings {
    Settings {
        format,
        frame,
        fit: Fit::Bars,
        limits_mb: limits,
        out_dir: out_dir.to_path_buf(),
        base_name: "result".to_string(),
        removed: Vec::new(),
    }
}

fn listed(ext: &str) -> FormatChoice {
    FormatChoice::Listed(FORMATS.iter().position(|format| format.ext == ext).unwrap())
}

fn probe_out(tools: &Tools, path: &Path) -> MediaInfo {
    media::probe(tools, path, &mut |_| {}).unwrap()
}

#[test]
fn joins_mixed_clips_and_splits_them_under_the_limit() {
    let Some(tools) = tools::locate(None) else { return };
    let dir = scratch("split");
    let sources = clips(&tools, &dir);
    let total: f64 = sources.iter().map(|clip| clip.duration).sum();
    let limits = vec![Some(1.0), Some(1.0), Some(1.0)];
    let chosen = settings(&dir, listed("mp4"), limits, Some((640, 360)));
    let built = plan::build(&sources, &chosen, &tools.encoders, &|path| path.exists());
    assert!(built.problem.is_none(), "{:?}", built.problem);
    let (done, commands) = run(built, &tools);
    assert_eq!(done.len(), 3);
    assert!(commands.iter().any(|command| command.contains("-pass 1")));
    for (_, path, bytes, over) in &done {
        assert!(!over);
        assert!(*bytes as f64 <= 1.0 * MB, "{path:?} is {bytes} bytes");
        let info = probe_out(&tools, path);
        assert_eq!((info.width, info.height), (640, 360));
        assert!(info.has_audio);
        assert!((info.duration - total / 3.0).abs() < 0.25, "{path:?} runs {}", info.duration);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_video_without_a_limit_keeps_every_second() {
    let Some(tools) = tools::locate(None) else { return };
    let dir = scratch("whole");
    let sources = clips(&tools, &dir);
    let total: f64 = sources.iter().map(|clip| clip.duration).sum();
    let chosen = settings(&dir, listed("mkv"), vec![None], None);
    let built = plan::build(&sources, &chosen, &tools.encoders, &|path| path.exists());
    let (done, _) = run(built, &tools);
    let info = probe_out(&tools, &done[0].1);
    assert_eq!((info.width, info.height), (1280, 720), "keeps the largest clip's size");
    assert!((info.duration - total).abs() < 0.3, "runs {} of {total}", info.duration);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn other_formats_export() {
    let Some(tools) = tools::locate(None) else { return };
    let dir = scratch("formats");
    let sources = clips(&tools, &dir);
    for format in ["webm", "avi", "mpg", "wmv", "mov", "ts", "flv", "m4v"] {
        let chosen = settings(&dir, listed(format), vec![Some(2.0)], Some((426, 240)));
        let built = plan::build(&sources, &chosen, &tools.encoders, &|path| path.exists());
        let (done, _) = run(built, &tools);
        assert!(!done[0].3, "{format} went over its limit");
        assert!(done[0].2 as f64 <= 2.0 * MB, "{format} is {} bytes", done[0].2);
        assert!(probe_out(&tools, &done[0].1).has_audio, "{format} lost its sound");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_typed_format_works() {
    let Some(tools) = tools::locate(None) else { return };
    let dir = scratch("typed");
    let sources = clips(&tools, &dir);
    let chosen = settings(&dir, FormatChoice::Other("ogv".to_string()), vec![None], Some((426, 240)));
    let built = plan::build(&sources, &chosen, &tools.encoders, &|path| path.exists());
    let (done, _) = run(built, &tools);
    assert!(done[0].1.extension().unwrap() == "ogv");
    assert!(done[0].2 > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn gifs_shrink_to_meet_the_limit() {
    let Some(tools) = tools::locate(None) else { return };
    let dir = scratch("gif");
    let sources = clips(&tools, &dir);
    let chosen = settings(&dir, listed("gif"), vec![Some(3.0)], None);
    let built = plan::build(&sources, &chosen, &tools.encoders, &|path| path.exists());
    let (done, _) = run(built, &tools);
    assert!(!done[0].3, "GIF stayed over: {} bytes", done[0].2);
    assert!(done[0].2 as f64 <= 3.0 * MB);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn removed_stretches_are_not_in_the_result() {
    let Some(tools) = tools::locate(None) else { return };
    let dir = scratch("cuts");
    let sources = clips(&tools, &dir);
    let total: f64 = sources.iter().map(|clip| clip.duration).sum();
    let mut chosen = settings(&dir, listed("mp4"), vec![Some(2.0), Some(2.0)], Some((640, 360)));
    // 3 s out of the first clip, the whole second clip, 1 s off the end of the third.
    chosen.removed = vec![vec![(2.0, 5.0)], vec![(0.0, 100.0)], vec![(5.0, 6.0)]];
    let built = plan::build(&sources, &chosen, &tools.encoders, &|path| path.exists());
    let (done, _) = run(built, &tools);
    let length: f64 = done.iter().map(|(_, path, _, _)| probe_out(&tools, path).duration).sum();
    let expected = total - 3.0 - sources[1].duration - 1.0;
    assert!((length - expected).abs() < 0.4, "got {length}, expected {expected}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_rotated_phone_video_reads_upright() {
    let Some(tools) = tools::locate(None) else { return };
    let dir = scratch("rotated");
    let flat = dir.join("flat.mp4");
    make_clip(&tools, &flat, "640x360", 3, 30, true);
    let rotated = dir.join("rotated.mp4");
    let status = quiet_command(tools.ffmpeg.as_os_str())
        .args(["-y", "-hide_banner", "-loglevel", "error", "-display_rotation", "90", "-i"])
        .arg(&flat)
        .args(["-c", "copy"])
        .arg(&rotated)
        .status()
        .unwrap();
    assert!(status.success());
    let info = probe_out(&tools, &rotated);
    assert_eq!((info.width, info.height), (360, 640));
    let chosen = settings(&dir, listed("mp4"), vec![Some(2.0)], None);
    let built = plan::build(std::slice::from_ref(&info), &chosen, &tools.encoders, &|path| path.exists());
    let (done, _) = run(built, &tools);
    let out = probe_out(&tools, &done[0].1);
    assert_eq!((out.width, out.height), (360, 640));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cancelling_removes_the_unfinished_file() {
    let Some(tools) = tools::locate(None) else { return };
    let dir = scratch("cancel");
    let sources = clips(&tools, &dir);
    let chosen = settings(&dir, listed("mp4"), vec![Some(5.0)], None);
    let built = plan::build(&sources, &chosen, &tools.encoders, &|path| path.exists());
    let output = built.parts[0].output.clone();
    let job = export::start(built, tools.clone(), Arc::new(|| {}));
    let mut asked = false;
    let mut was_cancelled = false;
    while let Ok(event) = job.events.recv() {
        match event {
            Event::Log(line) if line.kind == LogKind::Command && !asked => {
                job.cancel();
                asked = true;
            }
            Event::Finished { cancelled } => {
                was_cancelled = cancelled;
                break;
            }
            _ => {}
        }
    }
    assert!(was_cancelled);
    assert!(!output.exists());
    std::fs::remove_dir_all(dir).unwrap();
}
