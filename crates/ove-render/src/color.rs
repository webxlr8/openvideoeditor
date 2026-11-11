//! The grade model (`colorGrade`): lift/gamma/gain/offset wheels, curves, saturation and LUTs (presets or `.cube`
//! files, at the input or look stage), plus the reference match fit and scopes.
//!
//! A grade is baked into one 33³ lattice and applied per pixel with tetrahedral interpolation, so a full grade
//! costs the same as a single LUT however many controls are set.
//!
//! Order: input LUT → offset → lift → gamma → gain → tone curves (master, then red/green/blue) → hue-vs-hue →
//! hue-vs-lum → saturation × hue-vs-sat × lum-vs-sat → look LUT.

use std::sync::{Arc, OnceLock};

use rayon::prelude::*;
use serde_json::{Map, Value, json};

use crate::canvas::Rgba;

pub type Rgb = [f32; 3];

// ---------------------------------------------------------------------------------------------------------------
// LUTs
// ---------------------------------------------------------------------------------------------------------------

/// A 1D (per-channel) or 3D colour lookup table over a domain (usually 0..1).
#[derive(Debug, Clone, PartialEq)]
pub struct Lut {
    pub size: usize,
    pub three_d: bool,
    pub domain_min: Rgb,
    pub domain_max: Rgb,
    /// 1D: `size` entries. 3D: `size³` entries with red varying fastest (the `.cube` order).
    pub data: Vec<Rgb>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interp {
    Trilinear,
    Tetrahedral,
}

impl Lut {
    /// Builds a 3D LUT by sampling `f` on a `size`³ lattice over 0..1.
    pub fn from_fn(size: usize, f: impl Fn(Rgb) -> Rgb + Sync) -> Lut {
        let size = size.clamp(2, 129);
        let k = 1.0 / (size - 1) as f32;
        let data = (0..size * size * size)
            .into_par_iter()
            .map(|i| {
                let (r, g, b) = (i % size, (i / size) % size, i / (size * size));
                f([r as f32 * k, g as f32 * k, b as f32 * k])
            })
            .collect();
        Lut { size, three_d: true, domain_min: [0.0; 3], domain_max: [1.0; 3], data }
    }

    /// Parses Adobe/Resolve `.cube` text (LUT_1D_SIZE or LUT_3D_SIZE, optional DOMAIN_MIN/MAX and TITLE).
    pub fn parse_cube(text: &str) -> Result<Lut, String> {
        let (mut size, mut three_d) = (0usize, true);
        let (mut dmin, mut dmax) = ([0.0f32; 3], [1.0f32; 3]);
        let mut data = Vec::new();
        let triple = |rest: &[&str], line: usize| -> Result<Rgb, String> {
            if rest.len() < 3 {
                return Err(format!("line {line}: expected three numbers"));
            }
            let mut o = [0.0; 3];
            for (k, s) in rest.iter().take(3).enumerate() {
                o[k] = s.parse::<f32>().map_err(|_| format!("line {line}: '{s}' is not a number"))?;
                if !o[k].is_finite() {
                    return Err(format!("line {line}: non-finite value"));
                }
            }
            Ok(o)
        };
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            match parts[0].to_ascii_uppercase().as_str() {
                "TITLE" => {}
                "LUT_3D_SIZE" | "LUT_1D_SIZE" => {
                    three_d = parts[0].eq_ignore_ascii_case("LUT_3D_SIZE");
                    size = parts.get(1).and_then(|s| s.parse().ok()).ok_or(format!("line {}: bad size", n + 1))?;
                    let max = if three_d { 256 } else { 65_536 };
                    if !(2..=max).contains(&size) {
                        return Err(format!("line {}: size {size} out of range", n + 1));
                    }
                }
                "DOMAIN_MIN" => dmin = triple(&parts[1..], n + 1)?,
                "DOMAIN_MAX" => dmax = triple(&parts[1..], n + 1)?,
                "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                    let lo = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let hi = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
                    (dmin, dmax) = ([lo; 3], [hi; 3]);
                }
                s if s.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.' || c == '+') => {
                    data.push(triple(&parts, n + 1)?);
                }
                _ => {} // unknown keywords are ignored, as the format allows
            }
        }
        if size == 0 {
            return Err("missing LUT_3D_SIZE or LUT_1D_SIZE".into());
        }
        let want = if three_d { size * size * size } else { size };
        if data.len() != want {
            return Err(format!("expected {want} entries, found {}", data.len()));
        }
        if (0..3).any(|k| dmax[k] <= dmin[k]) {
            return Err("DOMAIN_MAX must exceed DOMAIN_MIN".into());
        }
        Ok(Lut { size, three_d, domain_min: dmin, domain_max: dmax, data })
    }

    /// `.cube` text for this LUT.
    pub fn to_cube(&self, title: &str, header: &str) -> String {
        let mut s = String::new();
        for l in header.lines() {
            s.push_str(&format!("# {l}\n"));
        }
        s.push_str(&format!("TITLE \"{title}\"\n"));
        s.push_str(&format!("LUT_{}_SIZE {}\n", if self.three_d { "3D" } else { "1D" }, self.size));
        let (a, b) = (self.domain_min, self.domain_max);
        s.push_str(&format!("DOMAIN_MIN {} {} {}\nDOMAIN_MAX {} {} {}\n", a[0], a[1], a[2], b[0], b[1], b[2]));
        for c in &self.data {
            s.push_str(&format!("{:.6} {:.6} {:.6}\n", c[0], c[1], c[2]));
        }
        s
    }

    fn norm(&self, c: Rgb) -> Rgb {
        let mut o = [0.0; 3];
        for k in 0..3 {
            let v = (c[k] - self.domain_min[k]) / (self.domain_max[k] - self.domain_min[k]);
            o[k] = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        }
        o
    }

    pub fn apply(&self, c: Rgb, interp: Interp) -> Rgb {
        let n = self.norm(c);
        let m = (self.size - 1) as f32;
        if !self.three_d {
            let mut o = [0.0; 3];
            for k in 0..3 {
                let x = n[k] * m;
                let i = (x as usize).min(self.size - 2);
                let f = x - i as f32;
                o[k] = self.data[i][k] + (self.data[i + 1][k] - self.data[i][k]) * f;
            }
            return o;
        }
        let s = self.size;
        let (x, y, z) = (n[0] * m, n[1] * m, n[2] * m);
        let (i, j, l) = ((x as usize).min(s - 2), (y as usize).min(s - 2), (z as usize).min(s - 2));
        let (fx, fy, fz) = (x - i as f32, y - j as f32, z - l as f32);
        let at = |a: usize, b: usize, c: usize| self.data[(l + c) * s * s + (j + b) * s + (i + a)];
        let c000 = at(0, 0, 0);
        let c111 = at(1, 1, 1);
        let mut o = [0.0; 3];
        match interp {
            Interp::Trilinear => {
                let (c100, c010, c001) = (at(1, 0, 0), at(0, 1, 0), at(0, 0, 1));
                let (c110, c101, c011) = (at(1, 1, 0), at(1, 0, 1), at(0, 1, 1));
                for k in 0..3 {
                    let x00 = c000[k] + (c100[k] - c000[k]) * fx;
                    let x10 = c010[k] + (c110[k] - c010[k]) * fx;
                    let x01 = c001[k] + (c101[k] - c001[k]) * fx;
                    let x11 = c011[k] + (c111[k] - c011[k]) * fx;
                    let y0 = x00 + (x10 - x00) * fy;
                    let y1 = x01 + (x11 - x01) * fy;
                    o[k] = y0 + (y1 - y0) * fz;
                }
            }
            Interp::Tetrahedral => {
                // Split the cube into six tetrahedra along its main diagonal; pick the one holding the point.
                let (a, b, wa, wb, wc) = if fx >= fy {
                    if fy >= fz {
                        (at(1, 0, 0), at(1, 1, 0), fx - fy, fy - fz, fz)
                    } else if fx >= fz {
                        (at(1, 0, 0), at(1, 0, 1), fx - fz, fz - fy, fy)
                    } else {
                        (at(0, 0, 1), at(1, 0, 1), fz - fx, fx - fy, fy)
                    }
                } else if fz >= fy {
                    (at(0, 0, 1), at(0, 1, 1), fz - fy, fy - fx, fx)
                } else if fz >= fx {
                    (at(0, 1, 0), at(0, 1, 1), fy - fz, fz - fx, fx)
                } else {
                    (at(0, 1, 0), at(1, 1, 0), fy - fx, fx - fz, fz)
                };
                let w0 = 1.0 - wa - wb - wc;
                for k in 0..3 {
                    o[k] = c000[k] * w0 + a[k] * wa + b[k] * wb + c111[k] * wc;
                }
            }
        }
        o
    }
}

/// Applies `lut` to the image, mixed by `intensity`.
pub fn apply_lut(img: &mut Rgba, lut: &Lut, intensity: f32) {
    let k = if intensity.is_finite() { intensity.clamp(0.0, 1.0) } else { 0.0 };
    if k <= 0.0 {
        return;
    }
    img.data.par_chunks_exact_mut(4).for_each(|p| {
        let c = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
        let o = lut.apply(c, Interp::Tetrahedral);
        for i in 0..3 {
            let v = c[i] + (o[i] - c[i]) * k;
            p[i] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    });
}

// ---------------------------------------------------------------------------------------------------------------
// Preset looks (our own, defined procedurally; `cargo run -p ove-render --example gen_luts` writes assets/luts)
// ---------------------------------------------------------------------------------------------------------------

pub const LUT_PRESETS: &[(&str, &str)] = &[
    ("cinematic", "Cinematic"),
    ("black-and-white", "Black & white"),
    ("vintage", "Vintage"),
    ("cool", "Cool"),
    ("warm", "Warm"),
    ("high-contrast", "High contrast"),
    ("faded-film", "Faded film"),
    ("technicolor", "Technicolor"),
    ("newsreel", "Newsreel"),
];

fn luma(c: Rgb) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn sat(c: Rgb, s: f32) -> Rgb {
    let l = luma(c);
    c.map(|v| l + (v - l) * s)
}

/// Smooth S-curve around 0.5 with strength `k` (0 = identity).
fn s_curve(v: f32, k: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    let s = v * v * (3.0 - 2.0 * v);
    v + (s - v) * k
}

fn split_tone(c: Rgb, shadows: Rgb, highlights: Rgb, amount: f32) -> Rgb {
    let l = luma(c);
    let (ws, wh) = ((1.0 - l).powi(2), l * l);
    [0, 1, 2].map(|k| c[k] + ((shadows[k] - 0.5) * ws + (highlights[k] - 0.5) * wh) * amount)
}

/// The look function of a preset id.
pub fn preset_fn(id: &str) -> Option<fn(Rgb) -> Rgb> {
    Some(match id {
        "cinematic" => |c| {
            let c = c.map(|v| s_curve(v, 0.35));
            let c = split_tone(c, [0.35, 0.55, 0.62], [0.68, 0.55, 0.4], 0.5);
            sat(c, 0.9)
        },
        "black-and-white" => |c| {
            // Panchromatic-ish mix that favours red a little, for skin and skies.
            let l = (0.33 * c[0] + 0.56 * c[1] + 0.11 * c[2]).clamp(0.0, 1.0);
            [s_curve(l, 0.3); 3]
        },
        "vintage" => |c| {
            let c = sat(c, 0.7);
            let c = [c[0] * 1.06 + 0.02, c[1] * 1.0 + 0.01, c[2] * 0.86 + 0.04];
            c.map(|v| 0.07 + v.clamp(0.0, 1.0) * 0.86)
        },
        "cool" => |c| {
            let c = [c[0] * 0.92, c[1] * 0.99, c[2] * 1.08 + 0.02];
            sat(c, 0.95)
        },
        "warm" => |c| {
            let c = [c[0] * 1.08 + 0.02, c[1] * 1.01, c[2] * 0.88];
            sat(c, 1.05)
        },
        "high-contrast" => |c| sat(c.map(|v| s_curve(s_curve(v, 1.0), 0.5)), 1.15),
        "faded-film" => |c| {
            let c = sat(c.map(|v| s_curve(v, -0.25)), 0.8);
            let c = split_tone(c, [0.45, 0.56, 0.52], [0.56, 0.52, 0.45], 0.3);
            c.map(|v| 0.1 + v.clamp(0.0, 1.0) * 0.8)
        },
        "technicolor" => |c| {
            // Exaggerate the separation between red and its complementary cyan-green, then saturate.
            let r = c[0] * 1.2 - (c[1] + c[2]) * 0.1;
            let g = c[1] * 1.1 - (c[0] + c[2]) * 0.05;
            let b = c[2] * 1.1 - (c[0] + c[1]) * 0.05;
            sat([r, g, b].map(|v| s_curve(v, 0.2)), 1.3)
        },
        "newsreel" => |c| {
            let l = s_curve(s_curve(luma(c), 0.8), 0.4);
            [l * 1.02 + 0.01, l * 0.98 + 0.005, l * 0.88]
        },
        _ => return None,
    })
}

/// A preset as a (cached) 33³ LUT.
pub fn preset_lut(id: &str) -> Option<Arc<Lut>> {
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, Arc<Lut>>>> = OnceLock::new();
    let f = preset_fn(id)?;
    let cache = CACHE.get_or_init(Default::default);
    let mut m = cache.lock().unwrap_or_else(|e| e.into_inner());
    Some(
        m.entry(id.to_string())
            .or_insert_with(|| Arc::new(Lut::from_fn(33, |c| f(c).map(|v| v.clamp(0.0, 1.0)))))
            .clone(),
    )
}

// ---------------------------------------------------------------------------------------------------------------
// Curves and wheels
// ---------------------------------------------------------------------------------------------------------------

/// A monotone cubic curve through control points (Fritsch–Carlson), optionally periodic over x in 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    xs: Vec<f32>,
    ys: Vec<f32>,
    ms: Vec<f32>,
}

impl Curve {
    pub fn new(mut pts: Vec<(f32, f32)>, periodic: bool) -> Option<Curve> {
        pts.retain(|p| p.0.is_finite() && p.1.is_finite());
        if pts.is_empty() {
            return None;
        }
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
        if periodic {
            let n = pts.len();
            let mut w: Vec<(f32, f32)> = pts.iter().map(|p| (p.0 - 1.0, p.1)).collect();
            w.extend(pts.iter().copied());
            w.extend(pts.iter().map(|p| (p.0 + 1.0, p.1)));
            if n == 1 {
                w = vec![(-1.0, pts[0].1), (2.0, pts[0].1)];
            }
            pts = w;
        }
        if pts.len() == 1 {
            pts.push((pts[0].0 + 1.0, pts[0].1));
        }
        let n = pts.len();
        let (xs, ys): (Vec<f32>, Vec<f32>) = pts.into_iter().unzip();
        let d: Vec<f32> = (0..n - 1).map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i])).collect();
        let mut ms = vec![0.0; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            ms[i] = if d[i - 1] * d[i] <= 0.0 { 0.0 } else { (d[i - 1] + d[i]) * 0.5 };
        }
        for i in 0..n - 1 {
            if d[i] == 0.0 {
                ms[i] = 0.0;
                ms[i + 1] = 0.0;
                continue;
            }
            let (a, b) = (ms[i] / d[i], ms[i + 1] / d[i]);
            let h = a * a + b * b;
            if h > 9.0 {
                let t = 3.0 / h.sqrt();
                ms[i] = t * a * d[i];
                ms[i + 1] = t * b * d[i];
            }
        }
        Some(Curve { xs, ys, ms })
    }

    /// Reads `[[x,y],…]` or `[{x,y},…]`.
    pub fn from_json(v: &Value, periodic: bool) -> Option<Curve> {
        let pts = v
            .as_array()?
            .iter()
            .filter_map(|p| match p {
                Value::Array(a) => Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)),
                Value::Object(o) => Some((o.get("x")?.as_f64()? as f32, o.get("y")?.as_f64()? as f32)),
                _ => None,
            })
            .collect();
        Curve::new(pts, periodic)
    }

    pub fn eval(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1];
        }
        let i = self.xs.partition_point(|&a| a <= x).saturating_sub(1).min(n - 2);
        let h = self.xs[i + 1] - self.xs[i];
        let t = (x - self.xs[i]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * self.ys[i]
            + (t3 - 2.0 * t2 + t) * h * self.ms[i]
            + (-2.0 * t3 + 3.0 * t2) * self.ys[i + 1]
            + (t3 - t2) * h * self.ms[i + 1]
    }
}

/// One wheel: per-channel values and a master (`y`) that applies to all three.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Wheel {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub y: f32,
}

impl Wheel {
    fn from_json(v: Option<&Value>) -> Wheel {
        let n = |x: Option<&Value>| x.and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(0.0) as f32;
        match v {
            Some(Value::Object(o)) => Wheel { r: n(o.get("r")), g: n(o.get("g")), b: n(o.get("b")), y: n(o.get("y")) },
            Some(Value::Array(a)) => Wheel { r: n(a.first()), g: n(a.get(1)), b: n(a.get(2)), y: n(a.get(3)) },
            _ => Wheel::default(),
        }
    }

    fn ch(&self, k: usize) -> f32 {
        [self.r, self.g, self.b][k] + self.y
    }

    pub fn to_json(self) -> Value {
        json!({"r": self.r, "g": self.g, "b": self.b, "y": self.y})
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LutStage {
    Input,
    Look,
}

/// The parsed `colorGrade` object (plus an adjustment item's `lutPresetId`/`lutIntensity`).
#[derive(Debug, Clone, Default)]
pub struct ColorGrade {
    pub lift: Wheel,
    pub gamma: Wheel,
    pub gain: Wheel,
    pub offset: Wheel,
    pub master: Option<Curve>,
    pub rgb: [Option<Curve>; 3],
    pub hue_vs_sat: Option<Curve>,
    pub hue_vs_hue: Option<Curve>,
    pub hue_vs_lum: Option<Curve>,
    pub lum_vs_sat: Option<Curve>,
    pub saturation: f32,
    /// (LUT, intensity, stage)
    pub luts: Vec<(Arc<Lut>, f32, LutStage)>,
}

/// Where a `.cube` file named in a grade is found: absolute, or relative to the project bundle.
pub type LutLoader<'a> = &'a dyn Fn(&str) -> Option<Arc<Lut>>;

impl ColorGrade {
    /// Parses an item's grade fields. Returns None when nothing would change the image.
    pub fn from_fields(f: &Map<String, Value>, load: LutLoader) -> Option<ColorGrade> {
        let mut g = ColorGrade { saturation: 1.0, ..Default::default() };
        if let Some(cg) = f.get("colorGrade").and_then(Value::as_object) {
            if let Some(w) = cg.get("wheels").and_then(Value::as_object) {
                g.lift = Wheel::from_json(w.get("lift"));
                g.gamma = Wheel::from_json(w.get("gamma"));
                g.gain = Wheel::from_json(w.get("gain"));
                g.offset = Wheel::from_json(w.get("offset"));
            }
            if let Some(c) = cg.get("curves").and_then(Value::as_object) {
                g.master = c.get("master").and_then(|v| Curve::from_json(v, false));
                for (k, name) in ["red", "green", "blue"].iter().enumerate() {
                    g.rgb[k] = c.get(*name).and_then(|v| Curve::from_json(v, false));
                }
                g.hue_vs_sat = c.get("hueVsSat").and_then(|v| Curve::from_json(v, true));
                g.hue_vs_hue = c.get("hueVsHue").and_then(|v| Curve::from_json(v, true));
                g.hue_vs_lum = c.get("hueVsLum").and_then(|v| Curve::from_json(v, true));
                g.lum_vs_sat = c.get("lumVsSat").and_then(|v| Curve::from_json(v, false));
            }
            if let Some(s) = cg.get("saturation").and_then(Value::as_f64).filter(|s| s.is_finite()) {
                g.saturation = s.clamp(0.0, 4.0) as f32;
            }
            if let Some(l) = cg.get("lut").and_then(Value::as_object) {
                let lut = l
                    .get("file")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .and_then(load)
                    .or_else(|| l.get("preset").and_then(Value::as_str).and_then(preset_lut));
                let k = l.get("intensity").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.0, 1.0) as f32;
                let stage = if l.get("stage").and_then(Value::as_str) == Some("input") {
                    LutStage::Input
                } else {
                    LutStage::Look
                };
                if let Some(lut) = lut {
                    g.luts.push((lut, k, stage));
                }
            }
        }
        if let Some(lut) = f.get("lutPresetId").and_then(Value::as_str).and_then(preset_lut) {
            let k = f.get("lutIntensity").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.0, 1.0) as f32;
            g.luts.push((lut, k, LutStage::Look));
        }
        let identity = g.lift == Wheel::default()
            && g.gamma == Wheel::default()
            && g.gain == Wheel::default()
            && g.offset == Wheel::default()
            && g.master.is_none()
            && g.rgb.iter().all(Option::is_none)
            && g.hue_vs_sat.is_none()
            && g.hue_vs_hue.is_none()
            && g.hue_vs_lum.is_none()
            && g.lum_vs_sat.is_none()
            && g.saturation == 1.0
            && g.luts.iter().all(|l| l.1 <= 0.0);
        (!identity).then_some(g)
    }

    fn luts_at(&self, c: Rgb, stage: LutStage) -> Rgb {
        let mut c = c;
        for (lut, k, s) in &self.luts {
            if *s == stage && *k > 0.0 {
                let o = lut.apply(c, Interp::Tetrahedral);
                c = [0, 1, 2].map(|i| c[i] + (o[i] - c[i]) * k);
            }
        }
        c
    }

    /// The grade as a function of one colour (0..1 in, 0..1 out).
    pub fn eval(&self, c: Rgb) -> Rgb {
        let mut c = self.luts_at(c, LutStage::Input);
        for (k, out) in c.iter_mut().enumerate() {
            let mut v = *out + self.offset.ch(k) * 0.5;
            v += self.lift.ch(k) * 0.5 * (1.0 - v);
            let gm = (1.0 + self.gamma.ch(k)).clamp(0.05, 10.0);
            v = v.max(0.0).powf(1.0 / gm);
            v *= (1.0 + self.gain.ch(k)).max(0.0);
            if let Some(m) = &self.master {
                v = m.eval(v);
            }
            if let Some(cv) = &self.rgb[k] {
                v = cv.eval(v);
            }
            *out = v.clamp(0.0, 1.0);
        }
        if self.hue_vs_hue.is_some() || self.hue_vs_lum.is_some() || self.hue_vs_sat.is_some() {
            let (h, s, v) = rgb_to_hsv(c);
            if let Some(cv) = &self.hue_vs_hue {
                c = hsv_to_rgb((h + (cv.eval(h) - 0.5)).rem_euclid(1.0), s, v);
            }
            if let Some(cv) = &self.hue_vs_lum {
                let d = (cv.eval(h) - 0.5) * s; // grey has no hue to key on
                c = c.map(|x| (x + d).clamp(0.0, 1.0));
            }
        }
        let mut s = self.saturation;
        if let Some(cv) = &self.hue_vs_sat {
            let (h, sat, _) = rgb_to_hsv(c);
            s *= 1.0 + (cv.eval(h) * 2.0 - 1.0) * sat.min(1.0).sqrt();
        }
        if let Some(cv) = &self.lum_vs_sat {
            s *= (cv.eval(luma(c)) * 2.0).max(0.0);
        }
        if s != 1.0 {
            let l = luma(c);
            c = c.map(|x| (l + (x - l) * s).clamp(0.0, 1.0));
        }
        self.luts_at(c, LutStage::Look).map(|x| x.clamp(0.0, 1.0))
    }

    pub fn bake(&self) -> Lut {
        Lut::from_fn(33, |c| self.eval(c))
    }
}

fn rgb_to_hsv(c: Rgb) -> (f32, f32, f32) {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let d = mx - mn;
    let h = if d <= 1e-6 {
        0.0
    } else if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0) / 6.0
    } else if mx == c[1] {
        ((c[2] - c[0]) / d + 2.0) / 6.0
    } else {
        ((c[0] - c[1]) / d + 4.0) / 6.0
    };
    (h, if mx > 0.0 { d / mx } else { 0.0 }, mx)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let f = h6 - h6.floor();
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match h6 as u32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Reference match
// ---------------------------------------------------------------------------------------------------------------

fn srgb_to_lin(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn lin_to_srgb(v: f32) -> f32 {
    let v = v.max(0.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

const WHITE: Rgb = [0.950_47, 1.0, 1.088_83];

fn lab_f(t: f32) -> f32 {
    if t > 0.008_856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 }
}

fn lab_finv(t: f32) -> f32 {
    if t > 0.206_893 { t * t * t } else { (t - 16.0 / 116.0) / 7.787 }
}

pub fn rgb_to_lab(c: Rgb) -> Rgb {
    let [r, g, b] = c.map(srgb_to_lin);
    let x = (0.412_456 * r + 0.357_576 * g + 0.180_438 * b) / WHITE[0];
    let y = (0.212_673 * r + 0.715_152 * g + 0.072_175 * b) / WHITE[1];
    let z = (0.019_334 * r + 0.119_192 * g + 0.950_304 * b) / WHITE[2];
    let (fx, fy, fz) = (lab_f(x), lab_f(y), lab_f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

pub fn lab_to_rgb(l: Rgb) -> Rgb {
    let fy = (l[0] + 16.0) / 116.0;
    let (fx, fz) = (fy + l[1] / 500.0, fy - l[2] / 200.0);
    let (x, y, z) = (lab_finv(fx) * WHITE[0], lab_finv(fy) * WHITE[1], lab_finv(fz) * WHITE[2]);
    let r = 3.240_454 * x - 1.537_138 * y - 0.498_531 * z;
    let g = -0.969_266 * x + 1.876_011 * y + 0.041_556 * z;
    let b = 0.055_643 * x - 0.204_026 * y + 1.057_225 * z;
    [r, g, b].map(|v| lin_to_srgb(v).clamp(0.0, 1.0))
}

fn pixels(img: &Rgba, max: usize) -> Vec<Rgb> {
    let n = (img.w * img.h) as usize;
    let step = (n / max.max(1)).max(1);
    (0..n).step_by(step).map(|i| [0, 1, 2].map(|k| img.data[i * 4 + k] as f32 / 255.0)).collect()
}

fn mean_std(v: &[Rgb]) -> (Rgb, Rgb) {
    let n = v.len().max(1) as f32;
    let mut m = [0.0; 3];
    for c in v {
        for k in 0..3 {
            m[k] += c[k] / n;
        }
    }
    let mut s = [0.0; 3];
    for c in v {
        for k in 0..3 {
            s[k] += (c[k] - m[k]).powi(2) / n;
        }
    }
    (m, s.map(f32::sqrt))
}

/// Fits the colour of `target` to `reference`: transfers the Lab mean and standard deviation, then expresses the
/// result as concrete per-channel gain and offset wheels (least squares over the frame). Returns the `wheels`
/// object to store in `colorGrade` (the reference itself is not kept).
pub fn fit_match(reference: &Rgba, target: &Rgba, strength: f32) -> Value {
    let k = strength.clamp(0.0, 1.0);
    let rp = pixels(reference, 20_000);
    let tp = pixels(target, 20_000);
    let rl: Vec<Rgb> = rp.iter().map(|&c| rgb_to_lab(c)).collect();
    let tl: Vec<Rgb> = tp.iter().map(|&c| rgb_to_lab(c)).collect();
    let (rm, rs) = mean_std(&rl);
    let (tm, ts) = mean_std(&tl);
    let mut gain = Wheel::default();
    let mut offset = Wheel::default();
    for ch in 0..3 {
        // Least-squares line from the target's channel to the transferred channel.
        let (mut sx, mut sy, mut sxx, mut sxy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for (c, l) in tp.iter().zip(&tl) {
            let moved: Rgb = [0, 1, 2].map(|i| (l[i] - tm[i]) * (rs[i] / ts[i].max(1e-3)) + rm[i]);
            let want = lab_to_rgb(moved)[ch];
            let want = c[ch] + (want - c[ch]) * k;
            let (x, y) = (c[ch] as f64, want as f64);
            sx += x;
            sy += y;
            sxx += x * x;
            sxy += x * y;
        }
        let n = tp.len().max(1) as f64;
        let den = n * sxx - sx * sx;
        let a = if den.abs() > 1e-9 { (n * sxy - sx * sy) / den } else { 1.0 };
        let a = a.clamp(0.2, 4.0);
        let b = (sy - a * sx) / n;
        let (g, o) = ((a - 1.0) as f32, (b * 2.0) as f32); // offset wheel is scaled by 0.5 in eval
        match ch {
            0 => (gain.r, offset.r) = (g, o),
            1 => (gain.g, offset.g) = (g, o),
            _ => (gain.b, offset.b) = (g, o),
        }
    }
    // eval applies offset before gain: (x + o/2)·(1+g) = a·x + b  ⇒  o = 2b / a.
    for (o, g) in [(&mut offset.r, gain.r), (&mut offset.g, gain.g), (&mut offset.b, gain.b)] {
        *o /= 1.0 + g;
    }
    json!({"lift": Wheel::default().to_json(), "gamma": Wheel::default().to_json(), "gain": gain.to_json(), "offset": offset.to_json()})
}

// ---------------------------------------------------------------------------------------------------------------
// Scopes
// ---------------------------------------------------------------------------------------------------------------

/// Histograms (red, green, blue, luma; 64 bins each, normalised to the tallest bin) for a scope display.
pub fn histogram(img: &Rgba) -> [Vec<f32>; 4] {
    let mut h = [[0u32; 64]; 4];
    for p in img.data.chunks_exact(4) {
        for k in 0..3 {
            h[k][(p[k] >> 2) as usize] += 1;
        }
        let l = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) as u32;
        h[3][(l.min(255) >> 2) as usize] += 1;
    }
    let max = h.iter().flat_map(|c| c.iter()).copied().max().unwrap_or(1).max(1) as f32;
    h.map(|c| c.iter().map(|&v| v as f32 / max).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Rgb, b: Rgb, eps: f32) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() <= eps)
    }

    #[test]
    fn cube_round_trip_and_identity() {
        let id = Lut::from_fn(9, |c| c);
        let text = id.to_cube("identity", "test");
        let back = Lut::parse_cube(&text).unwrap();
        assert_eq!(back.size, 9);
        for c in [[0.1, 0.5, 0.9], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.33, 0.77, 0.21]] {
            assert!(close(back.apply(c, Interp::Tetrahedral), c, 1e-5));
            assert!(close(back.apply(c, Interp::Trilinear), c, 1e-5));
        }
    }

    #[test]
    fn cube_1d_domain_and_errors() {
        let l = Lut::parse_cube("LUT_1D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\n0 0 0\n1 1 1\n").unwrap();
        assert!(close(l.apply([1.0, 0.5, 2.0], Interp::Tetrahedral), [0.5, 0.25, 1.0], 1e-6));
        assert!(Lut::parse_cube("LUT_3D_SIZE 2\n0 0 0\n").is_err());
        assert!(Lut::parse_cube("0 0 0\n").is_err());
        assert!(Lut::parse_cube("LUT_3D_SIZE 2\n0 0 x\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n").is_err());
    }

    #[test]
    fn tetrahedral_is_exact_on_lattice_and_matches_trilinear_on_linear_maps() {
        let f = |c: Rgb| [c[0] * 0.5 + 0.1, c[1] * 0.8, 1.0 - c[2]];
        let l = Lut::from_fn(17, f);
        for c in [[0.12, 0.6, 0.31], [0.9, 0.05, 0.5]] {
            assert!(close(l.apply(c, Interp::Tetrahedral), f(c), 1e-4));
            assert!(close(l.apply(c, Interp::Trilinear), f(c), 1e-4));
        }
    }

    #[test]
    fn presets_exist_and_stay_in_range() {
        assert_eq!(LUT_PRESETS.len(), 9);
        for (id, _) in LUT_PRESETS {
            let lut = preset_lut(id).unwrap();
            assert!(lut.data.iter().all(|c| c.iter().all(|v| (0.0..=1.0).contains(v))));
        }
        let bw = preset_fn("black-and-white").unwrap()([0.9, 0.2, 0.1]);
        assert!(bw[0] == bw[1] && bw[1] == bw[2]);
    }

    #[test]
    fn shipped_cube_files_match_the_presets() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/luts");
        for (id, _) in LUT_PRESETS {
            let text = std::fs::read_to_string(dir.join(format!("{id}.cube"))).expect("run the gen_luts example");
            let lut = Lut::parse_cube(&text).unwrap();
            assert_eq!(lut.size, 17);
            let f = preset_fn(id).unwrap();
            for c in [[0.0, 0.0, 0.0], [0.25, 0.5, 0.75], [1.0, 0.5, 0.0], [0.5, 0.5, 0.5]] {
                let want = f(c).map(|v| v.clamp(0.0, 1.0));
                assert!(close(lut.apply(c, Interp::Tetrahedral), want, 1e-4), "{id} drifted from its .cube file");
            }
        }
    }

    #[test]
    fn wheels_curves_and_saturation() {
        let none = |_: &str| None;
        let f = |v: Value| v.as_object().unwrap().clone();
        assert!(ColorGrade::from_fields(&f(json!({"colorGrade": {}})), &none).is_none());
        let g = ColorGrade::from_fields(
            &f(json!({"colorGrade": {"wheels": {"gain": {"r": 0.5, "g": 0, "b": 0, "y": 0}}}})),
            &none,
        )
        .unwrap();
        assert!(close(g.eval([0.4, 0.4, 0.4]), [0.6, 0.4, 0.4], 1e-5));
        let g = ColorGrade::from_fields(&f(json!({"colorGrade": {"saturation": 0}})), &none).unwrap();
        let o = g.eval([0.8, 0.2, 0.1]);
        assert!((o[0] - o[1]).abs() < 1e-5 && (o[1] - o[2]).abs() < 1e-5);
        let g = ColorGrade::from_fields(&f(json!({"colorGrade": {"curves": {"master": [[0, 1], [1, 0]]}}})), &none)
            .unwrap();
        assert!(close(g.eval([0.25, 0.25, 0.25]), [0.75, 0.75, 0.75], 1e-4), "inverting curve");
        let c = Curve::new(vec![(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)], false).unwrap();
        let mut last = -1.0;
        for i in 0..=100 {
            let v = c.eval(i as f32 / 100.0);
            assert!(v >= last - 1e-6, "monotone");
            last = v;
        }
        // Adjustment LUT preset fields.
        let g =
            ColorGrade::from_fields(&f(json!({"lutPresetId": "black-and-white", "lutIntensity": 1})), &none).unwrap();
        let o = g.eval([0.9, 0.2, 0.1]);
        assert!((o[0] - o[2]).abs() < 0.02);
    }

    #[test]
    fn lab_round_trip_and_match_moves_towards_reference() {
        for c in [[0.2, 0.4, 0.6], [0.9, 0.1, 0.3], [0.5, 0.5, 0.5]] {
            assert!(close(lab_to_rgb(rgb_to_lab(c)), c, 2e-3));
        }
        let mut warm = Rgba::new(16, 16);
        let mut cool = Rgba::new(16, 16);
        for (i, (a, b)) in warm.data.chunks_exact_mut(4).zip(cool.data.chunks_exact_mut(4)).enumerate() {
            let v = (i % 16) as u8 * 12 + 40;
            a.copy_from_slice(&[v.saturating_add(40), v, v.saturating_sub(30), 255]);
            b.copy_from_slice(&[v.saturating_sub(20), v, v.saturating_add(30), 255]);
        }
        let wheels = fit_match(&warm, &cool, 1.0);
        let fields = json!({"colorGrade": {"wheels": wheels}});
        let g = ColorGrade::from_fields(fields.as_object().unwrap(), &|_| None).unwrap();
        let mid = [100.0 / 255.0 - 20.0 / 255.0, 100.0 / 255.0, 130.0 / 255.0];
        let o = g.eval(mid);
        assert!(o[0] > mid[0] && o[2] < mid[2], "cool target warmed towards the reference: {o:?}");
    }
}
