//! Random documents and batches for property tests (feature `testgen`).
#![allow(
    clippy::expect_used,
    reason = "test support: generators only run inside property tests, where a panic is a failed test"
)]

use proptest::prelude::*;
use proptest::sample::select;
use serde_json::{Map, Value, json};

use crate::model::Document;
use crate::ops::{Op, apply_batch};
use crate::time::micros_to_json;

const ITEM_KINDS: &[&str] = &["video", "image", "audio", "text", "caption", "code", "adjustment", "shader"];
const TRACK_KINDS: &[&str] = &["video", "audio", "caption", "adjustment", "layout"];

/// A time in seconds on a millisecond grid, as JSON.
fn secs(ms: i64) -> Value {
    micros_to_json(ms * 1000)
}

/// Optional extra fields for an item of a kind.
pub fn arb_item_fields(kind: &'static str, media: Vec<String>) -> impl Strategy<Value = Map<String, Value>> {
    let has_media = matches!(kind, "video" | "image" | "audio") && !media.is_empty();
    (
        proptest::option::of("[A-Za-z ]{0,8}"),
        proptest::option::of(0u32..=100),
        proptest::option::of(0u32..=200),
        proptest::option::of((-500i32..500, 1u32..30)),
        proptest::option::of(0u32..2000),
        proptest::option::of(any::<bool>()),
        proptest::option::of(if has_media { select(media).boxed() } else { Just(String::new()).boxed() }),
        proptest::option::of("[a-z\\\\\"\n ]{0,6}"),
    )
        .prop_map(move |(name, opacity, volume, transform, trim, unknown, media_id, text)| {
            let mut m = Map::new();
            if let Some(n) = name {
                m.insert("name".into(), n.into());
            }
            if let Some(t) = trim {
                m.insert("trimStart".into(), secs(t as i64));
            }
            if let Some(id) = media_id
                && has_media
            {
                m.insert("mediaItemId".into(), id.into());
            }
            if kind == "text"
                && let Some(t) = text
            {
                m.insert("textContent".into(), t.into());
            }
            if let Some((x, s)) = transform {
                m.insert("transform".into(), json!({"x": x, "scale": s as f64 / 10.0}));
            }
            if let Some(o) = opacity {
                m.insert("opacity".into(), json!(o as f64 / 100.0));
            }
            if matches!(kind, "video" | "audio")
                && let Some(v) = volume
            {
                m.insert("volume".into(), v.into());
            }
            if let Some(u) = unknown {
                m.insert("zExtra".into(), json!({"flag": u}));
            }
            m
        })
}

/// A random valid document: 1–4 tracks, 0–3 media items, 0–14 items.
pub fn arb_document() -> impl Strategy<Value = Document> {
    (1usize..=4, 0usize..=3, select(&["16:9", "9:16", "1:1"][..]))
        .prop_flat_map(|(nt, nm, aspect)| {
            let tracks = proptest::collection::vec(select(TRACK_KINDS), nt);
            let media: Vec<String> = (0..nm).map(|i| format!("m{i}")).collect();
            let items = proptest::collection::vec(
                (0..nt, select(ITEM_KINDS), 0i64..20_000, 1i64..8_000).prop_flat_map({
                    let media = media.clone();
                    move |(t, k, s, d)| (Just((t, k, s, d)), arb_item_fields(k, media.clone()))
                }),
                0..=14,
            );
            (Just(aspect), tracks, Just(media), items)
        })
        .prop_map(|(aspect, tracks, media, items)| {
            let mut ops = vec![Op::SetAspectRatio { aspect_ratio: aspect.into() }];
            for (i, k) in tracks.iter().enumerate() {
                ops.push(Op::AddTrack {
                    track: json!({"id": format!("t{i}"), "name": format!("Track {i}"), "order": i, "type": k}),
                });
            }
            for m in &media {
                ops.push(Op::AddMedia {
                    media: json!({"id": m, "data": {"type": "video", "fileName": format!("{m}.mov"), "duration": 30}}),
                    entry: Some(json!({"mediaId": m, "nativePath": format!("/media/{m}.mov"), "ownedByBundle": false})),
                });
            }
            for (n, ((t, k, s, d), fields)) in items.into_iter().enumerate() {
                let mut item = Map::new();
                item.insert("id".into(), format!("i{n}").into());
                item.insert("trackId".into(), format!("t{t}").into());
                item.insert("type".into(), k.into());
                item.insert("startTime".into(), secs(s));
                item.insert("endTime".into(), secs(s + d));
                item.extend(fields);
                ops.push(Op::AddItem { item: Value::Object(item) });
            }
            apply_batch(&Document::new("p", "Generated", 1), &ops).expect("generated document is valid").doc
        })
}

/// Usually one of `ids`, sometimes `missing` (to exercise rejections).
fn mostly_valid(ids: Vec<String>, missing: &'static str) -> BoxedStrategy<String> {
    if ids.is_empty() {
        Just(missing.to_string()).boxed()
    } else {
        prop_oneof![12 => select(ids), 1 => Just(missing.to_string())].boxed()
    }
}

/// One random op against a document. Values are sometimes invalid so rejections are exercised too.
pub fn arb_op(doc: &Document) -> BoxedStrategy<Op> {
    let item_ids = mostly_valid(doc.project.items.keys().cloned().collect(), "ghost");
    let track_ids = mostly_valid(doc.project.tracks.iter().map(|t| t.id.clone()).collect(), "tghost");
    let n_tracks = doc.project.tracks.len();
    let media: Vec<String> = doc.project.media.keys().cloned().collect();
    let all_tracks: Vec<String> = doc.project.tracks.iter().map(|t| t.id.clone()).collect();
    let fresh = doc.project.items.len();

    let update_item = (
        item_ids.clone(),
        prop_oneof![
            (-1i64..30_000).prop_map(|ms| ("startTime", secs(ms))),
            (0i64..30_000).prop_map(|ms| ("endTime", secs(ms))),
            (0u32..=120).prop_map(|o| ("opacity", json!(o as f64 / 100.0))),
            track_ids.clone().prop_map(|t| ("trackId", Value::from(t))),
            select(&["normal", "screen", "glow"][..]).prop_map(|b| ("blendMode", Value::from(b))),
            "[a-z ]{0,6}".prop_map(|s| ("name", Value::from(s))),
            (0i64..3000).prop_map(|ms| ("trimEnd", secs(ms))),
            any::<i32>().prop_map(|x| ("zUnknown", Value::from(x))),
        ],
    )
        .prop_map(|(id, (k, v))| {
            let mut set = Map::new();
            set.insert(k.into(), v);
            Op::UpdateItem { id, set }
        });

    let add_item = (0..n_tracks.max(1), select(ITEM_KINDS), 0i64..20_000, 0i64..5_000, proptest::bool::weighted(0.1)).prop_map(
        move |(t, k, s, d, dup)| {
            let id = if dup && fresh > 0 { "i0".to_string() } else { format!("new{fresh}_{s}") };
            Op::AddItem {
                item: json!({"id": id, "trackId": format!("t{t}"), "type": k, "startTime": secs(s), "endTime": secs(s + d)}),
            }
        },
    );

    let remove_items = proptest::collection::vec(item_ids.clone(), 1..3).prop_map(|mut ids| {
        ids.dedup();
        Op::RemoveItems { ids }
    });
    let unset_item = (item_ids, select(&["name", "opacity", "zExtra", "transform", "trimStart", "endTime"][..]))
        .prop_map(|(id, f)| Op::UnsetItemFields { id, fields: vec![f.into()] });
    let update_track = (track_ids.clone(), any::<bool>(), "[A-Z][a-z]{0,5}").prop_map(|(id, b, name)| {
        let mut set = Map::new();
        set.insert("locked".into(), b.into());
        set.insert("name".into(), name.into());
        Op::UpdateTrack { id, set }
    });
    let unset_track = (track_ids.clone(), select(&["locked", "name", "visible"][..]))
        .prop_map(|(id, f)| Op::UnsetTrackFields { id, fields: vec![f.into()] });
    let add_track = (0..n_tracks + 2, select(TRACK_KINDS), proptest::bool::weighted(0.1)).prop_map(move |(o, k, dup)| Op::AddTrack {
        track: json!({"id": if dup { "t0".to_string() } else { format!("tn{o}") }, "name": "New", "order": o, "type": k}),
    });
    let remove_track = track_ids.prop_map(|id| Op::RemoveTrack { id });
    let reorder = Just(all_tracks).prop_shuffle().prop_map(|order| Op::ReorderTracks { order });
    let aspect = select(&["16:9", "4:5", "21:9"][..]).prop_map(|a| Op::SetAspectRatio { aspect_ratio: a.into() });
    let media_op = if media.is_empty() {
        Just(Op::AddMedia { media: json!({"id": "mx", "data": {"type": "image"}}), entry: None }).boxed()
    } else {
        (select(media), any::<bool>())
            .prop_map(|(id, remove)| {
                if remove {
                    Op::RemoveMedia { id }
                } else {
                    let mut entry_set = Map::new();
                    entry_set.insert("probeStatus".into(), "probed".into());
                    Op::UpdateMedia { id, set: Map::new(), unset: vec![], entry_set, entry_unset: vec![] }
                }
            })
            .boxed()
    };
    prop_oneof![
        4 => update_item,
        2 => add_item,
        1 => remove_items,
        1 => unset_item,
        1 => update_track,
        1 => unset_track,
        1 => add_track,
        1 => remove_track,
        1 => reorder,
        1 => aspect,
        1 => media_op,
    ]
    .boxed()
}

/// A document and a random batch of 1–6 ops against it.
pub fn arb_doc_and_batch() -> impl Strategy<Value = (Document, Vec<Op>)> {
    arb_document().prop_flat_map(|doc| {
        let ops = proptest::collection::vec(arb_op(&doc), 1..=6);
        (Just(doc), ops)
    })
}
