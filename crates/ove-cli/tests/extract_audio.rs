//! `ove media extract-audio` and `ove export --audio-only --loudness` (task 6.4 and 6.3 surfaces). Needs
//! `--features media` and the ffmpeg CLI for fixtures (`OVE_FIXTURE_FFMPEG` or `ffmpeg`; skipped otherwise).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]
#![cfg(feature = "media")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use serde_json::{Value, json};

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

fn ove(lib: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .args(args)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap()
}

fn json_of(o: &Output) -> Value {
    // Exports stream progress lines first; the result is the last line.
    let text = String::from_utf8_lossy(&o.stdout);
    serde_json::from_str(text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("")).unwrap_or_else(|_| {
        panic!("stdout {}\nstderr {}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    })
}

fn media_count(lib: &Path) -> usize {
    json_of(&ove(lib, &["media", "list", "Demo", "--all", "--json"]))["media"].as_array().unwrap().len()
}

#[test]
fn extract_audio_adds_one_entry_undo_removes_it_and_silent_video_is_refused() {
    if ffmpeg().is_none() {
        eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let with = make(
        d.path(),
        "talk.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=160x90:r=25:d=3",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ],
    );
    let without = make(
        d.path(),
        "mute.mp4",
        &["-f", "lavfi", "-i", "testsrc=s=160x90:r=25:d=2", "-c:v", "libx264", "-pix_fmt", "yuv420p"],
    );
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    library.create("Demo").unwrap().close().unwrap();
    let o = ove(&lib, &["media", "add", "Demo", with.to_str().unwrap(), without.to_str().unwrap(), "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(media_count(&lib), 2);
    // Extract as WAV: one new audio entry with the source's duration, inside the bundle.
    let o = ove(&lib, &["media", "extract-audio", "Demo", "talk.mp4", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v = json_of(&o);
    let path = PathBuf::from(v["path"].as_str().unwrap());
    assert!(path.is_file() && path.extension().unwrap() == "wav", "{v}");
    assert!(path.parent().unwrap().ends_with("media"));
    let list = json_of(&ove(&lib, &["media", "list", "Demo", "--all", "--json"]));
    let entry = list["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == v["media"])
        .unwrap_or_else(|| panic!("no {} in {list}", v["media"]))
        .clone();
    let src = list["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"].as_str().unwrap_or("").contains("talk"))
        .unwrap()
        .clone();
    eprintln!("extracted {entry}\nfrom {src}");
    let dur = |m: &Value| m["duration"].as_f64().or_else(|| m["durationSeconds"].as_f64()).unwrap_or(f64::NAN);
    assert!((dur(&entry) - 3.0).abs() <= 0.05, "duration {}", dur(&entry));
    assert_eq!(media_count(&lib), 3);
    // Undo removes the entry.
    assert_eq!(ove(&lib, &["undo", "Demo"]).status.code(), Some(0));
    assert_eq!(media_count(&lib), 2);
    // No audio stream: refused, nothing added.
    let o = ove(&lib, &["media", "extract-audio", "Demo", "mute.mp4", "--json"]);
    assert_eq!(o.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(e["error"]["code"], "media.noAudio");
    assert!(e["error"]["message"].as_str().unwrap().contains("no audio"));
    assert_eq!(media_count(&lib), 2);
    // Audio-only export with a loudness target from the command line.
    let out = d.path().join("mix.wav");
    let o = ove(&lib, &["export", "Demo", out.to_str().unwrap(), "--audio-only", "--loudness", "-16", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let r = json_of(&o);
    assert_eq!(r["container"], "wav");
    assert_eq!(r["loudness"]["targetLufs"], json!(-16.0));
    assert!((r["loudness"]["afterLufs"].as_f64().unwrap() + 16.0).abs() <= 0.5, "{r}");
    let o = ove(&lib, &["audio", "loudness", "Demo", "--timeline", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(json_of(&o)["timeline"], true);
}
