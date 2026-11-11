//! Shared maths for effects: resolved parameters, sampling, noise.

use serde_json::{Map, Value};

use super::spec::{EffectSpec, ParamDefault, ParamKind};
use crate::canvas::Rgba;
use crate::text::parse_color;

pub type V3 = [f32; 3];

/// Parameter values resolved against the contract: defaults filled, numbers clamped to their ranges.
#[derive(Debug, Clone, Default)]
pub struct Params {
    // Few entries per effect: a linear scan beats hashing on the per-pixel path.
    nums: Vec<(&'static str, f32)>,
    vecs: Vec<(&'static str, [f32; 3])>,
}

fn json_vec(v: &Value) -> Option<[f32; 3]> {
    match v {
        Value::Array(a) if a.len() >= 2 => {
            let g = |i: usize| a.get(i).and_then(Value::as_f64).map(|x| x as f32);
            Some([g(0)?, g(1)?, g(2).unwrap_or(0.0)])
        }
        Value::String(s) => parse_color(s).map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]),
        Value::Object(o) => {
            let g = |k: &str| o.get(k).and_then(Value::as_f64).map(|x| x as f32);
            Some([g("x").or(g("r"))?, g("y").or(g("g"))?, g("b").unwrap_or(0.0)])
        }
        _ => None,
    }
}

impl Params {
    pub fn resolve(spec: &EffectSpec, values: &Map<String, Value>) -> Params {
        let mut p = Params::default();
        for s in spec.params {
            let v = values.get(s.name);
            match (s.kind, s.default) {
                (ParamKind::Number { min, max } | ParamKind::Percent { min, max }, ParamDefault::Num(d)) => {
                    let x = v.and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite()).unwrap_or(d);
                    p.nums.push((s.name, x.clamp(min, max)));
                }
                (ParamKind::Toggle, ParamDefault::Num(d)) => {
                    let x = match v {
                        Some(Value::Bool(b)) => *b as u8 as f32,
                        Some(v) => v.as_f64().map(|x| (x >= 0.5) as u8 as f32).unwrap_or(d),
                        None => d,
                    };
                    p.nums.push((s.name, x));
                }
                (_, ParamDefault::Vec3(d)) => {
                    let c = v.and_then(json_vec).unwrap_or(d);
                    p.vecs.push((s.name, c.map(|x| if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 })));
                }
                (_, ParamDefault::Vec2(d)) => {
                    let c = v.and_then(json_vec).unwrap_or([d[0], d[1], 0.0]);
                    p.vecs.push((s.name, c.map(|x| if x.is_finite() { x.clamp(-1.0, 2.0) } else { 0.5 })));
                }
                _ => {}
            }
        }
        p
    }

    pub fn n(&self, k: &str) -> f32 {
        self.nums.iter().find(|e| e.0 == k).map_or(0.0, |e| e.1)
    }

    pub fn c(&self, k: &str) -> V3 {
        self.vecs.iter().find(|e| e.0 == k).map_or([0.0; 3], |e| e.1)
    }

    pub fn pt(&self, k: &str) -> (f32, f32) {
        let v = self.vecs.iter().find(|e| e.0 == k).map_or([0.5, 0.5, 0.0], |e| e.1);
        (v[0], v[1])
    }
}

/// Per-pixel context: the unmodified source, its size, the frame aspect (w/h), time (seconds, item-relative).
pub struct Ctx<'a> {
    pub src: &'a Rgba,
    pub w: f32,
    pub h: f32,
    pub aspect: f32,
    pub t: f32,
    pub p: &'a Params,
}

impl Ctx<'_> {
    /// Bilinear sample at normalised coordinates (clamped to the edge), RGB in 0..1.
    #[inline]
    pub fn sample(&self, u: f32, v: f32) -> V3 {
        let s = self.src;
        let x = (u * self.w - 0.5).clamp(0.0, self.w - 1.0);
        let y = (v * self.h - 0.5).clamp(0.0, self.h - 1.0);
        let (x0, y0) = (x as u32, y as u32);
        let (x1, y1) = ((x0 + 1).min(s.w - 1), (y0 + 1).min(s.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let px = |x: u32, y: u32| {
            let i = ((y * s.w + x) * 4) as usize;
            [s.data[i] as f32, s.data[i + 1] as f32, s.data[i + 2] as f32]
        };
        let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        let mut o = [0.0; 3];
        for k in 0..3 {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bot = c[k] + (d[k] - c[k]) * fx;
            o[k] = (top + (bot - top) * fy) / 255.0;
        }
        o
    }

    /// Aspect-corrected offset from a centre point: x scaled by the aspect so circles stay round.
    #[inline]
    pub fn rel(&self, u: f32, v: f32, c: (f32, f32)) -> (f32, f32) {
        ((u - c.0) * self.aspect, v - c.1)
    }

    /// Inverse of [`Ctx::rel`].
    #[inline]
    pub fn abs(&self, x: f32, y: f32, c: (f32, f32)) -> (f32, f32) {
        (x / self.aspect + c.0, y + c.1)
    }
}

#[inline]
pub fn luma(c: V3) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

#[inline]
pub fn mix(a: V3, b: V3, t: f32) -> V3 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

#[inline]
pub fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn fract(x: f32) -> f32 {
    x - x.floor()
}

#[inline]
pub fn rot(x: f32, y: f32, deg: f32) -> (f32, f32) {
    let (s, c) = deg.to_radians().sin_cos();
    (x * c - y * s, x * s + y * c)
}

/// Integer hash → 0..1.
#[inline]
pub fn hash21(x: i32, y: i32, seed: u32) -> f32 {
    let mut h =
        (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// Smooth value noise in 0..1.
#[inline]
pub fn noise2(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash21(xi, yi, seed);
    let b = hash21(xi + 1, yi, seed);
    let c = hash21(xi, yi + 1, seed);
    let d = hash21(xi + 1, yi + 1, seed);
    let top = a + (b - a) * ux;
    let bot = c + (d - c) * ux;
    top + (bot - top) * uy
}

/// Fractal noise (sum of octaves), roughly 0..1.
#[inline]
pub fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for o in 0..octaves {
        sum += amp * noise2(x * freq, y * freq, seed.wrapping_add(o * 101));
        norm += amp;
        amp *= 0.5;
        freq *= 2.02;
    }
    sum / norm.max(1e-6)
}

/// Linear mix across a list of colours by `t` in 0..1.
pub fn ramp(colors: &[V3], t: f32) -> V3 {
    let t = t.clamp(0.0, 1.0) * (colors.len() - 1) as f32;
    let i = (t as usize).min(colors.len() - 2);
    mix(colors[i], colors[i + 1], t - i as f32)
}
