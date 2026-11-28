//! add-transcript-editing 5.2: the transcript tools over `ove mcp`, one contract check per tool. The project's
//! transcript, speech mask and levels are cached in the bundle, so no model or runtime is needed.
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

use serde_json::{Value, json};

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Talk").unwrap();
    let ops: Vec<ove_core::ops::Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addMedia", "media": {"id": "m", "data": {"type": "audio", "fileName": "talk.wav", "duration": 12, "hasAudio": true}},
         "entry": {"mediaId": "m", "nativePath": d.path().join("talk.wav").to_string_lossy(), "ownedByBundle": false, "contentId": "cid"}},
        {"op": "addItem", "item": {"id": "v", "trackId": "t_main", "type": "audio", "startTime": 0, "endTime": 12, "mediaItemId": "m"}}
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
        words: vec![
            w("Hello", 0.2, 0.6),
            w("um", 0.8, 1.1),
            w("everyone.", 1.3, 1.8),
            w("We", 4.2, 4.5),
            w("like", 4.7, 5.0),
            w("editing.", 5.2, 5.8),
        ],
        duration_us: 12_000_000,
    };
    ove_transcribe::cache::store(&bundle, &ove_transcribe::cache::transcript_key("cid", "base", None, false), &t);
    let cell = ove_transcribe::vad::CELL_US;
    let probs = (0..(12_000_000 / cell) as usize)
        .map(|c| {
            if (c as i64 * cell) < 2_000_000 || (4_000_000..6_000_000).contains(&(c as i64 * cell)) {
                0.9
            } else {
                0.05
            }
        })
        .collect();
    ove_transcribe::vad::store(&bundle, "cid", &ove_transcribe::vad::SpeechMask { cell_us: cell, probs });
    let db = (0..1200)
        .map(|c| if (2_000_000..4_000_000).contains(&(c * 10_000)) || c * 10_000 >= 6_000_000 { -90.0 } else { -20.0 })
        .collect();
    ove_transcribe::levels::store(&bundle, "cid", &ove_transcribe::cuts::Envelope { cell_us: 10_000, db });
    (d, lib)
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
            .env("OVE_MCP_HOLD_MS", "0")
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
        let msg = json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        self.out.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("{e}: {reply}"))
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": args}))["result"].clone()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn ok(r: &Value) -> &Value {
    assert_eq!(r["isError"], false, "{r}");
    &r["structuredContent"]
}

fn history(m: &mut Mcp) -> usize {
    ok(&m.call("timeline_history", json!({"project": "Talk"})))["revisions"].as_array().unwrap().len()
}

#[test]
fn every_transcript_tool_answers_its_contract_over_mcp() {
    let (_d, lib) = fixture();
    let mut m = Mcp::start(&lib);
    m.request("initialize", json!({"protocolVersion": "2025-06-18", "clientInfo": {"name": "contract"}}));
    let tools = m.request("tools/list", json!({}))["result"]["tools"].as_array().unwrap().clone();
    for name in [
        "get_transcript",
        "search_transcript",
        "transcribe_media",
        "remove_words",
        "remove_silence",
        "find_fillers",
        "detect_silence",
    ] {
        let t = tools.iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("{name} is listed"));
        assert_eq!(t["inputSchema"]["type"], "object", "{name}");
    }
    // The spec's required arguments (docs/spec/mcp-tools.md) are required here too.
    let req = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap()["inputSchema"]["required"].clone();
    assert!(req("search_transcript").as_array().unwrap().contains(&json!("phrase")));
    assert!(req("transcribe_media").as_array().unwrap().contains(&json!("media")));

    // get_transcript: words with ids and the revision.
    let t = ok(&m.call("get_transcript", json!({"project": "Talk"}))).clone();
    assert_eq!(t["totalWords"], 6);
    let rev = t["rev"].as_i64().unwrap();
    // search_transcript: word-accurate hit and its timeline use.
    let s = ok(&m.call("search_transcript", json!({"project": "Talk", "phrase": "we LIKE"}))).clone();
    assert_eq!(s["hits"][0]["wordIds"], json!(["m@4200", "m@4700"]));
    // find_fillers: "um" selected, "like" flagged.
    let f = ok(&m.call("find_fillers", json!({"project": "Talk"}))).clone();
    assert!(f["fillers"].as_array().unwrap().iter().any(|x| x["id"] == "m@800" && x["selected"] == true));
    // detect_silence in speech mode: ranges, no revision.
    let n = history(&mut m);
    let d = ok(&m.call("detect_silence", json!({"project": "Talk", "media": "m", "mode": "speech"}))).clone();
    assert_eq!(d["speechFound"], true);
    assert_eq!(history(&mut m), n);
    // transcribe_media: a cached transcript comes back at once (or this build cannot recognise speech).
    let r = m.call("transcribe_media", json!({"project": "Talk", "media": "m", "format": "text"}));
    if r["isError"] == true {
        assert_eq!(r["structuredContent"]["error"]["code"], "speech.unavailable", "{r}");
    } else {
        assert_eq!(r["structuredContent"]["cached"], true);
    }
    let bad = m.call("transcribe_media", json!({"project": "Talk", "media": "m", "format": "srt"}));
    assert_eq!(bad["structuredContent"]["error"]["code"], "format.unsupported");
    // remove_words: a dry run, then the cut against the revision read above; one revision.
    let dry = ok(&m.call("remove_words", json!({"project": "Talk", "ids": ["m@800"], "apply": false}))).clone();
    assert_eq!(dry["dryRun"], true);
    assert!(dry["diff"].as_str().unwrap().contains("@@"));
    assert_eq!(history(&mut m), n);
    let cut = ok(&m.call("remove_words", json!({"project": "Talk", "ids": ["m@800"], "rev": rev}))).clone();
    assert_eq!(cut["removedWords"], json!(["m@800"]));
    assert_eq!(history(&mut m), n + 1);
    // A refusal names the word and makes no revision.
    let e = m.call("remove_words", json!({"project": "Talk", "ids": ["m@800"]}));
    assert_eq!(e["structuredContent"]["error"]["code"], "transcript.wordMissing");
    assert_eq!(history(&mut m), n + 1);
    // remove_silence: the pause between the sentences goes, as one revision.
    let r = ok(&m.call("remove_silence", json!({"project": "Talk"}))).clone();
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(history(&mut m), n + 2);
}
