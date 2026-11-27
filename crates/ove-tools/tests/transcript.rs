//! add-transcript-editing through the tool registry, headless: `get_transcript`, `remove_words`, `remove_silence`,
//! `detect_silence` in speech mode and `find_fillers`. Transcripts, speech masks and levels are cached in the bundle
//! (as the app and the command line leave them), so no model or runtime is needed.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code: a failure here is a failed test")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::project::{Mode, Project};
use ove_tools::{BundleCtx, Fail, Receipt, Surface};
use ove_transcribe::cuts::Envelope;
use ove_transcribe::vad::{CELL_US, SpeechMask};
use ove_transcribe::{Transcript, Word};
use serde_json::{Map, Value, json};

fn clock() -> Arc<dyn ove_core::Clock> {
    Arc::new(ove_core::SystemClock)
}

fn w(t: &str, s: f64, e: f64) -> Word {
    Word { text: t.into(), start_us: (s * 1e6) as i64, end_us: (e * 1e6) as i64, confidence: 0.9 }
}

/// "Talk": one 12 s clip. Speech 0–2 s, silence 2–4 s, speech 4–6 s, a music bed 6–10 s (6 dB below the speech),
/// speech 10–12 s; words in each speech part, one "um" and one "like".
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, clock()).unwrap();
    let mut p = library.create("Talk").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addMedia", "media": {"id": "m", "data": {"type": "audio", "fileName": "talk.wav", "duration": 12, "hasAudio": true}},
         "entry": {"mediaId": "m", "nativePath": d.path().join("talk.wav").to_string_lossy(), "ownedByBundle": false, "contentId": "cid"}},
        {"op": "addItem", "item": {"id": "v", "trackId": "t_main", "type": "audio", "startTime": 0, "endTime": 12, "mediaItemId": "m"}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let t = Transcript {
        language: "en".into(),
        model: "base".into(),
        words: vec![
            w("Hello", 0.2, 0.6),
            w("um", 0.8, 1.1),
            w("everyone.", 1.3, 1.8),
            w("We", 4.2, 4.5),
            w("like", 4.7, 5.0),
            w("editing.", 5.2, 5.8),
            w("Thanks.", 10.3, 10.9),
        ],
        duration_us: 12_000_000,
    };
    ove_transcribe::cache::store(&bundle, &ove_transcribe::cache::transcript_key("cid", "base", None, false), &t);
    let cells = (12_000_000 / CELL_US) as usize;
    let probs = (0..cells)
        .map(|c| {
            let t = c as i64 * CELL_US;
            if t < 2_000_000 || (4_000_000..6_000_000).contains(&t) || t >= 10_000_000 { 0.9 } else { 0.05 }
        })
        .collect();
    ove_transcribe::vad::store(&bundle, "cid", &SpeechMask { cell_us: CELL_US, probs });
    let db = (0..1200)
        .map(|c| {
            let t = c as i64 * 10_000;
            if (2_000_000..4_000_000).contains(&t) {
                -90.0
            } else if (6_000_000..10_000_000).contains(&t) {
                -26.0
            } else {
                -20.0
            }
        })
        .collect();
    ove_transcribe::levels::store(&bundle, "cid", &Envelope { cell_us: 10_000, db });
    (d, lib, bundle)
}

fn call(ctx: &mut BundleCtx, name: &str, a: Value) -> Result<Receipt, Fail> {
    let m: Map<String, Value> = a.as_object().cloned().unwrap_or_default();
    ove_tools::call(ctx, name, &m)
}

fn history_len(ctx: &mut BundleCtx) -> usize {
    call(ctx, "timeline_history", json!({"project": "Talk"})).unwrap().json["revisions"].as_array().unwrap().len()
}

fn printed(bundle: &Path) -> String {
    let p = Project::open(bundle, Mode::ReadOnly, clock()).unwrap();
    ove_projection::print(p.doc(), &Default::default())
}

fn mcp(lib: &Path) -> BundleCtx {
    BundleCtx::open(Some(lib), Surface::Mcp, Source::Mcp).unwrap()
}

#[test]
fn detection_only_leaves_the_document_unchanged() {
    let (_d, lib, bundle) = fixture();
    let mut ctx = mcp(&lib);
    let before = (history_len(&mut ctx), printed(&bundle));
    let r = call(&mut ctx, "detect_silence", json!({"project": "Talk", "media": "m", "mode": "speech"})).unwrap();
    assert_eq!(r.json["speechFound"], true);
    let ranges = r.json["media"][0]["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 1, "the silence only; the music bed is kept: {ranges:?}");
    let (a, b) = (ranges[0]["start"].as_f64().unwrap(), ranges[0]["end"].as_f64().unwrap());
    assert!((a - 2.15).abs() < 0.04 && (b - 3.85).abs() < 0.04, "{a}..{b}");
    assert_eq!((history_len(&mut ctx), printed(&bundle)), before);
}

#[test]
fn dead_air_removal_is_one_revision_and_the_preview_is_what_applies() {
    let (_d, lib, bundle) = fixture();
    let mut ctx = mcp(&lib);
    let n = history_len(&mut ctx);
    let before = printed(&bundle);
    let preview = call(&mut ctx, "remove_silence", json!({"project": "Talk", "apply": false})).unwrap();
    assert_eq!(preview.json["dryRun"], true);
    assert_eq!(history_len(&mut ctx), n, "a dry run makes no revision");
    let diff = preview.json["diff"].as_str().unwrap().to_string();
    assert!(diff.contains("@@"), "{diff}");
    let r = call(&mut ctx, "remove_silence", json!({"project": "Talk"})).unwrap();
    assert_eq!(r.json["changed"], true);
    assert_eq!(history_len(&mut ctx), n + 1, "one revision");
    let after = printed(&bundle);
    // The previewed diff, applied to the text before, gives the text after (the header's revision aside).
    let strip = |s: &str| {
        s.lines().filter(|l| !l.contains("rev")).collect::<Vec<_>>().join(
            "
",
        )
    };
    assert_eq!(strip(&ove_tools::tools::transcript::apply_diff(&before, &diff)), strip(&after));
    // Words after the silence moved 1.7 s earlier; the words themselves are all still there.
    let t = call(&mut ctx, "get_transcript", json!({"project": "Talk"})).unwrap();
    assert_eq!(t.json["totalWords"], 7);
    let we = t.json["words"].as_array().unwrap().iter().find(|w| w["text"] == "We").unwrap()["start"].as_f64().unwrap();
    assert!((we - (4.2 - 1.7)).abs() < 0.05, "{we}");
}

#[test]
fn removing_words_is_one_revision_and_a_refusal_makes_none() {
    let (_d, lib, _bundle) = fixture();
    let mut ctx = mcp(&lib);
    let n = history_len(&mut ctx);
    let e = call(&mut ctx, "remove_words", json!({"project": "Talk", "ids": ["m@800", "m@99999"]})).unwrap_err();
    assert_eq!(e.code, "transcript.wordMissing");
    assert_eq!(e.details["culprits"], json!(["m@99999"]));
    assert_eq!(history_len(&mut ctx), n);
    let t = call(&mut ctx, "get_transcript", json!({"project": "Talk"})).unwrap();
    let rev = t.json["rev"].as_i64().unwrap();
    // Two removals against the one read: ids stay valid after the first cut.
    let r = call(&mut ctx, "remove_words", json!({"project": "Talk", "ids": ["m@800"], "rev": rev})).unwrap();
    assert_eq!(r.json["removedWords"], json!(["m@800"]));
    let r =
        call(&mut ctx, "remove_words", json!({"project": "Talk", "match": ["editing."], "rev": rev, "pause": "tight"}))
            .unwrap();
    assert_eq!(r.json["removedWords"], json!(["m@5200"]));
    assert_eq!(history_len(&mut ctx), n + 2);
    let t = call(&mut ctx, "get_transcript", json!({"project": "Talk"})).unwrap();
    let texts: Vec<&str> = t.json["words"].as_array().unwrap().iter().map(|w| w["text"].as_str().unwrap()).collect();
    assert_eq!(texts, ["Hello", "everyone.", "We", "like", "Thanks."]);
}

#[test]
fn filler_candidates_select_hesitations_and_flag_context_words() {
    let (_d, lib, _bundle) = fixture();
    let mut ctx = mcp(&lib);
    let r = call(&mut ctx, "find_fillers", json!({"project": "Talk"})).unwrap();
    let f = r.json["fillers"].as_array().unwrap();
    let um = f.iter().find(|x| x["text"] == "um").unwrap();
    assert_eq!((um["selected"].clone(), um["id"].clone()), (json!(true), json!("m@800")));
    let like = f.iter().find(|x| x["text"] == "like").unwrap();
    assert_eq!((like["selected"].clone(), like["context"].clone()), (json!(false), json!(true)));
    assert!(f.iter().all(|x| x["text"] != "editing." && x["text"] != "Hello"), "no content word");
}

#[test]
fn search_finds_a_phrase_with_word_times_and_timeline_uses() {
    let (_d, lib, _bundle) = fixture();
    let mut ctx = mcp(&lib);
    let r = call(&mut ctx, "search_transcript", json!({"project": "Talk", "phrase": "LIKE editing"})).unwrap();
    let hits = r.json["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0]["sourceStart"].clone(), hits[0]["sourceEnd"].clone()), (json!(4.7), json!(5.8)));
    assert_eq!(hits[0]["wordIds"], json!(["m@4700", "m@5200"]));
    assert_eq!(hits[0]["uses"][0]["clipId"], "v");
    assert_eq!(r.json["untranscribed"], 0);
    assert!(
        call(&mut ctx, "search_transcript", json!({"project": "Talk", "phrase": "nothing here"})).unwrap().json["hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
