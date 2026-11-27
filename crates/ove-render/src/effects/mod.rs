//! Effects: the 41 catalogue effects as CPU per-pixel passes (rayon over rows).
//!
//! Pixel contract (design D5): an effect's per-pixel function returns RGBA. Colour effects are written as RGB
//! functions and keep their input alpha; keys and mattes return alpha and so change the layer's transparency.
//! Background generators fill their area and make it opaque by `amount`.
//!
//! An effect runs either on an item's decoded frame (`effects: [{id, type, ...params}]` on video and image items)
//! or as a full-canvas pass for a shader item (`shaderComposition {template, params, amount, fadeIn, fadeOut,
//! region}`). Parameters are resolved against the contract in [`spec`]: missing values take their defaults and
//! numbers are clamped to their ranges, so no input can produce NaN or a panic.
//!
//! `region` (`subject` / `background`) needs a subject matte, which the renderer does not produce yet; every region
//! falls back to the whole frame.

mod background;
mod distort;
pub mod spec;
mod stylize;
mod util;

use rayon::prelude::*;
use serde_json::{Map, Value};

pub use spec::{EFFECTS, EffectSpec, Group, find};
pub use util::Params;
use util::{Ctx, V3, V4};

use crate::canvas::Rgba;

/// An effect's per-pixel function.
#[derive(Clone, Copy)]
enum PixelFn {
    /// Colour only: the output keeps the input alpha.
    Rgb(fn(&Ctx, f32, f32) -> V3),
    /// Colour and alpha (keys, mattes).
    Rgba(fn(&Ctx, f32, f32) -> V4),
}

fn pixel_fn(id: &str) -> Option<PixelFn> {
    rgb_fn(id).map(PixelFn::Rgb).or_else(|| rgba_fn(id).map(PixelFn::Rgba))
}

/// Effects that write alpha.
fn rgba_fn(_id: &str) -> Option<fn(&Ctx, f32, f32) -> V4> {
    None
}

fn rgb_fn(id: &str) -> Option<fn(&Ctx, f32, f32) -> V3> {
    Some(match id {
        "duotone" => stylize::duotone,
        "tritone" => stylize::tritone,
        "gradient-map" => stylize::gradient_map,
        "posterize" => stylize::posterize,
        "solarize" => stylize::solarize,
        "halftone" => stylize::halftone,
        "dither" => stylize::dither,
        "engraving" => stylize::engraving,
        "watercolor" => stylize::watercolor,
        "pixelate" => stylize::pixelate,
        "chromatic-aberration" => stylize::chromatic_aberration,
        "glitch" => stylize::glitch,
        "vhs" => stylize::vhs,
        "crt-screen" => stylize::crt_screen,
        "light-leak" => stylize::light_leak,
        "lens-flare" => stylize::lens_flare,
        "zoom-blur" => stylize::zoom_blur,
        "tilt-shift" => stylize::tilt_shift,
        "bulge" => distort::bulge,
        "spherize" => distort::spherize,
        "lens-distortion" => distort::lens_distortion,
        "twirl" => distort::twirl,
        "wave-distortion" => distort::wave_distortion,
        "kaleidoscope" => distort::kaleidoscope,
        "mirror" => distort::mirror,
        "polar-coordinates" => distort::polar_coordinates,
        "fluted-glass" => distort::fluted_glass,
        "glass-tiles" => distort::glass_tiles,
        "aurora" => background::aurora,
        "mesh-gradient" => background::mesh_gradient,
        "flowing-gradient" => background::flowing_gradient,
        "plasma" => background::plasma,
        "swirl" => background::swirl,
        "marble" => background::marble,
        "godrays" => background::godrays,
        "sunburst" => background::sunburst,
        "spiral" => background::spiral,
        "conic-gradient" => background::conic_gradient,
        "voronoi" => background::voronoi,
        "stripes" => background::stripes,
        "studio-background" => background::studio_background,
        _ => return None,
    })
}

/// Fractal value noise in 0..1 (shared with transitions).
pub fn noise_fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    util::fbm(x, y, octaves, seed)
}

/// Integer hash to 0..1 (shared with transitions).
pub fn hash(x: i32, y: i32, seed: u32) -> f32 {
    util::hash21(x, y, seed)
}

/// Normalise an amount that may be given as 0..1 or 0..100.
pub fn unit_amount(v: Option<&Value>) -> f32 {
    match v.and_then(Value::as_f64) {
        Some(a) if a.is_finite() => {
            let a = a as f32;
            (if a > 1.0 { a / 100.0 } else { a }).clamp(0.0, 1.0)
        }
        _ => 1.0,
    }
}

/// Apply effect `id` to `img` in place, mixed by `amount` (0..1). `t` is item-relative seconds. Returns false for
/// an unknown id (the image is left untouched).
pub fn apply(img: &mut Rgba, id: &str, values: &Map<String, Value>, amount: f32, t: f32) -> bool {
    let (Some(spec), Some(f)) = (find(id), pixel_fn(id)) else { return false };
    let amount = if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 };
    if amount <= 0.0 || img.w == 0 || img.h == 0 {
        return true;
    }
    let params = Params::resolve(spec, values);
    let src = img.clone();
    let ctx = Ctx {
        src: &src,
        w: src.w as f32,
        h: src.h as f32,
        aspect: src.w as f32 / src.h as f32,
        t: if t.is_finite() { t } else { 0.0 },
        p: &params,
    };
    let generator = spec.group == Group::Background;
    let (w, h) = (img.w as usize, img.h as f32);
    img.data.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let v = (y as f32 + 0.5) / h;
        for x in 0..w {
            let u = (x as f32 + 0.5) / ctx.w;
            let px = &mut row[x * 4..x * 4 + 4];
            let (o, alpha) = match f {
                PixelFn::Rgb(f) => (f(&ctx, u, v), None),
                PixelFn::Rgba(f) => {
                    let o = f(&ctx, u, v);
                    ([o[0], o[1], o[2]], Some(o[3]))
                }
            };
            for k in 0..3 {
                let s = px[k] as f32 / 255.0;
                let val = if o[k].is_finite() { o[k] } else { s };
                px[k] = ((s + (val - s) * amount).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            if let Some(a) = alpha {
                let s = px[3] as f32 / 255.0;
                let val = if a.is_finite() { a } else { s };
                px[3] = ((s + (val - s) * amount).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            } else if generator {
                px[3] = (px[3] as f32 + (255.0 - px[3] as f32) * amount + 0.5) as u8;
            }
        }
    });
    true
}

/// The values an effect entry carries: a nested `params` object, or the entry's own keys.
fn entry_values(e: &Map<String, Value>) -> &Map<String, Value> {
    match e.get("params") {
        Some(Value::Object(p)) => p,
        _ => e,
    }
}

/// The runnable entries of an effect stack (`effects` field), in order: (effect id, parameter values, amount).
/// Disabled entries (`enabled: false`) and entries without a type are left out.
pub fn stack_entries(effects: &Value) -> Vec<(&str, &Map<String, Value>, f32)> {
    let Value::Array(list) = effects else { return Vec::new() };
    list.iter()
        .filter_map(Value::as_object)
        .filter(|e| e.get("enabled").and_then(Value::as_bool) != Some(false))
        .filter_map(|e| {
            let id = e.get("type").or_else(|| e.get("template")).and_then(Value::as_str)?;
            Some((id, entry_values(e), unit_amount(e.get("amount"))))
        })
        .collect()
}

/// One runnable effect: (effect id, parameter values, amount).
pub type Entry<'a> = (&'a str, std::borrow::Cow<'a, Map<String, Value>>, f32);

/// [`stack_entries`] at item-relative time `rel_us`: parameters animated by keyframe tracks
/// (`effects.<entry id>.<parameter>` in the item's `keyframeTracks`) take their value at that time, evaluated as
/// transform keyframes are (design D5). Without such tracks the values are the entries' own.
pub fn stack_entries_at<'a>(
    effects: &'a Value,
    keyframe_tracks: Option<&Value>,
    rel_us: ove_core::time::Micros,
) -> Vec<Entry<'a>> {
    let Value::Array(list) = effects else { return Vec::new() };
    let tracks: Vec<(&str, &str, &Value)> = keyframe_tracks
        .and_then(Value::as_array)
        .map(|t| {
            t.iter()
                .filter_map(|t| {
                    let prop = t.get("property")?.as_str()?;
                    let (id, param) = ove_core::effect_params::split_property(prop)?;
                    Some((id, param, t))
                })
                .collect()
        })
        .unwrap_or_default();
    list.iter()
        .filter_map(Value::as_object)
        .filter(|e| e.get("enabled").and_then(Value::as_bool) != Some(false))
        .filter_map(|e| {
            let kind = e.get("type").or_else(|| e.get("template")).and_then(Value::as_str)?;
            let entry_id = e.get("id").and_then(Value::as_str);
            let mut values = std::borrow::Cow::Borrowed(entry_values(e));
            for (id, param, track) in &tracks {
                if Some(*id) == entry_id
                    && let Some(v) = crate::eval::track_value(track, rel_us)
                {
                    values.to_mut().insert((*param).to_string(), serde_json::json!(v));
                }
            }
            Some((kind, values, unit_amount(e.get("amount"))))
        })
        .collect()
}

/// Apply an item's effect stack (`effects` field) in order. Disabled entries (`enabled: false`) are skipped.
pub fn apply_stack(img: &mut Rgba, effects: &Value, t: f32) {
    for (id, values, amount) in stack_entries(effects) {
        apply(img, id, values, amount, t);
    }
}

/// The overall strength of a shader item at item-relative time `t` of `dur` seconds: amount × fade in/out.
pub fn shader_strength(comp: &Map<String, Value>, t: f32, dur: f32) -> f32 {
    let amount = unit_amount(comp.get("amount"));
    let fade_in = comp.get("fadeIn").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let fade_out = comp.get("fadeOut").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let mut k = 1.0f32;
    if fade_in > 0.0 {
        k = k.min(t / fade_in);
    }
    if fade_out > 0.0 {
        k = k.min((dur - t) / fade_out);
    }
    amount * k.clamp(0.0, 1.0)
}

/// Apply a shader item's composition to the canvas.
pub fn apply_shader(canvas: &mut Rgba, comp: &Map<String, Value>, t: f32, dur: f32) {
    let Some(id) = comp.get("template").and_then(Value::as_str) else { return };
    let empty = Map::new();
    let params = comp.get("params").and_then(Value::as_object).unwrap_or(&empty);
    apply(canvas, id, params, shader_strength(comp, t, dur), t);
}

/// A procedural sample frame for previews and tests: sky gradient, sun, hills, a figure and colour bars.
pub fn sample_image(w: u32, h: u32) -> Rgba {
    let mut img = Rgba::new(w, h);
    let (fw, fh) = (w as f32, h as f32);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = ((x as f32 + 0.5) / fw, (y as f32 + 0.5) / fh);
            let mut c = util::mix([0.25, 0.45, 0.85], [0.98, 0.72, 0.5], v / 0.65);
            let (sx, sy) = ((u - 0.7) * fw / fh, v - 0.32);
            let sun = (sx * sx + sy * sy).sqrt();
            if sun < 0.11 {
                c = [1.0, 0.92, 0.55];
            }
            let hill = 0.62 + 0.06 * (u * 9.0).sin() + 0.03 * (u * 23.0).cos();
            if v > hill {
                c = util::mix([0.18, 0.5, 0.25], [0.06, 0.2, 0.1], (v - hill) / (1.0 - hill));
            }
            let (fx, fy) = ((u - 0.3) * fw / fh, v - 0.55);
            if (fx * fx + (fy + 0.12).powi(2)).sqrt() < 0.05 || (fx.abs() < 0.045 && (-0.07..0.12).contains(&fy)) {
                c = [0.85, 0.2, 0.3];
            }
            if v > 0.92 {
                let bars: [V3; 6] =
                    [[1., 1., 1.], [1., 1., 0.], [0., 1., 1.], [0., 1., 0.], [1., 0., 1.], [1., 0., 0.]];
                c = bars[((u * 6.0) as usize).min(5)];
            }
            let i = ((y * w + x) * 4) as usize;
            for (k, v) in c.iter().enumerate() {
                img.data[i + k] = (v.clamp(0.0, 1.0) * 255.0) as u8;
            }
            img.data[i + 3] = 255;
        }
    }
    img
}

/// A small thumbnail of effect `id` at its defaults over the sample frame (time 1 s so animated effects show).
pub fn preview(id: &str, w: u32, h: u32) -> Option<Rgba> {
    find(id)?;
    let mut img = sample_image(w, h);
    apply(&mut img, id, &Map::new(), 1.0, 1.0);
    Some(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use spec::{ParamDefault, ParamKind};

    fn values_at(spec: &EffectSpec, which: u8) -> Map<String, Value> {
        let mut m = Map::new();
        for p in spec.params {
            let v = match (p.kind, p.default) {
                (ParamKind::Number { min, max } | ParamKind::Percent { min, max }, ParamDefault::Num(d)) => {
                    json!([min, d, max][which as usize])
                }
                (ParamKind::Toggle, _) => json!(which == 2),
                (ParamKind::Point, _) => json!([[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]][which as usize]),
                (ParamKind::Color, _) => json!([[0.0, 0.0, 0.0], [0.5, 0.2, 0.9], [1.0, 1.0, 1.0]][which as usize]),
                _ => continue,
            };
            m.insert(p.name.into(), v);
        }
        m
    }

    /// Design D5: colour effects keep the input alpha of a semi-transparent layer; only generators fill it.
    #[test]
    fn existing_effects_keep_input_alpha() {
        let mut base = sample_image(64, 36);
        for (i, p) in base.data.chunks_exact_mut(4).enumerate() {
            p[3] = ((i * 7) % 256) as u8;
        }
        for spec in EFFECTS {
            let mut img = base.clone();
            apply(&mut img, spec.id, &Map::new(), 1.0, 1.0);
            let same = img.data.chunks_exact(4).zip(base.data.chunks_exact(4)).all(|(a, b)| a[3] == b[3]);
            if spec.group == Group::Background {
                assert!(img.data.chunks_exact(4).all(|p| p[3] == 255), "{} fills its area", spec.id);
            } else {
                assert!(same, "{} changed the layer's alpha", spec.id);
            }
        }
    }

    #[test]
    fn every_effect_is_implemented_and_survives_extreme_params() {
        for spec in EFFECTS {
            assert!(pixel_fn(spec.id).is_some(), "{} has no implementation", spec.id);
            for which in 0..3 {
                for t in [0.0, 3.7] {
                    let mut img = sample_image(48, 27);
                    assert!(apply(&mut img, spec.id, &values_at(spec, which), 1.0, t));
                }
            }
            // Garbage input: wrong types, NaN-like strings, out-of-range numbers.
            let mut junk = Map::new();
            for p in spec.params {
                junk.insert(p.name.into(), json!(if p.name.len() % 2 == 0 { json!(1e30) } else { json!("x") }));
            }
            let mut img = sample_image(16, 9);
            apply(&mut img, spec.id, &junk, 1.0, f32::NAN);
        }
    }

    #[test]
    fn effects_change_the_image_and_amount_zero_does_not() {
        let base = sample_image(64, 36);
        for spec in EFFECTS {
            let mut img = base.clone();
            apply(&mut img, spec.id, &Map::new(), 1.0, 1.0);
            assert_ne!(img.data, base.data, "{} at defaults left the frame unchanged", spec.id);
            let mut none = base.clone();
            apply(&mut none, spec.id, &Map::new(), 0.0, 1.0);
            assert_eq!(none.data, base.data);
        }
    }

    #[test]
    fn stack_reads_flat_and_nested_params() {
        let base = sample_image(32, 18);
        let mut a = base.clone();
        apply_stack(&mut a, &json!([{"id": "e1", "type": "posterize", "levels": 2}]), 0.0);
        let mut b = base.clone();
        apply_stack(&mut b, &json!([{"id": "e1", "type": "posterize", "params": {"levels": 2}}]), 0.0);
        assert_eq!(a.data, b.data);
        let mut c = base.clone();
        apply_stack(&mut c, &json!([{"id": "e1", "type": "posterize", "levels": 2, "enabled": false}]), 0.0);
        assert_eq!(c.data, base.data);
        // posterize to 2 levels leaves only 0 or 255 per channel
        assert!(a.data.chunks(4).all(|p| p[..3].iter().all(|&x| x == 0 || x == 255)));
    }

    /// The animated blur scenario: zoom blur intensity keyframed from 0 to 20 over one second is 10 at 0.5 s
    /// (linear), and the frame equals the effect at 10.
    #[test]
    fn keyframed_parameters_follow_their_tracks() {
        let fx = json!([{"id": "b", "type": "zoom-blur", "intensity": 50}]);
        let tracks = json!([{"property": "effects.b.intensity", "keyframes": [{"time": 0, "value": 0}, {"time": 1, "value": 20}]}]);
        let at = |us: i64| stack_entries_at(&fx, Some(&tracks), us)[0].1.get("intensity").and_then(Value::as_f64);
        assert_eq!(at(0), Some(0.0));
        assert_eq!(at(500_000), Some(10.0));
        assert_eq!(at(2_000_000), Some(20.0));
        let base = sample_image(48, 27);
        let mut animated = base.clone();
        for (id, values, amount) in stack_entries_at(&fx, Some(&tracks), 500_000) {
            apply(&mut animated, id, &values, amount, 0.5);
        }
        let mut fixed = base.clone();
        apply(&mut fixed, "zoom-blur", json!({"intensity": 10}).as_object().unwrap(), 1.0, 0.5);
        assert_eq!(animated.data, fixed.data);
        // Without tracks the entry's own value stays; tracks for other entries do not touch it.
        assert_eq!(stack_entries_at(&fx, None, 500_000)[0].1.get("intensity"), Some(&json!(50)));
        let other = json!([{"property": "effects.x.intensity", "keyframes": [{"time": 0, "value": 1}]}]);
        assert_eq!(stack_entries_at(&fx, Some(&other), 0)[0].1.get("intensity"), Some(&json!(50)));
    }

    /// ove-core keeps a copy of the numeric parameter names to check keyframe tracks: it must equal the contract.
    #[test]
    fn core_parameter_table_matches_the_contract() {
        use ove_core::effect_params::NUMERIC_PARAMS;
        assert_eq!(NUMERIC_PARAMS.len(), EFFECTS.len());
        for spec in EFFECTS {
            let want: Vec<&str> = spec
                .params
                .iter()
                .filter(|p| matches!(p.kind, ParamKind::Number { .. } | ParamKind::Percent { .. }))
                .map(|p| p.name)
                .collect();
            assert_eq!(ove_core::effect_params::params(spec.id), Some(&want[..]), "{}", spec.id);
        }
    }

    #[test]
    fn shader_fades() {
        let comp = json!({"template": "duotone", "amount": 100, "fadeIn": 1.0, "fadeOut": 2.0});
        let m = comp.as_object().unwrap();
        assert_eq!(shader_strength(m, 0.0, 10.0), 0.0);
        assert!((shader_strength(m, 0.5, 10.0) - 0.5).abs() < 1e-6);
        assert_eq!(shader_strength(m, 5.0, 10.0), 1.0);
        assert!((shader_strength(m, 9.0, 10.0) - 0.5).abs() < 1e-6);
    }
}
