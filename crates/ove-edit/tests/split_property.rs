//! fix-render-and-edit-correctness task 2.3: for random items (speed 0.25–4, both directions, random keyframes) the
//! two parts of a split show the same source time and the same keyframed values as the original item at 200 sampled
//! timeline times. 10,000 cases.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test code")]

use proptest::prelude::*;
use serde_json::{Value, json};

use ove_core::model::Item;
use ove_core::time::{Micros, micros_to_json};
use ove_edit::retime::{property_at, reversed, speed, split};

const S: Micros = 1_000_000;
const SAMPLES: i64 = 200;
/// Source times agree within this many microseconds: the parts' source fields are rounded to whole microseconds
/// (at most half a microsecond each, scaled by speed). One frame at 240 fps is 4,167 µs.
const SOURCE_TOLERANCE_US: f64 = 3.0;
const VALUE_TOLERANCE: f64 = 1e-6;
const PROPERTIES: &[&str] = &["opacity", "x", "y", "scale", "rotation"];
const EASINGS: &[&str] = &["linear", "ease-in", "ease-out", "ease-in-out"];

/// Source time shown at timeline time `t`, as the renderer maps it (continuous form).
fn source_at(it: &Item, t: Micros) -> f64 {
    let s = speed(it);
    let ts = it.time_field("trimStart") as f64;
    if reversed(it) { ts + (it.end - t) as f64 * s } else { ts + (t - it.start) as f64 * s }
}

/// A random keyframe track: (property, easing, keyframes as item-relative (µs, value)).
fn arb_track(len: Micros) -> impl Strategy<Value = (usize, usize, Vec<(Micros, f64)>)> {
    // Keyframes may lie before the start or after the end of the item, as they can in real documents.
    let time = -len / 2..len + len / 2;
    (0..PROPERTIES.len(), 0..EASINGS.len(), prop::collection::vec((time, -100.0f64..100.0), 1..7))
}

#[derive(Debug, Clone)]
struct Case {
    item: Item,
    at: Micros,
}

fn arb_case() -> impl Strategy<Value = Case> {
    (
        0..120 * S,                                   // start
        S / 10..60 * S,                               // length
        0..30 * S,                                    // trimStart
        prop::option::of(0..20 * S),                  // trimEnd
        0.25f64..4.0,                                 // speed
        any::<bool>(),                                // reverse
        prop::sample::select(vec!["video", "audio"]), // type
        0.0f64..1.0,                                  // split position
    )
        .prop_flat_map(|(start, len, trim_start, trim_end, spd, rev, kind, frac)| {
            prop::collection::vec(arb_track(len), 0..4).prop_map(move |tracks| {
                // Distinct properties per item: the first track of a property is the one that plays.
                let mut seen = Vec::new();
                let tracks: Vec<Value> = tracks
                    .into_iter()
                    .filter(|(p, _, _)| {
                        let fresh = !seen.contains(p);
                        seen.push(*p);
                        fresh
                    })
                    .map(|(p, e, kfs)| {
                        let kfs: Vec<Value> =
                            kfs.iter().map(|(t, v)| json!({"time": micros_to_json(*t), "value": v})).collect();
                        json!({"property": PROPERTIES[p], "easing": EASINGS[e], "keyframes": kfs})
                    })
                    .collect();
                let mut v = json!({"id": "a", "trackId": "v", "type": kind, "startTime": micros_to_json(start),
                    "endTime": micros_to_json(start + len), "mediaItemId": "m", "trimStart": micros_to_json(trim_start),
                    "playbackSpeed": spd, "keyframeTracks": tracks});
                if let Some(te) = trim_end {
                    v["trimEnd"] = micros_to_json(te);
                }
                if rev {
                    v["playbackDirection"] = json!("reverse");
                }
                let item = Item::from_json(&v, "item").unwrap();
                // Strictly inside the item.
                let at = start + 1 + ((len - 2) as f64 * frac) as Micros;
                Case { item, at }
            })
        })
}

/// The left and right parts of the split, as items.
fn parts(item: &Item, at: Micros) -> (Item, Item) {
    let s = split(item, at, "a-right").expect("the split time is inside the item");
    let mut left = item.to_json();
    for (k, v) in &s.left_set {
        left[k] = v.clone();
    }
    for k in &s.left_unset {
        left.as_object_mut().unwrap().shift_remove(k);
    }
    (Item::from_json(&left, "left").unwrap(), Item::from_json(&s.right, "right").unwrap())
}

/// Timeline times inside the item of the keyframes of one track.
fn keyframe_times(item: &Item, track: &Value) -> Vec<Micros> {
    track["keyframes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|k| ove_core::time::json_to_micros(&k["time"]))
        .map(|t| item.start + t)
        .filter(|t| (item.start..item.end).contains(t))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]

    #[test]
    fn split_parts_match_the_original_at_200_times(case in arb_case()) {
        let Case { item, at } = case;
        let (left, right) = parts(&item, at);
        prop_assert_eq!((left.start, left.end, right.start, right.end), (item.start, at, at, item.end));
        let len = item.end - item.start;
        let mut times: Vec<Micros> = (0..SAMPLES).map(|k| item.start + len * k / SAMPLES).collect();
        times.extend([at - 1, at, item.end - 1]);
        let tracks = item.fields.get("keyframeTracks").and_then(Value::as_array).cloned().unwrap_or_default();
        let all_keyframes: Vec<Micros> = tracks.iter().flat_map(|tr| keyframe_times(&item, tr)).collect();
        for &t in times.iter().chain(&all_keyframes) {
            let part = if t < at { &left } else { &right };
            let (want, got) = (source_at(&item, t), source_at(part, t));
            prop_assert!((want - got).abs() <= SOURCE_TOLERANCE_US, "source at {}: {} vs {}", t, want, got);
        }
        for track in &tracks {
            let property = track["property"].as_str().unwrap();
            // Linear tracks keep their curve everywhere; eased tracks keep their values at the cut and at every
            // original keyframe (design D3: the shape between the cut and the next keyframe is re-eased).
            let check: Vec<Micros> = if track["easing"] == "linear" {
                times.clone()
            } else {
                keyframe_times(&item, track).into_iter().chain([at]).collect()
            };
            for t in check {
                let part = if t < at { &left } else { &right };
                let (want, got) = (property_at(&item, property, t), property_at(part, property, t));
                match (want, got) {
                    (Some(w), Some(g)) => prop_assert!(
                        (w - g).abs() <= VALUE_TOLERANCE * w.abs().max(1.0),
                        "{} at {}: {} vs {}", property, t, w, g
                    ),
                    _ => prop_assert_eq!(want, got, "{} at {}", property, t),
                }
            }
        }
    }
}
