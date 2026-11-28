//! Layer geometry against the rendered frame (add-viewer-interaction-and-text-fills task 1.1): for random transforms,
//! rotations and crops of a video, an image and a boxed title, a layer's crop quad holds a sampled point exactly when
//! the layer's own rendered alpha there is above zero, for at least 99.5 % of the points. Hit testing picks the layer
//! under a click by its pixels, not its box. Needs the ffmpeg CLI to make the media (see `render.rs`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

#[path = "../../../tests/media/fixtures.rs"]
mod fixtures;

use std::path::{Path, PathBuf};
use std::process::Command;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_render::Renderer;
use ove_render::geometry::{hit_test, layer_geometry, layer_geometry_with, quad_contains};
use serde_json::{Value, json};

fn make(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let st = Command::new(fixtures::fixture_ffmpeg())
        .args(["-loglevel", "error", "-y"])
        .args(args)
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success(), "ffmpeg failed for {name}");
    out
}

/// A 320x180 red clip and a 200x300 green still.
fn media(dir: &Path) -> Vec<Op> {
    let clip =
        make(dir, "red.mp4", &["-f", "lavfi", "-i", "color=c=red:s=320x180:r=25", "-t", "2", "-pix_fmt", "yuv420p"]);
    // A one-frame clip stands in for the still: the LGPL decode build may have no PNG reader.
    let still = make(
        dir,
        "green.mp4",
        &["-f", "lavfi", "-i", "color=c=0x30c030:s=200x300", "-frames:v", "1", "-pix_fmt", "yuv420p"],
    );
    let add = |id: &str, p: &Path, kind: &str, (w, h): (u32, u32)| Op::AddMedia {
        media: json!({"id": id, "data": {"type": kind, "fileName": p.file_name().unwrap().to_string_lossy(),
            "duration": 2, "width": w, "height": h}}),
        entry: Some(json!({"mediaId": id, "nativePath": p.display().to_string(), "ownedByBundle": false})),
    };
    vec![add("clip", &clip, "video", (320, 180)), add("still", &still, "image", (200, 300))]
}

fn tracks() -> Vec<Op> {
    ["t1", "t2", "t3"]
        .iter()
        .enumerate()
        .map(|(n, id)| Op::AddTrack { track: json!({"id": id, "name": id, "order": n, "type": "video"}) })
        .collect()
}

fn build(base: &[Op], items: Vec<Value>) -> Document {
    let mut ops = base.to_vec();
    ops.extend(items.into_iter().map(|item| Op::AddItem { item }));
    apply_batch(&Document::new("p", "p", 1), &ops).unwrap().doc
}

/// A small deterministic generator (xorshift), so a failure names a reproducible case.
struct Gen(u64);
impl Gen {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
}

fn transform(g: &mut Gen) -> Value {
    json!({
        "x": g.range(-700.0, 700.0).round(), "y": g.range(-400.0, 400.0).round(),
        "scale": (g.range(0.2, 1.4) * 100.0).round() / 100.0, "rotation": g.range(-180.0, 180.0).round(),
        "flipX": g.next() < 0.2,
    })
}

fn crop(g: &mut Gen) -> Value {
    let mut c = || if g.next() < 0.5 { 0.0 } else { g.range(0.0, 40.0).round() };
    json!({"top": c(), "right": c(), "bottom": c(), "left": c()})
}

/// Cases: `OVE_PROPTEST_CASES` (each case renders three layers), 128 by default.
fn cases() -> u64 {
    std::env::var("OVE_PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(128)
}

#[test]
fn geometry_matches_the_rendered_alpha() {
    if !fixtures::require_tool(&fixtures::fixture_ffmpeg(), "it makes the geometry fixtures; set OVE_FIXTURE_FFMPEG") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut base = tracks();
    base.extend(media(dir.path()));
    let mut renderer = Renderer::new(None, 270);
    let (mut agree, mut total) = (0u64, 0u64);
    let mut worst: Option<(u64, String, f64)> = None;
    for case in 0..cases() {
        let mut g = Gen(0x9e37_79b9_7f4a_7c15 ^ (case + 1).wrapping_mul(0x2545_f491_4f6c_dd1d));
        let doc = build(
            &base,
            vec![
                json!({"id": "v", "trackId": "t3", "type": "video", "startTime": 0, "endTime": 2,
                    "mediaItemId": "clip", "transform": transform(&mut g), "crop": crop(&mut g)}),
                json!({"id": "i", "trackId": "t2", "type": "video", "startTime": 0, "endTime": 2,
                    "mediaItemId": "still", "transform": transform(&mut g), "crop": crop(&mut g)}),
                json!({"id": "t", "trackId": "t1", "type": "text", "startTime": 0, "endTime": 2,
                    "textContent": "Title", "fontSize": 90, "backgroundColor": "#202020",
                    "transform": transform(&mut g)}),
            ],
        );
        let geo = layer_geometry_with(&doc, 500_000, 270, &mut |l| renderer.source_size(&doc, l));
        assert_eq!(geo.layers.len(), 3, "case {case}: {:?}", geo.layers.iter().map(|l| &l.item_id).collect::<Vec<_>>());
        for layer in &geo.layers {
            let img = renderer.render_layer(&doc, 500_000, &layer.item_id).unwrap();
            assert_eq!((img.w, img.h), (geo.width, geo.height));
            let (mut ok, mut n) = (0u64, 0u64);
            for y in (0..img.h).step_by(2) {
                for x in (0..img.w).step_by(2) {
                    let inside = quad_contains(&layer.crop_quad, [x as f64 + 0.5, y as f64 + 0.5]);
                    let drawn = img.px(x, y)[3] > 0;
                    ok += u64::from(inside == drawn);
                    n += 1;
                }
            }
            let share = ok as f64 / n as f64;
            if worst.as_ref().is_none_or(|w| share < w.2) {
                worst = Some((case, layer.item_id.clone(), share));
            }
            agree += ok;
            total += n;
        }
    }
    let share = agree as f64 / total as f64;
    eprintln!("geometry agrees with the rendered alpha at {:.4}% of {total} points; worst {worst:?}", share * 100.0);
    assert!(share >= 0.995, "{:.4}% < 99.5%; worst {worst:?}", share * 100.0);
}

#[test]
fn a_click_on_a_transparent_part_of_a_title_selects_the_clip_below() {
    if !fixtures::require_tool(&fixtures::fixture_ffmpeg(), "it makes the geometry fixtures; set OVE_FIXTURE_FFMPEG") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut base = tracks();
    base.extend(media(dir.path()));
    let doc = build(
        &base,
        vec![
            json!({"id": "v", "trackId": "t3", "type": "video", "startTime": 0, "endTime": 2, "mediaItemId": "clip"}),
            json!({"id": "t", "trackId": "t1", "type": "text", "startTime": 0, "endTime": 2,
                "textContent": "I I", "fontSize": 300}),
        ],
    );
    let mut renderer = Renderer::new(None, 540);
    let geo = layer_geometry(&doc, 0, 540);
    let title = geo.layers.iter().find(|l| l.item_id == "t").unwrap();
    // The gap between the two letters is inside the title's box but has no title pixels.
    let img = renderer.render_layer(&doc, 0, "t").unwrap();
    let (cx, cy) = (img.w / 2, img.h / 2);
    assert!(quad_contains(&title.crop_quad, [cx as f64 + 0.5, cy as f64 + 0.5]));
    assert_eq!(img.px(cx, cy)[3], 0, "the middle of \"I I\" is a gap");
    assert_eq!(hit_test(&mut renderer, &doc, 0, 0.5, 0.5).as_deref(), Some("v"));
    // On a letter the title is chosen.
    let ink = (0..img.w).find(|&x| img.px(x, cy)[3] > 200).unwrap();
    let x = (ink as f64 + 2.0) / img.w as f64;
    assert_eq!(hit_test(&mut renderer, &doc, 0, x, 0.5).as_deref(), Some("t"));
    // Where nothing is drawn (the clip fills the frame, so look past the end of the timeline): no layer.
    assert_eq!(hit_test(&mut renderer, &doc, 5_000_000, 0.5, 0.5), None);
}
