//! Timeline evaluation: what is visible at a time, in which order, with which geometry and opacity. Pure (no I/O).
//!
//! Model (v1):
//! - Tracks composite bottom-up: the last track in order is the bottom layer, the first is the top.
//! - Hidden tracks, hidden items and pending placeholders render nothing.
//! - Visual items are placed by "fill height, centred" (the auto-reframe default); `transform` (pixels from the frame
//!   centre in the project's reference resolution, scale, rotation in degrees, flipX) is applied on top. An item with
//!   `autoReframe={false}` and no transform is fitted inside the frame instead.
//! - Keyframes (item-relative seconds) override x, y, scale, rotation and opacity.
//! - `transitionIn`/`transitionOut` of type `crossfade` (or `cross-dissolve`/`dissolve`) between two adjacent items on
//!   one track blend them over the transition duration after the cut, with the outgoing item continuing; any other
//!   transition type fades the item in/out over its duration.

use ove_core::model::{Document, Item};
use ove_core::time::{Micros, json_to_micros};
use serde_json::{Map, Value};

/// Reference frame size of an aspect ratio: the pixel space of transforms, font sizes and borders.
pub fn reference_size(aspect: &str) -> (u32, u32) {
    match aspect {
        "9:16" => (1080, 1920),
        "1:1" => (1080, 1080),
        "4:5" => (1080, 1350),
        "4:3" => (1440, 1080),
        "3:4" => (1080, 1440),
        _ => (1920, 1080),
    }
}

/// Canvas size for an output height (even dimensions, as encoders require).
pub fn canvas_size(aspect: &str, height: u32) -> (u32, u32) {
    let (rw, rh) = reference_size(aspect);
    let h = height.max(2) & !1;
    let w = ((rw as f64 * h as f64 / rh as f64).round() as u32).max(2) & !1;
    (w, h)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xform {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub rotation: f64,
    pub flip_x: bool,
}

impl Default for Xform {
    fn default() -> Self {
        Xform { x: 0.0, y: 0.0, scale: 1.0, rotation: 0.0, flip_x: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Source height fills the frame height, centred (then the transform).
    FillHeight,
    /// Whole source inside the frame.
    Fit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Multiply,
    Screen,
    Darken,
    Lighten,
    Add,
}

impl Blend {
    pub fn parse(s: Option<&str>) -> Blend {
        match s {
            Some("multiply") => Blend::Multiply,
            Some("screen") => Blend::Screen,
            Some("darken") => Blend::Darken,
            Some("lighten") => Blend::Lighten,
            Some("color-dodge") | Some("add") => Blend::Add,
            _ => Blend::Normal,
        }
    }
}

/// Basic colour adjustments (`colorGrading`), each roughly −1…1 (exposure in stops).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Grade {
    pub exposure: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub vibrance: f32,
    pub temperature: f32,
    pub tint: f32,
    pub gamma: f32,
    pub highlights: f32,
    pub shadows: f32,
}

impl Grade {
    pub fn from_fields(f: &Map<String, Value>) -> Option<Grade> {
        let g = f.get("colorGrading")?.as_object()?;
        let n = |k: &str| g.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let grade = Grade {
            exposure: n("exposure"),
            brightness: n("brightness"),
            contrast: n("contrast"),
            saturation: n("saturation"),
            vibrance: n("vibrance"),
            temperature: n("temperature"),
            tint: n("tint"),
            gamma: n("gamma"),
            highlights: n("highlights"),
            shadows: n("shadows"),
        };
        (grade != Grade::default()).then_some(grade)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayerKind {
    /// A frame of a video at source time `src_us`.
    Video {
        media_id: String,
        src_us: Micros,
    },
    Image {
        media_id: String,
    },
    Text,
    /// Caption text active at this time, with the index of the active word (for highlighting).
    Caption {
        words: Vec<String>,
        active_word: Option<usize>,
    },
    /// Applies its grade to everything below it.
    Adjustment,
    /// A motion-graphics composition (`codeComposition`), rendered by `ove-graphics` at item-relative time.
    Code,
}

/// One thing to draw, bottom-up.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub item_id: String,
    pub kind: LayerKind,
    pub opacity: f32,
    pub xform: Xform,
    pub placement: Placement,
    /// top, right, bottom, left in percent of the source.
    pub crop: [f32; 4],
    pub blend: Blend,
    /// Corner radius in reference pixels.
    pub border_radius: f32,
    pub grade: Option<Grade>,
    /// The item's fields (text style, caption style, …).
    pub fields: Map<String, Value>,
    /// Item-relative time and item duration (text animations, code items).
    pub rel_us: Micros,
    pub dur_us: Micros,
}

fn num(v: Option<&Value>, def: f64) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(def)
}

fn transition(item: &Item, key: &str) -> Option<(String, Micros)> {
    let t = item.fields.get(key)?.as_object()?;
    let kind = t.get("type").or_else(|| t.get("presetId")).and_then(Value::as_str).unwrap_or("fade").to_string();
    let d = t.get("duration").and_then(json_to_micros).unwrap_or(500_000);
    (d > 0).then_some((kind, d))
}

fn is_cross(kind: &str) -> bool {
    matches!(kind, "crossfade" | "cross-dissolve" | "dissolve" | "cross-fade")
}

/// Easing curve value for progress `p` in 0..1.
pub fn ease(name: &str, p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    match name {
        "ease-in" => p * p,
        "ease-out" => 1.0 - (1.0 - p) * (1.0 - p),
        "ease-in-out" => {
            if p < 0.5 {
                2.0 * p * p
            } else {
                1.0 - (-2.0 * p + 2.0).powi(2) / 2.0
            }
        }
        _ => p,
    }
}

/// Value of a keyframed property at item-relative time `rel_us`, if the item animates it.
pub fn keyframe_value(item: &Item, property: &str, rel_us: Micros) -> Option<f64> {
    let tracks = item.fields.get("keyframeTracks")?.as_array()?;
    let track = tracks.iter().find(|t| t.get("property").and_then(Value::as_str) == Some(property))?;
    let easing = track.get("easing").and_then(Value::as_str).unwrap_or("linear");
    let mut kfs: Vec<(Micros, f64)> = track
        .get("keyframes")?
        .as_array()?
        .iter()
        .filter_map(|k| Some((k.get("time").and_then(json_to_micros)?, k.get("value")?.as_f64()?)))
        .collect();
    if kfs.is_empty() {
        return None;
    }
    kfs.sort_by_key(|k| k.0);
    if rel_us <= kfs[0].0 {
        return Some(kfs[0].1);
    }
    let last = *kfs.last().unwrap();
    if rel_us >= last.0 {
        return Some(last.1);
    }
    let i = kfs.iter().position(|k| k.0 > rel_us).unwrap();
    let (a, b) = (kfs[i - 1], kfs[i]);
    let p = (rel_us - a.0) as f64 / (b.0 - a.0).max(1) as f64;
    Some(a.1 + (b.1 - a.1) * ease(easing, p))
}

/// Source time of a video/audio item at timeline time `t_us` (trim, speed, reverse).
pub fn source_time(item: &Item, t_us: Micros) -> Micros {
    let speed = num(item.fields.get("playbackSpeed"), 1.0).max(0.01);
    let trim = item.time_field("trimStart");
    let rel = ((t_us - item.start) as f64 * speed) as Micros;
    if item.fields.get("playbackDirection").and_then(Value::as_str) == Some("reverse") {
        let used = ((item.end - item.start) as f64 * speed) as Micros;
        (trim + used - rel - 1).max(trim)
    } else {
        trim + rel
    }
}

fn caption_words(item: &Item, rel_us: Micros) -> (Vec<String>, Option<usize>) {
    if let Some(segs) = item.fields.get("captionSegments").and_then(Value::as_array) {
        for s in segs {
            let (Some(a), Some(b)) =
                (s.get("startTime").and_then(json_to_micros), s.get("endTime").and_then(json_to_micros))
            else {
                continue;
            };
            if rel_us < a || rel_us >= b {
                continue;
            }
            if let Some(words) = s.get("wordTimings").and_then(Value::as_array).filter(|w| !w.is_empty()) {
                let list: Vec<String> =
                    words.iter().filter_map(|w| w.get("word").and_then(Value::as_str).map(str::to_string)).collect();
                // Word timings are relative to the item, like segments.
                let active = words.iter().position(|w| {
                    let ws = w.get("start").and_then(json_to_micros).unwrap_or(i64::MAX);
                    let we = w.get("end").and_then(json_to_micros).unwrap_or(i64::MIN);
                    rel_us >= ws && rel_us < we
                });
                return (list, active);
            }
            let text = s.get("text").and_then(Value::as_str).unwrap_or("");
            return (text.split_whitespace().map(str::to_string).collect(), None);
        }
        if !segs.is_empty() {
            return (Vec::new(), None);
        }
    }
    let text = item.fields.get("captionText").and_then(Value::as_str).unwrap_or("");
    (text.split_whitespace().map(str::to_string).collect(), None)
}

fn layer_for(doc: &Document, item: &Item, t_us: Micros, opacity_mul: f32) -> Option<Layer> {
    let f = &item.fields;
    let rel = t_us - item.start;
    let kind = match item.kind.as_str() {
        "video" => {
            let mid = item.media_id()?.to_string();
            let is_image =
                doc.project.media.get(&mid).and_then(|m| m.data()).and_then(|d| d.get("type")).and_then(Value::as_str)
                    == Some("image");
            if is_image {
                LayerKind::Image { media_id: mid }
            } else {
                LayerKind::Video { media_id: mid, src_us: source_time(item, t_us) }
            }
        }
        "image" => LayerKind::Image { media_id: item.media_id()?.to_string() },
        "text" => LayerKind::Text,
        "caption" => {
            let (words, active_word) = caption_words(item, rel);
            if words.is_empty() {
                return None;
            }
            LayerKind::Caption { words, active_word }
        }
        "adjustment" => LayerKind::Adjustment,
        "code" if f.get("codeComposition").is_some_and(|c| !c.is_null()) => LayerKind::Code,
        // audio has no picture; shader items arrive with the effects runtime.
        _ => return None,
    };
    let t = f.get("transform").and_then(Value::as_object);
    let tv = |k: &str, d: f64| num(t.and_then(|t| t.get(k)), d);
    let mut xform = Xform {
        x: tv("x", 0.0),
        y: tv("y", 0.0),
        scale: tv("scale", 1.0),
        rotation: tv("rotation", 0.0),
        flip_x: t.and_then(|t| t.get("flipX")).and_then(Value::as_bool).unwrap_or(false),
    };
    let mut opacity = num(f.get("opacity"), 1.0);
    for (prop, slot) in [("x", 0), ("y", 1), ("scale", 2), ("rotation", 3), ("opacity", 4)] {
        if let Some(v) = keyframe_value(item, prop, rel) {
            match slot {
                0 => xform.x = v,
                1 => xform.y = v,
                2 => xform.scale = v,
                3 => xform.rotation = v,
                _ => opacity = v,
            }
        }
    }
    let animated = f.get("keyframeTracks").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    let placement = if kind == LayerKind::Code
        || f.get("autoReframe").and_then(Value::as_bool) == Some(false) && t.is_none() && !animated
    {
        Placement::Fit
    } else {
        Placement::FillHeight
    };
    // Fades (non-crossfade transitions).
    if let Some((k, d)) = transition(item, "transitionIn")
        && !is_cross(&k)
        && rel < d
    {
        opacity *= rel as f64 / d as f64;
    }
    if let Some((k, d)) = transition(item, "transitionOut")
        && !is_cross(&k)
        && item.end - t_us < d
    {
        opacity *= (item.end - t_us) as f64 / d as f64;
    }
    let c = f.get("crop").and_then(Value::as_object);
    let cv = |k: &str| num(c.and_then(|c| c.get(k)), 0.0).clamp(0.0, 100.0) as f32;
    Some(Layer {
        item_id: item.id.clone(),
        kind,
        opacity: (opacity.clamp(0.0, 1.0) as f32) * opacity_mul,
        xform,
        placement,
        crop: [cv("top"), cv("right"), cv("bottom"), cv("left")],
        blend: Blend::parse(f.get("blendMode").and_then(Value::as_str)),
        border_radius: num(f.get("borderRadius"), 0.0).max(0.0) as f32,
        grade: Grade::from_fields(f),
        fields: f.clone(),
        rel_us: rel,
        dur_us: item.end - item.start,
    })
}

/// Layers visible at `t_us`, bottom first.
pub fn layers_at(doc: &Document, t_us: Micros) -> Vec<Layer> {
    let mut out = Vec::new();
    for track in doc.project.tracks.iter().rev() {
        if track.fields.get("visible").and_then(Value::as_bool) == Some(false) || track.kind == "audio" {
            continue;
        }
        let items = doc.items_on_track(&track.id);
        let visible = |i: &&Item| !i.is_pending() && i.fields.get("hidden").and_then(Value::as_bool) != Some(true);
        for (n, item) in items.iter().enumerate() {
            if !visible(item) {
                continue;
            }
            // Outgoing side of a crossfade: keep drawing past the cut, under the incoming item.
            if t_us >= item.end {
                let next = items[n + 1..].iter().find(|b| (b.start - item.end).abs() <= 1_000 && visible(b));
                let cross = next.and_then(|b| {
                    transition(b, "transitionIn")
                        .filter(|(k, _)| is_cross(k))
                        .or_else(|| transition(item, "transitionOut").filter(|(k, _)| is_cross(k)))
                });
                if let (Some(b), Some((_, d))) = (next, cross)
                    && t_us >= b.start
                    && t_us < b.start + d
                    && let Some(l) = layer_for(doc, item, t_us, 1.0)
                {
                    out.push(l);
                }
                continue;
            }
            if t_us < item.start {
                continue;
            }
            // Incoming side of a crossfade: fade in over the outgoing item.
            let prev = items[..n].iter().rev().find(|a| (item.start - a.end).abs() <= 1_000 && visible(a));
            let cross = prev.and_then(|a| {
                transition(item, "transitionIn")
                    .filter(|(k, _)| is_cross(k))
                    .or_else(|| transition(a, "transitionOut").filter(|(k, _)| is_cross(k)))
            });
            let mul = match cross {
                Some((_, d)) if t_us - item.start < d => ((t_us - item.start) as f64 / d as f64) as f32,
                _ => 1.0,
            };
            if let Some(l) = layer_for(doc, item, t_us, mul) {
                out.push(l);
            }
        }
    }
    out
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
            Op::AddMedia { media: json!({"id": "m", "data": {"type": "video", "duration": 100}}), entry: None },
        ];
        ops.extend(items.into_iter().map(|item| Op::AddItem { item }));
        apply_batch(&Document::new("p", "p", 1), &ops).unwrap().doc
    }

    #[test]
    fn order_bottom_up_and_hidden_pending_skipped() {
        let d = doc(vec![
            json!({"id": "a", "trackId": "main", "type": "video", "startTime": 0, "endTime": 5, "mediaItemId": "m"}),
            json!({"id": "t", "trackId": "top", "type": "text", "startTime": 0, "endTime": 5, "textContent": "Hi"}),
            json!({"id": "h", "trackId": "top", "type": "text", "startTime": 5, "endTime": 6, "hidden": true}),
            json!({"id": "p", "trackId": "main", "type": "video", "startTime": 5, "endTime": 6, "pending": true}),
        ]);
        let l = layers_at(&d, 1_000_000);
        assert_eq!(l.iter().map(|l| l.item_id.as_str()).collect::<Vec<_>>(), ["a", "t"]);
        assert!(layers_at(&d, 5_500_000).is_empty());
    }

    #[test]
    fn source_time_trim_speed_reverse() {
        let d = doc(vec![
            json!({"id": "a", "trackId": "main", "type": "video", "startTime": 2, "endTime": 6, "trimStart": 10, "mediaItemId": "m", "playbackSpeed": 2}),
            json!({"id": "r", "trackId": "top", "type": "video", "startTime": 0, "endTime": 4, "mediaItemId": "m", "playbackDirection": "reverse"}),
        ]);
        assert_eq!(source_time(d.item("a").unwrap(), 3_000_000), 12_000_000);
        assert_eq!(source_time(d.item("r").unwrap(), 0), 3_999_999);
        assert_eq!(source_time(d.item("r").unwrap(), 3_000_000), 999_999);
    }

    #[test]
    fn crossfade_overlaps_and_fades() {
        let d = doc(vec![
            json!({"id": "a", "trackId": "main", "type": "video", "startTime": 0, "endTime": 4, "mediaItemId": "m"}),
            json!({"id": "b", "trackId": "main", "type": "video", "startTime": 4, "endTime": 8, "mediaItemId": "m", "transitionIn": {"type": "crossfade", "duration": 1}}),
        ]);
        let mid = layers_at(&d, 4_500_000);
        assert_eq!(mid.iter().map(|l| l.item_id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!((mid[1].opacity - 0.5).abs() < 1e-3);
        assert_eq!(mid[0].kind, LayerKind::Video { media_id: "m".into(), src_us: 4_500_000 });
        assert_eq!(layers_at(&d, 5_100_000).len(), 1);
    }

    #[test]
    fn fade_in_out_and_keyframes() {
        let d = doc(vec![
            json!({"id": "a", "trackId": "main", "type": "video", "startTime": 0, "endTime": 4, "mediaItemId": "m",
            "transitionIn": {"type": "fade", "duration": 1}, "transitionOut": {"type": "fade", "duration": 2},
            "keyframeTracks": [{"property": "scale", "keyframes": [{"time": 0, "value": 1}, {"time": 4, "value": 2}]}]}),
        ]);
        assert!((layers_at(&d, 500_000)[0].opacity - 0.5).abs() < 1e-3);
        assert!((layers_at(&d, 3_000_000)[0].opacity - 0.5).abs() < 1e-3);
        assert!((layers_at(&d, 2_000_000)[0].xform.scale - 1.5).abs() < 1e-9);
        assert_eq!(ease("ease-in-out", 0.5), 0.5);
    }

    #[test]
    fn captions_pick_segment_and_word() {
        let d = doc(vec![json!({"id": "c", "trackId": "top", "type": "caption", "startTime": 10, "endTime": 14,
            "captionSegments": [{"id": "s", "text": "hello there", "startTime": 0, "endTime": 2,
                "wordTimings": [{"word": "hello", "start": 0, "end": 1}, {"word": "there", "start": 1, "end": 2}]}]})]);
        let l = layers_at(&d, 11_500_000);
        assert_eq!(l[0].kind, LayerKind::Caption { words: vec!["hello".into(), "there".into()], active_word: Some(1) });
        assert!(layers_at(&d, 13_000_000).is_empty(), "between segments nothing shows");
    }

    #[test]
    fn canvas_sizes_are_even() {
        assert_eq!(canvas_size("16:9", 540), (960, 540));
        assert_eq!(canvas_size("9:16", 1080), (608, 1080));
        assert_eq!(canvas_size("4:5", 720), (576, 720));
    }
}
