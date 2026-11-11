//! Text animations (`item.textAnimation`): an entrance and an optional exit built from the presets in
//! `docs/spec/presets.md` — basics (`none`, `fade`, `blur`), reveal (`typewriter`, `glitch`), move (`rise`, `drop`,
//! `slide-left`, `slide-right`, `settle-up`, `settle-down`) and scale (`zoom-in`, `zoom-out`, `pop`, `focus-pop`,
//! `rotate-in`).
//!
//! ```json
//! "textAnimation": { "preset": "rise", "duration": 0.6, "delay": 0, "easing": "ease-out",
//!                    "out": { "preset": "fade", "duration": 0.4 } }
//! ```
//! `duration`/`delay` are seconds (`speed` divides the duration). The exit plays the preset backwards and ends
//! `out.delay` seconds before the item ends. Pure functions of item-relative time, so preview and export agree.

use serde_json::{Map, Value};

use crate::canvas::Rgba;

/// Every preset id, in picker order.
pub const PRESETS: &[&str] = &[
    "none",
    "fade",
    "blur",
    "typewriter",
    "glitch",
    "rise",
    "drop",
    "slide-left",
    "slide-right",
    "settle-up",
    "settle-down",
    "zoom-in",
    "zoom-out",
    "pop",
    "focus-pop",
    "rotate-in",
];

/// One phase (entrance or exit).
#[derive(Debug, Clone, PartialEq)]
pub struct Phase {
    pub preset: String,
    /// Seconds.
    pub duration: f64,
    /// Seconds after the start (entrance) or before the end (exit).
    pub delay: f64,
    pub easing: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextAnimation {
    pub enter: Option<Phase>,
    pub exit: Option<Phase>,
}

fn phase(v: &Value, default_easing: &str) -> Option<Phase> {
    let (preset, o) = match v {
        Value::String(s) => (s.as_str(), None),
        Value::Object(o) => (o.get("preset").and_then(Value::as_str)?, Some(o)),
        _ => return None,
    };
    if preset == "none" || !PRESETS.contains(&preset) {
        return None;
    }
    let n = |k: &str| o.and_then(|o| o.get(k)).and_then(Value::as_f64);
    let speed = n("speed").filter(|s| *s > 0.0).unwrap_or(1.0);
    let default_duration = if preset == "typewriter" { 1.2 } else { 0.6 };
    Some(Phase {
        preset: preset.to_string(),
        duration: (n("duration").unwrap_or(default_duration) / speed).max(0.01),
        delay: n("delay").unwrap_or(0.0).max(0.0),
        easing: o.and_then(|o| o.get("easing")).and_then(Value::as_str).unwrap_or(default_easing).to_string(),
    })
}

impl TextAnimation {
    pub fn from_fields(f: &Map<String, Value>) -> Option<TextAnimation> {
        let v = f.get("textAnimation")?;
        let enter = phase(v, "ease-out");
        let exit = v.get("out").and_then(|o| phase(o, "ease-in"));
        (enter.is_some() || exit.is_some()).then_some(TextAnimation { enter, exit })
    }

    /// State at `rel` seconds into an item lasting `dur` seconds.
    pub fn state(&self, rel: f64, dur: f64) -> AnimState {
        let mut s = AnimState::default();
        if let Some(p) = &self.enter {
            let t = ((rel - p.delay) / p.duration).clamp(0.0, 1.0);
            s.combine(&preset_state(&p.preset, ease(&p.easing, t), t, rel));
        }
        if let Some(p) = &self.exit {
            let start = dur - p.delay - p.duration;
            let t = 1.0 - ((rel - start) / p.duration).clamp(0.0, 1.0);
            s.combine(&preset_state(&p.preset, ease(&p.easing, t), t, rel));
        }
        s
    }
}

/// How a text layer looks at one moment. Lengths are in units of the font size.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimState {
    pub opacity: f64,
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    /// Degrees.
    pub rotation: f64,
    pub blur: f64,
    /// Fraction of characters shown (typewriter).
    pub reveal: f64,
    /// Glitch strength 0…1 and a per-frame seed.
    pub glitch: f64,
    pub seed: u64,
}

impl Default for AnimState {
    fn default() -> Self {
        AnimState { opacity: 1.0, dx: 0.0, dy: 0.0, scale: 1.0, rotation: 0.0, blur: 0.0, reveal: 1.0, glitch: 0.0, seed: 0 }
    }
}

impl AnimState {
    fn combine(&mut self, o: &AnimState) {
        self.opacity *= o.opacity;
        self.dx += o.dx;
        self.dy += o.dy;
        self.scale *= o.scale;
        self.rotation += o.rotation;
        self.blur += o.blur;
        self.reveal = self.reveal.min(o.reveal);
        if o.glitch > self.glitch {
            self.glitch = o.glitch;
            self.seed = o.seed;
        }
    }

    pub fn is_identity(&self) -> bool {
        *self == AnimState::default()
    }
}

/// Easing by name: the timeline names plus `back` (overshoot) and `elastic`.
pub fn ease(name: &str, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match name {
        "linear" => t,
        "ease-in" => t * t * t,
        "ease-in-out" => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        "back" => back(t),
        "elastic" => {
            if t <= 0.0 || t >= 1.0 {
                t
            } else {
                2f64.powf(-10.0 * t) * ((t * 10.0 - 0.75) * std::f64::consts::TAU / 3.0).sin() + 1.0
            }
        }
        _ => 1.0 - (1.0 - t).powi(3),
    }
}

fn back(t: f64) -> f64 {
    let c1 = 1.70158;
    1.0 + (c1 + 1.0) * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// Preset at eased progress `p` (0 hidden → 1 settled); `t` is the linear progress, `rel` the item time.
fn preset_state(preset: &str, p: f64, t: f64, rel: f64) -> AnimState {
    let d = AnimState::default();
    let fade = p.clamp(0.0, 1.0);
    match preset {
        "fade" => AnimState { opacity: fade, ..d },
        "blur" => AnimState { opacity: fade, blur: (1.0 - p) * 0.3, ..d },
        "typewriter" => AnimState { reveal: t, ..d },
        "glitch" => {
            // Flickers on with jitter; the seed changes every 1/24 s so frames differ but repeat exactly.
            let seed = (rel * 24.0).floor().max(0.0) as u64;
            let on = t >= 1.0 || t > 0.6 || hash01(seed ^ 0x9e37) < t * 1.4;
            AnimState { opacity: if on { 1.0 } else { 0.0 }, glitch: 1.0 - t, seed, dx: (hash01(seed) - 0.5) * 0.12 * (1.0 - t), ..d }
        }
        "rise" => AnimState { opacity: fade, dy: (1.0 - p) * 0.6, ..d },
        "drop" => AnimState { opacity: fade, dy: -(1.0 - p) * 0.6, ..d },
        "slide-left" => AnimState { opacity: fade, dx: (1.0 - p) * 1.5, ..d },
        "slide-right" => AnimState { opacity: fade, dx: -(1.0 - p) * 1.5, ..d },
        "settle-up" => AnimState { opacity: (t * 2.0).min(1.0), dy: (1.0 - back(t)) * 0.35, ..d },
        "settle-down" => AnimState { opacity: (t * 2.0).min(1.0), dy: -(1.0 - back(t)) * 0.35, ..d },
        "zoom-in" => AnimState { opacity: fade, scale: 0.4 + 0.6 * p, ..d },
        "zoom-out" => AnimState { opacity: fade, scale: 1.6 - 0.6 * p, ..d },
        "pop" => AnimState { opacity: (t * 3.0).min(1.0), scale: back(t).max(0.0), ..d },
        "focus-pop" => AnimState { opacity: fade, scale: 1.35 - 0.35 * p, blur: (1.0 - p) * 0.25, ..d },
        "rotate-in" => AnimState { opacity: fade, scale: 0.6 + 0.4 * p, rotation: -(1.0 - p) * 60.0, ..d },
        _ => d,
    }
}

fn hash01(mut x: u64) -> f64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ceb9fe1a85ec53);
    x ^= x >> 33;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Gaussian-like blur (three box passes) of a straight-alpha image, grown by the radius so nothing is clipped.
pub fn blur(src: &Rgba, radius: f32) -> Rgba {
    let r = radius.round() as i32;
    if r < 1 {
        return src.clone();
    }
    let pad = (r * 3) as u32;
    let (w, h) = (src.w + 2 * pad, src.h + 2 * pad);
    // Premultiplied float planes.
    let mut buf = vec![[0f32; 4]; (w * h) as usize];
    for y in 0..src.h {
        for x in 0..src.w {
            let i = ((y * src.w + x) * 4) as usize;
            let a = src.data[i + 3] as f32 / 255.0;
            buf[((y + pad) * w + x + pad) as usize] =
                [src.data[i] as f32 * a, src.data[i + 1] as f32 * a, src.data[i + 2] as f32 * a, a * 255.0];
        }
    }
    let box_r = (r / 2).max(1) as usize;
    for _ in 0..3 {
        box_pass(&mut buf, w as usize, h as usize, box_r, true);
        box_pass(&mut buf, w as usize, h as usize, box_r, false);
    }
    let mut out = Rgba::new(w, h);
    for (i, p) in buf.iter().enumerate() {
        let a = p[3] / 255.0;
        if a <= 0.0 {
            continue;
        }
        for c in 0..3 {
            out.data[i * 4 + c] = (p[c] / a).clamp(0.0, 255.0) as u8;
        }
        out.data[i * 4 + 3] = p[3].clamp(0.0, 255.0) as u8;
    }
    out
}

fn box_pass(buf: &mut [[f32; 4]], w: usize, h: usize, r: usize, horizontal: bool) {
    let (n, lines) = if horizontal { (w, h) } else { (h, w) };
    let idx = |line: usize, k: usize| if horizontal { line * w + k } else { k * w + line };
    let norm = 1.0 / (2 * r + 1) as f32;
    let mut tmp = vec![[0f32; 4]; n];
    for line in 0..lines {
        let mut acc = [0f32; 4];
        for k in 0..=r.min(n - 1) {
            let p = buf[idx(line, k)];
            for c in 0..4 {
                acc[c] += p[c];
            }
        }
        for (k, slot) in tmp.iter_mut().enumerate() {
            for c in 0..4 {
                slot[c] = acc[c] * norm;
            }
            if k + r + 1 < n {
                let p = buf[idx(line, k + r + 1)];
                for c in 0..4 {
                    acc[c] += p[c];
                }
            }
            if k >= r {
                let p = buf[idx(line, k - r)];
                for c in 0..4 {
                    acc[c] -= p[c];
                }
            }
        }
        for (k, v) in tmp.iter().enumerate() {
            buf[idx(line, k)] = *v;
        }
    }
}

/// Glitch: red and blue channels pulled apart horizontally and a few bands shifted, by `amount` pixels.
pub fn glitch(src: &Rgba, amount: f32, seed: u64) -> Rgba {
    let shift = amount.round() as i32;
    if shift < 1 {
        return src.clone();
    }
    let pad = shift as u32 * 2;
    let (w, h) = (src.w + 2 * pad, src.h);
    let at = |x: i32, y: u32| -> [u8; 4] {
        let sx = x - pad as i32;
        if sx < 0 || sx >= src.w as i32 {
            return [0; 4];
        }
        src.px(sx as u32, y)
    };
    // Bands: a handful of horizontal slices get an extra offset.
    let bands: Vec<(u32, u32, i32)> = (0..4)
        .map(|k| {
            let y0 = (hash01(seed * 7 + k) * h as f64) as u32;
            let hh = ((hash01(seed * 13 + k) * 0.12 + 0.03) * h as f64) as u32;
            let off = ((hash01(seed * 31 + k) - 0.5) * 2.0 * shift as f64) as i32;
            (y0, y0 + hh.max(1), off)
        })
        .collect();
    let mut out = Rgba::new(w, h);
    for y in 0..h {
        let band = bands.iter().find(|b| y >= b.0 && y < b.1).map_or(0, |b| b.2);
        for x in 0..w as i32 {
            let base = x - band;
            let r = at(base - shift, y);
            let g = at(base, y);
            let b = at(base + shift, y);
            let a = r[3].max(g[3]).max(b[3]);
            if a == 0 {
                continue;
            }
            let i = ((y * w + x as u32) * 4) as usize;
            out.data[i] = r[0].max(((g[0] as u16 * g[3] as u16) / 255) as u8);
            out.data[i + 1] = g[1];
            out.data[i + 2] = b[2].max(((g[2] as u16 * g[3] as u16) / 255) as u8);
            out.data[i + 3] = a;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn anim(v: Value) -> TextAnimation {
        TextAnimation::from_fields(json!({ "textAnimation": v }).as_object().unwrap()).unwrap()
    }

    #[test]
    fn parses_presets_with_defaults() {
        let a = anim(json!({"preset": "rise", "duration": 1.0, "delay": 0.5, "out": "fade"}));
        assert_eq!(a.enter.as_ref().unwrap().duration, 1.0);
        assert_eq!(a.exit.as_ref().unwrap().preset, "fade");
        assert_eq!(a.exit.as_ref().unwrap().easing, "ease-in");
        assert!(TextAnimation::from_fields(json!({"textAnimation": {"preset": "none"}}).as_object().unwrap()).is_none());
        assert!(TextAnimation::from_fields(json!({"textAnimation": {"preset": "bogus"}}).as_object().unwrap()).is_none());
        let fast = anim(json!({"preset": "fade", "duration": 1.0, "speed": 2}));
        assert_eq!(fast.enter.unwrap().duration, 0.5);
    }

    #[test]
    fn every_preset_starts_changed_and_ends_settled() {
        for &p in &PRESETS[1..] {
            let a = anim(json!({"preset": p, "duration": 1.0}));
            let start = a.state(0.0, 10.0);
            let end = a.state(1.0, 10.0);
            assert!(!start.is_identity(), "{p} changes the text at the start: {start:?}");
            assert!(end.is_identity() || p == "glitch" && end.glitch == 0.0 && end.opacity == 1.0, "{p} settles: {end:?}");
            // Determinism: same time → same state.
            assert_eq!(a.state(0.37, 10.0), a.state(0.37, 10.0));
        }
    }

    #[test]
    fn delay_and_exit() {
        let a = anim(json!({"preset": "fade", "duration": 1.0, "delay": 1.0, "out": {"preset": "rise", "duration": 1.0, "delay": 0.5}}));
        assert_eq!(a.state(0.5, 10.0).opacity, 0.0, "waits for the delay");
        assert!(a.state(5.0, 10.0).is_identity());
        let leaving = a.state(9.0, 10.0);
        assert!(leaving.opacity < 1.0 && leaving.dy > 0.0, "{leaving:?}");
        assert_eq!(a.state(9.6, 10.0).opacity, 0.0, "gone before the end");
        let tw = anim(json!({"preset": "typewriter", "duration": 2.0, "easing": "linear"}));
        assert!((tw.state(1.0, 5.0).reveal - 0.5).abs() < 1e-9);
    }

    #[test]
    fn blur_and_glitch_keep_ink() {
        let mut img = Rgba::new(20, 10);
        for y in 3..7 {
            for x in 5..15 {
                img.data[((y * 20 + x) * 4) as usize..][..4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let b = blur(&img, 4.0);
        assert!(b.w > img.w && b.h > img.h);
        let ink = |i: &Rgba| i.data.chunks(4).map(|p| p[3] as u64).sum::<u64>();
        let (a, c) = (ink(&img), ink(&b));
        assert!((c as f64 / a as f64 - 1.0).abs() < 0.1, "blur preserves coverage {a} vs {c}");
        let g = glitch(&img, 3.0, 7);
        assert_eq!(g.h, img.h);
        assert!(g.data.chunks(4).any(|p| p[3] > 0 && p[0] != p[2]), "channels split");
        assert_eq!(glitch(&img, 3.0, 7), g, "deterministic");
    }
}
