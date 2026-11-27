//! The coordinate grid on inspected frames: golden pictures, grid units that match item transforms, and export
//! frames that never carry it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::Path;

use ove_analyze::Image;
use ove_analyze::grid::{overlay, step};
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

#[test]
fn golden_grids() {
    let mut wide = Image::filled(640, 360, [24, 24, 24, 255]);
    overlay(&mut wide, (1920, 1080));
    golden("grid-16x9-640.png", &wide);
    let mut tall = Image::filled(360, 640, [24, 24, 24, 255]);
    overlay(&mut tall, (1080, 1920));
    golden("grid-9x16-360.png", &tall);
}

/// A 16:9 project with one white title moved to x = −320.
fn titled(x: f64) -> Document {
    let mut doc = Document::new("p_1", "Grid", 0);
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t", "name": "Text", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "title", "trackId": "t", "type": "text", "startTime": 0, "endTime": 4,
            "textContent": "HHHH", "fontSize": 120, "color": "#ffffff", "transform": {"x": x, "y": 0}}}
    ]))
    .unwrap();
    doc = ove_core::apply_batch(&doc, &ops).unwrap().doc;
    doc
}

/// Mean x of the bright pixels.
fn bright_centre(img: &Image) -> f64 {
    let (mut sum, mut n) = (0.0, 0.0);
    for y in 0..img.height {
        for x in 0..img.width {
            let p = img.px(x, y);
            if p[0] > 128 && p[1] > 128 && p[2] > 128 {
                sum += x as f64;
                n += 1.0;
            }
        }
    }
    assert!(n > 0.0, "the title was drawn");
    sum / n
}

#[test]
fn an_item_at_transform_x_minus_320_is_centred_on_the_line_labelled_minus_320() {
    let doc = titled(-320.0);
    let frame = ove_analyze::look(&doc, None, &[1.0], 640).unwrap().remove(0);
    let line = frame.width as f64 / 2.0 + -320.0 * frame.width as f64 / 1920.0;
    assert_eq!(step((1920, 1080)), 160, "−320 is a grid line (2 steps left of the centre)");
    let centre = bright_centre(&frame);
    assert!((centre - line).abs() <= 4.0, "title centre {centre:.1} px, grid line −320 at {line:.1} px");
    // On the grid frame the line is drawn there.
    let mut seen = frame.clone();
    overlay(&mut seen, (1920, 1080));
    assert_ne!(seen.px(line.round() as u32, 20), frame.px(line.round() as u32, 20));
}

#[test]
fn rendered_frames_never_carry_the_grid() {
    let doc = titled(0.0);
    let before = ove_analyze::look(&doc, None, &[1.0], 320).unwrap().remove(0);
    let mut shown = before.clone();
    overlay(&mut shown, (1920, 1080));
    assert_ne!(shown, before);
    // Rendering again (as export does) after grid frames were made gives the same pixels as before.
    let after = ove_analyze::look(&doc, None, &[1.0], 320).unwrap().remove(0);
    assert_eq!(after, before);
}
