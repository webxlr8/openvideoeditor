//! Text animations, font resolution and code items through the real renderer (no media files needed).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_render::Renderer;
use ove_render::canvas::Rgba;
use serde_json::{Value, json};

fn doc(items: Vec<Value>) -> Document {
    let mut ops = vec![Op::AddTrack { track: json!({"id": "top", "name": "Top", "order": 0, "type": "video"}) }];
    ops.extend(items.into_iter().map(|item| Op::AddItem { item }));
    apply_batch(&Document::new("p", "p", 1), &ops).unwrap().doc
}

fn text(extra: Value) -> Document {
    let mut item = json!({"id": "t", "trackId": "top", "type": "text", "startTime": 0, "endTime": 4,
        "textContent": "Motion", "fontSize": 160, "color": "#ffffff"});
    for (k, v) in extra.as_object().unwrap() {
        item[k] = v.clone();
    }
    doc(vec![item])
}

/// (lit pixel count, centroid x, centroid y) of bright pixels.
fn ink(f: &Rgba) -> (usize, f64, f64) {
    let (mut n, mut sx, mut sy) = (0usize, 0f64, 0f64);
    for y in 0..f.h {
        for x in 0..f.w {
            if f.px(x, y)[0] > 40 {
                n += 1;
                sx += x as f64;
                sy += y as f64;
            }
        }
    }
    (n, sx / n.max(1) as f64, sy / n.max(1) as f64)
}

fn at(d: &Document, t: f64) -> Rgba {
    Renderer::new(None, 360).render(d, (t * 1e6) as i64)
}

#[test]
fn rise_fades_and_moves_in_then_settles() {
    let still = ink(&at(&text(json!({})), 2.0));
    let d = text(json!({"textAnimation": {"preset": "rise", "duration": 1.0, "easing": "linear"}}));
    assert_eq!(ink(&at(&d, 0.0)).0, 0, "invisible at the start");
    let mid = ink(&at(&d, 0.5));
    assert!(mid.0 > 0 && mid.2 > still.2 + 5.0, "half way: visible and lower ({mid:?} vs {still:?})");
    assert_eq!(ink(&at(&d, 2.0)), still, "settled = unanimated");
}

#[test]
fn exit_and_scale_presets() {
    let still = ink(&at(&text(json!({})), 2.0));
    let d = text(json!({"textAnimation": {"preset": "none", "out": {"preset": "zoom-out", "duration": 1.0}}}));
    assert_eq!(ink(&at(&d, 1.0)), still);
    let leaving = ink(&at(&d, 3.3));
    assert!(leaving.0 > 0 && leaving.0 != still.0, "{leaving:?}");
    let pop = text(json!({"textAnimation": {"preset": "pop", "duration": 1.0}}));
    assert!(ink(&at(&pop, 0.15)).0 < still.0, "pop starts small");
}

#[test]
fn typewriter_reveals_left_to_right_and_blur_glitch_render() {
    let still = ink(&at(&text(json!({})), 2.0));
    let d = text(json!({"textAnimation": {"preset": "typewriter", "duration": 2.0, "easing": "linear"}}));
    let half = ink(&at(&d, 1.0));
    assert!(half.0 > 0 && half.0 < still.0 && half.1 < still.1, "left half only: {half:?} vs {still:?}");
    for preset in ["blur", "glitch", "focus-pop", "rotate-in", "slide-left", "settle-down"] {
        let d = text(json!({"textAnimation": {"preset": preset, "duration": 1.0}}));
        let mid = at(&d, 0.5);
        assert_ne!(ink(&mid), still, "{preset} differs mid-way");
        assert_eq!(mid, at(&d, 0.5), "{preset} is deterministic");
        assert_eq!(ink(&at(&d, 1.5)), still, "{preset} settles");
    }
}

#[test]
fn font_family_weight_and_style_resolve() {
    let base = ink(&at(&text(json!({})), 1.0)).0;
    let heavy = ink(&at(&text(json!({"fontFamily": "Inter", "fontWeight": 900})), 1.0)).0;
    let light = ink(&at(&text(json!({"fontFamily": "Inter", "fontWeight": 200})), 1.0)).0;
    assert!(heavy > base && base > light, "{light} < {base} < {heavy}");
    let serif = ink(&at(&text(json!({"fontFamily": "Playfair Display"})), 1.0)).0;
    let display = ink(&at(&text(json!({"fontFamily": "Bebas Neue"})), 1.0)).0;
    assert!(serif != base && display != base && serif != display);
    let missing = ink(&at(&text(json!({"fontFamily": "Definitely Not Installed"})), 1.0)).0;
    assert!(missing > 0, "falls back to a real face");
}

#[test]
fn code_items_render_templates_and_sources() {
    let d = doc(vec![json!({"id": "c", "trackId": "top", "type": "code", "startTime": 0, "endTime": 4,
        "codeComposition": {"template": "title-card", "props": {"title": "Hello"}}})]);
    let f = at(&d, 2.0);
    // The title card paints a gradient over the whole frame.
    let corner = f.px(2, 2);
    assert!(corner[2] > 60, "gradient background {corner:?}");
    assert!(ink(&f).0 > 200, "title letters");
    assert_ne!(at(&d, 0.1), at(&d, 2.0), "animates over time");

    let src = doc(vec![json!({"id": "c", "trackId": "top", "type": "code", "startTime": 0, "endTime": 2,
        "codeComposition": {"source": "export default function C({ time, width }) {\n  return <div style={{ position: 'absolute', left: time * width / 2, top: 0, width: 200, height: 200, backgroundColor: '#ff0000' }} />;\n}"}})]);
    let red_x = |f: &Rgba| {
        let xs: Vec<u32> = (0..f.w).filter(|&x| f.px(x, 10)[0] > 200 && f.px(x, 10)[1] < 60).collect();
        xs.iter().sum::<u32>() as f64 / xs.len().max(1) as f64
    };
    let (a, b) = (red_x(&at(&src, 0.2)), red_x(&at(&src, 1.0)));
    assert!(b > a + 50.0, "box moves right with time: {a} → {b}");

    // Broken code renders nothing (and does not panic); the rest of the frame is unaffected.
    let bad = doc(vec![json!({"id": "c", "trackId": "top", "type": "code", "startTime": 0, "endTime": 2,
        "codeComposition": {"source": "export default function C() { for (;;) {} }"}})]);
    assert_eq!(ink(&at(&bad, 0.5)).0, 0);
}
