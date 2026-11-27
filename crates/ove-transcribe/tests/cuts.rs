//! add-transcript-editing group 2: removing words. Pause settings, speeds below 1×, reverse clips, edge snapping,
//! captions following the cut, refusals, and a 10,000-case property test that the remaining words are the transcript
//! minus the selection.
#![allow(clippy::unwrap_used, clippy::panic, reason = "test code: a failure here is a failed test")]

use std::collections::HashMap;
use std::path::PathBuf;

use ove_core::Document;
use ove_core::ops::{Op, apply_batch};
use ove_transcribe::cuts::{CutError, Envelope, IdRange, Pause, Selection, cut_ranges, plan_cut, select};
use ove_transcribe::timeline::{TimelineWord, all_clips, words};
use ove_transcribe::{Transcript, Word};
use proptest::prelude::*;
use serde_json::{Value, json};

fn w(t: &str, s: f64, e: f64) -> Word {
    Word { text: t.into(), start_us: (s * 1e6) as i64, end_us: (e * 1e6) as i64, confidence: 0.9 }
}

/// Words "w0".."wN": each `len` long with `gap` between, from 1 s.
fn transcript(n: usize, len: f64, gap: f64) -> Transcript {
    let words =
        (0..n).map(|i| w(&format!("w{i}"), 1.0 + i as f64 * (len + gap), 1.0 + i as f64 * (len + gap) + len)).collect();
    Transcript { language: "en".into(), model: "base".into(), words, duration_us: 600_000_000 }
}

struct Setup {
    doc: Document,
    t: Transcript,
}

fn setup(t: Transcript, speed: f64, reverse: bool, locked: bool, captions: Option<Value>) -> Setup {
    let src_end = t.words.last().map(|w| w.end_us as f64 / 1e6 + 1.0).unwrap_or(10.0);
    let len = src_end / speed;
    let mut ops = vec![
        Op::AddTrack { track: json!({"id": "t_main", "name": "Main", "order": 1, "type": "video", "locked": locked}) },
        Op::AddMedia {
            media: json!({"id": "m", "data": {"type": "video", "fileName": "talk.mp4", "duration": 600, "hasAudio": true}}),
            entry: Some(json!({"mediaId": "m", "nativePath": "/media/talk.mp4", "ownedByBundle": false})),
        },
        Op::AddItem {
            item: json!({"id": "v", "trackId": "t_main", "type": "video", "startTime": 0, "endTime": len,
                "mediaItemId": "m", "trimStart": 0, "playbackSpeed": speed,
                "playbackDirection": if reverse { "reverse" } else { "forward" }}),
        },
    ];
    if let Some(c) = captions {
        ops.push(Op::AddTrack {
            track: json!({"id": "t_captions", "name": "Captions", "order": 0, "type": "caption"}),
        });
        ops.push(Op::AddItem { item: c });
    }
    Setup { doc: apply_batch(&Document::new("p", "p", 1), &ops).unwrap().doc, t }
}

fn tl_words(doc: &Document, t: &Transcript) -> Vec<TimelineWord> {
    let clips = all_clips(doc, &|_| Some(PathBuf::from("/media/talk.mp4")));
    words(&clips, &HashMap::from([("m".to_string(), t.clone())])).0
}

fn ids() -> impl FnMut() -> String {
    let mut n = 0;
    move || {
        n += 1;
        format!("new{n}")
    }
}

/// Cuts the selection and returns the document after it.
fn cut(s: &Setup, sel: &Selection, pause: Pause, env: Option<Envelope>) -> Result<Document, CutError> {
    let ws = tl_words(&s.doc, &s.t);
    let chosen = select(&ws, sel)?;
    let clips = all_clips(&s.doc, &|_| Some(PathBuf::from("/media/talk.mp4")));
    let ranges = cut_ranges(&ws, &chosen, &clips, pause, &|_| env.clone());
    let removed = chosen.iter().map(|&i| ws[i].id.clone()).collect();
    let plan = plan_cut(&s.doc, ranges, removed, &mut ids())?;
    Ok(apply_batch(&s.doc, &plan.plan.ops).unwrap().doc)
}

fn ids_of(ws: &[TimelineWord]) -> Vec<String> {
    ws.iter().map(|w| w.id.clone()).collect()
}

fn joined_pause(doc: &Document, t: &Transcript, before: &str, after: &str) -> f64 {
    let ws = tl_words(doc, t);
    let a = ws.iter().find(|w| w.text == before).unwrap();
    let b = ws.iter().find(|w| w.text == after).unwrap();
    b.start - a.end
}

#[test]
fn pause_settings_change_the_joined_pause() {
    let s = setup(transcript(5, 0.4, 0.4), 1.0, false, false, None);
    let sel = Selection { ids: vec!["m@2600".into()], ..Default::default() }; // w2
    let mut pauses = Vec::new();
    for p in [Pause::Tight, Pause::Balanced, Pause::Loose] {
        let after = cut(&s, &sel, p, None).unwrap();
        let ws = tl_words(&after, &s.t);
        assert_eq!(ids_of(&ws), ["m@1000", "m@1800", "m@3400", "m@4200"], "{p:?}");
        pauses.push(joined_pause(&after, &s.t, "w1", "w3"));
    }
    // 60 ms, 150 ms and 320 ms, within one frame (30 fps) of rounding at each edge.
    for (got, want) in pauses.iter().zip([0.06, 0.15, 0.32]) {
        assert!((got - want).abs() <= 0.067, "pause {got} for {want}");
    }
    assert!(pauses[0] < pauses[1] && pauses[1] < pauses[2], "{pauses:?}");
}

#[test]
fn speed_below_one_and_reverse_clips_cut_the_right_word() {
    for (speed, reverse) in [(0.5, false), (0.5, true), (2.0, true)] {
        let s = setup(transcript(6, 0.3, 0.3), speed, reverse, false, None);
        let all = ids_of(&tl_words(&s.doc, &s.t));
        let sel = Selection { ids: vec!["m@2200".into(), "m@2800".into()], ..Default::default() };
        let after = cut(&s, &sel, Pause::Balanced, None).unwrap();
        let left = ids_of(&tl_words(&after, &s.t));
        let want: Vec<String> = all.into_iter().filter(|i| i != "m@2200" && i != "m@2800").collect();
        assert_eq!(left, want, "speed {speed} reverse {reverse}");
    }
}

#[test]
fn edges_snap_to_the_quietest_point_without_crossing_a_neighbour() {
    let s = setup(transcript(5, 0.4, 0.4), 1.0, false, false, None);
    let ws = tl_words(&s.doc, &s.t);
    let chosen = select(&ws, &Selection { ids: vec!["m@2600".into()], ..Default::default() }).unwrap();
    let clips = all_clips(&s.doc, &|_| Some(PathBuf::from("/x")));
    // Loud everywhere except one quiet cell at 2.245 s (the computed edge is 2.275 s: w1 ends at 2.2 s, plus 75 ms).
    let mut db = vec![-20.0f32; 1000];
    db[224] = -70.0;
    let env = Envelope { cell_us: 10_000, db };
    let r = cut_ranges(&ws, &chosen, &clips, Pause::Balanced, &|_| Some(env.clone()));
    assert_eq!(r[0].start, 2_245_000);
    // The end edge has no quiet cell: the loudness is flat, so it stays within ±40 ms of 3.325 s.
    assert!((r[0].end - 3_325_000).abs() <= 40_000);
    // With the tight pause the edge (2.23 s) is 30 ms from w1: a quiet cell inside w1 is never used.
    let mut db = vec![-20.0f32; 1000];
    db[219] = -90.0; // 2.19–2.20 s, inside w1 (1.8–2.2 s)
    let env = Envelope { cell_us: 10_000, db };
    let r = cut_ranges(&ws, &chosen, &clips, Pause::Tight, &|_| Some(env.clone()));
    assert!(r[0].start >= 2_200_000, "{}", r[0].start);
}

#[test]
fn seams_get_a_short_fade_and_the_cut_is_one_batch() {
    let s = setup(transcript(5, 0.4, 0.4), 1.0, false, false, None);
    let after =
        cut(&s, &Selection { ids: vec!["m@2600".into()], ..Default::default() }, Pause::Balanced, None).unwrap();
    let items = after.items_on_track("t_main");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].fields["audioFadeOut"]["duration"], json!(0.01));
    assert_eq!(items[1].fields["audioFadeIn"]["duration"], json!(0.01));
    assert_eq!(items[0].end, items[1].start, "the gap is closed");
}

fn caption() -> Value {
    // "w0 w1 w2 w3 w4" over the five words (0.4 s long, 0.4 s apart, from 1 s); times relative to the item start.
    let timings: Vec<Value> = (0..5)
        .map(|i| json!({"word": format!("w{i}"), "start": 0.8 * i as f64, "end": 0.8 * i as f64 + 0.4}))
        .collect();
    json!({"id": "cap", "trackId": "t_captions", "type": "caption", "startTime": 1.0, "endTime": 4.6,
        "captionText": "w0 w1 w2 w3 w4",
        "captionSegments": [{"id": "s1", "text": "w0 w1 w2 w3 w4", "startTime": 0, "endTime": 3.6, "wordTimings": timings}]})
}

#[test]
fn caption_shortened_keeps_its_words_aligned() {
    let s = setup(transcript(5, 0.4, 0.4), 1.0, false, false, Some(caption()));
    let sel = Selection { ranges: vec![IdRange { from: "m@1800".into(), to: "m@2600".into() }], ..Default::default() };
    let after = cut(&s, &sel, Pause::Balanced, None).unwrap();
    let cap = after.item("cap").unwrap();
    assert_eq!(cap.fields["captionText"], "w0 w3 w4");
    // Each remaining caption word is heard when it is shown: within one frame of the timeline word.
    let ws = tl_words(&after, &s.t);
    let seg = &cap.fields["captionSegments"][0];
    for t in seg["wordTimings"].as_array().unwrap() {
        let word = ws.iter().find(|x| x.text == t["word"]).unwrap();
        let shown = cap.start as f64 / 1e6 + t["start"].as_f64().unwrap();
        assert!((shown - word.start).abs() <= 0.034, "{} shown at {shown}, heard at {}", word.text, word.start);
    }
}

#[test]
fn a_caption_with_every_word_cut_is_removed() {
    let s = setup(transcript(5, 0.4, 0.4), 1.0, false, false, Some(caption()));
    let sel = Selection { matches: vec!["w0 w1 w2 w3 w4".into()], ..Default::default() };
    let after = cut(&s, &sel, Pause::Tight, None).unwrap();
    assert!(after.item("cap").is_none());
}

#[test]
fn refusals_name_the_cause_and_change_nothing() {
    let s = setup(transcript(5, 0.4, 0.4), 1.0, false, false, None);
    let e = cut(
        &s,
        &Selection { ids: vec!["m@2600".into(), "m@99999".into(), "x@1".into()], ..Default::default() },
        Pause::Balanced,
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, "transcript.wordMissing");
    assert_eq!(e.culprits, ["m@99999", "x@1"]);
    let e = cut(&s, &Selection::default(), Pause::Balanced, None).unwrap_err();
    assert_eq!(e.code, "request.invalid");
    // A locked track blocks the ripple: named with its item.
    let locked = setup(transcript(5, 0.4, 0.4), 1.0, false, true, None);
    let e = cut(&locked, &Selection { ids: vec!["m@2600".into()], ..Default::default() }, Pause::Balanced, None)
        .unwrap_err();
    assert_eq!(e.code, "ripple.blocked");
    assert_eq!(e.culprits, ["v"]);
}

#[test]
fn ten_separate_words_are_one_batch() {
    let s = setup(transcript(30, 0.3, 0.3), 1.0, false, false, None);
    let ws = tl_words(&s.doc, &s.t);
    let sel = Selection { ids: ws.iter().step_by(3).take(10).map(|w| w.id.clone()).collect(), ..Default::default() };
    let chosen = select(&ws, &sel).unwrap();
    let clips = all_clips(&s.doc, &|_| Some(PathBuf::from("/x")));
    let plan =
        plan_cut(&s.doc, cut_ranges(&ws, &chosen, &clips, Pause::Balanced, &|_| None), vec![], &mut ids()).unwrap();
    assert_eq!(plan.ranges.len(), 10);
    // Applying the one batch and undoing it (its inverse) gives the document back exactly.
    let after = apply_batch(&s.doc, &plan.plan.ops).unwrap().doc;
    let back = apply_batch(&after, &ove_core::diff::diff_documents(&after, &s.doc)).unwrap().doc;
    assert_eq!(back, s.doc);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]
    #[test]
    fn remaining_words_are_the_transcript_minus_the_selection(
        lens in prop::collection::vec((0.12f64..0.8, 0.0f64..0.9), 3..25),
        picks in prop::collection::vec(any::<bool>(), 25),
        speed in prop::sample::select(vec![0.5, 1.0, 1.5, 2.0]),
        reverse in any::<bool>(),
        pause in prop::sample::select(vec![Pause::Tight, Pause::Balanced, Pause::Loose]),
    ) {
        let mut t0 = 1.0;
        let words: Vec<Word> = lens.iter().enumerate().map(|(i, (len, gap))| {
            let x = w(&format!("w{i}"), t0, t0 + len);
            t0 += len + gap;
            x
        }).collect();
        let t = Transcript { language: "en".into(), model: "base".into(), words, duration_us: 600_000_000 };
        let s = setup(t, speed, reverse, false, None);
        let ws = tl_words(&s.doc, &s.t);
        let sel_ids: Vec<String> = ws.iter().zip(&picks).filter(|(_, p)| **p).map(|(w, _)| w.id.clone()).collect();
        prop_assume!(!sel_ids.is_empty());
        let after = cut(&s, &Selection { ids: sel_ids.clone(), ..Default::default() }, pause, None).unwrap();
        let left = ids_of(&tl_words(&after, &s.t));
        let want: Vec<String> = ids_of(&ws).into_iter().filter(|i| !sel_ids.contains(i)).collect();
        prop_assert_eq!(left, want);
    }
}
