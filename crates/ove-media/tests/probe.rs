//! Task 2.2: probe snapshot tests against `ffprobe` for every matrix file.

mod common;

use std::process::Command;

use common::{matrix, media};
use ove_media::{MediaError, probe};
use serde_json::Value;

fn ffprobe(path: &std::path::Path) -> Value {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_format", "-show_streams", "-of", "json"])
        .arg(path)
        .output()
        .expect("ffprobe on PATH");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn rate(s: &str) -> (i32, i32) {
    let (n, d) = s.split_once('/').unwrap();
    (n.parse().unwrap(), d.parse().unwrap())
}

#[test]
fn probe_matches_ffprobe_for_every_matrix_file() {
    let mut checked = 0;
    for name in matrix() {
        let path = media(&name);
        if name == "truncated_nomoov.mp4" {
            continue;
        }
        let info = probe(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let r = ffprobe(&path);
        let fmt = &r["format"];
        assert_eq!(info.container, fmt["format_name"].as_str().unwrap(), "{name}: container");
        let dur: f64 = fmt["duration"].as_str().unwrap().parse().unwrap();
        let ours = info.duration_us.unwrap() as f64 / 1e6;
        assert!((ours - dur).abs() < 0.001, "{name}: duration {ours} vs {dur}");
        assert_eq!(info.stream_count, r["streams"].as_array().unwrap().len(), "{name}: stream count");

        let streams = r["streams"].as_array().unwrap();
        let v = streams.iter().find(|s| s["codec_type"] == "video");
        match (&info.video, v) {
            (Some(ours), Some(v)) => {
                assert_eq!(ours.codec, v["codec_name"].as_str().unwrap(), "{name}: codec");
                assert_eq!(ours.profile.as_deref(), v["profile"].as_str(), "{name}: profile");
                assert_eq!(ours.width as u64, v["width"].as_u64().unwrap(), "{name}: width");
                assert_eq!(ours.height as u64, v["height"].as_u64().unwrap(), "{name}: height");
                assert_eq!(
                    ours.pixel_format.as_ref().map(|p| p.name.as_str()),
                    v["pix_fmt"].as_str(),
                    "{name}: pix_fmt"
                );
                assert_eq!(
                    (ours.frame_rate.num, ours.frame_rate.den),
                    rate(v["avg_frame_rate"].as_str().unwrap()),
                    "{name}: avg_frame_rate"
                );
                assert_eq!(
                    (ours.base_frame_rate.num, ours.base_frame_rate.den),
                    rate(v["r_frame_rate"].as_str().unwrap()),
                    "{name}: r_frame_rate"
                );
                if let Some(n) = v["nb_frames"].as_str() {
                    assert_eq!(ours.frame_count, Some(n.parse().unwrap()), "{name}: nb_frames");
                }
                assert_eq!(ours.variable_frame_rate, name == "vfr_phone.mp4", "{name}: VFR detection");
            }
            (None, None) => {}
            (a, b) => panic!("{name}: video stream mismatch ours={a:?} ffprobe={b:?}"),
        }

        let a = streams.iter().find(|s| s["codec_type"] == "audio");
        match (&info.audio, a) {
            (Some(ours), Some(a)) => {
                assert_eq!(ours.codec, a["codec_name"].as_str().unwrap(), "{name}: audio codec");
                assert_eq!(ours.profile.as_deref(), a["profile"].as_str(), "{name}: audio profile");
                assert_eq!(ours.sample_rate.to_string(), a["sample_rate"].as_str().unwrap(), "{name}: rate");
                assert_eq!(ours.channels as u64, a["channels"].as_u64().unwrap(), "{name}: channels");
                if let Some(l) = a["channel_layout"].as_str() {
                    assert_eq!(ours.layout, l, "{name}: layout");
                }
                assert_eq!(ours.sample_format, a["sample_fmt"].as_str().unwrap(), "{name}: sample_fmt");
            }
            (None, None) => {}
            (a, b) => panic!("{name}: audio stream mismatch ours={a:?} ffprobe={b:?}"),
        }
        checked += 1;
    }
    assert!(checked >= 17, "checked {checked} files");
}

#[test]
fn probe_reports_plain_errors() {
    let missing = std::path::Path::new("/definitely/not/here.mov");
    assert!(matches!(probe(missing), Err(MediaError::NotFound { .. })));

    let e = probe(&media("truncated_nomoov.mp4")).unwrap_err();
    assert_eq!(e.code(), "media.corrupt", "{e}");
    assert!(e.to_string().contains("truncated_nomoov.mp4"), "message names the file: {e}");

    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("notes.mov");
    std::fs::write(&txt, b"this is not a movie").unwrap();
    let e = probe(&txt).unwrap_err();
    assert!(matches!(e, MediaError::Corrupt { .. } | MediaError::Unsupported { .. }), "{e:?}");
}

#[test]
fn probe_is_fast() {
    // The 20 MB DNxHR MXF and VFR file must probe without reading the whole file.
    for name in ["dnxhr_hq.mxf", "vfr_phone.mp4", "prores_4444.mov"] {
        let path = media(name);
        let t = std::time::Instant::now();
        probe(&path).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        eprintln!("probe {name}: {ms:.1} ms");
        assert!(ms < 200.0, "{name} probe took {ms} ms");
    }
}
