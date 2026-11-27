//! The verify lint `text.outsideTitleSafe` (add-viewer-interaction-and-text-fills task 3.2): a title whose right edge
//! is 2 % from the frame edge is reported with the item and time; titles and captions inside title-safe are not.
#![cfg(feature = "media")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::collections::BTreeSet;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_projection::verify::{Context, Finding, Registry};
use ove_render::geometry::layer_geometry;
use ove_tools::tools::geometry::{lint::CODE, register_lints};
use serde_json::{Value, json};

fn doc(items: Vec<Value>) -> Document {
    let mut ops = vec![
        Op::AddTrack { track: json!({"id": "text", "name": "Text", "order": 0, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "caps", "name": "Captions", "order": 1, "type": "caption"}) },
    ];
    ops.extend(items.into_iter().map(|item| Op::AddItem { item }));
    apply_batch(&Document::new("p", "p", 1), &ops).unwrap().doc
}

fn findings(d: &Document) -> Vec<Finding> {
    let mut r = Registry::default();
    register_lints(&mut r);
    r.run(&Context { doc: d, offline: &BTreeSet::new() }).into_iter().filter(|f| f.lint == CODE).collect()
}

fn title(id: &str, x: f64, start: f64, end: f64) -> Value {
    json!({"id": id, "trackId": "text", "type": "text", "startTime": start, "endTime": end, "textContent": "Safe areas",
        "fontSize": 80, "transform": {"x": x, "y": 0}})
}

#[test]
fn a_title_2_percent_from_the_edge_is_reported_with_its_time() {
    // Measure the title's width, then place its right edge at 98 % of the 1920 px frame from 1 s on (keyframes).
    let probe = doc(vec![title("t", 0.0, 0.0, 2.0)]);
    let q = layer_geometry(&probe, 0, 1080).layers[0].crop_quad;
    let width = q[1][0] - q[0][0];
    let x = 0.98 * 1920.0 - 960.0 - width / 2.0;
    let mut item = title("t", 0.0, 0.0, 2.0);
    item["keyframeTracks"] = json!([{"property": "x", "keyframes": [{"time": 0, "value": 0}, {"time": 1, "value": x}, {"time": 2, "value": x}]}]);
    let d = doc(vec![item]);
    let f = findings(&d);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].item_ids, vec!["t".to_string()]);
    assert!(f[0].message.contains("right edge at 98.0 % of the width"), "{}", f[0].message);
    // The first sampled time it is outside is named (sampled every 0.25 s: 1 s here).
    let at: f64 = f[0].message.split(" at ").nth(1).unwrap().split(" s").next().unwrap().parse().unwrap();
    assert!(at > 0.5 && at <= 1.0, "{}", f[0].message);
}

#[test]
fn titles_and_captions_inside_title_safe_are_not_reported() {
    let d = doc(vec![
        title("t", 0.0, 0.0, 2.0),
        json!({"id": "c", "trackId": "caps", "type": "caption", "startTime": 0, "endTime": 2,
            "captionText": "Inside the safe area"}),
    ]);
    assert!(findings(&d).is_empty(), "{:?}", findings(&d));
    // A caption moved down to the bottom edge is reported.
    let d = doc(vec![json!({"id": "c", "trackId": "caps", "type": "caption", "startTime": 0, "endTime": 2,
        "captionText": "Too low", "transform": {"y": 120}})]);
    let f = findings(&d);
    assert_eq!(f.len(), 1);
    assert!(
        f[0].message.starts_with("caption c is outside") && f[0].message.contains("bottom edge"),
        "{}",
        f[0].message
    );
}
