//! Golden render of keyframe easings and crop keyframes (add-markers-keyframe-lanes-and-versions task 3.1), through
//! the full renderer that export uses; preview evaluates layers with the same `eval` code.
//!
//! The fixture is a white full-frame layer over black. For each easing (linear, ease-in, ease-out, ease-in-out,
//! hold) its opacity has keyframes 0 at 0 s, with that easing, and 1 at 2 s; the frame is rendered at 0.5 s, 1 s and
//! 1.5 s, so its grey level is the easing curve. A second layer has `crop.left` keyframes 0 at 0 s and 50 at 1 s
//! (percent), rendered at 0.5 s: the left quarter is black. Every frame is checked against the expected values and
//! against `tests/golden/keyframes-32x18.pam` (all frames stacked), with a tolerance of one 8-bit step.
//! Regenerate the golden file with `OVE_UPDATE_GOLDEN=1` after checking it by eye.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

mod golden_util;

use golden_util::{TOLERANCE, golden_or_bless, max_diff};
use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::schema::KEYFRAME_EASINGS;
use ove_render::{Renderer, Rgba};
use serde_json::{Value, json};

const GOLDEN: &str = "keyframes-32x18.pam";
const OUT_H: u32 = 18;
const WHITE: &str = "export default function White() {
  return <Fill><div style={{ position: 'absolute', left: 0, top: 0, width: 1920, height: 1080,
    backgroundColor: '#ffffff' }} /></Fill>;
}";

fn doc(tracks: Value) -> Document {
    let ops = vec![
        Op::SetAspectRatio { aspect_ratio: "16:9".into() },
        Op::AddTrack { track: json!({"id": "v", "name": "V", "order": 0, "type": "video"}) },
        Op::AddItem {
            item: json!({"id": "w", "trackId": "v", "type": "code", "startTime": 0, "endTime": 4,
                "codeComposition": {"source": WHITE}, "keyframeTracks": tracks}),
        },
    ];
    apply_batch(&Document::new("p", "Keyframes", 1), &ops).unwrap().doc
}

fn opacity_doc(easing: &str) -> Document {
    doc(
        json!([{"property": "opacity", "keyframes": [{"time": 0, "value": 0, "easing": easing}, {"time": 2, "value": 1}]}]),
    )
}

fn crop_doc() -> Document {
    doc(json!([{"property": "crop.left", "keyframes": [{"time": 0, "value": 0}, {"time": 1, "value": 50}]}]))
}

fn render(d: &Document, t_us: i64) -> Rgba {
    Renderer::new(None, OUT_H).render(d, t_us)
}

/// Mean red level of a frame's columns `[x0, x1)`.
fn level(img: &Rgba, x0: u32, x1: u32) -> f64 {
    let mut sum = 0u64;
    let mut n = 0u64;
    for y in 0..img.h {
        for x in x0..x1 {
            sum += img.px(x, y)[0] as u64;
            n += 1;
        }
    }
    sum as f64 / n as f64
}

#[test]
fn easings_and_crop_keyframes_match_their_curves_and_golden_frames() {
    let mut frames: Vec<Rgba> = Vec::new();
    for e in KEYFRAME_EASINGS {
        for t in [500_000, 1_000_000, 1_500_000] {
            let img = render(&opacity_doc(e), t);
            let want = ove_core::retime::ease(e, t as f64 / 2e6) * 255.0;
            let got = level(&img, 0, img.w);
            assert!((got - want).abs() <= 2.0, "{e} at {t} us: grey {got:.1}, expected {want:.1}");
            frames.push(img);
        }
    }
    let crop = render(&crop_doc(), 500_000);
    let quarter = crop.w / 4;
    assert!(level(&crop, 0, quarter - 1) <= 1.0, "the left quarter is cropped away");
    assert!(level(&crop, quarter + 1, crop.w) >= 254.0, "the rest shows the layer");
    frames.push(crop);

    let (w, h) = (frames[0].w, frames[0].h);
    assert_eq!((w, h), (32, 18));
    let stacked: Vec<u8> = frames.iter().flat_map(|f| f.data.iter().copied()).collect();
    let total_h = h * frames.len() as u32;
    let (gw, gh, golden) = golden_or_bless(GOLDEN, w, total_h, &stacked);
    assert_eq!((gw, gh), (w, total_h));
    let d = max_diff(&stacked, &golden);
    assert!(d <= TOLERANCE, "keyframe frames changed (max channel difference {d})");
}

#[test]
fn hold_steps_at_the_next_keyframe() {
    // Spec scenario "Hold keyframe": opacity 1 at 0 s with hold, 0 at 2 s.
    let d = doc(
        json!([{"property": "opacity", "keyframes": [{"time": 0, "value": 1, "easing": "hold"}, {"time": 2, "value": 0}]}]),
    );
    assert!(level(&render(&d, 1_900_000), 0, 32) >= 254.0, "1.9 s: still fully visible");
    assert!(level(&render(&d, 2_000_000), 0, 32) <= 1.0, "2 s: gone");
}
