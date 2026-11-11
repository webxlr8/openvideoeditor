//! The renderer: evaluated layers → one RGBA frame at a canvas size.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ove_core::model::Document;
use ove_core::time::Micros;
use serde_json::{Map, Value};

use crate::canvas::{DrawParams, Place, Rgba, draw, grade};
use crate::captions::{CaptionCache, CaptionPlacement, draw_caption};
use crate::eval::{LayerKind, Placement, canvas_size, layers_at, reference_size};
use crate::sources::{Sources, media_path};
use crate::text::{Align, TextStyle, parse_color, render_words, words_of};

/// Renders timeline frames. Keeps decoders and rasterised text between frames.
pub struct Renderer {
    bundle: Option<PathBuf>,
    height: u32,
    pub sources: Sources,
    text: HashMap<String, (u64, Arc<Rgba>)>,
    captions: CaptionCache,
    frames: u64,
}

fn num(f: &Map<String, Value>, k: &str) -> Option<f64> {
    f.get(k).and_then(Value::as_f64)
}

fn color(f: &Map<String, Value>, k: &str) -> Option<[u8; 4]> {
    f.get(k).and_then(Value::as_str).and_then(parse_color)
}

/// Text style of a text item, in output pixels (`k` = output px per reference px).
pub fn text_style(f: &Map<String, Value>, k: f32, frame_w: f32) -> TextStyle {
    let size = num(f, "fontSize").unwrap_or(96.0) as f32 * k;
    let bold = match f.get("fontWeight") {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(400.0) >= 600.0,
        Some(Value::String(s)) => matches!(s.as_str(), "bold" | "bolder" | "600" | "700" | "800" | "900"),
        _ => false,
    };
    let align = match f.get("textAlign").and_then(Value::as_str) {
        Some("left") => Align::Left,
        Some("right") => Align::Right,
        _ => Align::Center,
    };
    let stroke = match (color(f, "strokeColor"), num(f, "strokeWidth")) {
        (Some(c), Some(w)) if w > 0.0 => Some((c, w as f32 * k)),
        _ => None,
    };
    let shadow = color(f, "shadowColor").map(|c| {
        (c, num(f, "shadowOffsetX").unwrap_or(0.0) as f32 * k, num(f, "shadowOffsetY").unwrap_or(4.0) as f32 * k)
    });
    let background = color(f, "backgroundColor").map(|mut c| {
        if let Some(o) = num(f, "backgroundOpacity") {
            c[3] = (o.clamp(0.0, 1.0) * c[3] as f64) as u8;
        }
        (c, num(f, "backgroundPadding").unwrap_or(16.0) as f32 * k)
    });
    TextStyle {
        size,
        color: color(f, "color").unwrap_or([255, 255, 255, 255]),
        bold,
        align,
        line_height: num(f, "lineHeight").unwrap_or(1.2) as f32,
        letter_spacing: num(f, "letterSpacing").unwrap_or(0.0) as f32 * k,
        stroke,
        shadow,
        background,
        max_width: frame_w * 0.9,
        highlight: None,
    }
}

/// A plain caption style (bold white, dark outline, active word highlighted). The renderer draws captions with the
/// caption styles in `captions`; this remains for callers that want a single text image.
pub fn caption_style(f: &Map<String, Value>, k: f32, frame_w: f32, active: Option<usize>) -> TextStyle {
    let size = num(f, "captionFontSize").unwrap_or(64.0) as f32 * k;
    TextStyle {
        size,
        color: color(f, "captionTextColor").unwrap_or([255, 255, 255, 255]),
        bold: true,
        align: Align::Center,
        line_height: 1.15,
        letter_spacing: 0.0,
        stroke: Some(([0, 0, 0, 230], (size * 0.07).max(1.5))),
        shadow: None,
        background: None,
        max_width: frame_w * 0.86,
        highlight: active.map(|i| (i, color(f, "captionHighlightColor").unwrap_or([255, 212, 0, 255]))),
    }
}

impl Renderer {
    /// `height` is the output height in pixels; the width follows the project's aspect ratio.
    pub fn new(bundle: Option<&Path>, height: u32) -> Self {
        Renderer {
            bundle: bundle.map(Path::to_path_buf),
            height,
            sources: Sources::default(),
            text: HashMap::new(),
            captions: CaptionCache::default(),
            frames: 0,
        }
    }

    pub fn size(&self, doc: &Document) -> (u32, u32) {
        canvas_size(&doc.project.aspect_ratio, self.height)
    }

    /// Renders the frame at timeline time `t_us`.
    pub fn render(&mut self, doc: &Document, t_us: Micros) -> Rgba {
        let (w, h) = self.size(doc);
        let (_, ref_h) = reference_size(&doc.project.aspect_ratio);
        let k = h as f64 / ref_h as f64;
        let mut canvas = Rgba::filled(w, h, [0, 0, 0, 255]);
        for layer in layers_at(doc, t_us) {
            let x = &layer.xform;
            let flip = if x.flip_x { -1.0 } else { 1.0 };
            let centre = (w as f64 / 2.0 + x.x * k, h as f64 / 2.0 + x.y * k);
            match &layer.kind {
                LayerKind::Video { media_id, .. } | LayerKind::Image { media_id } => {
                    let image = matches!(layer.kind, LayerKind::Image { .. });
                    let src_us = match &layer.kind {
                        LayerKind::Video { src_us, .. } => *src_us,
                        _ => 0,
                    };
                    let Some(path) = media_path(doc, self.bundle.as_deref(), media_id) else { continue };
                    // Decode at the size it is shown (up to 2× for zoomed layers), never above 2160 lines.
                    let max_h = ((h as f64 * x.scale.abs().clamp(1.0, 2.0)).ceil() as u32).min(2160);
                    let Some(frame) = self.sources.frame(&layer.item_id, &path, src_us, max_h, image) else { continue };
                    let frame = match &layer.grade {
                        Some(g) => {
                            let mut f = (*frame).clone();
                            grade(&mut f, g);
                            Arc::new(f)
                        }
                        None => frame,
                    };
                    let base = match layer.placement {
                        Placement::FillHeight => h as f64 / frame.h as f64,
                        Placement::Fit => (w as f64 / frame.w as f64).min(h as f64 / frame.h as f64),
                    };
                    let s = base * x.scale;
                    let place = Place { cx: centre.0, cy: centre.1, sx: s * flip, sy: s, rotation: x.rotation };
                    let params = DrawParams {
                        opacity: layer.opacity,
                        blend: layer.blend,
                        crop: layer.crop,
                        radius: (layer.border_radius as f64 * k / s.abs().max(1e-6)) as f32,
                    };
                    draw(&mut canvas, &frame, place, &params);
                }
                LayerKind::Text => {
                    let content = layer.fields.get("textContent").and_then(Value::as_str).unwrap_or("").to_string();
                    if content.trim().is_empty() {
                        continue;
                    }
                    let st = text_style(&layer.fields, k as f32, w as f32);
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    content.hash(&mut hasher);
                    format!("{st:?}").hash(&mut hasher);
                    let key = hasher.finish();
                    let img = match self.text.get(&layer.item_id) {
                        Some((hk, img)) if *hk == key => img.clone(),
                        _ => {
                            let img = Arc::new(render_words(&words_of(&content), &st));
                            self.text.insert(layer.item_id.clone(), (key, img.clone()));
                            img
                        }
                    };
                    let place =
                        Place { cx: centre.0, cy: centre.1, sx: x.scale * flip, sy: x.scale, rotation: x.rotation };
                    let params = DrawParams { opacity: layer.opacity, blend: layer.blend, ..Default::default() };
                    draw(&mut canvas, &img, place, &params);
                }
                LayerKind::Caption { words, .. } => {
                    // Caption styles: see `captions` and `captions_presets`.
                    let rel = doc.item(&layer.item_id).map(|i| t_us - i.start).unwrap_or(0);
                    let at = CaptionPlacement {
                        k: k as f32,
                        dx: x.x * k,
                        dy: x.y * k,
                        scale: x.scale,
                        opacity: layer.opacity,
                    };
                    draw_caption(&mut canvas, &mut self.captions, &layer.fields, words, rel, at);
                }
                LayerKind::Adjustment => {
                    if let Some(g) = &layer.grade {
                        grade(&mut canvas, g);
                    }
                }
            }
        }
        self.frames += 1;
        if self.frames.is_multiple_of(120) {
            self.sources.trim(240);
        }
        for p in canvas.data.chunks_exact_mut(4) {
            p[3] = 255;
        }
        canvas
    }
}
