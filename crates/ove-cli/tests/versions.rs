//! add-markers-keyframe-lanes-and-versions 4.3: `ove version save|list|print|diff|restore`, and the documented
//! examples in docs/cli/versions.md with their exit codes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::project::{Mode, Project};
use serde_json::{Value, json};

fn ove(lib: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .args(args)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap()
}

fn ok(o: &std::process::Output) -> Value {
    assert!(o.status.success(), "{} / {}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    p.apply(
        vec![
            Op::AddTrack { track: json!({"id": "t", "name": "Main", "order": 0, "type": "video"}) },
            Op::AddItem {
                item: json!({"id": "a", "trackId": "t", "type": "text", "startTime": 0, "endTime": 4, "name": "Intro"}),
            },
            Op::AddItem { item: json!({"id": "b", "trackId": "t", "type": "text", "startTime": 4, "endTime": 8}) },
        ],
        Source::Cli,
        None,
    )
    .unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    (d, lib, bundle)
}

fn trim_a(bundle: &Path, end: f64) {
    let mut p = Project::open(bundle, Mode::Write, Arc::new(ove_core::SystemClock)).unwrap();
    let set = json!({"endTime": end}).as_object().unwrap().clone();
    p.apply(vec![Op::UpdateItem { id: "a".into(), set }], Source::Gui, None).unwrap();
    p.close().unwrap();
}

fn a_end(bundle: &Path) -> i64 {
    Project::open(bundle, Mode::ReadOnly, Arc::new(ove_core::SystemClock)).unwrap().doc().item("a").unwrap().end
}

#[test]
fn save_list_diff_print_and_restore() {
    let (_d, lib, bundle) = setup();
    ok(&ove(&lib, &["--json", "version", "save", "Demo", "v1", "--note", "first cut"]));
    trim_a(&bundle, 3.0);
    ok(&ove(&lib, &["--json", "version", "save", "Demo", "v2"]));
    let list = ok(&ove(&lib, &["--json", "version", "list", "Demo"]));
    let names: Vec<&str> = list["versions"].as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["v1", "v2"]);
    assert_eq!(list["versions"][0]["note"], "first cut");

    // The spec scenario: `ove version diff <project> "v1" "v2" --json` prints the classified changes and exits 0.
    let diff = ok(&ove(&lib, &["version", "diff", "Demo", "v1", "v2", "--json"]));
    assert_eq!(diff["changes"].as_array().unwrap().len(), 1);
    let c = &diff["changes"][0];
    assert_eq!(
        (c["entity"].as_str(), c["id"].as_str(), c["class"].as_str()),
        (Some("item"), Some("a"), Some("trimmed"))
    );
    assert_eq!((c["before"]["end"].as_f64(), c["after"]["end"].as_f64()), (Some(4.0), Some(3.0)));

    let print = ok(&ove(&lib, &["--json", "version", "print", "Demo", "v1"]));
    assert!(print["text"].as_str().unwrap().contains("end={4}"));

    let dup = ove(&lib, &["--json", "version", "save", "Demo", "v1"]);
    assert_eq!(dup.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&dup.stderr).unwrap();
    assert_eq!(e["error"]["code"], "version.exists");

    ok(&ove(&lib, &["--json", "version", "restore", "Demo", "v1"]));
    assert_eq!(a_end(&bundle), 4_000_000);
    // One undo returns to before the restore.
    ok(&ove(&lib, &["--json", "undo", "Demo"]));
    assert_eq!(a_end(&bundle), 3_000_000);
}

/// Runs every `ove …  # exit N` line of the ```sh block in docs/cli/versions.md, trimming `a` after the first save as
/// the page describes.
#[test]
fn documented_examples_run_with_documented_exit_codes() {
    let (_d, lib, bundle) = setup();
    let doc =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/cli/versions.md")).unwrap();
    let mut in_sh = false;
    let mut ran = 0;
    for line in doc.lines() {
        if line.starts_with("```") {
            in_sh = line == "```sh";
            continue;
        }
        if !in_sh || !line.starts_with("ove ") {
            continue;
        }
        let (cmd, comment) = line.split_once('#').unwrap();
        let want: i32 = comment.trim().strip_prefix("exit ").unwrap().parse().unwrap();
        let args = shell_words(cmd);
        let out = ove(&lib, &args.iter().skip(1).map(String::as_str).collect::<Vec<_>>());
        assert_eq!(out.status.code(), Some(want), "`{}`\n{}", cmd.trim(), String::from_utf8_lossy(&out.stderr));
        if ran == 0 {
            trim_a(&bundle, 3.0);
        }
        ran += 1;
    }
    assert_eq!(ran, 7);
}

/// Splits a command line on spaces, keeping "double-quoted" words together.
fn shell_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in s.trim().chars() {
        match ch {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
