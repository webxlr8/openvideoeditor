//! Analysis on synthetic media. Needs the ffmpeg CLI to make the media (`OVE_FIXTURE_FFMPEG` or `ffmpeg` on PATH);
//! skipped with a message otherwise.

use std::path::{Path, PathBuf};
use std::process::Command;

use ove_analyze::{FilmstripOpts, frame_stats};
use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use serde_json::json;

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
}

fn make(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let st = Command::new(ffmpeg().unwrap()).args(["-loglevel", "error", "-y"]).args(args).arg(&out).status().unwrap();
    assert!(st.success(), "ffmpeg failed for {name}");
    out
}

macro_rules! need_ffmpeg {
    () => {
        if ffmpeg().is_none() {
            eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
            return;
        }
    };
}

/// 1 s of a moving test pattern, then 1 s of solid blue, 320×180 at 25 fps.
fn cut_clip(dir: &Path) -> PathBuf {
    make(
        dir,
        "cut.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=320x180:r=25:d=1",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x180:r=25:d=1",
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-crf",
            "16",
        ],
    )
}

#[test]
fn scenes_find_the_hard_cut() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let clip = cut_clip(d.path());
    let s = ove_analyze::scenes(&clip, 0.3).unwrap();
    assert_eq!(s.cuts.len(), 1, "{s:?}");
    assert!((s.cuts[0].time - 1.0).abs() <= 0.041, "{s:?}");
    assert!((s.duration - 2.0).abs() < 0.05, "{s:?}");
    assert_eq!(s.frames, 50);
}

#[test]
fn grab_and_filmstrip() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let clip = cut_clip(d.path());
    let img = ove_analyze::grab(&clip, 1.5, Some(160)).unwrap();
    assert_eq!((img.width, img.height), (160, 90));
    let [r, g, b, a] = img.px(80, 45);
    assert!(b > 200 && r < 30 && g < 30 && a == 255, "{:?}", img.px(80, 45));
    // Upscaling works too, and past the end gives the last frame.
    let big = ove_analyze::grab(&clip, 9.0, Some(640)).unwrap();
    assert_eq!((big.width, big.height), (640, 360));
    assert!(big.px(320, 180)[2] > 200);
    let (sheet, times) =
        ove_analyze::filmstrip(&clip, &FilmstripOpts { count: 4, tile_width: 80, ..Default::default() }).unwrap();
    assert_eq!(times, vec![0.25, 0.75, 1.25, 1.75]);
    let gap = ove_analyze::image::SHEET_GAP;
    assert_eq!((sheet.width, sheet.height), (4 * 80 + 5 * gap, 45 + 2 * gap));
    // Last tile is blue.
    assert!(sheet.px(gap + 3 * (80 + gap) + 40, gap + 22)[2] > 200);
    assert_eq!(&sheet.png()[1..4], b"PNG");
}

#[test]
fn loudness_levels_and_silence() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    // Stereo 1 kHz sine at −20 dBFS peak for 10 s: −20 LUFS by BS.1770 calibration (997 Hz mono 0 dBFS = −3.01).
    let tone = make(
        d.path(),
        "tone.wav",
        &["-f", "lavfi", "-i", "aevalsrc=0.1*sin(2*PI*1000*t)|0.1*sin(2*PI*1000*t):s=48000:d=10", "-c:a", "pcm_f32le"],
    );
    let l = ove_analyze::loudness(&tone).unwrap();
    assert_eq!(l.channels, 2);
    let i = l.integrated_lufs.unwrap();
    assert!((i - -20.0).abs() <= 0.5, "{l:?}");
    assert!((l.sample_peak_dbfs - -20.0).abs() < 0.05, "{l:?}");
    assert!(l.true_peak_dbtp >= l.sample_peak_dbfs && l.true_peak_dbtp < -19.5, "{l:?}");
    assert!(l.loudness_range_lu < 0.5, "{l:?}");
    assert!((l.duration - 10.0).abs() < 0.01);

    // Tone, 1 s of silence, tone.
    let gappy = make(
        d.path(),
        "gappy.wav",
        &["-f", "lavfi", "-i", "aevalsrc=0.5*sin(2*PI*440*t)*(lt(t\\,1)+gte(t\\,2)):s=48000:d=3", "-c:a", "pcm_s16le"],
    );
    let quiet = ove_analyze::silence(&gappy, -50.0, 0.5).unwrap();
    assert_eq!(quiet.len(), 1, "{quiet:?}");
    assert!((quiet[0].start - 1.0).abs() <= 0.011 && (quiet[0].end - 2.0).abs() <= 0.011, "{quiet:?}");
    assert!(ove_analyze::silence(&gappy, -50.0, 1.5).unwrap().is_empty());

    let lv = ove_analyze::levels(&gappy, 3).unwrap();
    assert_eq!(lv.peaks_db.len(), 3);
    assert!((lv.peaks_db[0] - -6.02).abs() < 0.1, "{lv:?}");
    assert!((lv.rms_db[0] - -9.03).abs() < 0.2, "{lv:?}");
    assert!(lv.peaks_db[1] <= -100.0, "{lv:?}");
    let wf = ove_analyze::waveform_png(&gappy, 300, 60).unwrap();
    assert_eq!((wf.width, wf.height), (300, 60));
    // Loud column has a tall bar, the silent middle only the centre line.
    let lit = |x: u32| (0..60).filter(|y| wf.px(x, *y)[2] == 255).count();
    assert!(lit(50) >= 28 && lit(150) <= 2, "{} {}", lit(50), lit(150));

    // A file without audio is a clear error.
    let silent_video = cut_clip(d.path());
    assert_eq!(ove_analyze::loudness(&silent_video).unwrap_err().code(), "media.noAudio");
}

#[test]
fn look_renders_the_timeline() {
    // No media needed: a text item over the empty (black) canvas.
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t", "name": "Text", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "x", "trackId": "t", "type": "text", "startTime": 0, "endTime": 2,
          "textContent": "HELLO", "name": "Title", "fontSize": 200, "color": "#ffffff"}}
    ]))
    .unwrap();
    let doc = apply_batch(&Document::new("p", "Look", 1), &ops).unwrap().doc;
    let frames = ove_analyze::look(&doc, None, &[1.0, 3.0], 320).unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!((frames[0].width, frames[0].height), (320, 180));
    let with_text = frame_stats(&frames[0]);
    let empty = frame_stats(&frames[1]);
    assert!(with_text.mean_luma > 0.005 && with_text.black_fraction < 1.0, "{with_text:?}");
    assert_eq!(empty.black_fraction, 1.0, "{empty:?}");
    assert_eq!(ove_analyze::timeline_times(&doc, 2, None), vec![0.5, 1.5]);
    let s = ove_analyze::sheet(&frames, 2);
    assert_eq!(s.height, 180 + 2 * ove_analyze::image::SHEET_GAP);
}

/// Integrated loudness and LRA agree with FFmpeg's `ebur128` filter on a varying signal (pink noise with a level
/// step and a low tone).
#[test]
fn loudness_matches_ffmpeg_ebur128() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let src = make(
        d.path(),
        "mix.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=color=pink:amplitude=0.3:sample_rate=48000:seed=7:d=12",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=80:sample_rate=48000:d=12",
            "-filter_complex",
            "[0:a]volume='if(lt(t,6),1,0.25)':eval=frame[n];[n][1:a]amix=inputs=2:normalize=0,pan=stereo|c0=c0|c1=0.7*c0",
            "-c:a",
            "pcm_f32le",
        ],
    );
    let out = Command::new(ffmpeg().unwrap())
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(&src)
        .args(["-af", "ebur128", "-f", "null", "-"])
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&out.stderr);
    let summary = &log[log.rfind("Summary:").expect("ebur128 summary")..];
    let value = |key: &str| -> f64 {
        let line = summary.lines().find(|l| l.trim_start().starts_with(key)).unwrap();
        line.split_whitespace().nth(1).unwrap().parse().unwrap()
    };
    let (ff_i, ff_lra) = (value("I:"), value("LRA:"));
    let ours = ove_analyze::loudness(&src).unwrap();
    let i = ours.integrated_lufs.unwrap();
    assert!((i - ff_i).abs() <= 0.2, "ours {i} vs ffmpeg {ff_i}");
    assert!((ours.loudness_range_lu - ff_lra).abs() <= 0.6, "ours {} vs ffmpeg {ff_lra}", ours.loudness_range_lu);
}
