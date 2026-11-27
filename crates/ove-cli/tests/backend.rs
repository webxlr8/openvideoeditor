//! Forcing the compositing backend from the command line (task 3.3): `ove render --backend cpu|gpu` (the `export`
//! command), `ove look --backend`, and the refusal of an unknown name. Needs `--features media` and the ffmpeg CLI
//! for the fixture (`OVE_FIXTURE_FFMPEG` or `ffmpeg` on PATH); skipped with a message otherwise.
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

use serde_json::Value;

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
        .env_remove("OVE_BACKEND")
        .output()
        .unwrap()
}

fn ok(lib: &Path, args: &[&str]) -> Value {
    let out = run(lib, args);
    assert!(out.status.success(), "ove {args:?}\nstderr: {}", String::from_utf8_lossy(&out.stderr));
    // `export --json` prints progress lines first; the result is the last line.
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(text.lines().last().unwrap()).unwrap()
}

#[test]
fn render_and_look_take_a_forced_backend() {
    let Some(ff) = ffmpeg() else {
        eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let clip = d.path().join("clip.mp4");
    let st = Command::new(ff)
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=320x180:r=25:d=1"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(st.success());
    ok(&lib, &["project", "create", "B"]);
    ok(&lib, &["media", "add", "B", clip.to_str().unwrap()]);

    let out = d.path().join("cpu.mp4");
    let r = ok(&lib, &["render", "B", out.to_str().unwrap(), "--height", "180", "--backend", "cpu"]);
    assert_eq!(r["backend"]["name"], "cpu", "{r}");
    assert!(out.is_file());

    // A GPU, when this machine has one: the output says so.
    let gpu =
        run(&lib, &["render", "B", d.path().join("gpu.mp4").to_str().unwrap(), "--height", "180", "--backend", "gpu"]);
    if gpu.status.success() {
        let text = String::from_utf8_lossy(&gpu.stdout);
        let v: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(v["backend"]["name"], "gpu", "{v}");
        assert!(v["backend"]["adapter"].is_string());
        eprintln!("gpu render on {}", v["backend"]["adapter"]);
    } else {
        let err = String::from_utf8_lossy(&gpu.stderr);
        assert!(err.contains("gpu.unavailable"), "{err}");
        eprintln!("no GPU on this machine: {err}");
    }

    // look: the same choice, reported with the frames.
    let look = ok(&lib, &["look", "B", "--at", "0.5", "--stats", "--width", "320", "--backend", "cpu"]);
    assert_eq!(look["backend"]["name"], "cpu");
    let auto = ok(&lib, &["look", "B", "--at", "0.5", "--stats", "--width", "320"]);
    let name = auto["backend"]["name"].as_str().unwrap();
    assert!(name == "cpu" || name == "gpu", "{auto}");
    assert!(!auto["backend"]["reason"].as_str().unwrap().is_empty());
    let mean = |v: &Value| v["frames"][0]["stats"]["meanLuma"].as_f64().unwrap();
    assert!((mean(&look) - mean(&auto)).abs() < 0.01, "both backends show the same frame");

    // An unknown backend name is refused with the valid names.
    let bad = run(&lib, &["look", "B", "--at", "0.5", "--stats", "--backend", "metal"]);
    assert_eq!(bad.status.code(), Some(1));
    let err = String::from_utf8_lossy(&bad.stderr);
    assert!(err.contains("auto, cpu or gpu"), "{err}");
}
