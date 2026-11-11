//! Executes the examples in docs/cli/timeline.md and checks each command's documented exit code, then runs the
//! success and refusal paths of every command (task 6.1 / 6.2).

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

fn ove(lib: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .current_dir(cwd)
        .arg("--library")
        .arg(lib)
        .args(args)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap()
}

/// A library with project Demo (main track, two clips, one pending placeholder) and files edited.tsx / broken.tsx.
fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let clock = std::sync::Arc::new(ove_core::SystemClock);
    let library = ove_store::Library::open(&lib, clock).unwrap();
    let mut p = library.create("Demo").unwrap();
    let ops: Vec<ove_core::Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "a", "trackId": "main", "type": "video", "startTime": 0, "endTime": 2, "name": "Opening", "pending": true, "pendingDescription": "wide shot"}},
        {"op": "addItem", "item": {"id": "b", "trackId": "main", "type": "text", "startTime": 2, "endTime": 4, "textContent": "Hi", "name": "Title"}}
    ]))
    .unwrap();
    p.apply(ops, ove_core::Source::Gui, None).unwrap();
    p.flush_projection();
    let text = std::fs::read_to_string(p.bundle().join("timeline.tsx")).unwrap();
    p.close().unwrap();
    std::fs::write(d.path().join("edited.tsx"), text.replacen("name=\"Title\"", "name=\"Title card\"", 1)).unwrap();
    std::fs::write(d.path().join("broken.tsx"), text.replacen("start={2}", "start={1 + 1}", 1)).unwrap();
    (d, lib)
}

#[test]
fn documented_examples_run_with_documented_exit_codes() {
    let (d, lib) = setup();
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cli/timeline.md")).unwrap();
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
        let (cmd, comment) = line.split_once('#').expect("example has an `# exit N` comment");
        let want: i32 = comment.trim().strip_prefix("exit ").unwrap().parse().unwrap();
        let args: Vec<&str> = cmd.split_whitespace().skip(1).collect();
        let out = ove(&lib, d.path(), &args);
        assert_eq!(
            out.status.code(),
            Some(want),
            "`{}`\nstdout: {}\nstderr: {}",
            cmd.trim(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if args.contains(&"--json") && want == 0 {
            serde_json::from_slice::<Value>(&out.stdout).expect("one JSON object on stdout");
        }
        ran += 1;
    }
    assert!(ran >= 10, "only {ran} examples found");
}

#[test]
fn apply_json_shape_undo_and_refusals() {
    let (d, lib) = setup();
    let out = ove(&lib, d.path(), &["source", "apply", "Demo", "edited.tsx", "--json"]);
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    for k in ["rev", "added", "updated", "removed", "conflicts", "warnings", "changed"] {
        assert!(v.get(k).is_some(), "missing {k}: {v}");
    }
    assert_eq!(v["updated"], 1);
    let applied_rev = v["rev"].as_i64().unwrap();

    // The bundle's projection follows the new revision.
    let bundle = lib.join("Demo.ove");
    let tsx = std::fs::read_to_string(bundle.join("timeline.tsx")).unwrap();
    assert!(tsx.contains(&format!("rev: {applied_rev}")));
    assert!(tsx.contains("Title card"));

    // Same file again: no revision.
    let v: Value =
        serde_json::from_slice(&ove(&lib, d.path(), &["source", "apply", "Demo", "edited.tsx", "--json"]).stdout)
            .unwrap();
    assert_eq!(v["changed"], false);

    // Refused: errors are reported with positions and nothing changes.
    let out = ove(&lib, d.path(), &["source", "check", "Demo", "broken.tsx", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "check.failed");
    assert_eq!(e["error"]["details"]["diagnostics"][0]["code"], "literal.only");

    // History and undo to a revision.
    let h: Value = serde_json::from_slice(&ove(&lib, d.path(), &["history", "Demo", "--json"]).stdout).unwrap();
    let first = h["revisions"][0]["rev"].as_i64().unwrap();
    let out = ove(&lib, d.path(), &["undo", "Demo", &first.to_string(), "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let shown: Value =
        serde_json::from_slice(&ove(&lib, d.path(), &["source", "print", "Demo", "--json"]).stdout).unwrap();
    assert!(shown["text"].as_str().unwrap().contains("name=\"Title\""));

    // Pruned/old base: the edited file's rev is fine (retained), a fake old rev is refused.
    let stale = std::fs::read_to_string(d.path().join("edited.tsx")).unwrap();
    let rev_line = stale.lines().find(|l| l.contains("rev:")).unwrap().to_string();
    std::fs::write(d.path().join("stale.tsx"), stale.replace(&rev_line, " * rev: 5")).unwrap();
    let out = ove(&lib, d.path(), &["source", "apply", "Demo", "stale.tsx", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "rev.pruned");

    // Lock held by another writer: refused naming the holder.
    let clock = std::sync::Arc::new(ove_core::SystemClock);
    let _held = ove_store::Project::open(&bundle, ove_store::Mode::Write, clock).unwrap();
    let out = ove(&lib, d.path(), &["source", "apply", "Demo", "edited.tsx", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "project.locked");
    assert!(e["error"]["message"].as_str().unwrap().contains("pid"));

    // Unknown project.
    assert_eq!(ove(&lib, d.path(), &["verify", "Nope"]).status.code(), Some(1));
}

#[test]
fn verify_reports_pending_and_exits_1() {
    let (d, lib) = setup();
    let out = ove(&lib, d.path(), &["verify", "Demo", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["details"]["findings"][0]["lint"], "pending");
}
