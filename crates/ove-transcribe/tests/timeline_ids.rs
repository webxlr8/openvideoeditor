//! add-transcript-editing task 1.3: word ids are anchored to the source, so edits elsewhere on the timeline (moving,
//! trimming, re-timing or speeding up another clip) leave every id unchanged, and a trimmed clip's remaining words
//! keep theirs.
#![allow(clippy::unwrap_used, reason = "test code: a failure here is a failed test")]

use std::collections::HashMap;
use std::path::PathBuf;

use ove_core::Document;
use ove_core::ops::{Op, apply_batch};
use ove_transcribe::timeline::{source_clips, words};
use ove_transcribe::{Transcript, Word};
use proptest::prelude::*;
use serde_json::json;

fn transcript(seed: u64) -> Transcript {
    let words = (0..40)
        .map(|i| {
            let s = 500_000 * i + (seed % 7) as i64 * 10_000;
            Word { text: format!("w{i}"), start_us: s, end_us: s + 400_000, confidence: 0.9 }
        })
        .collect();
    Transcript { language: "en".into(), model: "base".into(), words, duration_us: 20_000_000 }
}

fn doc() -> Document {
    let mut ops = vec![Op::AddTrack { track: json!({"id": "t_main", "name": "Main", "order": 0, "type": "video"}) }];
    for m in ["a", "b"] {
        ops.push(Op::AddMedia {
            media: json!({"id": m, "data": {"type": "video", "fileName": format!("{m}.mp4"), "duration": 20, "hasAudio": true}}),
            entry: Some(json!({"mediaId": m, "nativePath": format!("/media/{m}.mp4"), "ownedByBundle": false})),
        });
    }
    ops.push(Op::AddItem {
        item: json!({"id": "ca", "trackId": "t_main", "type": "video", "startTime": 0, "endTime": 10, "mediaItemId": "a", "trimStart": 2}),
    });
    ops.push(Op::AddItem {
        item: json!({"id": "cb", "trackId": "t_main", "type": "video", "startTime": 10, "endTime": 20, "mediaItemId": "b"}),
    });
    apply_batch(&Document::new("p", "p", 1), &ops).unwrap().doc
}

fn ids(d: &Document, clip: &str) -> Vec<String> {
    let clips = source_clips(d, &[], &|m| Some(PathBuf::from(format!("/media/{m}.mp4")))).unwrap();
    let t = HashMap::from([("a".to_string(), transcript(1)), ("b".to_string(), transcript(2))]);
    words(&clips, &t).0.into_iter().filter(|w| w.clip_id == clip).map(|w| w.id).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]
    #[test]
    fn ids_survive_edits_elsewhere(shift in 0.0f64..30.0, trim in 0.0f64..8.0, len in 1.0f64..10.0, speed in 0.5f64..3.0) {
        let before = doc();
        let (a0, b0) = (ids(&before, "ca"), ids(&before, "cb"));
        // Edit clip b: move it, trim its start, change its length and speed.
        let set = json!({"startTime": 10.0 + shift, "endTime": 10.0 + shift + len, "trimStart": trim, "playbackSpeed": speed});
        let op = Op::UpdateItem { id: "cb".into(), set: set.as_object().unwrap().clone() };
        let after = apply_batch(&before, &[op]).unwrap().doc;
        // Clip a's words keep their ids, in order.
        prop_assert_eq!(&ids(&after, "ca"), &a0);
        // Clip b's remaining words keep the ids they had.
        let b1 = ids(&after, "cb");
        prop_assert!(b1.iter().all(|id| b0.contains(id) || id.starts_with("b@")), "{:?}", b1);
        let all_b: Vec<String> = transcript(2).words.iter().map(|w| ove_transcribe::timeline::word_id("b", w.start_us)).collect();
        prop_assert!(b1.iter().all(|id| all_b.contains(id)));
    }
}
