//! Shared generators for the planner tests: timelines on a 25 fps grid with video and audio tracks, items that do
//! not overlap within a track, sometimes linked pairs, speed, reverse playback and linear keyframes.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, reason = "test support")]

use proptest::prelude::*;
use serde_json::{Value, json};

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::time::{Micros, micros_to_json};

pub const FPS: f64 = 25.0;
/// One frame at 25 fps.
pub const F: Micros = 40_000;
pub const S: Micros = 1_000_000;

pub fn secs(us: Micros) -> Value {
    micros_to_json(us)
}

/// One generated item: track index, gap before it (frames), length (frames), in point (frames), speed ×2, reverse,
/// keyframes, linked to an audio partner.
#[derive(Debug, Clone)]
pub struct Gen {
    pub gap: i64,
    pub len: i64,
    pub trim: i64,
    pub fast: bool,
    pub reverse: bool,
    pub keyframes: Option<(f64, f64)>,
    pub linked: bool,
}

fn arb_gen() -> impl Strategy<Value = Gen> {
    (
        0i64..40,
        2i64..80,
        0i64..200,
        prop::bool::weighted(0.2),
        prop::bool::weighted(0.15),
        prop::option::of((0.0f64..1.0, 0.0f64..1.0)),
        prop::bool::weighted(0.4),
    )
        .prop_map(|(gap, len, trim, fast, reverse, keyframes, linked)| Gen {
            gap,
            len,
            trim,
            fast,
            reverse,
            keyframes,
            linked,
        })
}

/// Track flags: (syncLock, locked) for the extra tracks.
#[derive(Debug, Clone)]
pub struct Tracks {
    pub music_sync: bool,
    pub title_locked: bool,
}

/// A document: track `v1` (video), `a1` (audio, partners of linked video items), `music` (audio), `titles` (text).
pub fn build(gens: &[Gen], music: &[(i64, i64)], titles: &[(i64, i64)], t: &Tracks) -> Document {
    let mut ops = vec![
        Op::UpdateProject { set: json!({"fps": FPS}).as_object().unwrap().clone(), unset: vec![] },
        Op::AddTrack {
            track: json!({"id": "titles", "name": "Titles", "order": 0, "type": "video", "locked": t.title_locked}),
        },
        Op::AddTrack { track: json!({"id": "v1", "name": "V1", "order": 1, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "a1", "name": "A1", "order": 2, "type": "audio"}) },
        Op::AddTrack {
            track: json!({"id": "music", "name": "Music", "order": 3, "type": "audio", "syncLock": t.music_sync}),
        },
        Op::AddMedia {
            media: json!({"id": "m", "data": {"type": "video", "fileName": "cam.mov", "duration": 60}}),
            entry: Some(json!({"mediaId": "m", "nativePath": "/x/cam.mov", "ownedByBundle": false})),
        },
    ];
    let mut cursor = 0;
    for (n, g) in gens.iter().enumerate() {
        let start = cursor + g.gap * F;
        let end = start + g.len * F;
        cursor = end;
        let mut it = json!({"id": format!("v{n}"), "trackId": "v1", "type": "video", "startTime": secs(start),
            "endTime": secs(end), "mediaItemId": "m", "trimStart": secs(g.trim * F)});
        if g.fast {
            it["playbackSpeed"] = json!(2);
        }
        if g.reverse {
            it["playbackDirection"] = json!("reverse");
        }
        if let Some((a, b)) = g.keyframes {
            it["keyframeTracks"] = json!([{"property": "opacity", "keyframes": [
                {"time": 0, "value": a}, {"time": secs(g.len * F / 2), "value": b}, {"time": secs(g.len * F), "value": a}]}]);
        }
        if g.linked {
            it["link"] = json!(format!("L{n}"));
            let mut a = it.clone();
            a["id"] = json!(format!("a{n}"));
            a["trackId"] = json!("a1");
            a["type"] = json!("audio");
            a.as_object_mut().unwrap().remove("keyframeTracks");
            ops.push(Op::AddItem { item: a });
        }
        ops.push(Op::AddItem { item: it });
    }
    let mut c = 0;
    for (n, (gap, len)) in music.iter().enumerate() {
        let s = c + gap * F;
        c = s + len * F;
        ops.push(Op::AddItem {
            item: json!({"id": format!("mu{n}"), "trackId": "music", "type": "audio", "startTime": secs(s), "endTime": secs(c), "mediaItemId": "m"}),
        });
    }
    let mut c = 0;
    for (n, (gap, len)) in titles.iter().enumerate() {
        let s = c + gap * F;
        c = s + len * F;
        ops.push(Op::AddItem {
            item: json!({"id": format!("t{n}"), "trackId": "titles", "type": "text", "startTime": secs(s), "endTime": secs(c), "textContent": "T"}),
        });
    }
    apply_batch(&Document::new("p", "Planner test", 1), &ops).expect("generated document is valid").doc
}

pub fn arb_doc() -> impl Strategy<Value = Document> {
    (
        prop::collection::vec(arb_gen(), 1..8),
        prop::collection::vec((0i64..30, 5i64..100), 0..3),
        prop::collection::vec((0i64..50, 5i64..60), 0..3),
        any::<bool>(),
        prop::bool::weighted(0.15),
    )
        .prop_map(|(g, m, t, music_sync, title_locked)| build(&g, &m, &t, &Tracks { music_sync, title_locked }))
}

/// Applies a plan's ops, checks the result equals the plan's own document, and checks the inverse restores `doc`.
pub fn apply_checked(doc: &Document, plan: &ove_edit::Plan) -> Document {
    if plan.ops.is_empty() {
        return doc.clone();
    }
    let applied = apply_batch(doc, &plan.ops).expect("a plan applies");
    assert!(applied.doc.content_eq(&plan.result), "plan result differs from applying its ops");
    let back = apply_batch(&applied.doc, &applied.inverse).expect("the inverse applies");
    assert!(back.doc.content_eq(doc), "apply then inverse must return the original document");
    applied.doc
}
