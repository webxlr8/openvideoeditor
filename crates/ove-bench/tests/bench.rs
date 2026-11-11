//! The benchmark exits 0 and writes JSON valid against `schema.json`; bad input exits 1 with a message.

use std::path::{Path, PathBuf};
use std::process::Command;

fn media(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let p = root.join("tests/media/files").join(name);
    if !p.exists() {
        let st = Command::new("python3").arg(root.join("tests/media/media.py")).arg("generate").status().unwrap();
        assert!(st.success());
    }
    p
}

fn bench(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ove-bench")).args(args).output().unwrap()
}

fn schema() -> jsonschema::Validator {
    let s: serde_json::Value = serde_json::from_str(include_str!("../schema.json")).expect("schema.json parses");
    jsonschema::validator_for(&s).expect("schema compiles")
}

#[test]
fn report_matches_schema() {
    let v = schema();
    for name in ["h264_longgop.mp4", "vfr_phone.mp4", "prores_hq.mov"] {
        let path = media(name);
        let out = bench(&[path.to_str().unwrap(), "--seconds", "1.5", "--seeks", "20"]);
        assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let errors: Vec<String> = v.iter_errors(&json).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{name}: {errors:?}\n{json:#}");
        assert_eq!(json["seekUs"]["count"], 20);
        assert_eq!(json["seekUs"]["exact"], true, "{name}");
        assert!(json["presentedFrames"].as_u64().unwrap() > 20, "{name}: {json:#}");
        assert_eq!(json["media"]["variableFrameRate"], name == "vfr_phone.mp4");
    }
}

#[test]
fn writes_to_file() {
    let dir = std::env::temp_dir().join(format!("ove-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out_path = dir.join("r.json");
    let path = media("prores_proxy.mov");
    let out = bench(&[path.to_str().unwrap(), "--seconds", "0.5", "--seeks", "5", "--out", out_path.to_str().unwrap()]);
    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    assert!(schema().is_valid(&json));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failures_exit_nonzero_with_message() {
    let out = bench(&["/no/such/file.mov"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not exist"));
    let out = bench(&[media("truncated_nomoov.mp4").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let out = bench(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    let out = bench(&[]);
    assert_eq!(out.status.code(), Some(2));
}
