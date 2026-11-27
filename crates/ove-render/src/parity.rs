//! Backend parity by tolerance (design D3): runs the same operations on two backends and measures how far the frames
//! are apart. Floating-point results differ between shader code and Rust, so frames are compared with tolerances,
//! not hashes: the largest channel difference, the mean channel difference and the 99th percentile CIEDE2000.
//!
//! Inputs are one mosaic frame with four test charts (75 % colour bars, a grey ramp, a hue and saturation sweep and
//! an alpha checker) and three reference pictures, drawn procedurally so the harness needs no media files. Cases
//! cover layer drawing, all blend modes, the basic grade, the grade lattice, the LUT presets, every effect and every
//! visual transition.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::backend::{Backend, Tex};
use crate::canvas::{DrawParams, Place, Rgba};
use crate::color::{ColorGrade, LUT_PRESETS, delta_e00, lab_of_linear, lin_table, preset_lut};
use crate::eval::{Blend, Grade};
use crate::transitions::{Role, TRANSITIONS, TransState, split};

/// Largest differences two backends may show on one case.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tolerance {
    /// Largest channel difference, in 8-bit steps.
    pub max: u8,
    /// Mean channel difference, in 8-bit steps.
    pub mean: f64,
    /// 99th percentile CIEDE2000 over the pixels.
    pub de_p99: f64,
}

/// The parity tolerance of the specification: max 2/255, mean 0.25/255, CIEDE2000 p99 1.0.
pub const TOLERANCE: Tolerance = Tolerance { max: 2, mean: 0.25, de_p99: 1.0 };

/// How far two frames are apart.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    pub max: u8,
    pub mean: f64,
    pub de_p99: f64,
}

impl Metrics {
    pub fn within(&self, t: &Tolerance) -> bool {
        self.max <= t.max && self.mean <= t.mean && self.de_p99 <= t.de_p99
    }
}

impl std::fmt::Display for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "max {}/255, mean {:.3}/255, CIEDE2000 p99 {:.3}", self.max, self.mean, self.de_p99)
    }
}

/// Compares two frames of the same size (RGBA; alpha counts in the channel differences, colour in CIEDE2000).
pub fn compare(a: &Rgba, b: &Rgba) -> Metrics {
    assert_eq!((a.w, a.h), (b.w, b.h), "frames to compare differ in size");
    let lin = lin_table();
    let n = (a.w * a.h) as usize;
    let mut max = 0u8;
    let mut sum = 0u64;
    let mut des: Vec<f64> = Vec::new();
    for (p, q) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
        for k in 0..4 {
            let d = p[k].abs_diff(q[k]);
            max = max.max(d);
            sum += d as u64;
        }
        if p[..3] != q[..3] {
            let la = lab_of_linear(lin[p[0] as usize], lin[p[1] as usize], lin[p[2] as usize]);
            let lb = lab_of_linear(lin[q[0] as usize], lin[q[1] as usize], lin[q[2] as usize]);
            des.push(delta_e00(la, lb));
        }
    }
    // Pixels with equal colour have ΔE 0 and sort first.
    let rank = ((0.99 * n as f64).ceil() as usize).clamp(1, n.max(1));
    let zeros = n - des.len();
    let de_p99 = if rank <= zeros {
        0.0
    } else {
        des.sort_by(f64::total_cmp);
        des[rank - zeros - 1]
    };
    Metrics { max, mean: sum as f64 / (n.max(1) * 4) as f64, de_p99 }
}

fn put(img: &mut Rgba, x: u32, y: u32, c: [f32; 4]) {
    let i = ((y * img.w + x) * 4) as usize;
    for (d, v) in img.data[i..i + 4].iter_mut().zip(c) {
        *d = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
}

fn tile(w: u32, h: u32, f: impl Fn(f32, f32) -> [f32; 4]) -> Rgba {
    let mut img = Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            put(&mut img, x, y, f((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32));
        }
    }
    img
}

fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = (h.rem_euclid(1.0)) * 6.0;
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

/// The four test charts at `w`×`h` each.
pub fn charts(w: u32, h: u32) -> Vec<(&'static str, Rgba)> {
    const BARS: [[f32; 3]; 7] = [
        [0.75, 0.75, 0.75],
        [0.75, 0.75, 0.0],
        [0.0, 0.75, 0.75],
        [0.0, 0.75, 0.0],
        [0.75, 0.0, 0.75],
        [0.75, 0.0, 0.0],
        [0.0, 0.0, 0.75],
    ];
    vec![
        (
            "colour bars",
            tile(w, h, |u, _| {
                let c = BARS[((u * 7.0) as usize).min(6)];
                [c[0], c[1], c[2], 1.0]
            }),
        ),
        ("grey ramp", tile(w, h, |u, v| if v < 0.5 { [u, u, u, 1.0] } else { [(u * 16.0).floor() / 15.0; 4] })),
        (
            "hue sweep",
            tile(w, h, |u, v| {
                let c = hsv(u, 1.0 - v * 0.8, 0.35 + 0.65 * (1.0 - (v - 0.5).abs()));
                [c[0], c[1], c[2], 1.0]
            }),
        ),
        (
            "alpha checker",
            tile(w, h, |u, v| {
                let check = ((u * 12.0) as u32 + (v * 8.0) as u32).is_multiple_of(2);
                let a = 0.15 + 0.85 * u;
                if check { [0.9, 0.3, 0.2, a] } else { [0.2, 0.5, 0.9, a * v] }
            }),
        ),
    ]
}

/// Three reference pictures at `w`×`h` each: a landscape, a skin-tone portrait stand-in and a textured interior.
pub fn reference_frames(w: u32, h: u32) -> Vec<(&'static str, Rgba)> {
    let noise = |x: f32, y: f32, seed: u32| crate::effects::noise_fbm(x, y, 4, seed);
    vec![
        ("landscape", crate::effects::sample_image(w, h)),
        (
            "portrait",
            tile(w, h, |u, v| {
                let (x, y) = ((u - 0.5) * w as f32 / h as f32, v - 0.45);
                let face = (x * x / 0.06 + y * y / 0.11).sqrt();
                let n = noise(u * 30.0, v * 30.0, 3) * 0.06;
                if face < 1.0 {
                    let shade = 1.0 - face * 0.35;
                    [0.86 * shade + n, 0.64 * shade + n, 0.52 * shade + n, 1.0]
                } else {
                    let g = 0.18 + 0.3 * v + n;
                    [g * 0.7, g * 0.8, g, 1.0]
                }
            }),
        ),
        (
            "interior",
            tile(w, h, |u, v| {
                let n = noise(u * 12.0, v * 7.0, 9);
                let wood = 0.5 + 0.5 * ((u * 40.0 + n * 6.0).sin());
                let lamp = (-((u - 0.75).powi(2) + (v - 0.3).powi(2)) * 18.0).exp();
                let base = [0.35 + 0.25 * wood, 0.22 + 0.15 * wood, 0.12 + 0.05 * wood];
                [base[0] + lamp * 0.6, base[1] + lamp * 0.55, base[2] + lamp * 0.4, 1.0]
            }),
        ),
    ]
}

/// One frame holding every chart and reference picture as tiles (4 columns, 2 rows; the last tile repeats the
/// landscape mirrored). `w` and `h` are the full frame size.
pub fn mosaic(w: u32, h: u32) -> Rgba {
    let (tw, th) = ((w / 4).max(1), (h / 2).max(1));
    let mut tiles: Vec<Rgba> = charts(tw, th).into_iter().chain(reference_frames(tw, th)).map(|(_, i)| i).collect();
    let mut last = tiles[4].clone();
    for y in 0..th {
        let row = &mut last.data[(y * tw * 4) as usize..((y + 1) * tw * 4) as usize];
        let flipped: Vec<u8> = row.chunks_exact(4).rev().flatten().copied().collect();
        row.copy_from_slice(&flipped);
    }
    tiles.push(last);
    let mut out = Rgba::filled(w, h, [0, 0, 0, 255]);
    for (n, t) in tiles.iter().enumerate() {
        let (ox, oy) = ((n as u32 % 4) * tw, (n as u32 / 4) * th);
        for y in 0..th.min(h - oy) {
            for x in 0..tw.min(w - ox) {
                let s = ((y * tw + x) * 4) as usize;
                let d = (((oy + y) * w + ox + x) * 4) as usize;
                out.data[d..d + 4].copy_from_slice(&t.data[s..s + 4]);
            }
        }
    }
    out
}

/// The layer drawn on top of the mosaic in drawing, blend and transition cases: the mosaic turned upside down,
/// with alpha falling from opaque at the top to a quarter at the bottom.
pub fn top_layer(mosaic: &Rgba) -> Rgba {
    let (w, h) = (mosaic.w, mosaic.h);
    let mut out = Rgba::new(w, h);
    for y in 0..h {
        let a = 1.0 - 0.75 * y as f32 / h.max(1) as f32;
        for x in 0..w {
            let s = (((h - 1 - y) * w + x) * 4) as usize;
            let d = ((y * w + x) * 4) as usize;
            out.data[d..d + 3].copy_from_slice(&mosaic.data[s..s + 3]);
            out.data[d + 3] = (mosaic.data[s + 3] as f32 * a + 0.5) as u8;
        }
    }
    out
}

/// What a case does.
#[derive(Debug, Clone)]
pub enum Op {
    /// Draws the top layer with this placement and these parameters.
    Draw(Place, DrawParams),
    Grade(Grade),
    /// A LUT preset at an intensity.
    Lut(&'static str, f32),
    /// A full `colorGrade` baked into the lattice.
    Lattice(Value),
    /// An effect at its defaults (time 1 s, amount 1).
    Effect(&'static str),
    /// An effect with these parameter values (time 1 s, amount 1).
    EffectWith(&'static str, Value),
    /// A transition of the top layer over the mosaic at progress `q`.
    Transition(&'static str, Role, f32),
    /// An effect mixed back at half strength (partial adjustment layer).
    Mix(&'static str),
}

#[derive(Debug, Clone)]
pub struct Case {
    pub name: String,
    pub op: Op,
}

fn case(name: impl Into<String>, op: Op) -> Case {
    Case { name: name.into(), op }
}

const ALL_BLENDS: [(&str, Blend); 16] = [
    ("normal", Blend::Normal),
    ("multiply", Blend::Multiply),
    ("screen", Blend::Screen),
    ("overlay", Blend::Overlay),
    ("darken", Blend::Darken),
    ("lighten", Blend::Lighten),
    ("color-dodge", Blend::ColorDodge),
    ("color-burn", Blend::ColorBurn),
    ("hard-light", Blend::HardLight),
    ("soft-light", Blend::SoftLight),
    ("difference", Blend::Difference),
    ("exclusion", Blend::Exclusion),
    ("hue", Blend::Hue),
    ("saturation", Blend::Saturation),
    ("color", Blend::Color),
    ("luminosity", Blend::Luminosity),
];

/// Every parity case for frames of `w`×`h`.
pub fn cases(w: u32, h: u32) -> Vec<Case> {
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let aligned = Place { cx, cy, sx: 1.0, sy: 1.0, rotation: 0.0 };
    let k = h as f64 / 1080.0;
    let mut out = vec![
        case("draw aligned at 70 % opacity", Op::Draw(aligned, DrawParams { opacity: 0.7, ..Default::default() })),
        case(
            "draw rotated, scaled, mirrored, cropped, rounded",
            Op::Draw(
                Place { cx: cx + 37.0 * k, cy: cy - 21.0 * k, sx: -0.83, sy: 0.83, rotation: 7.5 },
                DrawParams { opacity: 0.9, blend: Blend::Normal, crop: [5.0, 10.0, 8.0, 3.0], radius: 60.0 * k as f32 },
            ),
        ),
        case("draw enlarged 1.7x", Op::Draw(Place { cx, cy, sx: 1.7, sy: 1.7, rotation: -3.0 }, DrawParams::default())),
    ];
    for (name, mode) in ALL_BLENDS {
        out.push(case(format!("blend {name}"), Op::Draw(aligned, DrawParams { blend: mode, ..Default::default() })));
        out.push(case(
            format!("blend {name}, transformed"),
            Op::Draw(
                Place { cx, cy, sx: 0.9, sy: 0.9, rotation: 4.0 },
                DrawParams { opacity: 0.8, blend: mode, ..Default::default() },
            ),
        ));
    }
    out.push(case(
        "grade adjustments",
        Op::Grade(Grade {
            exposure: 0.3,
            brightness: 0.05,
            contrast: 0.2,
            saturation: 0.3,
            vibrance: 0.2,
            temperature: 0.4,
            tint: -0.2,
            gamma: 0.1,
            highlights: -0.3,
            shadows: 0.2,
        }),
    ));
    out.push(case(
        "grade lattice (wheels, curves, saturation)",
        Op::Lattice(json!({
            "wheels": {"lift": {"r": 0.02, "g": 0.0, "b": -0.03, "y": 0.01}, "gamma": {"r": 0.0, "g": 0.05, "b": 0.0, "y": 0.1},
                       "gain": {"r": 0.1, "g": 0.0, "b": -0.05, "y": 0.0}, "offset": {"r": 0.0, "g": 0.0, "b": 0.02, "y": 0.0}},
            "curves": {"master": [[0, 0], [0.25, 0.2], [0.75, 0.82], [1, 1]], "hueVsSat": [[0, 0.5], [0.3, 0.7], [1, 0.5]]},
            "saturation": 1.2
        })),
    ));
    for (id, _) in LUT_PRESETS {
        out.push(case(format!("lut {id}"), Op::Lut(id, 0.8)));
    }
    for spec in crate::effects::EFFECTS {
        out.push(case(format!("effect {}", spec.id), Op::Effect(spec.id)));
    }
    for t in TRANSITIONS.iter().filter(|t| !t.audio_only) {
        out.push(case(format!("transition {}", t.id), Op::Transition(t.id, Role::In, 0.4)));
    }
    for id in ["push-left", "cross-zoom"] {
        out.push(case(format!("transition {id} (outgoing side)"), Op::Transition(id, Role::OutJoin, 0.4)));
    }
    for (name, v) in [
        ("blue key, edge blur 4, choke 0.3", json!({"keyColor": [0.2, 0.3, 0.75], "edgeBlur": 4, "choke": 0.3})),
        (
            "green key, matte view, levels",
            json!({"matteView": true, "matteBlack": 0.1, "matteWhite": 0.8, "choke": -0.4}),
        ),
    ] {
        out.push(case(format!("effect chroma-key ({name})"), Op::EffectWith("chroma-key", v)));
    }
    out.push(case("adjustment at half strength", Op::Mix("duotone")));
    out
}

/// The shared inputs of a run.
pub struct Inputs {
    pub mosaic: Arc<Rgba>,
    pub top: Arc<Rgba>,
}

impl Inputs {
    pub fn new(w: u32, h: u32) -> Inputs {
        let mosaic = mosaic(w, h);
        let top = top_layer(&mosaic);
        Inputs { mosaic: Arc::new(mosaic), top: Arc::new(top) }
    }
}

/// Runs one case on `backend`; returns the resulting frame.
pub fn run(backend: &mut dyn Backend, case: &Case, inputs: &Inputs) -> Rgba {
    let canvas = backend.upload(&inputs.mosaic);
    let mut extra: Vec<Tex> = Vec::new();
    match &case.op {
        Op::Draw(place, params) => {
            let top = backend.upload(&inputs.top);
            backend.draw(canvas, top, *place, params);
            extra.push(top);
        }
        Op::Grade(g) => backend.grade(canvas, g),
        Op::Lut(id, k) => {
            if let Some(l) = preset_lut(id) {
                backend.lut(canvas, &l, *k);
            }
        }
        Op::Lattice(v) => {
            let mut f = Map::new();
            f.insert("colorGrade".into(), v.clone());
            if let Some(g) = ColorGrade::from_fields(&f, &|_| None) {
                backend.lut(canvas, &Arc::new(g.bake()), 1.0);
            }
        }
        Op::Effect(id) => {
            backend.effect(canvas, id, &Map::new(), 1.0, 1.0);
        }
        Op::EffectWith(id, v) => {
            backend.effect(canvas, id, v.as_object().unwrap_or(&Map::new()), 1.0, 1.0);
        }
        Op::Transition(kind, role, q) => {
            let (w, h) = backend.size(canvas);
            let off = backend.create(w, h, [0, 0, 0, 0]);
            let top = backend.upload(&inputs.top);
            let aligned = Place { cx: w as f64 / 2.0, cy: h as f64 / 2.0, sx: 1.0, sy: 1.0, rotation: 0.0 };
            backend.draw(off, top, aligned, &DrawParams::default());
            let (_, dir) = split(kind, None);
            let st = TransState { kind: kind.to_string(), dir, role: *role, q: *q };
            crate::render::transition_layer(backend, canvas, off, &st, 1.0, Blend::Normal);
            if let Some((id, params, amount)) = st.post_effect() {
                backend.effect(canvas, id, &params, amount, 1.0);
            }
            extra.extend([off, top]);
        }
        Op::Mix(id) => {
            let before = backend.copy(canvas);
            backend.effect(canvas, id, &Map::new(), 1.0, 1.0);
            backend.mix(canvas, before, 0.5);
            extra.push(before);
        }
    }
    let out = backend.read(canvas);
    backend.release(canvas);
    for t in extra {
        backend.release(t);
    }
    out
}

/// One case's result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub name: String,
    pub metrics: Metrics,
    pub pass: bool,
}

/// Runs `cases` on both backends at `w`×`h` and measures each pair against `tolerance`.
pub fn check(
    reference: &mut dyn Backend,
    candidate: &mut dyn Backend,
    cases: &[Case],
    w: u32,
    h: u32,
    tolerance: &Tolerance,
) -> Vec<Row> {
    let inputs = Inputs::new(w, h);
    cases
        .iter()
        .map(|c| {
            let a = run(reference, c, &inputs);
            let b = run(candidate, c, &inputs);
            let metrics = compare(&a, &b);
            Row { name: c.name.clone(), pass: metrics.within(tolerance), metrics }
        })
        .collect()
}

/// The failed rows, one per line with the measured difference; empty when every case passed.
pub fn failures(rows: &[Row]) -> String {
    rows.iter().filter(|r| !r.pass).map(|r| format!("{}: {}", r.name, r.metrics)).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::CpuBackend;

    #[test]
    fn identical_frames_measure_zero() {
        let m = mosaic(64, 36);
        let c = compare(&m, &m.clone());
        assert_eq!((c.max, c.mean, c.de_p99), (0, 0.0, 0.0));
    }

    #[test]
    fn metrics_follow_the_differences() {
        let a = Rgba::filled(10, 10, [100, 100, 100, 255]);
        let mut b = a.clone();
        b.data[0] = 103; // one pixel, one channel, 3 steps
        let m = compare(&a, &b);
        assert_eq!(m.max, 3);
        assert!((m.mean - 3.0 / 400.0).abs() < 1e-9);
        assert_eq!(m.de_p99, 0.0, "1 % of pixels may differ before p99 moves");
        let mut c = a.clone();
        for p in c.data.chunks_exact_mut(4).take(5) {
            p[2] = 140;
        }
        let m = compare(&a, &c);
        assert!(m.de_p99 > 5.0, "{m}");
        assert!(!m.within(&TOLERANCE));
    }

    #[test]
    fn catalogue_is_covered() {
        let cs = cases(64, 36);
        let count = |p: &str| cs.iter().filter(|c| c.name.starts_with(p)).count();
        assert_eq!(count("blend "), 32, "16 modes, aligned and transformed");
        assert_eq!(count("effect "), crate::effects::EFFECTS.len() + 2, "every effect, and two more chroma keys");
        assert_eq!(count("lut "), 9);
        assert_eq!(count("transition "), TRANSITIONS.len() - 1 + 2);
    }

    /// The CPU backend with one deliberately wrong operation: the radial wipe runs a little ahead.
    struct Skewed(CpuBackend);

    impl Backend for Skewed {
        fn name(&self) -> &'static str {
            "skewed"
        }
        fn create(&mut self, w: u32, h: u32, fill: [u8; 4]) -> Tex {
            self.0.create(w, h, fill)
        }
        fn upload(&mut self, img: &Arc<Rgba>) -> Tex {
            self.0.upload(img)
        }
        fn read(&mut self, t: Tex) -> Rgba {
            self.0.read(t)
        }
        fn release(&mut self, t: Tex) {
            self.0.release(t)
        }
        fn size(&self, t: Tex) -> (u32, u32) {
            self.0.size(t)
        }
        fn cpu(&mut self, t: Tex, f: &mut dyn FnMut(&mut Rgba)) {
            self.0.cpu(t, f)
        }
        fn transition_mask(&mut self, t: Tex, st: &TransState) {
            let mut st = st.clone();
            if st.kind == "radial-wipe" {
                st.q += 0.05;
            }
            self.0.transition_mask(t, &st)
        }
    }

    #[test]
    fn a_wrong_transition_fails_and_is_named() {
        let rows = check(&mut CpuBackend::new(), &mut Skewed(CpuBackend::new()), &cases(96, 54), 96, 54, &TOLERANCE);
        let report = failures(&rows);
        assert!(report.starts_with("transition radial-wipe: max "), "{report}");
        assert_eq!(report.lines().count(), 1, "only the changed case fails: {report}");
    }

    #[test]
    fn the_cpu_backend_matches_itself() {
        let rows = check(&mut CpuBackend::new(), &mut CpuBackend::new(), &cases(96, 54), 96, 54, &TOLERANCE);
        assert!(rows.iter().all(|r| r.metrics.max == 0), "{}", failures(&rows));
    }
}
