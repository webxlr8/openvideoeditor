//! add-semantic-media-search tasks 5.1 and 5.2: `ove media search|transcript|overview|organize` headless on a closed
//! project, the model-not-installed answer, and an MCP session that pastes a search hit's clip fragment into
//! `timeline.tsx` and applies it. The visual model is not needed: an index file is written directly (the "more like
//! this" path compares stored frames), and spoken search reads a cached transcript.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use ove_vision::index::{DIM, IndexData, Provenance, index_path, normalise, write};
use serde_json::{Value, json};

fn unit(seed: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..DIM).map(|i| (((i + 1) * (seed + 5) * 7919) % 1000) as f32 / 1000.0 - 0.5).collect();
    normalise(&mut v);
    v
}

/// A library with project "Lib": a Main track and two video media items, "harbour" (indexed: shots 0–10 s and
/// 10–30 s) and "market" (indexed, one shot), plus a transcript for "harbour".
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Lib").unwrap();
    let media = |id: &str, file: &str, cid: &str| {
        json!({"op": "addMedia", "media": {"id": id, "data": {"type": "video", "fileName": file, "duration": 30, "hasAudio": true}},
               "entry": {"mediaId": id, "nativePath": d.path().join(file).to_string_lossy(), "ownedByBundle": false, "contentId": cid}})
    };
    let ops: Vec<ove_core::ops::Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
        media("m1", "harbour.mov", "c1"),
        media("m2", "market.mov", "c2"),
    ]))
    .unwrap();
    p.apply(ops, ove_core::revision::Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let prov =
        Provenance { model: "fixture".into(), model_hash: "0".into(), sampler: 1, preprocessing: "fixture".into() };
    let a = unit(1);
    let mut near = a.clone();
    near[0] += 0.05;
    normalise(&mut near);
    write(
        &index_path(&bundle, "c1"),
        &IndexData {
            provenance: prov.clone(),
            content_id: "c1".into(),
            duration: 30.0,
            embeddings: [a.clone(), unit(2), near].concat(),
            times: vec![5.0, 14.0, 22.5],
            row_shot: vec![0, 1, 1],
            shots: vec![(0.0, 10.0), (10.0, 30.0)],
            cuts: vec![10.0],
            cut_scores: vec![0.8],
            frames: 750,
        },
    )
    .unwrap();
    write(
        &index_path(&bundle, "c2"),
        &IndexData {
            provenance: prov,
            content_id: "c2".into(),
            duration: 30.0,
            embeddings: unit(3),
            times: vec![15.0],
            row_shot: vec![0],
            shots: vec![(0.0, 30.0)],
            cuts: vec![],
            cut_scores: vec![],
            frames: 750,
        },
    )
    .unwrap();
    let w = |t: &str, s: f64| ove_transcribe::Word {
        text: t.into(),
        start_us: (s * 1e6) as i64,
        end_us: (s * 1e6) as i64 + 300_000,
        confidence: 0.9,
    };
    let t = ove_transcribe::Transcript {
        language: "en".into(),
        model: "base".into(),
        words: vec![w("So", 2.0), w("the", 2.4), w("price", 2.8), w("is", 3.2), w("right.", 3.6)],
        duration_us: 30_000_000,
    };
    ove_transcribe::cache::store(&bundle, &ove_transcribe::cache::transcript_key("c1", "base", None, false), &t);
    (d, lib, bundle)
}

/// Runs `ove --library LIB --json ARGS…` with an empty model folder (the visual model is not installed).
fn ove(lib: &Path, models: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .arg("--json")
        .args(args)
        .env_remove("OVE_LIBRARY")
        .env("OVE_MODELS_DIR", models)
        .output()
        .unwrap();
    // Results go to stdout, errors (as JSON with --json) to stderr.
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let text = if stdout.is_empty() { String::from_utf8_lossy(&out.stderr).trim().to_string() } else { stdout };
    let v = serde_json::from_str(&text).unwrap_or_else(|_| json!({"raw": text}));
    (out.status.code().unwrap_or(-1), v)
}

#[test]
fn spoken_search_and_the_model_not_installed_answer_headless() {
    let (d, lib, _) = fixture();
    let models = d.path().join("models");
    std::fs::create_dir_all(&models).unwrap();
    let (code, v) = ove(&lib, &models, &["media", "search", "Lib", "\"the price is\""]);
    assert_eq!(code, 0, "{v}");
    let spoken = v["spoken"].as_array().unwrap();
    assert_eq!(spoken.len(), 1, "{v}");
    assert_eq!(spoken[0]["text"], "the price is");
    assert_eq!((spoken[0]["in"].as_f64().unwrap(), spoken[0]["out"].as_f64().unwrap()), (2.25, 3.65));
    assert_eq!(spoken[0]["clip"], r#"<clip start={0} end={1.4} in={2.25} src="harbour" />"#);
    // The visual model is missing: no moments, its size reported, nothing downloaded.
    assert_eq!(v["moments"], json!([]));
    assert_eq!(v["index"]["visual"], "notInstalled");
    assert!(v["index"]["downloadBytes"].as_u64().unwrap() > 300_000_000, "{v}");
    assert_eq!(std::fs::read_dir(&models).unwrap().count(), 0, "nothing was downloaded");
    assert_eq!(v["index"]["untranscribed"], 1);
    // Files match names.
    let (_, v) = ove(&lib, &models, &["media", "search", "Lib", "market", "--scope", "files"]);
    assert_eq!(v["files"][0]["media"], "m2");
    assert!(v.get("spoken").is_none());
}

#[test]
fn more_like_this_transcript_overview_and_organize_on_a_closed_project() {
    let (d, lib, bundle) = fixture();
    let models = d.path().join("models");
    std::fs::create_dir_all(&models).unwrap();
    // More like this from the first shot of "harbour": that shot first, then the similar frame of its second shot.
    let (code, v) = ove(&lib, &models, &["media", "search", "Lib", "--like", "harbour@4", "--scope", "moments"]);
    assert_eq!(code, 0, "{v}");
    let m = v["moments"].as_array().unwrap();
    assert_eq!((m[0]["media"].as_str(), m[0]["best"].as_f64()), (Some("m1"), Some(5.0)), "{v}");
    assert_eq!(m[0]["clip"], r#"<clip start={0} end={4} in={3} src="harbour" />"#);
    assert_eq!((m[1]["media"].as_str(), m[1]["best"].as_f64()), (Some("m1"), Some(22.5)), "{v}");
    assert!(m.iter().all(|x| x["media"] != "m2"), "an unrelated shot is below the similarity floor");

    let (code, v) = ove(&lib, &models, &["media", "transcript", "Lib", "harbour", "--from", "2.5"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["words"][0]["text"], "the");
    let (code, v) = ove(&lib, &models, &["media", "transcript", "Lib", "market"]);
    assert_ne!(code, 0);
    assert_eq!(v["error"]["code"], "transcript.missing", "{v}");

    let (code, v) = ove(
        &lib,
        &models,
        &[
            "media",
            "organize",
            "Lib",
            r#"[{"op":"move","media":["m1","m2"],"folder":"Selects"},{"op":"tag","media":"m1","add":["boat"]}]"#,
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["changed"], true);
    let (_, v) = ove(&lib, &models, &["media", "search", "Lib", "boat", "--scope", "files"]);
    assert_eq!(v["files"][0]["media"], "m1", "tags are searchable");
    let printed = std::fs::read_to_string(bundle.join("timeline.tsx")).unwrap_or_default();
    assert!(printed.contains("Selects"), "the header lists the folder: {printed}");
}

struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    id: i64,
}

impl Mcp {
    fn start(lib: &Path, models: &Path) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ove"))
            .arg("--library")
            .arg(lib)
            .arg("mcp")
            .env_remove("OVE_LIBRARY")
            .env("OVE_MCP_HOLD_MS", "0")
            .env("OVE_MODELS_DIR", models)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut m = Mcp { child, out, id: 0 };
        m.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
        );
        m.notify("notifications/initialized");
        m
    }

    fn send(&mut self, v: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str) {
        self.send(json!({"jsonrpc": "2.0", "method": method}));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let id = self.id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "the server closed");
            let v: Value = serde_json::from_str(&line).unwrap();
            if v["id"] == id {
                return v;
            }
        }
    }

    fn call(&mut self, name: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}));
        assert_eq!(r["result"]["isError"], false, "{name}: {r}");
        r["result"]["structuredContent"].clone()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_hit_pasted_into_the_timeline_applies_and_plays_its_range() {
    let (d, lib, bundle) = fixture();
    let models = d.path().join("models");
    std::fs::create_dir_all(&models).unwrap();
    let mut mcp = Mcp::start(&lib, &models);
    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> =
        tools["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    for t in ["search_media", "media_transcript", "inspect_media", "organize_media"] {
        assert!(names.contains(&t), "{t} is listed");
    }
    for (args, section) in [
        (json!({"project": "Lib", "similarTo": {"media": "harbour", "at": 4}, "scope": "moments"}), "moments"),
        (json!({"project": "Lib", "query": "\"price is right\"", "scope": "spoken"}), "spoken"),
    ] {
        let hit = mcp.call("search_media", args)[section][0].clone();
        let fragment = hit["clip"].as_str().unwrap().to_string();
        let printed = mcp.call("print_timeline", json!({"project": "Lib"}));
        let text = printed["text"].as_str().unwrap().to_string();
        let rev = printed["rev"].clone();
        let anchor = text.lines().find(|l| l.contains("<track") && l.contains("t_main")).unwrap().to_string();
        let applied = mcp.call(
            "apply_timeline",
            json!({"project": "Lib", "rev": rev, "edits": [{"oldText": anchor, "newText": format!("{anchor}\n      {fragment}")}]}),
        );
        assert_eq!(applied["changed"], true, "{applied}");
        // The new clip plays the hit's range.
        let doc = ove_store::project::Project::open(
            &bundle,
            ove_store::project::Mode::ReadOnly,
            Arc::new(ove_core::SystemClock),
        )
        .unwrap()
        .doc()
        .clone();
        let (from, to) = (hit["in"].as_f64().unwrap(), hit["out"].as_f64().unwrap());
        let clip = doc
            .project
            .items
            .values()
            .find(|i| {
                i.media_id() == Some("m1")
                    && (i.get("trimStart").and_then(|v| v.as_f64()).unwrap_or(0.0) - from).abs() < 1e-6
            })
            .unwrap_or_else(|| panic!("no clip at {from}: {:?}", doc.project.items.keys().collect::<Vec<_>>()));
        assert!(((clip.end - clip.start) as f64 / 1e6 - (to - from)).abs() < 1e-3);
        // Clear the track for the next paste.
        let printed = mcp.call("print_timeline", json!({"project": "Lib"}));
        let text = printed["text"].as_str().unwrap().to_string();
        let line = text.lines().find(|l| l.contains("src=\"harbour\"")).unwrap().to_string();
        mcp.call(
            "apply_timeline",
            json!({"project": "Lib", "rev": printed["rev"], "edits": [{"oldText": format!("{line}\n"), "newText": ""}]}),
        );
    }
}

/// Splits a documented command line: whitespace separates arguments, single quotes keep text together.
fn shell_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in line.chars() {
        match c {
            '\'' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// docs/cli/media-search.md (task 5.3): every documented example runs on the fixture with its documented exit code.
#[test]
fn documented_examples_run_with_documented_exit_codes() {
    let doc =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/cli/media-search.md")).unwrap();
    let (d, lib, _) = fixture();
    let models = d.path().join("models");
    std::fs::create_dir_all(&models).unwrap();
    let mut in_sh = false;
    let mut ran = 0;
    for line in doc.lines() {
        if line.starts_with("```") {
            in_sh = line == "```sh";
            continue;
        }
        if !in_sh || !line.starts_with("ove ") || !line.contains("# exit") {
            continue;
        }
        let (cmd, comment) = line.rsplit_once('#').unwrap();
        let want: i32 = comment.trim().strip_prefix("exit ").unwrap().parse().unwrap();
        let words = shell_words(cmd);
        let args: Vec<&str> = words.iter().skip(1).map(String::as_str).filter(|a| *a != "--json").collect();
        let (code, v) = ove(&lib, &models, &args);
        assert_eq!(code, want, "`{}`: {v}", cmd.trim());
        ran += 1;
    }
    assert_eq!(ran, 8);
    assert_eq!(std::fs::read_dir(&models).unwrap().count(), 0, "nothing was downloaded");
}
