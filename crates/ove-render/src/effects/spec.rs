//! The effect catalogue: identifiers and parameter contracts (docs/spec/presets.md, Shader effects). The algorithms
//! are our own (see the per-group modules); only names, ranges and defaults follow the contract.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ParamKind {
    /// RGB in 0..1.
    Color,
    /// 0..1 style value with a range.
    Percent {
        min: f32,
        max: f32,
    },
    Number {
        min: f32,
        max: f32,
    },
    /// Normalised frame position, x right, y down, 0..1.
    Point,
    /// On/off (stored as 0 or 1).
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ParamDefault {
    Num(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamSpec {
    pub name: &'static str,
    pub kind: ParamKind,
    pub default: ParamDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Stylize,
    Distort,
    Background,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub group: Group,
    pub params: &'static [ParamSpec],
    /// Changes over time (uses the item-relative time).
    pub animated: bool,
}

const fn c(name: &'static str, r: f32, g: f32, b: f32) -> ParamSpec {
    ParamSpec { name, kind: ParamKind::Color, default: ParamDefault::Vec3([r, g, b]) }
}
const fn pc(name: &'static str, min: f32, max: f32, d: f32) -> ParamSpec {
    ParamSpec { name, kind: ParamKind::Percent { min, max }, default: ParamDefault::Num(d) }
}
const fn n(name: &'static str, min: f32, max: f32, d: f32) -> ParamSpec {
    ParamSpec { name, kind: ParamKind::Number { min, max }, default: ParamDefault::Num(d) }
}
const fn pt(name: &'static str, x: f32, y: f32) -> ParamSpec {
    ParamSpec { name, kind: ParamKind::Point, default: ParamDefault::Vec2([x, y]) }
}
const fn tg(name: &'static str, d: f32) -> ParamSpec {
    ParamSpec { name, kind: ParamKind::Toggle, default: ParamDefault::Num(d) }
}
const fn e(
    id: &'static str,
    label: &'static str,
    group: Group,
    animated: bool,
    params: &'static [ParamSpec],
) -> EffectSpec {
    EffectSpec { id, label, group, params, animated }
}

use Group::{Background as B, Distort as D, Stylize as S};

#[rustfmt::skip]
pub const EFFECTS: &[EffectSpec] = &[
    // stylize
    e("duotone", "Duotone", S, false, &[c("shadows", 0.09, 0.11, 0.36), c("highlights", 1.0, 0.55, 0.42), pc("balance", 0.0, 1.0, 0.5)]),
    e("tritone", "Tritone", S, false, &[c("shadows", 0.11, 0.122, 0.302), c("midtones", 0.831, 0.337, 0.416), c("highlights", 1.0, 0.914, 0.722), pc("midpoint", 0.0, 1.0, 0.5)]),
    e("gradient-map", "Gradient map", S, true, &[c("shadows", 0.102, 0.043, 0.18), c("midtones", 0.914, 0.271, 0.376), c("highlights", 0.976, 0.929, 0.412), n("speed", -2.0, 2.0, 0.15), n("contrast", 0.0, 3.0, 1.0), pc("blackPoint", 0.0, 1.0, 0.0), pc("whitePoint", 0.0, 1.0, 1.0), pc("strength", 0.0, 1.0, 1.0)]),
    e("posterize", "Posterize", S, false, &[n("levels", 2.0, 20.0, 5.0)]),
    e("solarize", "Solarize", S, false, &[pc("threshold", 0.0, 1.0, 0.5), pc("strength", 0.0, 1.0, 1.0)]),
    e("halftone", "Halftone", S, false, &[n("frequency", 10.0, 300.0, 60.0), n("angle", 0.0, 360.0, 45.0), n("cmyk", 0.0, 1.0, 0.0), n("misprint", 0.0, 0.01, 0.0)]),
    e("dither", "Dither", S, false, &[n("pixelSize", 1.0, 20.0, 4.0), pc("threshold", 0.0, 1.0, 0.5), pc("spread", 0.0, 1.0, 1.0), c("dark", 0.0, 0.0, 0.0), c("light", 1.0, 1.0, 1.0), tg("sourceColors", 0.0)]),
    e("engraving", "Engraving", S, false, &[n("frequency", 20.0, 300.0, 90.0), n("angle", 0.0, 360.0, 8.0), tg("crossHatch", 1.0), n("relief", 0.0, 2.0, 0.6), pc("waviness", 0.0, 1.0, 0.35), n("contrast", 0.25, 3.0, 1.15), c("inkColor", 0.102, 0.078, 0.063), c("paperColor", 0.957, 0.933, 0.886)]),
    e("watercolor", "Watercolor", S, false, &[n("brushSize", 1.0, 12.0, 4.0), n("bleed", 0.0, 3.0, 1.0), pc("paper", 0.0, 1.0, 0.35), c("paperColor", 0.984, 0.969, 0.925), pc("strength", 0.0, 1.0, 1.0)]),
    e("pixelate", "Pixelate", S, false, &[n("cells", 1.0, 200.0, 50.0), pc("gap", 0.0, 0.95, 0.0), pc("roundness", 0.0, 1.0, 0.0)]),
    e("chromatic-aberration", "Chromatic aberration", S, false, &[pc("strength", 0.0, 1.0, 0.2), n("angle", 0.0, 360.0, 0.0), n("redOffset", -2.0, 2.0, -1.0), n("greenOffset", -2.0, 2.0, 0.0), n("blueOffset", -2.0, 2.0, 1.0)]),
    e("glitch", "Glitch", S, true, &[pc("intensity", 0.0, 1.0, 0.5), n("speed", 0.0, 5.0, 1.0), n("rgbShift", 0.0, 20.0, 5.0), n("blockDensity", 2.0, 50.0, 10.0), pc("colorBarIntensity", 0.0, 1.0, 0.2), pc("mirrorAmount", 0.0, 1.0, 0.3), pc("scanlineIntensity", 0.0, 1.0, 0.2), n("seed", 0.0, 100.0, 3.8)]),
    e("vhs", "VHS", S, true, &[n("wobble", 0.0, 5.0, 1.0), pc("scanlineNoise", 0.0, 1.0, 0.6), n("smear", -2.0, 2.0, 0.2), n("speed", 0.0, 3.0, 1.0)]),
    e("crt-screen", "CRT screen", S, false, &[n("pixelSize", 8.0, 128.0, 128.0), n("colorShift", 0.0, 10.0, 1.0), pc("scanlineIntensity", 0.0, 1.0, 0.3), n("scanlineFrequency", 100.0, 800.0, 200.0), n("brightness", 0.5, 2.0, 1.0), pc("vignetteIntensity", 0.0, 1.0, 1.0), pc("vignetteRadius", 0.0, 1.0, 0.5)]),
    e("light-leak", "Light leak", S, true, &[pt("position", 0.95, 0.6), n("spread", 0.1, 1.5, 0.55), pc("intensity", 0.0, 1.0, 0.15), pc("streaks", 0.0, 1.0, 0.6), c("hotColor", 1.0, 0.953, 0.769), c("midColor", 1.0, 0.478, 0.184), c("fringeColor", 0.651, 0.239, 0.561), n("speed", 0.0, 4.0, 1.0)]),
    e("lens-flare", "Lens flare", S, true, &[pt("position", 0.3, 0.7), pc("intensity", 0.0, 2.0, 0.5), pc("ghosts", 0.0, 1.0, 0.4), pc("halo", 0.0, 1.0, 0.4), pc("starburst", 0.0, 1.0, 0.3), pc("streak", 0.0, 1.0, 0.15), n("speed", 0.0, 3.0, 0.5)]),
    e("zoom-blur", "Zoom blur", S, false, &[n("intensity", 0.0, 100.0, 30.0), pt("center", 0.5, 0.5)]),
    e("tilt-shift", "Tilt-shift", S, false, &[n("intensity", 0.0, 100.0, 50.0), pc("width", 0.0, 1.0, 0.3), pc("falloff", 0.0, 1.0, 0.3), n("angle", 0.0, 360.0, 0.0), pt("center", 0.5, 0.5)]),
    // distort
    e("bulge", "Bulge", D, false, &[pt("center", 0.5, 0.5), n("strength", -1.0, 1.0, 0.6), n("radius", 0.1, 3.0, 1.0), pc("falloff", 0.0, 1.0, 0.5)]),
    e("spherize", "Spherize", D, false, &[n("radius", 0.1, 3.0, 1.0), n("depth", 0.0, 3.0, 1.0), pt("center", 0.5, 0.5), pt("lightPosition", 0.3, 0.7), pc("lightIntensity", 0.0, 1.0, 0.5), pc("lightSoftness", 0.0, 1.0, 0.5), c("lightColor", 1.0, 1.0, 1.0)]),
    e("lens-distortion", "Lens distortion", D, false, &[n("bulge", -1.0, 1.0, 0.0), pc("roundness", 0.0, 1.0, 0.0), pc("spread", 0.0, 1.0, 0.6), n("angle", 0.0, 360.0, 0.0), n("swirl", -1.0, 1.0, 0.35), pc("clearCenter", 0.0, 1.0, 0.8), pt("center", 0.5, 0.5)]),
    e("twirl", "Twirl", D, false, &[pt("center", 0.5, 0.5), n("intensity", -5.0, 5.0, 0.5)]),
    e("wave-distortion", "Wave", D, true, &[pc("strength", 0.0, 1.0, 0.3), n("frequency", 0.1, 10.0, 1.0), n("speed", 0.0, 5.0, 1.0), n("angle", 0.0, 360.0, 0.0)]),
    e("kaleidoscope", "Kaleidoscope", D, false, &[pt("center", 0.5, 0.5), n("segments", 2.0, 24.0, 6.0), n("angle", 0.0, 360.0, 0.0)]),
    e("mirror", "Mirror", D, false, &[pt("center", 0.5, 0.5), n("angle", 0.0, 360.0, 0.0)]),
    e("polar-coordinates", "Polar coordinates", D, false, &[pt("center", 0.5, 0.5), n("wrap", 0.0, 2.0, 1.0), n("radius", 0.0, 2.0, 1.0), pc("intensity", 0.0, 1.0, 1.0)]),
    e("fluted-glass", "Fluted glass", D, true, &[n("frequency", 1.0, 20.0, 10.0), n("angle", 0.0, 360.0, 0.0), n("refraction", 0.0, 4.0, 1.5), pc("softness", 0.0, 1.0, 0.5), pc("aberration", 0.0, 1.0, 0.2), n("highlight", 0.0, 2.0, 0.2), n("speed", -1.0, 1.0, 0.0)]),
    e("glass-tiles", "Glass tiles", D, false, &[n("intensity", 0.0, 10.0, 2.0), n("tileCount", 5.0, 50.0, 20.0), n("rotation", 0.0, 360.0, 0.0), pc("roundness", 0.0, 1.0, 0.0)]),
    // background
    e("aurora", "Aurora", B, true, &[c("sky", 0.02, 0.039, 0.102), c("colorA", 0.647, 0.2, 0.973), c("colorB", 0.133, 0.933, 0.533), c("colorC", 0.086, 0.58, 0.91), pc("intensity", 0.0, 1.0, 0.8), pc("height", 0.1, 2.0, 1.2), pc("waviness", 0.0, 2.0, 0.5), n("speed", -10.0, 10.0, 5.0)]),
    e("mesh-gradient", "Mesh gradient", B, true, &[c("color1", 0.102, 0.02, 0.2), c("color2", 0.427, 0.184, 0.82), c("color3", 0.878, 0.294, 0.62), c("color4", 1.0, 0.549, 0.259), c("color5", 1.0, 0.875, 0.557), n("smoothness", 0.0, 5.0, 2.0), n("swirl", -1.0, 1.0, 0.3), n("speed", 0.0, 10.0, 1.0)]),
    e("flowing-gradient", "Flowing gradient", B, true, &[c("colorA", 0.039, 0.0, 0.082), c("colorB", 0.42, 0.09, 0.902), c("colorC", 1.0, 0.302, 0.416), c("colorD", 1.0, 0.42, 0.208), n("distortion", 0.0, 2.0, 0.5), n("seed", 0.0, 100.0, 0.0), n("speed", 0.0, 10.0, 1.0)]),
    e("plasma", "Plasma", B, true, &[c("colorA", 0.439, 0.094, 0.745), c("colorB", 0.0, 0.0, 0.0), n("density", 0.0, 4.0, 2.0), pc("warp", 0.0, 1.0, 0.4), n("intensity", 0.1, 3.0, 1.5), pc("balance", 0.0, 1.0, 0.5), n("speed", 0.0, 5.0, 2.0)]),
    e("swirl", "Swirl", B, true, &[c("colorA", 0.071, 0.459, 0.847), c("colorB", 0.882, 0.569, 0.212), n("detail", 0.0, 5.0, 4.0), pc("blend", 0.0, 1.0, 0.45), n("speed", 0.0, 5.0, 1.0)]),
    e("marble", "Marble", B, true, &[c("colorA", 0.95, 0.93, 0.89), c("colorB", 0.7, 0.62, 0.5), c("colorC", 0.25, 0.22, 0.2), n("scale", 0.1, 10.0, 0.8), n("turbulence", 0.0, 50.0, 20.0), n("seed", 0.0, 100.0, 0.0), n("speed", 0.0, 0.25, 0.05)]),
    e("godrays", "God rays", B, true, &[pt("center", 0.0, 1.0), c("rayColor", 0.259, 0.514, 0.984), c("background", 0.0, 0.0, 0.0), pc("density", 0.0, 1.0, 0.3), pc("intensity", 0.0, 1.0, 0.8), pc("spotty", 0.0, 1.0, 1.0), n("speed", 0.0, 2.0, 0.5)]),
    e("sunburst", "Sunburst", B, true, &[c("color", 1.0, 0.867, 0.533), c("background", 0.0, 0.0, 0.0), pt("center", 0.5, 0.5), n("rayCount", 3.0, 64.0, 12.0), pc("softness", 0.0, 1.0, 0.3), n("radius", 0.0, 1.2, 0.8), pc("feather", 0.0, 1.0, 0.5), n("speed", -2.0, 2.0, 0.2)]),
    e("spiral", "Spiral", B, true, &[c("colorA", 0.06, 0.07, 0.16), c("colorB", 0.98, 0.84, 0.58), pc("strokeWidth", 0.0, 1.0, 0.5), pc("strokeFalloff", 0.0, 1.0, 0.4), pc("softness", 0.0, 1.0, 0.15), n("scale", 0.1, 5.0, 3.0), pt("center", 0.5, 0.5), n("speed", -3.0, 3.0, 1.0)]),
    e("conic-gradient", "Conic gradient", B, false, &[c("colorA", 1.0, 0.0, 0.502), c("colorB", 0.0, 0.749, 1.0), pt("center", 0.5, 0.5), n("rotation", 0.0, 360.0, 0.0), n("repeat", 1.0, 24.0, 1.0)]),
    e("voronoi", "Voronoi", B, true, &[c("colorA", 0.192, 0.525, 0.812), c("colorB", 0.988, 0.008, 0.867), c("borderColor", 0.0, 0.0, 0.0), n("scale", 1.0, 20.0, 6.0), pc("edgeIntensity", 0.0, 1.0, 0.5), n("edgeSoftness", 0.0, 0.4, 0.05), n("speed", 0.0, 5.0, 0.5)]),
    e("stripes", "Stripes", B, true, &[c("colorA", 0.0, 0.0, 0.0), c("colorB", 1.0, 1.0, 1.0), n("angle", 0.0, 360.0, 45.0), n("density", 1.0, 30.0, 5.0), pc("balance", 0.0, 1.0, 0.5), n("softness", 0.0, 0.5, 0.0), n("speed", -2.0, 2.0, 0.2)]),
    e("studio-background", "Studio background", B, true, &[c("color", 0.847, 0.859, 0.925), c("lightColor", 0.835, 0.894, 0.918), pc("intensity", 0.0, 1.0, 0.4), pc("softness", 0.0, 1.0, 0.5), pt("center", 0.5, 0.2), pc("vignette", 0.0, 1.0, 0.0), n("speed", -5.0, 5.0, 2.0)]),
];

pub fn find(id: &str) -> Option<&'static EffectSpec> {
    EFFECTS.iter().find(|e| e.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_matches_the_contract() {
        assert_eq!(EFFECTS.len(), 41);
        let mut ids: Vec<_> = EFFECTS.iter().map(|e| e.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 41);
        assert_eq!(EFFECTS.iter().filter(|e| e.group == Group::Background).count(), 13);
        for e in EFFECTS {
            for p in e.params {
                if let (ParamKind::Number { min, max } | ParamKind::Percent { min, max }, ParamDefault::Num(d)) =
                    (p.kind, p.default)
                {
                    assert!(min <= d && d <= max, "{}.{}", e.id, p.name);
                }
            }
        }
    }
}
