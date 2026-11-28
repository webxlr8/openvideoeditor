//! The chroma key through the command line (task 5.2): `ove look --view matte` shows the matte that the effect's own
//! matte view shows, without changing the project. Needs `--features media` and the ffmpeg CLI for the fixture
//! (`OVE_FIXTURE_FFMPEG` or `ffmpeg` on PATH); skipped with a message otherwise.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]
#![cfg(feature = "media")]

use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;

use serde_json::{Value, json};

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
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

fn ok(lib: &Path, args: &[&str]) -> Value {
    let out = run(lib, args);
    assert!(out.status.success(), "ove {args:?}\nstderr: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn set_effects(bundle: &Path, item: &str, effects: Value) {
    let mut p =
        ove_store::project::Project::open(bundle, ove_store::project::Mode::Write, Arc::new(ove_core::SystemClock))
            .unwrap();
    let ops: Vec<ove_core::Op> =
        serde_json::from_value(json!([{"op": "updateItem", "id": item, "set": {"effects": effects}}])).unwrap();
    p.apply(ops, ove_core::Source::Cli, None).unwrap();
    p.close().unwrap();
}

#[test]
fn look_view_matte_matches_the_effects_matte_view() {
    let Some(ff) = ffmpeg() else {
        eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    // A green screen with a red box in the middle third.
    let clip = d.path().join("plate.mp4");
    let st = Command::new(ff)
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "color=c=0x1fb838:s=320x180:r=25:d=1"])
        .args(["-vf", "drawbox=x=107:y=60:w=106:h=60:color=0xcc2020:t=fill", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(st.success());
    ok(&lib, &["project", "create", "K"]);
    let added = ok(&lib, &["media", "add", "K", clip.to_str().unwrap()]);
    let item = added["clips"][0].as_str().unwrap().to_string();
    let bundle = lib.join("K.ove");
    let look = |extra: &[&str]| {
        let mut args = vec!["look", "K", "--at", "0.5", "--stats", "--width", "320", "--backend", "cpu"];
        args.extend_from_slice(extra);
        ok(&lib, &args)["frames"][0]["stats"].clone()
    };
    set_effects(&bundle, &item, json!([{"id": "k", "type": "chroma-key", "keyColor": [0.12, 0.72, 0.22]}]));
    let agent = look(&["--view", "matte"]);
    // About a ninth of the frame (the box) is kept: white in the matte.
    let mean = agent["meanLuma"].as_f64().unwrap();
    assert!((0.07..0.16).contains(&mean), "matte mean {mean}: {agent}");
    // The project still shows the result, not the matte.
    assert_ne!(look(&[]), agent);
    // The effect's own matte view gives the same frame.
    set_effects(
        &bundle,
        &item,
        json!([{"id": "k", "type": "chroma-key", "keyColor": [0.12, 0.72, 0.22], "matteView": true}]),
    );
    assert_eq!(look(&[]), agent);
    // The viewer sampler (task 5.2): source colours before grading, under a point or a rectangle.
    let doc = |b: &Path| {
        ove_store::project::Project::open(b, ove_store::project::Mode::ReadOnly, Arc::new(ove_core::SystemClock))
            .unwrap()
            .doc()
            .clone()
    };
    let mut r = ove_render::Renderer::new(Some(&bundle), 180);
    ove_media::init();
    let d0 = doc(&bundle);
    let near = |c: [f32; 3], want: [f32; 3]| c.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.06);
    let box_colour = r.sample_source(&d0, &item, 500_000, [0.5, 0.5, 0.5, 0.5]).unwrap().unwrap();
    assert!(near(box_colour, [0.8, 0.125, 0.125]), "{box_colour:?}");
    let screen = r.sample_source(&d0, &item, 500_000, [0.02, 0.05, 0.2, 0.25]).unwrap().unwrap();
    assert!(near(screen, [0.12, 0.72, 0.22]), "{screen:?}");
    // Graded clips sample the source, not the graded result.
    let mut p =
        ove_store::project::Project::open(&bundle, ove_store::project::Mode::Write, Arc::new(ove_core::SystemClock))
            .unwrap();
    let ops: Vec<ove_core::Op> = serde_json::from_value(json!([{"op": "updateItem", "id": item, "set": {"colorGrading": {"saturation": -1}, "transform": {"scale": 0.5}}}])).unwrap();
    p.apply(ops, ove_core::Source::Cli, None).unwrap();
    p.close().unwrap();
    let d1 = doc(&bundle);
    let graded = r.sample_source(&d1, &item, 500_000, [0.5, 0.5, 0.5, 0.5]).unwrap().unwrap();
    assert!(near(graded, [0.8, 0.125, 0.125]), "{graded:?}");
    // Outside the (now half-size) layer: nothing.
    assert_eq!(r.sample_source(&d1, &item, 500_000, [0.05, 0.05, 0.05, 0.05]).unwrap(), None);
    assert!(r.sample_source(&d1, &item, 5_000_000, [0.5, 0.5, 0.5, 0.5]).is_err(), "not on screen then");
    // An unknown view is refused.
    assert_eq!(run(&lib, &["look", "K", "--at", "0.5", "--stats", "--view", "alpha"]).status.code(), Some(1));
}
