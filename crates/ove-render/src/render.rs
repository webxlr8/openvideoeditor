//! The renderer: evaluated layers → one RGBA frame at a canvas size.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ove_core::model::Document;
use ove_core::time::Micros;
use serde_json::{Map, Value};

use crate::canvas::{DrawParams, Place, Rgba, draw, grade};
use crate::color::{ColorGrade, Lut, apply_lut};
use crate::eval::{Blend, Layer, LayerKind, Placement, canvas_size, layers_at, reference_size};
use crate::sources::{Sources, media_path};
use crate::text::{Align, TextStyle, parse_color, render_words, words_of};
use crate::{effects, layouts, transitions};

/// Renders timeline frames. Keeps decoders and rasterised text between frames.
pub struct Renderer {
    bundle: Option<PathBuf>,
    height: u32,
    pub sources: Sources,
    text: HashMap<String, (u64, Arc<Rgba>)>,
    /// Baked grade per item, keyed by the grade fields it was baked from.
    grades: HashMap<String, (String, Option<Arc<Lut>>)>,
    luts: HashMap<PathBuf, Option<Arc<Lut>>>,
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

/// Caption style (readable default: bold white, dark outline, active word highlighted).
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
            grades: HashMap::new(),
            luts: HashMap::new(),
            frames: 0,
        }
    }

    pub fn size(&self, doc: &Document) -> (u32, u32) {
        canvas_size(&doc.project.aspect_ratio, self.height)
    }

    /// Loads a `.cube` file named by a grade (absolute, or relative to the bundle), cached by path.
    fn load_lut(&mut self, file: &str) -> Option<Arc<Lut>> {
        let p = PathBuf::from(file);
        let p = if p.is_absolute() { p } else { self.bundle.as_ref().map(|b| b.join(&p)).unwrap_or(p) };
        self.luts
            .entry(p.clone())
            .or_insert_with(|| {
                let text = std::fs::read_to_string(&p).ok()?;
                match Lut::parse_cube(&text) {
                    Ok(l) => Some(Arc::new(l)),
                    Err(e) => {
                        tracing::warn!("LUT {}: {e}", p.display());
                        None
                    }
                }
            })
            .clone()
    }

    /// The item's full grade (wheels, curves, saturation, LUTs) baked into one lattice, cached per item.
    fn color_grade(&mut self, item_id: &str, f: &Map<String, Value>) -> Option<Arc<Lut>> {
        let cg = f.get("colorGrade");
        let preset = f.get("lutPresetId");
        if cg.is_none() && preset.is_none() {
            return None;
        }
        let key = format!("{cg:?}|{preset:?}|{:?}", f.get("lutIntensity"));
        if let Some((k, lut)) = self.grades.get(item_id)
            && *k == key
        {
            return lut.clone();
        }
        let file = cg.and_then(|c| c.get("lut")).and_then(|l| l.get("file")).and_then(Value::as_str);
        let loaded = file.and_then(|p| self.load_lut(p));
        let loader = move |_: &str| loaded.clone();
        let lut = ColorGrade::from_fields(f, &loader).map(|g| Arc::new(g.bake()));
        self.grades.insert(item_id.to_string(), (key, lut.clone()));
        lut
    }

    /// Draws one layer into `target` with the given opacity and blend (the caller handles transitions).
    #[allow(clippy::too_many_arguments)]
    fn draw_layer(&mut self, doc: &Document, target: &mut Rgba, layer: &Layer, k: f64, opacity: f32, blend: Blend) {
        let (w, h) = (target.w, target.h);
        let x = &layer.xform;
        let flip = if x.flip_x { -1.0 } else { 1.0 };
        // A layout slot replaces the frame as the area the layer is placed in.
        let (area_x, area_y, area_w, area_h) = match layer.slot {
            Some(s) => (s[0] as f64 * w as f64, s[1] as f64 * h as f64, s[2] as f64 * w as f64, s[3] as f64 * h as f64),
            None => (0.0, 0.0, w as f64, h as f64),
        };
        let centre = (area_x + area_w / 2.0 + x.x * k, area_y + area_h / 2.0 + x.y * k);
        match &layer.kind {
            LayerKind::Video { media_id, .. } | LayerKind::Image { media_id } => {
                let image = matches!(layer.kind, LayerKind::Image { .. });
                let src_us = match &layer.kind {
                    LayerKind::Video { src_us, .. } => *src_us,
                    _ => 0,
                };
                let Some(path) = media_path(doc, self.bundle.as_deref(), media_id) else { return };
                // Decode at the size it is shown (up to 2× for zoomed layers), never above 2160 lines.
                let max_h = ((h as f64 * x.scale.abs().clamp(1.0, 2.0)).ceil() as u32).min(2160);
                let Some(frame) = self.sources.frame(&layer.item_id, &path, src_us, max_h, image) else { return };
                let cg = self.color_grade(&layer.item_id, &layer.fields);
                let fx = layer.fields.get("effects").filter(|e| e.as_array().is_some_and(|a| !a.is_empty()));
                let frame = if layer.grade.is_some() || cg.is_some() || fx.is_some() {
                    let mut f = (*frame).clone();
                    if let Some(g) = &layer.grade {
                        grade(&mut f, g);
                    }
                    if let Some(l) = &cg {
                        apply_lut(&mut f, l, 1.0);
                    }
                    if let Some(e) = fx {
                        effects::apply_stack(&mut f, e, layer.rel_s);
                    }
                    Arc::new(f)
                } else {
                    frame
                };
                let (s, crop) = match layer.slot {
                    Some(_) => {
                        let (s, c) = layouts::cover(frame.w as f64, frame.h as f64, area_w, area_h, x.scale);
                        (s, [0, 1, 2, 3].map(|i| c[i].max(layer.crop[i])))
                    }
                    None => {
                        let base = match layer.placement {
                            Placement::FillHeight => h as f64 / frame.h as f64,
                            Placement::Fit => (w as f64 / frame.w as f64).min(h as f64 / frame.h as f64),
                        };
                        (base * x.scale, layer.crop)
                    }
                };
                let place = Place { cx: centre.0, cy: centre.1, sx: s * flip, sy: s, rotation: x.rotation };
                let params = DrawParams {
                    opacity,
                    blend,
                    crop,
                    radius: (layer.border_radius as f64 * k / s.abs().max(1e-6)) as f32,
                };
                draw(target, &frame, place, &params);
            }
            LayerKind::Text => {
                let content = layer.fields.get("textContent").and_then(Value::as_str).unwrap_or("").to_string();
                if content.trim().is_empty() {
                    return;
                }
                let st = text_style(&layer.fields, k as f32, area_w as f32);
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
                let place = Place { cx: centre.0, cy: centre.1, sx: x.scale * flip, sy: x.scale, rotation: x.rotation };
                draw(target, &img, place, &DrawParams { opacity, blend, ..Default::default() });
            }
            LayerKind::Caption { words, active_word } => {
                let st = caption_style(&layer.fields, k as f32, w as f32, *active_word);
                let img = render_words(words, &st);
                let pos = layer.fields.get("captionPosition").and_then(Value::as_str).unwrap_or("bottom");
                let cy = match pos {
                    "top" => h as f64 * 0.16,
                    "center" | "middle" => h as f64 * 0.5,
                    _ => h as f64 * 0.80,
                } + x.y * k;
                let place = Place { cx: w as f64 / 2.0 + x.x * k, cy, sx: x.scale, sy: x.scale, rotation: 0.0 };
                draw(target, &img, place, &DrawParams { opacity, ..Default::default() });
            }
            LayerKind::Adjustment => {
                let cg = self.color_grade(&layer.item_id, &layer.fields);
                let fx = layer.fields.get("effects").filter(|e| e.as_array().is_some_and(|a| !a.is_empty()));
                if layer.grade.is_none() && cg.is_none() && fx.is_none() {
                    return;
                }
                let before = (opacity < 1.0).then(|| target.clone());
                if let Some(g) = &layer.grade {
                    grade(target, g);
                }
                if let Some(l) = &cg {
                    apply_lut(target, l, 1.0);
                }
                if let Some(e) = fx {
                    effects::apply_stack(target, e, layer.rel_s);
                }
                if let Some(b) = before {
                    mix_back(target, &b, opacity);
                }
            }
            LayerKind::Shader => {
                let Some(comp) = layer.fields.get("shaderComposition").and_then(Value::as_object) else { return };
                let Some(id) = comp.get("template").and_then(Value::as_str) else { return };
                let empty = Map::new();
                let params = comp.get("params").and_then(Value::as_object).unwrap_or(&empty);
                let amount = effects::shader_strength(comp, layer.rel_s, layer.dur_s) * opacity;
                effects::apply(target, id, params, amount, layer.rel_s);
            }
        }
    }

    /// Renders the frame at timeline time `t_us`.
    pub fn render(&mut self, doc: &Document, t_us: Micros) -> Rgba {
        let (w, h) = self.size(doc);
        let (_, ref_h) = reference_size(&doc.project.aspect_ratio);
        let k = h as f64 / ref_h as f64;
        let mut canvas = Rgba::filled(w, h, [0, 0, 0, 255]);
        for layer in layers_at(doc, t_us) {
            let Some(st) = layer.trans.clone() else {
                self.draw_layer(doc, &mut canvas, &layer, k, layer.opacity, layer.blend);
                continue;
            };
            let m = st.motion();
            if matches!(layer.kind, LayerKind::Adjustment | LayerKind::Shader) {
                self.draw_layer(doc, &mut canvas, &layer, k, layer.opacity * m.opacity, layer.blend);
            } else {
                // Draw the layer on its own, then mask, move and blur it as the transition says.
                let mut off = Rgba::new(w, h);
                self.draw_layer(doc, &mut off, &layer, k, 1.0, Blend::Normal);
                st.apply_mask(&mut off);
                if m.blur > 0.5 {
                    let p = serde_json::json!({ "intensity": m.blur });
                    effects::apply(&mut off, "zoom-blur", p.as_object().unwrap_or(&Map::new()), 1.0, 0.0);
                }
                let place = Place {
                    cx: w as f64 * (0.5 + m.dx as f64),
                    cy: h as f64 * (0.5 + m.dy as f64),
                    sx: m.scale as f64,
                    sy: m.scale as f64,
                    rotation: 0.0,
                };
                let params =
                    DrawParams { opacity: layer.opacity * m.opacity, blend: layer.blend, ..Default::default() };
                draw(&mut canvas, &off, place, &params);
            }
            transitions::apply_post(&mut canvas, &st, layer.rel_s);
        }
        self.frames += 1;
        if self.frames.is_multiple_of(120) {
            self.sources.trim(240);
            if self.grades.len() > 256 {
                self.grades.clear();
            }
        }
        for p in canvas.data.chunks_exact_mut(4) {
            p[3] = 255;
        }
        canvas
    }
}

/// `dst = before + (dst − before)·k` (partial-strength adjustment layers).
fn mix_back(dst: &mut Rgba, before: &Rgba, k: f32) {
    use rayon::prelude::*;
    dst.data.par_iter_mut().zip(before.data.par_iter()).for_each(|(d, b)| {
        *d = (*b as f32 + (*d as f32 - *b as f32) * k).round().clamp(0.0, 255.0) as u8;
    });
}
