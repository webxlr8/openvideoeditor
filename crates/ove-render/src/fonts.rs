//! Font faces for timeline text: `fontFamily`/`fontWeight`/`fontStyle` resolved through the font library
//! (bundled OFL families, then installed system fonts, then Inter), with the variable `wght` axis applied.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use ab_glyph::{FontArc, FontVec, VariableFont};
use serde_json::{Map, Value};

/// A requested face.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontSpec {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
}

/// Reads `fontWeight` (number, numeric string or keyword) as 100…1000.
pub fn weight_of(v: Option<&Value>) -> u16 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(400.0).clamp(1.0, 1000.0) as u16,
        Some(Value::String(s)) => match s.as_str() {
            "bold" | "bolder" => 700,
            "lighter" => 300,
            "normal" | "" => 400,
            s => s.parse::<f64>().map(|n| n.clamp(1.0, 1000.0) as u16).unwrap_or(400),
        },
        _ => 400,
    }
}

/// The face a text item asks for, or `None` for the default (Inter regular/bold by `fontWeight`).
pub fn spec_of(f: &Map<String, Value>, family_key: &str) -> Option<FontSpec> {
    let family = f.get(family_key).and_then(Value::as_str).unwrap_or("").trim();
    let weight = weight_of(f.get("fontWeight"));
    let italic = matches!(f.get("fontStyle").and_then(Value::as_str), Some("italic" | "oblique"));
    let default_family = family.is_empty() || ove_graphics::fonts::normalise_family(family) == "inter";
    if default_family && !italic && matches!(weight, 400 | 700) {
        return None;
    }
    let family = if family.is_empty() { ove_graphics::fonts::DEFAULT_FAMILY } else { family };
    Some(FontSpec { family: family.to_string(), weight, italic })
}

/// The ab_glyph font for a spec (cached).
#[allow(
    clippy::expect_used,
    reason = "the fallback is the bundled default family, compiled in and parsed by the crate's tests"
)]
pub fn face(spec: &FontSpec) -> FontArc {
    static CACHE: OnceLock<Mutex<HashMap<FontSpec, FontArc>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(f) = cache.lock().unwrap_or_else(PoisonError::into_inner).get(spec) {
        return f.clone();
    }
    let r = ove_graphics::fonts::resolve(&spec.family, spec.weight, spec.italic);
    let font = FontVec::try_from_vec_and_index(r.data.as_ref().clone(), r.index)
        .or_else(|_| {
            let d = ove_graphics::fonts::resolve(ove_graphics::fonts::DEFAULT_FAMILY, spec.weight, spec.italic);
            FontVec::try_from_vec_and_index(d.data.as_ref().clone(), 0)
        })
        .map(|mut f| {
            if let Some(w) = r.variable_weight {
                f.set_variation(b"wght", w);
            }
            FontArc::new(f)
        })
        .expect("bundled default font parses");
    let mut c = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if c.len() > 64 {
        c.clear();
    }
    c.insert(spec.clone(), font.clone());
    font
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn specs_from_fields() {
        let m = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(spec_of(&m(json!({"fontFamily": "Inter", "fontWeight": 700})), "fontFamily"), None);
        assert_eq!(spec_of(&m(json!({})), "fontFamily"), None);
        let s = spec_of(&m(json!({"fontFamily": "Lora", "fontWeight": "600", "fontStyle": "italic"})), "fontFamily");
        assert_eq!(s, Some(FontSpec { family: "Lora".into(), weight: 600, italic: true }));
        assert_eq!(spec_of(&m(json!({"fontWeight": 300})), "fontFamily").unwrap().family, "Inter");
        assert_eq!(weight_of(Some(&json!("bold"))), 700);
    }
}
