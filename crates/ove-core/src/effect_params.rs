//! The numeric parameters of the catalogue effects (docs/spec/presets.md), by effect type: the parameters that
//! accept keyframes. A keyframe track animates one of them with the property `effects.<effect entry id>.<parameter>`.
//! `ove-render`'s effect contract is the source; a test there keeps this table equal to it.

/// Property prefix of a keyframe track on an effect parameter.
pub const PREFIX: &str = "effects.";

#[rustfmt::skip]
pub const NUMERIC_PARAMS: &[(&str, &[&str])] = &[
    ("duotone", &["balance"]),
    ("tritone", &["midpoint"]),
    ("gradient-map", &["speed", "contrast", "blackPoint", "whitePoint", "strength"]),
    ("posterize", &["levels"]),
    ("solarize", &["threshold", "strength"]),
    ("halftone", &["frequency", "angle", "cmyk", "misprint"]),
    ("dither", &["pixelSize", "threshold", "spread"]),
    ("engraving", &["frequency", "angle", "relief", "waviness", "contrast"]),
    ("watercolor", &["brushSize", "bleed", "paper", "strength"]),
    ("pixelate", &["cells", "gap", "roundness"]),
    ("chromatic-aberration", &["strength", "angle", "redOffset", "greenOffset", "blueOffset"]),
    ("glitch", &["intensity", "speed", "rgbShift", "blockDensity", "colorBarIntensity", "mirrorAmount", "scanlineIntensity", "seed"]),
    ("vhs", &["wobble", "scanlineNoise", "smear", "speed"]),
    ("crt-screen", &["pixelSize", "colorShift", "scanlineIntensity", "scanlineFrequency", "brightness", "vignetteIntensity", "vignetteRadius"]),
    ("light-leak", &["spread", "intensity", "streaks", "speed"]),
    ("lens-flare", &["intensity", "ghosts", "halo", "starburst", "streak", "speed"]),
    ("zoom-blur", &["intensity"]),
    ("tilt-shift", &["intensity", "width", "falloff", "angle"]),
    ("bulge", &["strength", "radius", "falloff"]),
    ("spherize", &["radius", "depth", "lightIntensity", "lightSoftness"]),
    ("lens-distortion", &["bulge", "roundness", "spread", "angle", "swirl", "clearCenter"]),
    ("twirl", &["intensity"]),
    ("wave-distortion", &["strength", "frequency", "speed", "angle"]),
    ("kaleidoscope", &["segments", "angle"]),
    ("mirror", &["angle"]),
    ("polar-coordinates", &["wrap", "radius", "intensity"]),
    ("fluted-glass", &["frequency", "angle", "refraction", "softness", "aberration", "highlight", "speed"]),
    ("glass-tiles", &["intensity", "tileCount", "rotation", "roundness"]),
    ("aurora", &["intensity", "height", "waviness", "speed"]),
    ("mesh-gradient", &["smoothness", "swirl", "speed"]),
    ("flowing-gradient", &["distortion", "seed", "speed"]),
    ("plasma", &["density", "warp", "intensity", "balance", "speed"]),
    ("swirl", &["detail", "blend", "speed"]),
    ("marble", &["scale", "turbulence", "seed", "speed"]),
    ("godrays", &["density", "intensity", "spotty", "speed"]),
    ("sunburst", &["rayCount", "softness", "radius", "feather", "speed"]),
    ("spiral", &["strokeWidth", "strokeFalloff", "softness", "scale", "speed"]),
    ("conic-gradient", &["rotation", "repeat"]),
    ("voronoi", &["scale", "edgeIntensity", "edgeSoftness", "speed"]),
    ("stripes", &["angle", "density", "balance", "softness", "speed"]),
    ("studio-background", &["intensity", "softness", "vignette", "speed"]),
];

/// The numeric parameters of effect type `kind`, or `None` for an unknown type.
pub fn params(kind: &str) -> Option<&'static [&'static str]> {
    NUMERIC_PARAMS.iter().find(|(k, _)| *k == kind).map(|(_, p)| *p)
}

/// Splits `effects.<entry id>.<parameter>` into the entry id and the parameter.
pub fn split_property(property: &str) -> Option<(&str, &str)> {
    let rest = property.strip_prefix(PREFIX)?;
    let (id, param) = rest.rsplit_once('.')?;
    (!id.is_empty() && !param.is_empty()).then_some((id, param))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_names_split() {
        assert_eq!(split_property("effects.e1.intensity"), Some(("e1", "intensity")));
        assert_eq!(split_property("effects.fx.a.radius"), Some(("fx.a", "radius")));
        assert_eq!(split_property("effects..radius"), None);
        assert_eq!(split_property("opacity"), None);
        assert_eq!(params("zoom-blur"), Some(&["intensity"][..]));
        assert_eq!(params("nope"), None);
    }
}
