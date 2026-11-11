//! Projection performance (task 7.2). `cargo test --release -p ove-projection --test perf -- --ignored --nocapture`.

use std::time::Instant;

use ove_core::ops::{Op, apply_batch};
use ove_projection::{ApplyInput, plan, print};
use serde_json::json;

pub fn big(n: usize) -> ove_core::Document {
    let mut ops = Vec::new();
    for t in 0..8 {
        let kind = ["video", "video", "caption", "video", "video", "audio", "audio", "adjustment"][t];
        ops.push(Op::AddTrack {
            track: json!({"id": format!("t{t}"), "name": format!("Track {t}"), "order": t, "type": kind}),
        });
    }
    for m in 0..50 {
        ops.push(Op::AddMedia { media: json!({"id": format!("m{m}"), "data": {"type": "video", "fileName": format!("clip{m}.mov"), "duration": 120, "width": 3840, "height": 2160}}), entry: Some(json!({"mediaId": format!("m{m}"), "nativePath": format!("/Volumes/Footage/clip{m}.mov"), "ownedByBundle": false, "probeStatus": "probed"})) });
    }
    for i in 0..n {
        let t = i % 8;
        let start = (i / 8) as f64 * 2.5;
        let item = match t {
            2 => {
                json!({"id": format!("i{i}"), "trackId": "t2", "type": "caption", "startTime": start, "endTime": start + 2.4, "captionText": "Some words here", "captionSegments": [{"id": "s", "text": "Some words here", "startTime": 0, "endTime": 2.4}]})
            }
            5 | 6 => {
                json!({"id": format!("i{i}"), "trackId": format!("t{t}"), "type": "audio", "startTime": start, "endTime": start + 2.4, "mediaItemId": format!("m{}", i % 50), "volume": 80})
            }
            7 => {
                json!({"id": format!("i{i}"), "trackId": "t7", "type": "adjustment", "startTime": start, "endTime": start + 2.4, "colorGrading": {"contrast": 0.1}})
            }
            _ => {
                json!({"id": format!("i{i}"), "trackId": format!("t{t}"), "type": "video", "startTime": start, "endTime": start + 2.4, "name": format!("Shot {i}"), "trimStart": 1.25, "mediaItemId": format!("m{}", i % 50), "transform": {"x": 12, "scale": 1.1}, "opacity": 0.95})
            }
        };
        ops.push(Op::AddItem { item });
    }
    apply_batch(&ove_core::Document::new("p", "Big", 7), &ops).unwrap().doc
}

#[test]
#[ignore = "performance; run in release"]
fn print_and_apply_5000_items() {
    let doc = big(5000);
    let mut prints = Vec::new();
    let mut text = String::new();
    for _ in 0..10 {
        let t = Instant::now();
        text = print(&doc, &Default::default());
        prints.push(t.elapsed());
    }
    prints.sort();
    // One-attribute edit, then check + apply (plan + validated batch).
    let edited = text.replacen("name=\"Shot 2600\"", "name=\"Shot 2600 (final)\"", 1);
    assert!(edited != text, "edit target not found");
    let mut applies = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        let p = plan(&ApplyInput {
            text: &edited,
            current: &doc,
            base_for: &|_| Ok(doc.clone()),
            bundle: None,
            prefer: Default::default(),
            now_ms: 1,
        })
        .unwrap();
        let _ = apply_batch(&doc, &p.ops).unwrap();
        applies.push(t.elapsed());
        assert_eq!(p.counts.updated, 1);
    }
    applies.sort();
    eprintln!(
        "PERF 5,000-item projection {:.1} KB: print p50 {:?}; one-item check + apply p50 {:?} (max {:?})",
        text.len() as f64 / 1024.0,
        prints[5],
        applies[5],
        applies[9]
    );
}
