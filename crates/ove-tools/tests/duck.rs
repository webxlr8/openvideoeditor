//! MCP contract of `duck_audio` (task 5.3): listed on every agent surface with its arguments, a dry run returns the
//! plan per music item without a revision, a run is one revision that `undo_timeline` reverts, and an unknown id is
//! refused by name (`cargo test -p ove-tools --features media`).
#![cfg(feature = "media")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::Path;
use std::sync::Arc;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::project::{Mode, Project};
use ove_tools::{BundleCtx, Surface};
use serde_json::{Value, json};

/// A 16-bit stereo 48 kHz WAV of `secs`, amplitude `amp(t)` of a sine at `freq`.
fn wav(path: &Path, secs: f64, freq: f64, amp: impl Fn(f64) -> f64) {
    let frames = (secs * 48_000.0) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + frames * 4).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    for v in [1u16, 2] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for v in [48_000u32, 192_000] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for v in [4u16, 16] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(frames * 4).to_le_bytes());
    for i in 0..frames {
        let t = i as f64 / 48_000.0;
        let s = ((amp(t) * (std::f64::consts::TAU * freq * t).sin()) * 32767.0) as i16;
        b.extend_from_slice(&s.to_le_bytes());
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let (dlg, mus) = (d.path().join("dialogue.wav"), d.path().join("music.wav"));
    wav(&dlg, 10.0, 1000.0, |t| if (2.0..4.0).contains(&t) || (6.0..7.0).contains(&t) { 0.3 } else { 0.0 });
    wav(&mus, 10.0, 440.0, |_| 0.2);
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    let mut ops = vec![];
    for (i, (id, path)) in [("dlg", &dlg), ("mus", &mus)].iter().enumerate() {
        ops.push(
            json!({"op": "addTrack", "track": {"id": format!("t_{id}"), "name": id, "order": i, "type": "audio"}}),
        );
        ops.push(json!({"op": "addMedia", "media": {"id": format!("m_{id}"), "data": {"type": "audio", "fileName": "x.wav", "duration": 10}},
            "entry": {"mediaId": format!("m_{id}"), "nativePath": path.display().to_string(), "ownedByBundle": false}}));
        ops.push(json!({"op": "addItem", "item": {"id": id, "trackId": format!("t_{id}"), "type": "audio", "startTime": 0, "endTime": 10, "mediaItemId": format!("m_{id}")}}));
    }
    let ops: Vec<Op> = serde_json::from_value(Value::Array(ops)).unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    (d, lib, bundle)
}

fn volume_keys(bundle: &Path) -> Option<Value> {
    let p = Project::open(bundle, Mode::ReadOnly, Arc::new(ove_core::SystemClock)).unwrap();
    let tracks = p.doc().item("mus").unwrap().get("keyframeTracks")?;
    tracks.as_array()?.iter().find(|t| t["property"] == "volume").cloned()
}

#[test]
fn duck_audio_over_mcp() {
    // Listed on MCP with its arguments.
    let list = ove_tools::mcp_list(Surface::Mcp);
    let tool = list["tools"].as_array().unwrap().iter().find(|t| t["name"] == "duck_audio").expect("duck_audio on MCP");
    let schema = &tool["inputSchema"];
    for k in ["project", "music", "under", "depth", "attack", "release", "hold", "source", "mode", "dryRun"] {
        assert!(schema["properties"].get(k).is_some(), "no `{k}` in {schema}");
    }
    let required: Vec<&str> = schema["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(required.contains(&"music") && required.contains(&"under"), "{required:?}");
    assert!(ove_tools::anthropic_list().iter().any(|t| t["name"] == "duck_audio"), "in-app too");

    let (_d, lib, bundle) = setup();
    let mut mcp = BundleCtx::open(Some(&lib), Surface::Mcp, Source::Mcp).unwrap();
    let call = |mcp: &mut BundleCtx, a: Value| ove_tools::call(mcp, "duck_audio", a.as_object().unwrap());
    // Dry run: keyframes per music item, the document unchanged.
    let r = call(&mut mcp, json!({"project": "Demo", "music": ["mus"], "under": "dlg", "dryRun": true})).unwrap();
    assert_eq!(r.json["dryRun"], true);
    assert_eq!(r.json["items"][0]["id"], "mus");
    assert!(r.json["items"][0]["keyframes"].as_array().unwrap().len() >= 8);
    assert_eq!(r.json["speech"], json!([[2.0, 4.0], [6.0, 7.0]]));
    assert!(volume_keys(&bundle).is_none());
    // Run: one revision; undo_timeline reverts it.
    let r = call(&mut mcp, json!({"project": "Demo", "music": "mus", "under": "dlg", "depth": -9})).unwrap();
    assert!(r.json["rev"].as_i64().is_some());
    assert!(volume_keys(&bundle).is_some());
    ove_tools::call(&mut mcp, "undo_timeline", json!({"project": "Demo"}).as_object().unwrap()).unwrap();
    assert!(volume_keys(&bundle).is_none(), "undo restores the previous keyframes");
    // Unknown id: refused by name, nothing changed.
    let e = call(&mut mcp, json!({"project": "Demo", "music": "ghost", "under": "dlg"})).unwrap_err();
    assert_eq!(e.code, "item.notFound");
    assert!(e.message.contains("ghost"));
    // Asking for the transcript when none is cached is refused, not silently switched to the level.
    let e =
        call(&mut mcp, json!({"project": "Demo", "music": "mus", "under": "dlg", "source": "transcript"})).unwrap_err();
    assert_eq!(e.code, "duck.noTranscript");
}
