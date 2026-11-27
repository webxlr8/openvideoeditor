//! `ove color inspect` on synthetic images (needs `--features media` and the ffmpeg CLI for fixtures:
//! `OVE_FIXTURE_FFMPEG` or `ffmpeg` on PATH; skipped with a message otherwise): repeatable output, the refusals,
//! isolation under a title, and the agent loop that applies `suggestedOps` until the shot matches its reference.
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

fn run(lib: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .arg("--json")
        .args(args)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap()
}

fn ove(lib: &Path, args: &[&str]) -> Value {
    let out = run(lib, args);
    assert!(out.status.success(), "ove {args:?}\nstderr: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn apply(bundle: &Path, ops: Value) {
    let mut p =
        ove_store::project::Project::open(bundle, ove_store::project::Mode::Write, Arc::new(ove_core::SystemClock))
            .unwrap();
    let ops: Vec<ove_core::Op> = serde_json::from_value(ops).unwrap();
    p.apply(ops, ove_core::Source::Cli, None).unwrap();
    p.close().unwrap();
}

/// A library with project Shots: `ref.mp4` (0–2 s) and a warm copy `warm.mp4` (2–4 s) on Main. Returns the
/// temporary folder, the library, the bundle and the two clip ids.
fn setup() -> Option<(tempfile::TempDir, PathBuf, PathBuf, String, String)> {
    if ffmpeg().is_none() {
        eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
        return None;
    }
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let enc = ["-c:v", "libx264", "-crf", "4", "-pix_fmt", "yuv420p", "-t", "2"];
    let reference =
        make(d.path(), "ref.mp4", &[&["-f", "lavfi", "-i", "testsrc2=s=320x180:r=25:d=2"][..], &enc].concat());
    let warm = make(
        d.path(),
        "warm.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x180:r=25:d=2",
            "-vf",
            "lutrgb=r='clip(val*1.1+22,0,255)':g='clip(val*1.03+5,0,255)':b='val*0.8'",
        ]
        .iter()
        .chain(enc.iter())
        .copied()
        .collect::<Vec<_>>(),
    );
    ove(&lib, &["project", "create", "Shots"]);
    let added = ove(&lib, &["media", "add", "Shots", reference.to_str().unwrap(), warm.to_str().unwrap()]);
    let clips = added["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 2, "{added}");
    let bundle = lib.join("Shots.ove");
    Some((d, lib, bundle, clips[0].as_str().unwrap().to_string(), clips[1].as_str().unwrap().to_string()))
}

#[test]
fn inspect_fields_repeatability_and_refusals() {
    let Some((_d, lib, bundle, ref_clip, _)) = setup() else { return };
    let args = ["color", "inspect", "Shots", "--at", "1", "--height", "180"];
    let a = run(&lib, &args);
    let b = run(&lib, &args);
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    assert_eq!(a.stdout, b.stdout, "byte-identical repeat output");
    let v: Value = serde_json::from_slice(&a.stdout).unwrap();
    assert_eq!(v["frame"], 30);
    assert_eq!(v["height"], 180);
    let s = &v["stats"];
    for k in ["mean", "p2", "p98"] {
        assert!(s["luma"][k].is_number(), "luma.{k}: {s}");
    }
    assert_eq!(s["luma"]["histogram"].as_array().unwrap().len(), 16);
    assert!(s["clipping"]["low"].is_number() && s["clipping"]["high"].is_number());
    for z in ["shadows", "midtones", "highlights"] {
        assert!(s["zones"].get(z).is_some(), "zone {z}");
    }
    assert!(s["balance"]["a"].is_number() && s["balance"]["b"].is_number());
    assert!(s["saturation"].as_f64().unwrap() > 0.1, "testsrc2 is colourful: {s}");
    assert_eq!(s["hue"].as_array().unwrap().len(), 12);
    assert!(s["colorful"].as_f64().unwrap() > 10.0);
    assert!(s["skin"].get("deviation").is_some());

    // The plain text report.
    let text = Command::new(env!("CARGO_BIN_EXE_ove"))
        .args(["--library", lib.to_str().unwrap(), "color", "inspect", "Shots", "--at", "1", "--height", "180"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.starts_with("frame 30 at 1.000 s"), "{text}");
    assert!(text.contains("luma mean") && text.contains("skin"), "{text}");

    // Past the end: exit 1, with the timeline duration.
    let late = run(&lib, &["color", "inspect", "Shots", "--at", "60", "--height", "180"]);
    assert_eq!(late.status.code(), Some(1));
    let err = String::from_utf8_lossy(&late.stderr);
    assert!(err.contains("time.outOfRange") && err.contains("4.000 s long"), "{err}");

    // A reference that does not exist: exit 1, naming it.
    let missing = run(&lib, &["color", "inspect", "Shots", "--at", "1", "--reference", "m_nope", "--height", "180"]);
    assert_eq!(missing.status.code(), Some(1));
    let err = String::from_utf8_lossy(&missing.stderr);
    assert!(err.contains("reference.notFound") && err.contains("m_nope"), "{err}");

    // Isolation needs an item; an unknown item is refused.
    assert_eq!(run(&lib, &["color", "inspect", "Shots", "--at", "1", "--isolate"]).status.code(), Some(1));
    assert_eq!(run(&lib, &["color", "inspect", "Shots", "--item", "nope"]).status.code(), Some(1));

    // Isolated item under a title: the readout is the clip's alone.
    let clean = ove(&lib, &["color", "inspect", "Shots", "--at", "1", "--height", "180"]);
    apply(
        &bundle,
        json!([{"op": "addItem", "item": {"id": "title", "trackId": "t_text", "type": "text", "startTime": 0,
               "endTime": 2, "textContent": "BIG TITLE", "fontSize": 420, "color": "#ff00ff",
               "backgroundColor": "#ff00ff"}}]),
    );
    let covered = ove(&lib, &["color", "inspect", "Shots", "--at", "1", "--height", "180"]);
    let alone =
        ove(&lib, &["color", "inspect", "Shots", "--at", "1", "--item", &ref_clip, "--isolate", "--height", "180"]);
    let magenta = |v: &Value| v["stats"]["hue"][10].as_f64().unwrap();
    assert!(magenta(&covered) > magenta(&clean) + 5.0, "the title shows in the composite: {covered}");
    assert!((magenta(&alone) - magenta(&clean)).abs() < 1.0, "isolated: {} vs {}", alone["stats"], clean["stats"]);
    let luma = |v: &Value| v["stats"]["luma"]["mean"].as_f64().unwrap();
    assert!((luma(&alone) - luma(&clean)).abs() < 0.5, "{} vs {}", luma(&alone), luma(&clean));
    assert_eq!(alone["isolate"], true);
}

/// Task 1.4: a mock agent applies `suggestedOps` to the warm copy and inspects again, up to three times; the mean
/// CIEDE2000 gap to the reference falls below 2.
#[test]
fn agent_loop_matches_a_warm_shot() {
    let Some((_d, lib, bundle, ref_clip, warm_clip)) = setup() else { return };
    let inspect = |lib: &Path| {
        ove(
            lib,
            &[
                "color",
                "inspect",
                "Shots",
                "--item",
                &warm_clip,
                "--isolate",
                "--reference",
                &ref_clip,
                "--height",
                "180",
            ],
        )
    };
    let first = inspect(&lib);
    let start = first["gap"]["mean"].as_f64().unwrap();
    assert!(start > 4.0, "the warm shift is visible: {}", first["gap"]);
    assert_eq!(first["reference"]["kind"], "item");
    let mut v = first;
    let mut gaps = vec![start];
    for round in 0..3 {
        let ops = v["suggestedOps"].clone();
        assert_eq!(ops[0]["op"], "updateItem", "{ops}");
        assert_eq!(ops[0]["id"], warm_clip.as_str());
        if round == 0 {
            // The same patch as a text edit for apply_timeline: apply it through the projection.
            let edit = &v["suggestedEdit"];
            let (old, new) =
                (edit["edits"][0]["oldText"].as_str().unwrap(), edit["edits"][0]["newText"].as_str().unwrap());
            let printed = std::fs::read_to_string(bundle.join("timeline.tsx")).unwrap();
            assert_eq!(printed.matches(old).count(), 1, "oldText occurs once in the printed timeline");
            let file = bundle.parent().unwrap().join("edited.tsx");
            std::fs::write(&file, printed.replacen(old, new, 1)).unwrap();
            ove(&lib, &["source", "apply", "Shots", file.to_str().unwrap()]);
        } else {
            apply(&bundle, ops);
        }
        v = inspect(&lib);
        gaps.push(v["gap"]["mean"].as_f64().unwrap());
    }
    // Below 2 within three rounds, and further rounds (through suggestedOps) do not overshoot.
    assert!(gaps[1..].iter().any(|g| *g < 2.0), "gaps per round: {gaps:?}");
    assert!(*gaps.last().unwrap() < 2.0, "gaps per round: {gaps:?}");
    eprintln!("agent loop: mean ΔE00 per round {gaps:?}");
}
