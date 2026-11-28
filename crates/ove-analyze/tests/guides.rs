//! Viewer guides in agent frames (`look --overlay`, add-viewer-interaction-and-text-fills tasks 3.1 and 3.2): golden
//! pictures, and an exported video that never carries them.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::Path;
use std::sync::atomic::AtomicBool;

use ove_analyze::Image;
use ove_analyze::guides::{draw, parse};
use ove_core::model::Document;
use ove_core::ops::Op;
use serde_json::json;

fn golden(name: &str, img: &Image) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    let png = img.png().unwrap();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &png).unwrap();
    }
    let want =
        std::fs::read(&path).unwrap_or_else(|_| panic!("{} is missing; run with UPDATE_GOLDEN=1", path.display()));
    assert!(want == png, "{name} changed (UPDATE_GOLDEN=1 rewrites it after a deliberate change)");
}

fn drawn(w: u32, h: u32, spec: &str) -> Image {
    let mut img = Image::filled(w, h, [40, 40, 40, 255]);
    draw(&mut img, &parse(spec).unwrap());
    img
}

#[test]
fn golden_overlays() {
    golden("overlay-safe-grid-16x9.png", &drawn(640, 360, "safe,grid"));
    golden("overlay-mask-9x16-centre-grid4-16x9.png", &drawn(640, 360, "mask:9:16,centre,grid:4"));
    golden("overlay-mask-239-9x16.png", &drawn(360, 640, "mask:2.39:1,safe"));
}

#[test]
fn safe_areas_sit_at_their_insets() {
    let img = drawn(1000, 500, "safe");
    // Title safe: 5 % in from each edge; action safe: 3.5 %.
    // Lines are 90 % of their colour over the frame: title safe green, action safe blue.
    assert_eq!(img.px(50, 250), [4, 234, 130, 255], "title-safe left edge at x = 50");
    let action = img.px(35, 250);
    assert!(action[0] < 10 && action[1] == 184 && action[2] > 230, "action-safe left edge at x = 35: {action:?}");
    assert_eq!(img.px(42, 250), [40, 40, 40, 255]);
}

/// A 16:9 project with a dark grey still.
fn dark_title() -> Document {
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t", "name": "Text", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "title", "trackId": "t", "type": "text", "startTime": 0, "endTime": 1,
            "textContent": "Guides", "fontSize": 90, "color": "#808080"}}
    ]))
    .unwrap();
    ove_core::apply_batch(&Document::new("p_1", "Guides", 0), &ops).unwrap().doc
}

#[test]
fn exports_never_carry_the_guides() {
    let doc = dark_title();
    let frame = ove_analyze::look(&doc, None, &[0.5], 320).unwrap().remove(0);
    let mut shown = frame.clone();
    draw(&mut shown, &parse("safe,grid,centre,mask:1:1").unwrap());
    // On the agent frame the title-safe line is drawn at x = 16 of 320 (5 %).
    let (x, y) = (16, 90);
    assert!(shown.px(x, y)[1] > 150, "the guide is on the agent frame: {:?}", shown.px(x, y));
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("guides.mp4");
    ove_render::export(&doc, None, &out, &ove_render::ExportSettings::h264(180, 25), &AtomicBool::new(false), |_| {})
        .unwrap();
    let exported = ove_analyze::video::grab(&out, 0.5, Some(320)).unwrap();
    assert_eq!((exported.width, exported.height), (320, 180));
    // The exported frame is black where the guides are drawn, and matches the frame without guides.
    for (gx, gy) in [(x, y), (106, 60), (213, 120), (50, 30), (300, 170)] {
        let p = exported.px(gx, gy);
        let q = frame.px(gx, gy);
        assert!(p[..3].iter().zip(&q[..3]).all(|(a, b)| a.abs_diff(*b) <= 12), "({gx}, {gy}): {p:?} vs {q:?}");
    }
    assert_ne!(shown.px(160, 90), frame.px(160, 90), "the centre cross is only on the agent frame");
}
