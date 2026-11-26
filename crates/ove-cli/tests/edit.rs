//! `ove edit` and MCP `edit_timeline` (add-edit-planners-and-links 5.4): success, refusal, dry run, stale-revision
//! rebase, and the MCP contract.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::project::{Mode, Project};
use serde_json::{Value, json};

fn ove(lib: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .args(args)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap()
}

fn out_json(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)))
}

fn err_json(o: &Output) -> Value {
    serde_json::from_slice(&o.stderr).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stderr)))
}

/// Project Demo at 25 fps: v1 has a [0,4) b [4,8) c [8,12); a1 holds b's linked sound; music [0,12) on a track with
/// sync lock off; a locked titles track with t [10,12).
fn setup(title_locked: bool) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "updateProject", "set": {"fps": 25}},
        {"op": "addTrack", "track": {"id": "titles", "name": "Titles", "order": 0, "type": "video", "locked": title_locked}},
        {"op": "addTrack", "track": {"id": "v1", "name": "V1", "order": 1, "type": "video"}},
        {"op": "addTrack", "track": {"id": "a1", "name": "A1", "order": 2, "type": "audio"}},
        {"op": "addTrack", "track": {"id": "music", "name": "Music", "order": 3, "type": "audio", "syncLock": false}},
        {"op": "addItem", "item": {"id": "t", "trackId": "titles", "type": "text", "startTime": 10, "endTime": 12, "textContent": "End"}},
        {"op": "addItem", "item": {"id": "a", "trackId": "v1", "type": "text", "startTime": 0, "endTime": 4, "textContent": "A"}},
        {"op": "addItem", "item": {"id": "b", "trackId": "v1", "type": "text", "startTime": 4, "endTime": 8, "textContent": "B", "link": "L"}},
        {"op": "addItem", "item": {"id": "bs", "trackId": "a1", "type": "text", "startTime": 4, "endTime": 8, "textContent": "B sound", "link": "L"}},
        {"op": "addItem", "item": {"id": "c", "trackId": "v1", "type": "text", "startTime": 8, "endTime": 12, "textContent": "C"}},
        {"op": "addItem", "item": {"id": "mu", "trackId": "music", "type": "text", "startTime": 0, "endTime": 12, "textContent": "M"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    p.close().unwrap();
    (d, lib)
}

fn open(lib: &Path) -> Project {
    let library = ove_store::Library::open(lib, Arc::new(ove_core::SystemClock)).unwrap();
    Project::open(&library.find("Demo").unwrap(), Mode::ReadOnly, Arc::new(ove_core::SystemClock)).unwrap()
}

fn start(p: &Project, id: &str) -> i64 {
    p.doc().item(id).unwrap().start
}

#[test]
fn ripple_delete_applies_one_revision() {
    let (_d, lib) = setup(false);
    let rev0 = open(&lib).rev();
    let o = ove(&lib, &["--json", "edit", "ripple-delete", "Demo", "--items", "b"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v = out_json(&o);
    assert_eq!(v["verb"], "ripple-delete");
    assert!(v["timeMap"]["tracks"]["v1"].is_array(), "{v}");
    let p = open(&lib);
    assert!(p.rev() > rev0);
    assert!(p.doc().item("b").is_none() && p.doc().item("bs").is_none(), "the linked sound goes too");
    assert_eq!(start(&p, "c"), 4_000_000);
    assert_eq!(start(&p, "t"), 6_000_000, "the sync-locked titles track follows");
    assert_eq!(start(&p, "mu"), 0, "the music track has sync lock off");
    let h = out_json(&ove(&lib, &["--json", "history", "Demo"]));
    let revs = h["revisions"].as_array().unwrap();
    assert_eq!(revs.len(), 2, "one revision for the whole ripple");
    assert_eq!(revs[1]["source"], "cli");
}

#[test]
fn dry_run_prints_the_plan_and_changes_nothing() {
    // Spec scenario "CLI dry run".
    let (_d, lib) = setup(false);
    let rev0 = open(&lib).rev();
    let o =
        ove(&lib, &["edit", "ripple-delete", "Demo", "--track", "a1", "--ranges", "5.0-6.0", "--dry-run", "--json"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v = out_json(&o);
    assert_eq!(v["dryRun"], true);
    assert!(!v["ops"].as_array().unwrap().is_empty());
    assert_eq!(open(&lib).rev(), rev0, "no new revision");
}

#[test]
fn a_blocked_ripple_is_refused_with_its_culprits() {
    let (_d, lib) = setup(true);
    let rev0 = open(&lib).rev();
    let o = ove(&lib, &["--json", "edit", "ripple-delete", "Demo", "--items", "b"]);
    assert_eq!(o.status.code(), Some(1));
    let e = err_json(&o);
    assert_eq!(e["error"]["code"], "ripple.blocked");
    assert_eq!(e["error"]["details"]["culprits"], json!(["t"]));
    assert_eq!(open(&lib).rev(), rev0);
    // A usage problem is a refusal too.
    let o = ove(&lib, &["--json", "edit", "trim", "Demo", "--item", "c", "--to", "9"]);
    assert_eq!(err_json(&o)["error"]["code"], "argument.missing");
}

#[test]
fn a_stale_request_is_rebased_not_refused() {
    // Spec scenario "Stale revision is rebased, not refused": a split of c at 10 s asked at the revision before a
    // ripple that moved c 4 s earlier lands at 6 s.
    let (_d, lib) = setup(false);
    let old = open(&lib).rev();
    assert!(ove(&lib, &["edit", "ripple-delete", "Demo", "--items", "b"]).status.success());
    let o = ove(&lib, &["--json", "edit", "split", "Demo", "--item", "c", "--at", "10", "--rev", &old.to_string()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let p = open(&lib);
    let c = p.doc().item("c").unwrap();
    assert_eq!((c.start, c.end), (4_000_000, 6_000_000));
    // A revision newer than the project is refused.
    let o = ove(
        &lib,
        &["--json", "edit", "split", "Demo", "--item", "c", "--at", "5", "--rev", &(p.rev() + 1000).to_string()],
    );
    assert_eq!(err_json(&o)["error"]["code"], "rev.newer");
}

#[test]
fn every_verb_parses_and_plans() {
    let (_d, lib) = setup(false);
    for args in [
        vec!["split", "--at", "2", "--tracks", "v1"],
        vec!["trim", "--item", "a", "--edge", "end", "--to", "3"],
        vec!["ripple-trim", "--item", "a", "--edge", "end", "--to", "3"],
        vec!["delete", "--items", "a"],
        vec![
            "insert",
            "--track",
            "v1",
            "--at",
            "4",
            "--items-json",
            r#"[{"type":"text","startTime":0,"endTime":1,"textContent":"N"}]"#,
        ],
        vec![
            "overwrite",
            "--track",
            "v1",
            "--at",
            "4",
            "--items-json",
            r#"[{"type":"text","startTime":0,"endTime":1,"textContent":"N"}]"#,
        ],
        vec!["lift", "--tracks", "v1", "--range", "1-2"],
        vec!["extract", "--tracks", "v1", "--range", "1-2"],
        vec!["slide", "--item", "b", "--delta", "-0.5"],
        vec!["roll", "--left", "a", "--right", "b", "--to", "3"],
        vec!["move", "--item", "c", "--start", "13"],
        vec!["request", "--request", r#"{"op":"split","at":6}"#],
    ] {
        let verb = args[0];
        let mut argv = vec!["--json", "edit", verb, "Demo"];
        argv.extend(&args[1..]);
        argv.push("--dry-run");
        let o = ove(&lib, &argv);
        assert!(o.status.success(), "{verb}: {}", String::from_utf8_lossy(&o.stderr));
        assert!(!out_json(&o)["ops"].as_array().unwrap().is_empty(), "{verb} planned nothing");
    }
    // close-gap and slip refuse here (no gap; text items have no source): structured refusals.
    let o = ove(&lib, &["--json", "edit", "close-gap", "Demo", "--track", "v1", "--at", "2"]);
    assert_eq!(err_json(&o)["error"]["code"], "gap.none");
    let o = ove(&lib, &["--json", "edit", "slip", "Demo", "--item", "a", "--delta", "1"]);
    assert_eq!(err_json(&o)["error"]["code"], "slip.noSource");
}

/// Runs every `ove …  # exit N` line of the ```sh block in docs/cli/edit.md against the setup project.
#[test]
fn documented_examples_run_with_documented_exit_codes() {
    let (d, lib) = setup(false);
    let doc = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/cli/edit.md")).unwrap();
    let mut ran = 0;
    let mut in_sh = false;
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
        let args: Vec<&str> = cmd.split_whitespace().skip(1).collect();
        let out = Command::new(env!("CARGO_BIN_EXE_ove"))
            .current_dir(d.path())
            .arg("--library")
            .arg(&lib)
            .args(&args)
            .env_remove("OVE_LIBRARY")
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(want),
            "`{}`
{}",
            cmd.trim(),
            String::from_utf8_lossy(&out.stderr)
        );
        if args.contains(&"--json") && want == 0 {
            out_json(&out);
        }
        ran += 1;
    }
    assert_eq!(ran, 9);
}

struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    id: i64,
}

impl Mcp {
    fn start(lib: &Path) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
            .arg("--library")
            .arg(lib)
            .arg("mcp")
            .env_remove("OVE_LIBRARY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Mcp { child, out, id: 0 }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params})).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn mcp_edit_timeline_contract() {
    let (_d, lib) = setup(false);
    let old = open(&lib).rev();
    let mut m = Mcp::start(&lib);
    m.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
    );
    let tools = m.request("tools/list", json!({}))["result"]["tools"].as_array().unwrap().clone();
    let tool = tools.iter().find(|t| t["name"] == "edit_timeline").expect("edit_timeline is listed");
    assert_eq!(tool["inputSchema"]["required"], json!(["project", "request"]));
    assert_eq!(tool["annotations"]["readOnlyHint"], false);
    for k in ["project", "request", "rev", "preview"] {
        assert!(tool["inputSchema"]["properties"][k].is_object(), "missing {k}");
    }
    let call = |m: &mut Mcp, args: Value| {
        m.request("tools/call", json!({"name": "edit_timeline", "arguments": args}))["result"].clone()
    };
    // Preview: the plan, no revision.
    let r =
        call(&mut m, json!({"project": "Demo", "request": r#"{"op":"rippleDelete","items":["b"]}"#, "preview": true}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["dryRun"], true);
    assert_eq!(open(&lib).rev(), old);
    // Apply.
    let r = call(&mut m, json!({"project": "Demo", "request": r#"{"op":"rippleDelete","items":["b"]}"#}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(start(&open(&lib), "c"), 4_000_000);
    // A stale rev is rebased.
    let r = call(&mut m, json!({"project": "Demo", "rev": old, "request": r#"{"op":"split","items":["c"],"at":10}"#}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(open(&lib).doc().item("c").unwrap().end, 6_000_000);
    // A refusal is a tool error with the code and the culprits.
    let r = call(&mut m, json!({"project": "Demo", "request": r#"{"op":"split","items":["nope"],"at":1}"#}));
    assert_eq!(r["isError"], true);
    assert_eq!(r["structuredContent"]["error"]["code"], "item.missing");
    assert_eq!(r["structuredContent"]["error"]["details"]["culprits"], json!(["nope"]));
    let h = open(&lib);
    let last = h.latest_record().unwrap().unwrap();
    assert_eq!(last.source, Source::Mcp);
}
