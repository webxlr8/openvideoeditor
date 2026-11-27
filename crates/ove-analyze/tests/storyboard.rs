//! Storyboard of a 10-minute clip: at most 36 tiles, and every shot that scene detection reports has a tile.
//! Needs an FFmpeg command line with test sources to make the clip (`OVE_FIXTURE_FFMPEG`); skipped without one.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::process::Command;

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let ok = Command::new(&bin)
        .args(["-hide_banner", "-f", "lavfi", "-i", "color=c=black:s=16x16:d=0.1", "-f", "null", "-"])
        .output()
        .ok()
        .is_some_and(|o| o.status.success());
    ok.then_some(bin)
}

#[test]
fn ten_minutes_fit_36_tiles_with_one_per_shot() {
    let Some(ff) = ffmpeg() else {
        eprintln!("skipped: no ffmpeg command line with test sources (set OVE_FIXTURE_FFMPEG)");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let clip = d.path().join("ten.mp4");
    // Nine shots of different lengths (600 s in all), each a different colour.
    let shots = [
        ("black", 40),
        ("white", 95),
        ("red", 55),
        ("green", 80),
        ("blue", 45),
        ("yellow", 70),
        ("gray", 60),
        ("magenta", 75),
        ("cyan", 80),
    ];
    let mut args: Vec<String> = vec!["-loglevel".into(), "error".into(), "-y".into()];
    for (c, s) in shots {
        args.extend(["-f", "lavfi", "-i"].map(String::from));
        args.push(format!("color=c={c}:s=160x90:r=10:d={s}"));
    }
    let inputs: String = (0..shots.len()).map(|i| format!("[{i}:v]")).collect();
    args.push("-filter_complex".into());
    args.push(format!("{inputs}concat=n={}:v=1:a=0[v]", shots.len()));
    args.extend(["-map", "[v]", "-g", "50", "-pix_fmt", "yuv420p"].map(String::from));
    args.push(clip.display().to_string());
    let st = Command::new(&ff).args(&args).status().unwrap();
    assert!(st.success(), "ffmpeg could not make the fixture");

    let scenes = ove_analyze::scenes(&clip, 0.3).unwrap();
    assert!(scenes.duration > 590.0, "{}", scenes.duration);
    let (sheet, tiles) = ove_analyze::storyboard::media(&clip, 160).unwrap();
    assert!(tiles.len() <= 36, "{} tiles", tiles.len());
    assert!(sheet.width > 0 && sheet.height > 0);
    let mut bounds: Vec<f64> = vec![0.0];
    bounds.extend(scenes.cuts.iter().map(|c| c.time));
    bounds.push(scenes.duration + 1.0);
    assert_eq!(bounds.len() - 1, shots.len(), "scene detection found the {} shots: {:?}", shots.len(), scenes.cuts);
    for w in bounds.windows(2) {
        assert!(
            tiles.iter().any(|t| t.at >= w[0] - 0.05 && t.at < w[1]),
            "no tile in the shot {:.1}–{:.1} s; tiles at {:?}",
            w[0],
            w[1],
            tiles.iter().map(|t| t.at).collect::<Vec<_>>()
        );
    }
    println!("10-minute clip: {} tiles, {} shots", tiles.len(), shots.len());
}
