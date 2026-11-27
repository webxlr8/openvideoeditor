//! Executes the examples in docs/cli/timeline.md and checks each command's documented exit code, then runs the
//! success and refusal paths of every command (task 6.1 / 6.2).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

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
    // Volume keyframes (docs/cli/timeline.md): a fade from 100 % to 0 % over the clip's first second, and a value
    // over the 200 % limit, which is refused.
    let fade = r#"pendingDescription="wide shot" keyframeTracks={[{property: "volume", keyframes: [{time: 0, value: 100}, {time: 1, value: 0}]}]}"#;
    std::fs::write(d.path().join("fade.tsx"), text.replacen("pendingDescription=\"wide shot\"", fade, 1)).unwrap();
    std::fs::write(
        d.path().join("loud.tsx"),
        text.replacen("pendingDescription=\"wide shot\"", &fade.replace("value: 0}", "value: 250}"), 1),
    )
    .unwrap();
    (d, lib)
}

/// Runs every `ove …  # exit N` line of a ```sh block in a docs/cli page and returns how many ran.
fn run_examples(page: &str, lib: &Path, cwd: &Path) -> usize {
    let doc = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/cli").join(page)).unwrap();
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
        let (cmd, comment) = line.split_once('#').expect("example has an `# exit N` comment");
        let want: i32 = comment.trim().strip_prefix("exit ").unwrap().parse().unwrap();
        let args: Vec<&str> = cmd.split_whitespace().skip(1).collect();
        let out = ove(lib, cwd, &args);
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
    ran
}

#[test]
fn documented_examples_run_with_documented_exit_codes() {
    let (d, lib) = setup();
    let ran = run_examples("timeline.md", &lib, d.path());
    assert!(ran >= 10, "only {ran} examples found");
}

#[test]
fn project_examples() {
    let (d, lib) = setup();
    assert_eq!(run_examples("project.md", &lib, d.path()), 7);
    let out = ove(&lib, d.path(), &["project", "show", "PromoV2", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["aspectRatio"], "9:16");
    assert_eq!(v["tracks"].as_array().unwrap().len(), 5);
    // Remove with --yes moves it away (to the Trash; on CI hosts without one the refusal is acceptable).
    let out = ove(&lib, d.path(), &["project", "remove", "Draft", "--yes", "--json"]);
    if out.status.success() {
        assert_eq!(ove(&lib, d.path(), &["project", "show", "Draft"]).status.code(), Some(1));
    }
}

/// docs/cli/tasks.md: `ove tasks` without the app (no background work exists outside it).
#[test]
fn tasks_examples() {
    let (d, lib) = setup();
    assert_eq!(run_examples("tasks.md", &lib, d.path()), 4);
}

#[test]
fn skills_examples() {
    let (d, lib) = setup();
    assert_eq!(run_examples("skills.md", &lib, d.path()), 5);
    // A reviewed draft is saved with `add`, and keeps its drafted source.
    let draft = d.path().join("draft.md");
    let text = "---\nname: Calm captions\ndescription: Caption a talk calmly.\nsource: drafted\n---\n\n1. Step.\n";
    std::fs::write(&draft, text).unwrap();
    let out = ove(&lib, d.path(), &["skills", "add", draft.to_str().unwrap(), "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!((v["id"].as_str(), v["origin"]["kind"].as_str()), (Some("calm-captions"), Some("drafted")));
    assert_eq!(
        ove(&lib, d.path(), &["skills", "add", draft.to_str().unwrap(), "--id", "calm-captions"]).status.code(),
        Some(1)
    );
    let out = ove(&lib, d.path(), &["skills", "list", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: Vec<&str> = v["skills"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["caption-styling", "colour-matching", "interview-cutting", "calm-captions"]);
    let out = ove(&lib, d.path(), &["skills", "show", "nope", "--json"]);
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "skill.unknown");
    // The guide written by `ove skills install` names the skill tools.
    let guide = d.path().join("guide");
    assert!(ove(&lib, d.path(), &["skills", "install", "--dir", guide.to_str().unwrap()]).status.success());
    let text = std::fs::read_to_string(guide.join("openvideoeditor/SKILL.md")).unwrap();
    assert!(text.contains("read_skill") && text.contains("list_skills"));
}

#[test]
fn interchange_examples_and_round_trip() {
    let (d, lib) = setup();
    assert_eq!(run_examples("interchange.md", &lib, d.path()), 7);
    // The import above added the exported tracks again: the title now exists twice, the pending clip too.
    let v: Value = serde_json::from_slice(&ove(&lib, d.path(), &["source", "print", "Demo", "--json"]).stdout).unwrap();
    let text = v["text"].as_str().unwrap();
    assert_eq!(text.matches("Imported").count(), 1, "{text}");
    assert_eq!(text.matches("textContent=\"Hi\"").count(), 2, "{text}");
    let fcpxml = std::fs::read_to_string(d.path().join("cut.fcpxml")).unwrap();
    assert!(fcpxml.contains("1001/30000s"), "{fcpxml}");
    // Garbage is refused without a revision.
    std::fs::write(d.path().join("bad.xml"), "<nope/>").unwrap();
    let out = ove(&lib, d.path(), &["timeline", "import", "Demo", "bad.xml", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "import.invalid");
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

#[test]
fn tool_examples() {
    let (d, lib) = setup();
    assert_eq!(run_examples("tools.md", &lib, d.path()), 5);
}

/// add-transcript-editing 1.5: the transcript page's examples, and the documented JSON shape, on a project whose one
/// clip (source 2–8 s at 2× from timeline 10 s) has a cached transcript.
#[test]
fn transcript_examples_and_json_shape() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, std::sync::Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Interview").unwrap();
    let media = d.path().join("talk.wav");
    std::fs::write(&media, vec![1u8; 4096]).unwrap();
    let ops: Vec<ove_core::ops::Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addMedia", "media": {"id": "m_talk", "data": {"type": "audio", "fileName": "talk.wav", "duration": 60, "hasAudio": true}},
         "entry": {"mediaId": "m_talk", "nativePath": media.to_string_lossy(), "ownedByBundle": false, "contentId": "c0ffee"}},
        {"op": "addItem", "item": {"id": "v1", "trackId": "t_main", "type": "audio", "startTime": 10, "endTime": 13,
         "mediaItemId": "m_talk", "trimStart": 2, "playbackSpeed": 2}}
    ]))
    .unwrap();
    p.apply(ops, ove_core::revision::Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let w = |t: &str, s: f64, e: f64| ove_transcribe::Word {
        text: t.into(),
        start_us: (s * 1e6) as i64,
        end_us: (e * 1e6) as i64,
        confidence: 0.9,
    };
    let t = ove_transcribe::Transcript {
        language: "en".into(),
        model: "base".into(),
        words: vec![w("Hello", 2.5, 3.0), w("there.", 3.0, 3.5), w("Um", 4.0, 4.4), w("next", 6.0, 6.5)],
        duration_us: 60_000_000,
    };
    ove_transcribe::cache::store(&bundle, &ove_transcribe::cache::transcript_key("c0ffee", "base", None, false), &t);

    let out = ove(&lib, d.path(), &["transcript", "Interview", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["rev"].as_i64().is_some());
    assert_eq!(v["totalWords"], 4);
    assert_eq!(v["untranscribed"], json!([]));
    let first = &v["words"][0];
    for k in ["id", "text", "start", "end", "clipId", "mediaId", "sourceStart", "sourceEnd", "filler"] {
        assert!(first.get(k).is_some(), "missing {k}: {first}");
    }
    assert_eq!(first["id"], "m_talk@2500");
    assert_eq!(first["start"], 10.25);
    assert_eq!(v["words"][2]["filler"], true);
    let out = ove(&lib, d.path(), &["transcript", "Interview", "--segments", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["segments"][0]["text"], "Hello there.");
    assert_eq!(v["segments"][1]["wordIds"], json!(["m_talk@4000", "m_talk@6000"]));
    let out = ove(&lib, d.path(), &["transcript", "Interview", "--limit", "2", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["nextOffset"], 2);
    // Last: the examples include one cut.
    assert_eq!(run_examples("transcript.md", &lib, d.path()), 11);
}
