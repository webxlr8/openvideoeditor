//! Layer geometry (add-viewer-interaction-and-text-fills, design D1): where every visible layer is on screen at a
//! time — the corners of its source after placement and transform, the corners of the part its crop keeps, and its
//! stacking order — and hit testing from that geometry plus the layer's own rendered pixels.
//!
//! The geometry comes from the compositor's own data: [`crate::eval::layers_at`] for what is visible and in which
//! order, and the placement functions the renderer draws with ([`media_placement`], [`TextFrame`], [`code_place`],
//! the caption layout). Transition motion (a push or a slide while it runs) is not part of the geometry: it is the
//! layer's resting place.

use std::sync::Arc;

use ove_core::model::Document;
use ove_core::time::Micros;
use serde::Serialize;
use serde_json::Value;

use crate::canvas::{Place, Rgba};
use crate::captions::{CaptionCache, CaptionPlacement, draw_caption};
use crate::eval::{Layer, LayerKind, canvas_size, layers_at, reference_size};
use crate::render::{Renderer, TextFrame, code_place, code_size, layer_area, media_placement};

/// A point in canvas pixels: x to the right, y down, from the frame's top-left corner.
pub type Point = [f64; 2];

/// Where one visible layer is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerGeometry {
    pub item_id: String,
    /// The item's type (`video`, `image`, `text`, `caption`, `code`, `adjustment`, `shader`).
    pub item_type: String,
    /// Stacking order: 0 is the bottom layer.
    pub index: usize,
    /// The whole source's top-left, top-right, bottom-right and bottom-left corners after placement and transform (a
    /// mirrored layer's source top-left lands on the right).
    pub quad: [Point; 4],
    /// The same corners of the part the crop keeps.
    pub crop_quad: [Point; 4],
    /// Clockwise degrees.
    pub rotation: f64,
    pub opacity: f32,
    /// False for adjustment and shader layers: they have no pixels of their own and cover the whole frame.
    pub selectable: bool,
}

/// The geometry of a frame: its size and its visible layers, bottom first.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameGeometry {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<LayerGeometry>,
}

/// The pixel size of a video or image source from its media record (`data.width`, `data.height`).
pub fn recorded_size(doc: &Document, layer: &Layer) -> Option<(u32, u32)> {
    let media_id = match &layer.kind {
        LayerKind::Video { media_id, .. } | LayerKind::Image { media_id } => media_id,
        _ => return None,
    };
    let data = doc.project.media.get(media_id)?.data()?;
    let n = |k: &str| data.get(k).and_then(Value::as_f64).filter(|v| *v >= 1.0).map(|v| v as u32);
    Some((n("width")?, n("height")?))
}

/// Layer geometry at `t_us` on a canvas `height` pixels high (the width follows the aspect ratio), with video and
/// image sizes from their media records. Layers whose size is unknown are left out.
pub fn layer_geometry(doc: &Document, t_us: Micros, height: u32) -> FrameGeometry {
    layer_geometry_with(doc, t_us, height, &mut |l| recorded_size(doc, l))
}

/// [`layer_geometry`] with the video and image sizes from `media_size` (for example the decoded frames).
pub fn layer_geometry_with(
    doc: &Document,
    t_us: Micros,
    height: u32,
    media_size: &mut dyn FnMut(&Layer) -> Option<(u32, u32)>,
) -> FrameGeometry {
    let (w, h) = canvas_size(&doc.project.aspect_ratio, height);
    let (_, ref_h) = reference_size(&doc.project.aspect_ratio);
    let k = h as f64 / ref_h as f64;
    let mut layers = Vec::new();
    for (index, layer) in layers_at(doc, t_us).into_iter().enumerate() {
        let item_type = doc.item(&layer.item_id).map(|i| i.kind.clone()).unwrap_or_default();
        let Some((quad, crop_quad, rotation, selectable)) = locate(doc, &layer, w, h, k, media_size) else { continue };
        layers.push(LayerGeometry {
            item_id: layer.item_id.clone(),
            item_type,
            index,
            quad,
            crop_quad,
            rotation,
            opacity: layer.opacity,
            selectable,
        });
    }
    FrameGeometry { width: w, height: h, layers }
}

/// The corners of the `sw`×`sh` source rectangle `[left, top, right, bottom]` (source pixels) placed by `place`.
fn corners(place: &Place, sw: f64, sh: f64, [l, t, r, b]: [f64; 4]) -> [Point; 4] {
    let (sin, cos) = place.rotation.to_radians().sin_cos();
    [(l, t), (r, t), (r, b), (l, b)].map(|(u, v)| {
        let (lx, ly) = ((u - sw / 2.0) * place.sx, (v - sh / 2.0) * place.sy);
        [lx * cos - ly * sin + place.cx, lx * sin + ly * cos + place.cy]
    })
}

/// Whole-source and cropped corners of a `sw`×`sh` source placed by `place` with `crop` (percent per side: top,
/// right, bottom, left).
fn quads(place: &Place, sw: f64, sh: f64, crop: [f32; 4]) -> ([Point; 4], [Point; 4]) {
    let c = crop.map(|v| v.clamp(0.0, 100.0) as f64 / 100.0);
    let kept = [sw * c[3], sh * c[0], sw - sw * c[1], sh - sh * c[2]];
    (corners(place, sw, sh, [0.0, 0.0, sw, sh]), corners(place, sw, sh, kept))
}

type Located = ([Point; 4], [Point; 4], f64, bool);

fn locate(
    doc: &Document,
    layer: &Layer,
    w: u32,
    h: u32,
    k: f64,
    media_size: &mut dyn FnMut(&Layer) -> Option<(u32, u32)>,
) -> Option<Located> {
    let (_, _, area_w, area_h, centre) = layer_area(layer, w, h, k);
    match &layer.kind {
        LayerKind::Video { .. } | LayerKind::Image { .. } => {
            let (fw, fh) = media_size(layer)?;
            let (place, crop) = media_placement(layer, fw, fh, w, h, k);
            let (quad, crop_quad) = quads(&place, fw as f64, fh as f64, crop);
            Some((quad, crop_quad, place.rotation, true))
        }
        LayerKind::Text => {
            let text = TextFrame::of(layer, k, area_w)?;
            let img = text.animated(Arc::new(text.raster()));
            let place = text.place(layer, centre);
            // The renderer draws text without a crop.
            let (quad, crop_quad) = quads(&place, img.w as f64, img.h as f64, [0.0; 4]);
            Some((quad, crop_quad, place.rotation, true))
        }
        LayerKind::Code => {
            let (cw, ch) = code_size(layer, &doc.project.aspect_ratio)?;
            let place = code_place(layer, area_w, area_h, centre, (cw, ch));
            let (quad, crop_quad) = quads(&place, cw as f64, ch as f64, layer.crop);
            Some((quad, crop_quad, place.rotation, true))
        }
        LayerKind::Caption { words, .. } => {
            // Captions are laid out word by word on the canvas: their box is the bounds of what they draw.
            let mut canvas = Rgba::new(w, h);
            let x = &layer.xform;
            let at = CaptionPlacement { k: k as f32, dx: x.x * k, dy: x.y * k, scale: x.scale, opacity: 1.0 };
            draw_caption(&mut canvas, &mut CaptionCache::default(), &layer.fields, words, layer.rel_us, at);
            let [x0, y0, x1, y1] = alpha_bounds(&canvas)?;
            let quad = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
            Some((quad, quad, 0.0, true))
        }
        LayerKind::Adjustment | LayerKind::Shader => {
            let (fw, fh) = (w as f64, h as f64);
            let quad = [[0.0, 0.0], [fw, 0.0], [fw, fh], [0.0, fh]];
            Some((quad, quad, 0.0, false))
        }
    }
}

/// The bounds `[x0, y0, x1, y1]` (pixel edges) of the pixels with alpha above zero, if any.
fn alpha_bounds(img: &Rgba) -> Option<[f64; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (i, px) in img.data.chunks_exact(4).enumerate() {
        if px[3] == 0 {
            continue;
        }
        let (x, y) = (i as u32 % img.w, i as u32 / img.w);
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x + 1);
        y1 = y1.max(y + 1);
    }
    (x1 > x0).then_some([x0 as f64, y0 as f64, x1 as f64, y1 as f64])
}

/// True when `p` is inside the convex quad `q` (either winding; the edges count as inside).
pub fn quad_contains(q: &[Point; 4], p: Point) -> bool {
    let mut sign = 0.0f64;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if cross.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// The item whose layer is topmost under the point (`x`, `y` as fractions of the frame, 0..1) at `t_us`: its crop
/// quad holds the point and its own rendered pixel there has alpha above zero. `None` when no layer is drawn there.
/// The geometry uses the decoded frame sizes and the renderer's canvas size.
pub fn hit_test(renderer: &mut Renderer, doc: &Document, t_us: Micros, x: f64, y: f64) -> Option<String> {
    let (w, h) = renderer.size(doc);
    let geometry = layer_geometry_with(doc, t_us, h, &mut |l| renderer.source_size(doc, l));
    let p = [x.clamp(0.0, 1.0) * w as f64, y.clamp(0.0, 1.0) * h as f64];
    let (px, py) = ((p[0] as u32).min(w.saturating_sub(1)), (p[1] as u32).min(h.saturating_sub(1)));
    for layer in geometry.layers.iter().rev().filter(|l| l.selectable) {
        if !quad_contains(&layer.crop_quad, p) {
            continue;
        }
        let Some(img) = renderer.render_layer(doc, t_us, &layer.item_id) else { continue };
        if img.w == w && img.h == h && img.px(px, py)[3] > 0 {
            return Some(layer.item_id.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::ops::{Op, apply_batch};
    use serde_json::json;

    fn doc(items: Vec<Value>) -> Document {
        let mut ops = vec![
            Op::AddTrack { track: json!({"id": "top", "name": "Top", "order": 0, "type": "video"}) },
            Op::AddTrack { track: json!({"id": "main", "name": "Main", "order": 1, "type": "video"}) },
            Op::AddMedia {
                media: json!({"id": "m", "data": {"type": "video", "duration": 100, "width": 1280, "height": 720}}),
                entry: None,
            },
        ];
        ops.extend(items.into_iter().map(|item| Op::AddItem { item }));
        apply_batch(&Document::new("p", "p", 1), &ops).unwrap().doc
    }

    fn close(a: Point, b: Point) -> bool {
        (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
    }

    #[test]
    fn a_full_frame_clip_covers_the_frame() {
        let d = doc(vec![
            json!({"id": "a", "trackId": "main", "type": "video", "startTime": 0, "endTime": 5, "mediaItemId": "m"}),
        ]);
        let g = layer_geometry(&d, 1_000_000, 1080);
        assert_eq!((g.width, g.height), (1920, 1080));
        let l = &g.layers[0];
        assert_eq!((l.item_id.as_str(), l.item_type.as_str(), l.index, l.selectable), ("a", "video", 0, true));
        assert_eq!(l.quad, [[0.0, 0.0], [1920.0, 0.0], [1920.0, 1080.0], [0.0, 1080.0]]);
        assert_eq!(l.crop_quad, l.quad);
    }

    #[test]
    fn transform_rotation_and_crop_move_the_corners() {
        let d = doc(vec![json!({"id": "a", "trackId": "main", "type": "video", "startTime": 0, "endTime": 5,
            "mediaItemId": "m", "transform": {"x": 100, "y": -50, "scale": 0.5, "rotation": 90},
            "crop": {"right": 50}})]);
        let l = &layer_geometry(&d, 0, 1080).layers[0];
        // Half size: 960x540 around (1060, 490), turned a quarter clockwise: the source's top-left goes top-right.
        let want = [[1330.0, 10.0], [1330.0, 970.0], [790.0, 970.0], [790.0, 10.0]];
        for (got, want) in l.quad.iter().zip(want) {
            assert!(close(*got, want), "{:?}", l.quad);
        }
        // The right half is cropped: the kept part ends half way down the turned source.
        assert!(close(l.crop_quad[1], [1330.0, 490.0]) && close(l.crop_quad[2], [790.0, 490.0]), "{:?}", l.crop_quad);
        assert_eq!(l.rotation, 90.0);
    }

    #[test]
    fn stacking_follows_the_compositor_and_adjustments_are_not_selectable() {
        let d = doc(vec![
            json!({"id": "a", "trackId": "main", "type": "video", "startTime": 0, "endTime": 5, "mediaItemId": "m"}),
            json!({"id": "adj", "trackId": "top", "type": "adjustment", "startTime": 0, "endTime": 5,
                "colorGrading": {"saturation": -1}}),
        ]);
        let g = layer_geometry(&d, 0, 540);
        let ids: Vec<_> = g.layers.iter().map(|l| (l.item_id.as_str(), l.index, l.selectable)).collect();
        assert_eq!(ids, [("a", 0, true), ("adj", 1, false)]);
        assert_eq!(g.width, 960);
    }

    #[test]
    fn text_quads_scale_with_the_canvas() {
        let d = doc(vec![json!({"id": "t", "trackId": "top", "type": "text", "startTime": 0, "endTime": 5,
            "textContent": "Hello", "fontSize": 100, "backgroundColor": "#000000", "transform": {"y": 300}})]);
        let full = &layer_geometry(&d, 0, 1080).layers[0];
        let half = &layer_geometry(&d, 0, 540).layers[0];
        let centre = |q: &[Point; 4]| [(q[0][0] + q[2][0]) / 2.0, (q[0][1] + q[2][1]) / 2.0];
        assert!(close(centre(&full.quad), [960.0, 840.0]), "{:?}", full.quad);
        assert!(close(centre(&half.quad), [480.0, 420.0]), "{:?}", half.quad);
        let width = |q: &[Point; 4]| q[1][0] - q[0][0];
        assert!((width(&full.quad) / width(&half.quad) - 2.0).abs() < 0.05);
    }

    #[test]
    fn unknown_media_sizes_and_hidden_items_are_left_out() {
        let mut d = doc(vec![
            json!({"id": "a", "trackId": "main", "type": "video", "startTime": 0, "endTime": 5, "mediaItemId": "m"}),
            json!({"id": "t", "trackId": "top", "type": "text", "startTime": 0, "endTime": 5, "textContent": " "}),
        ]);
        assert_eq!(layer_geometry(&d, 0, 540).layers.len(), 1, "empty text draws nothing");
        let set = json!({"data": {"type": "video"}}).as_object().unwrap().clone();
        let ops = [Op::UpdateMedia {
            id: "m".into(),
            set,
            unset: vec![],
            entry_set: Default::default(),
            entry_unset: vec![],
        }];
        d = apply_batch(&d, &ops).unwrap().doc;
        assert!(layer_geometry(&d, 0, 540).layers.is_empty());
    }

    #[test]
    fn quads_contain_points_in_either_winding() {
        let q = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!(quad_contains(&q, [5.0, 5.0]) && quad_contains(&q, [10.0, 5.0]));
        assert!(!quad_contains(&q, [10.5, 5.0]));
        let mirrored = [q[1], q[0], q[3], q[2]];
        assert!(quad_contains(&mirrored, [5.0, 5.0]) && !quad_contains(&mirrored, [-1.0, 5.0]));
    }
}
