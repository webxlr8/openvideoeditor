//! `chroma-key` (design D6): removes a key colour from a layer, writing alpha.
//!
//! Distance is measured in the BT.709 CbCr plane with chroma divided by luma, so a dark or unevenly lit screen
//! keys like a bright one, and relative to the key's own chroma, so a desaturated screen still keys. The matte is a
//! smooth step from `tolerance` to `tolerance + softness`, then edge blur (an average over a disc of `edgeBlur`
//! pixels), choke (positive shrinks the matte, negative grows it) and the black and white matte levels. Spill
//! suppression pulls the key channel toward the mean of the other two channels near the key colour and restores
//! the luma it took. `matteView` shows the matte as grey instead of the result.
//!
//! The GPU backend runs the same arithmetic in WGSL (`ove-gpu` compositor), checked by the parity tests.

use super::util::*;

/// Parameter order (contract): keyColor, tolerance, softness, spill, choke, matteBlack, matteWhite, edgeBlur,
/// matteView.
fn ycc(c: V3) -> V3 {
    let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    [y, (c[2] - y) / 1.8556, (c[0] - y) / 1.5748]
}

/// Chroma divided by luma (luma floored at 0.1).
fn norm_chroma(c: V3) -> (f32, f32) {
    let [y, cb, cr] = ycc(c);
    let l = y.max(0.1);
    (cb / l, cr / l)
}

/// Distance of `c` from the key in units of the key's normalised chroma.
fn distance(c: V3, key: (f32, f32), key_len: f32) -> f32 {
    let (cb, cr) = norm_chroma(c);
    (cb - key.0).hypot(cr - key.1) / key_len
}

struct Key {
    key: (f32, f32),
    len: f32,
    tol: f32,
    soft: f32,
}

impl Key {
    fn new(p: &Params) -> Key {
        let key = norm_chroma(p.c("keyColor"));
        Key { key, len: key.0.hypot(key.1).max(0.1), tol: p.n("tolerance"), soft: p.n("softness").max(0.001) }
    }

    /// The raw matte (0 keyed out, 1 kept) of colour `c`, and its distance from the key.
    fn matte(&self, c: V3) -> (f32, f32) {
        let d = distance(c, self.key, self.len);
        (smoothstep(self.tol, self.tol + self.soft, d), d)
    }
}

/// A copy of `doc` with every chroma key showing its matte (`look --view matte`).
pub fn matte_view(doc: &ove_core::model::Document) -> ove_core::model::Document {
    let mut out = doc.clone();
    for (_, item) in doc.project.items.iter() {
        let Some(list) = item.fields.get("effects").and_then(serde_json::Value::as_array) else { continue };
        if !list.iter().any(|e| e.get("type").and_then(serde_json::Value::as_str) == Some("chroma-key")) {
            continue;
        }
        let mut list = list.clone();
        for e in list.iter_mut().filter(|e| e.get("type").and_then(serde_json::Value::as_str) == Some("chroma-key")) {
            if let Some(o) = e.as_object_mut() {
                match o.get_mut("params").and_then(serde_json::Value::as_object_mut) {
                    Some(p) => {
                        p.insert("matteView".into(), serde_json::json!(true));
                    }
                    None => {
                        o.insert("matteView".into(), serde_json::json!(true));
                    }
                }
            }
        }
        let mut it = (**item).clone();
        it.fields.insert("effects".into(), serde_json::Value::Array(list));
        out.project.items.insert(it.id.clone(), std::sync::Arc::new(it));
    }
    out
}

pub fn chroma_key(c: &Ctx, u: f32, v: f32) -> V4 {
    let p = c.p;
    let k = Key::new(p);
    let s = c.sample(u, v);
    let (m0, d) = k.matte(s);
    // Edge blur: the matte averaged over a disc of `edgeBlur` pixels (golden-angle taps, as the disc blur).
    let r = p.n("edgeBlur");
    let mut m = m0;
    if r >= 0.5 {
        let taps = 16;
        let mut acc = 0.0;
        for i in 0..taps {
            let a = i as f32 * 2.399_963;
            let rr = r * ((i as f32 + 0.5) / taps as f32).sqrt();
            acc += k.matte(c.sample(u + a.cos() * rr / c.w, v + a.sin() * rr / c.h)).0;
        }
        m = acc / taps as f32;
    }
    // Choke: a threshold shift of the (blurred) matte; positive shrinks, negative grows.
    let ch = p.n("choke");
    m = if ch > 0.0 { ((m - ch) / (1.0 - ch).max(1e-3)).clamp(0.0, 1.0) } else { (m / (1.0 + ch).max(1e-3)).min(1.0) };
    // Matte levels.
    let (lo, hi) = (p.n("matteBlack"), p.n("matteWhite"));
    m = ((m - lo) / (hi - lo).max(1e-3)).clamp(0.0, 1.0);
    if p.n("matteView") >= 0.5 {
        return [m, m, m, 1.0];
    }
    // Spill suppression near the key colour: the key channel toward the mean of the other two, luma restored.
    let key = p.c("keyColor");
    let ch_key = if key[2] > key[1] { 2 } else { 1 };
    let (o1, o2) = if ch_key == 1 { (0, 2) } else { (0, 1) };
    let near = 1.0 - smoothstep(k.tol, k.tol + k.soft * 2.0 + 1.0, d);
    let limit = (s[o1] + s[o2]) * 0.5;
    let spill = (s[ch_key] - limit).max(0.0) * p.n("spill") * near;
    let mut out = s;
    out[ch_key] -= spill;
    let dl = spill * [0.2126, 0.7152, 0.0722][ch_key];
    for x in &mut out {
        *x = (*x + dl).clamp(0.0, 1.0);
    }
    let a = c.texel(u, v)[3] * m;
    [out[0], out[1], out[2], a]
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value, json};

    use crate::canvas::Rgba;
    use crate::effects::apply;

    /// A test plate: a foreground (skin, red, grey and a fourth patch of the screen's complementary colour) with a
    /// soft-edged round matte over a shaded `screen`, with screen light spilling onto the foreground near its edge.
    /// Returns the plate, the true matte and the true foreground colours.
    pub fn plate(w: u32, h: u32, screen: [f32; 3]) -> (Rgba, Vec<f32>, Vec<[f32; 3]>) {
        let fourth = if screen[2] > screen[1] { [0.85, 0.72, 0.2] } else { [0.2, 0.3, 0.75] };
        let mut img = Rgba::new(w, h);
        let (mut alpha, mut fg) = (Vec::new(), Vec::new());
        for y in 0..h {
            for x in 0..w {
                let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                let (dx, dy) = ((u - 0.5) * w as f32 / h as f32, v - 0.5);
                let r = (dx * dx + dy * dy).sqrt();
                let a = ((0.32 - r) / 0.02).clamp(0.0, 1.0);
                let f = if dx < -0.1 {
                    [0.86, 0.64, 0.52]
                } else if dy < -0.1 {
                    [0.8, 0.15, 0.2]
                } else if dy > 0.1 {
                    fourth
                } else {
                    [0.55, 0.55, 0.55]
                };
                // Uneven screen lighting, darker towards the bottom right.
                let shade = 1.0 - 0.35 * (u * 0.6 + v * 0.4);
                let bg = screen.map(|c| c * shade);
                let spill = 0.12 * (1.0 - a) * a * 4.0;
                let mut c = [0.0; 3];
                for k in 0..3 {
                    c[k] = f[k] * a + bg[k] * (1.0 - a) + spill * screen[k];
                }
                let i = ((y * w + x) * 4) as usize;
                for (d, v) in img.data[i..i + 3].iter_mut().zip(c) {
                    *d = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
                img.data[i + 3] = 255;
                alpha.push(a);
                fg.push(f);
            }
        }
        (img, alpha, fg)
    }

    fn key(img: &Rgba, params: Value) -> Rgba {
        let mut out = img.clone();
        let m: Map<String, Value> = params.as_object().cloned().unwrap_or_default();
        assert!(apply(&mut out, "chroma-key", &m, 1.0, 0.0));
        out
    }

    /// (mean matte error, edge spill, interior mean CIEDE2000) of a keyed plate against the truth.
    fn score(out: &Rgba, alpha: &[f32], fg: &[[f32; 3]], key_ch: usize) -> (f32, f32, f64) {
        let (mut sad, mut spill, mut edge_n, mut de, mut de_n) = (0.0, 0.0, 0, 0.0, 0);
        for (i, p) in out.data.chunks_exact(4).enumerate() {
            let a = alpha[i];
            sad += (p[3] as f32 / 255.0 - a).abs();
            let c = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
            if a > 0.05 && a < 0.95 {
                let others = (0..3).filter(|k| *k != key_ch).map(|k| c[k]).fold(0.0f32, f32::max);
                spill += (c[key_ch] - others).max(0.0);
                edge_n += 1;
            }
            if a >= 1.0 {
                let t = fg[i].map(|x| (x * 255.0 + 0.5) as u8);
                de += crate::color::delta_e00(crate::color::lab_u8([p[0], p[1], p[2]]), crate::color::lab_u8(t));
                de_n += 1;
            }
        }
        (sad / alpha.len() as f32, spill / edge_n.max(1) as f32, de / de_n.max(1) as f64)
    }

    #[test]
    fn green_and_blue_screens_key_to_the_true_matte() {
        for (screen, key_ch, label) in [([0.12, 0.72, 0.22], 1, "green"), ([0.1, 0.22, 0.78], 2, "blue")] {
            let (plate, alpha, fg) = plate(192, 108, screen);
            let out = key(&plate, json!({"keyColor": screen, "edgeBlur": 1}));
            let (sad, spill, de) = score(&out, &alpha, &fg, key_ch);
            eprintln!("{label}: mean matte error {sad:.4}, edge spill {spill:.4}, interior dE00 {de:.3}");
            assert!(sad < 0.02, "{label} matte error {sad}");
            assert!(spill < 0.02, "{label} edge spill {spill}");
            assert!(de < 1.0, "{label} interior colour {de}");
        }
    }

    #[test]
    fn a_key_colour_not_in_the_frame_leaves_the_layer_unchanged() {
        // Skin, red, blue, grey and a dark-to-light ramp of each: no green anywhere.
        let mut img = Rgba::new(64, 36);
        for (i, p) in img.data.chunks_exact_mut(4).enumerate() {
            let (x, y) = ((i % 64) as f32 / 63.0, (i / 64) as f32 / 35.0);
            let c = [[0.86, 0.64, 0.52], [0.8, 0.15, 0.2], [0.2, 0.3, 0.75], [0.55, 0.55, 0.55]][(x * 3.99) as usize];
            for k in 0..3 {
                p[k] = (c[k] * (0.2 + 0.8 * y) * 255.0 + 0.5) as u8;
            }
            p[3] = 255;
        }
        let out = key(&img, json!({"keyColor": [0.1, 0.8, 0.2]}));
        assert_eq!(out.data, img.data);
    }

    #[test]
    fn matte_view_is_grey_and_opaque() {
        let (plate, _, _) = plate(64, 36, [0.12, 0.72, 0.22]);
        let out = key(&plate, json!({"keyColor": [0.12, 0.72, 0.22], "matteView": true}));
        assert!(out.data.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255));
        assert_eq!(out.px(1, 1)[0], 0, "screen is black in the matte");
        assert_eq!(out.px(32, 18)[0], 255, "foreground is white");
    }

    #[test]
    fn a_dark_screen_still_keys() {
        let (plate, alpha, fg) = plate(96, 54, [0.04, 0.24, 0.07]);
        let out = key(&plate, json!({"keyColor": [0.04, 0.24, 0.07]}));
        let (sad, _, _) = score(&out, &alpha, &fg, 1);
        assert!(sad < 0.03, "dark screen matte error {sad}");
    }
}
