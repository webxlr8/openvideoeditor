//! The shot sampler on a fixture of concatenated clips with known cuts (add-semantic-media-search task 2.1), and the
//! cut list against scene detection (task 2.3). Needs the ffmpeg CLI to make the media (`OVE_FIXTURE_FFMPEG` or
//! `ffmpeg` on PATH); skipped with a message otherwise.
#![cfg(feature = "media")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ove_vision::sampler::{SAMPLE_WIDTH, decode_at, plan, points_for, survey};

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
}

macro_rules! need_ffmpeg {
    () => {
        if ffmpeg().is_none() {
            eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
            return;
        }
    };
}

const FPS: f64 = 25.0;

/// Segments of the fixture: a lavfi source and its length in seconds (whole frames at 25 fps).
const SEGMENTS: [(&str, f64); 5] = [
    ("testsrc2=s=320x180:r=25", 3.2),
    ("mandelbrot=s=320x180:r=25", 8.8),
    ("smptehdbars=s=320x180:r=25", 1.4),
    ("color=c=black:s=320x180:r=25", 1.0),
    ("life=s=320x180:r=25:mold=10:ratio=0.3:seed=7", 5.6),
];

/// The concatenated fixture and its true cut times.
pub fn shots_clip(dir: &Path) -> (PathBuf, Vec<f64>) {
    let out = dir.join("shots.mp4");
    let mut cmd = Command::new(ffmpeg().unwrap());
    cmd.args(["-loglevel", "error", "-y"]);
    for (src, d) in SEGMENTS {
        cmd.args(["-f", "lavfi", "-t", &d.to_string(), "-i", src]);
    }
    let inputs: String = (0..SEGMENTS.len()).map(|i| format!("[{i}:v]format=yuv420p,setsar=1[v{i}];")).collect();
    let concat: String = (0..SEGMENTS.len()).map(|i| format!("[v{i}]")).collect();
    let filter = format!("{inputs}{concat}concat=n={}:v=1:a=0", SEGMENTS.len());
    cmd.args(["-filter_complex", &filter, "-pix_fmt", "yuv420p", "-c:v", "libx264", "-crf", "18", "-g", "50"]);
    let st = cmd.arg(&out).status().unwrap();
    assert!(st.success(), "ffmpeg failed");
    let mut cuts = Vec::new();
    let mut t = 0.0;
    for (_, d) in &SEGMENTS[..SEGMENTS.len() - 1] {
        t += d;
        cuts.push((t * 1000.0f64).round() / 1000.0);
    }
    (out, cuts)
}

#[test]
fn cuts_are_frame_accurate_and_samples_follow_the_per_shot_rule() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let (clip, truth) = shots_clip(d.path());
    let t0 = std::time::Instant::now();
    let s = survey(&clip).unwrap();
    let survey_time = t0.elapsed();
    let p = plan(&s);
    eprintln!(
        "survey: {} frames in {survey_time:?} ({:.0} frames/s); cuts {:?}",
        p.frames,
        p.frames as f64 / survey_time.as_secs_f64(),
        p.cuts.iter().map(|c| c.time).collect::<Vec<_>>()
    );
    assert_eq!(p.cuts.len(), truth.len(), "{:?}", p.cuts);
    for (got, want) in p.cuts.iter().zip(&truth) {
        let frames = ((got.time - want) * FPS).abs();
        assert!(frames <= 2.0 + 1e-6, "cut at {} is {frames} frames from {want}", got.time);
    }
    assert_eq!(p.shots.len(), SEGMENTS.len());
    for (i, ((a, b), (src, _))) in p.shots.iter().zip(SEGMENTS).enumerate() {
        let n = p.picks.iter().filter(|k| k.shot as usize == i).count();
        if src.starts_with("color=c=black") {
            assert_eq!(n, 0, "the black shot gets no sample");
        } else {
            assert_eq!(n, points_for(b - a), "shot {i} ({a}–{b}): {n} samples");
        }
        for k in p.picks.iter().filter(|k| k.shot as usize == i) {
            assert!(k.time >= *a && k.time < *b, "pick {} outside shot {a}–{b}", k.time);
        }
    }
    let times: Vec<f64> = p.picks.iter().map(|k| k.time).collect();
    let t0 = std::time::Instant::now();
    let frames = decode_at(&clip, &times).unwrap();
    eprintln!("decoded {} samples at 256 px in {:?}", frames.len(), t0.elapsed());
    assert_eq!(frames.len(), times.len());
    assert!(frames.iter().all(|f| f.width == SAMPLE_WIDTH && f.height == 144), "{:?}", frames[0].width);
}

/// The sampler's cut list is the one scene detection reports at its default threshold, so scene detection can
/// answer from the index.
#[test]
fn the_cut_list_matches_scene_detection() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let (clip, _) = shots_clip(d.path());
    let scenes = ove_analyze::scenes(&clip, 0.3).unwrap();
    let p = plan(&survey(&clip).unwrap());
    assert_eq!(p.cuts, scenes.cuts);
    assert_eq!(p.frames, scenes.frames);
    assert!((p.duration - scenes.duration).abs() < 1e-3);
}
