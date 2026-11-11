//! The font library: open-licence families bundled with the app (SIL OFL 1.1, files and licences under
//! `assets/fonts/library/<family>/`), plus system fonts looked up by family name. Used by the motion-graphics
//! renderer (Takumi) and by timeline text and captions (ove-render).

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use serde::Serialize;

/// Weight range a face covers: a variable font spans a range, a static font one weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightRange {
    pub min: u16,
    pub max: u16,
}

/// One bundled font file.
#[derive(Debug, Clone, Copy)]
pub struct Face {
    pub family: &'static str,
    pub file: &'static str,
    pub bytes: &'static [u8],
    pub italic: bool,
    pub weight: WeightRange,
}

macro_rules! face {
    ($family:literal, $dir:literal, $file:literal, $italic:expr, $min:literal, $max:literal) => {
        Face {
            family: $family,
            file: $file,
            bytes: include_bytes!(concat!("../../../assets/fonts/library/", $dir, "/", $file)),
            italic: $italic,
            weight: WeightRange { min: $min, max: $max },
        }
    };
}

/// Every bundled face. Variable fonts carry their weight axis range.
pub static FACES: &[Face] = &[
    face!("Inter", "inter", "Inter[opsz,wght].ttf", false, 100, 900),
    face!("Inter", "inter", "Inter-Italic[opsz,wght].ttf", true, 100, 900),
    face!("Roboto", "roboto", "Roboto[wdth,wght].ttf", false, 100, 900),
    face!("Roboto", "roboto", "Roboto-Italic[wdth,wght].ttf", true, 100, 900),
    face!("Montserrat", "montserrat", "Montserrat[wght].ttf", false, 100, 900),
    face!("Montserrat", "montserrat", "Montserrat-Italic[wght].ttf", true, 100, 900),
    face!("Poppins", "poppins", "Poppins-Regular.ttf", false, 400, 400),
    face!("Poppins", "poppins", "Poppins-Italic.ttf", true, 400, 400),
    face!("Poppins", "poppins", "Poppins-SemiBold.ttf", false, 600, 600),
    face!("Poppins", "poppins", "Poppins-Bold.ttf", false, 700, 700),
    face!("Poppins", "poppins", "Poppins-Black.ttf", false, 900, 900),
    face!("Playfair Display", "playfairdisplay", "PlayfairDisplay[wght].ttf", false, 400, 900),
    face!("Playfair Display", "playfairdisplay", "PlayfairDisplay-Italic[wght].ttf", true, 400, 900),
    face!("Bebas Neue", "bebasneue", "BebasNeue-Regular.ttf", false, 400, 400),
    face!("Oswald", "oswald", "Oswald[wght].ttf", false, 200, 700),
    face!("Lora", "lora", "Lora[wght].ttf", false, 400, 700),
    face!("Lora", "lora", "Lora-Italic[wght].ttf", true, 400, 700),
    face!("Space Grotesk", "spacegrotesk", "SpaceGrotesk[wght].ttf", false, 300, 700),
    face!("JetBrains Mono", "jetbrainsmono", "JetBrainsMono[wght].ttf", false, 100, 800),
    face!("Caveat", "caveat", "Caveat[wght].ttf", false, 400, 700),
    face!("Anton", "anton", "Anton-Regular.ttf", false, 400, 400),
    face!("DM Sans", "dmsans", "DMSans[opsz,wght].ttf", false, 100, 1000),
    face!("Archivo Black", "archivoblack", "ArchivoBlack-Regular.ttf", false, 400, 400),
    face!("Instrument Serif", "instrumentserif", "InstrumentSerif-Regular.ttf", false, 400, 400),
    face!("Instrument Serif", "instrumentserif", "InstrumentSerif-Italic.ttf", true, 400, 400),
    face!("Dancing Script", "dancingscript", "DancingScript[wght].ttf", false, 400, 700),
];

/// The family used when a requested family is unknown.
pub const DEFAULT_FAMILY: &str = "Inter";

/// A family as listed in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyInfo {
    pub family: String,
    /// `bundled` (ships with the app) or `system` (installed on this computer).
    pub source: String,
    pub weights: WeightRange,
    pub italic: bool,
    pub category: String,
}

fn category(family: &str) -> &'static str {
    match family {
        "Playfair Display" | "Lora" | "Instrument Serif" => "serif",
        "Bebas Neue" | "Anton" | "Archivo Black" | "Oswald" => "display",
        "JetBrains Mono" => "mono",
        "Caveat" | "Dancing Script" => "handwriting",
        _ => "sans-serif",
    }
}

/// Normalises a family name for lookup (`playfair display`, `PlayfairDisplay`, `"Playfair Display", serif`).
pub fn normalise_family(name: &str) -> String {
    let first = name.split(',').next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'');
    first.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '_').flat_map(char::to_lowercase).collect()
}

/// Bundled families with their weight ranges.
pub fn bundled_families() -> Vec<FamilyInfo> {
    let mut map: BTreeMap<&str, FamilyInfo> = BTreeMap::new();
    for f in FACES {
        let e = map.entry(f.family).or_insert_with(|| FamilyInfo {
            family: f.family.to_string(),
            source: "bundled".into(),
            weights: f.weight,
            italic: false,
            category: category(f.family).into(),
        });
        e.weights.min = e.weights.min.min(f.weight.min);
        e.weights.max = e.weights.max.max(f.weight.max);
        e.italic |= f.italic;
    }
    map.into_values().collect()
}

/// System fonts (fontdb), loaded once. Empty when none are installed.
fn system_db() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        if std::env::var_os("OVE_NO_SYSTEM_FONTS").is_none() {
            db.load_system_fonts();
        }
        db
    })
}

/// Bundled families followed by installed system families that are not bundled.
pub fn all_families() -> Vec<FamilyInfo> {
    let mut out = bundled_families();
    let bundled: std::collections::BTreeSet<String> = out.iter().map(|f| normalise_family(&f.family)).collect();
    let mut sys: BTreeMap<String, FamilyInfo> = BTreeMap::new();
    for face in system_db().faces() {
        let Some((name, _)) = face.families.first() else { continue };
        if bundled.contains(&normalise_family(name)) || name.starts_with('.') {
            continue;
        }
        let w = face.weight.0;
        let e = sys.entry(name.clone()).or_insert_with(|| FamilyInfo {
            family: name.clone(),
            source: "system".into(),
            weights: WeightRange { min: w, max: w },
            italic: false,
            category: "system".into(),
        });
        e.weights.min = e.weights.min.min(w);
        e.weights.max = e.weights.max.max(w);
        e.italic |= face.style != fontdb::Style::Normal;
    }
    out.extend(sys.into_values());
    out
}

/// Font data resolved for a request: bytes, face index, and the weight to set on a variable `wght` axis.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub family: String,
    pub data: Arc<Vec<u8>>,
    pub index: u32,
    /// Weight to apply via the `wght` axis (variable fonts), `None` for static faces.
    pub variable_weight: Option<f32>,
    pub italic: bool,
}

/// Resolves family + weight + italic to font data: bundled first (closest weight, italic preferred when asked),
/// then system fonts, then the default family.
pub fn resolve(family: &str, weight: u16, italic: bool) -> Resolved {
    let want = normalise_family(family);
    let pick = |fam: &str| -> Option<&'static Face> {
        let faces: Vec<&Face> = FACES.iter().filter(|f| normalise_family(f.family) == fam).collect();
        if faces.is_empty() {
            return None;
        }
        let score = |f: &&Face| {
            let wd = f.weight.min.saturating_sub(weight) + weight.saturating_sub(f.weight.max);
            (if f.italic == italic { 0 } else { 10_000 }) + wd as u32
        };
        faces.into_iter().min_by_key(score)
    };
    if let Some(f) = pick(&want) {
        return from_face(f, weight);
    }
    if !want.is_empty()
        && let Some(r) = system(family, weight, italic)
    {
        return r;
    }
    from_face(pick(&normalise_family(DEFAULT_FAMILY)).expect("default family is bundled"), weight)
}

fn from_face(f: &'static Face, weight: u16) -> Resolved {
    type Cache = std::sync::Mutex<BTreeMap<&'static str, Arc<Vec<u8>>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let data = CACHE
        .get_or_init(Default::default)
        .lock()
        .expect("font cache")
        .entry(f.file)
        .or_insert_with(|| Arc::new(f.bytes.to_vec()))
        .clone();
    Resolved {
        family: f.family.to_string(),
        data,
        index: 0,
        variable_weight: (f.weight.min != f.weight.max).then(|| weight.clamp(f.weight.min, f.weight.max) as f32),
        italic: f.italic,
    }
}

fn system(family: &str, weight: u16, italic: bool) -> Option<Resolved> {
    let db = system_db();
    let name = family.split(',').next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'');
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(name)],
        weight: fontdb::Weight(weight),
        stretch: fontdb::Stretch::Normal,
        style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
    };
    let id = db.query(&query)?;
    let (src, index) = db.face_source(id)?;
    let data = match src {
        fontdb::Source::Binary(b) => b.as_ref().as_ref().to_vec(),
        fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => std::fs::read(p).ok()?,
    };
    Some(Resolved { family: name.to_string(), data: Arc::new(data), index, variable_weight: None, italic })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_face_parses() {
        for f in FACES {
            assert!(ttf_header_ok(f.bytes), "{}", f.file);
        }
        assert!(bundled_families().len() >= 16);
    }

    fn ttf_header_ok(b: &[u8]) -> bool {
        b.len() > 12 && (&b[..4] == b"\x00\x01\x00\x00" || &b[..4] == b"OTTO" || &b[..4] == b"true")
    }

    #[test]
    fn resolution_and_fallback() {
        let r = resolve("Playfair Display", 700, false);
        assert_eq!(r.family, "Playfair Display");
        assert_eq!(r.variable_weight, Some(700.0));
        assert!(!r.italic);
        assert!(resolve("playfairdisplay", 400, true).italic);
        let p = resolve("\"Poppins\", sans-serif", 650, false);
        assert_eq!(p.family, "Poppins");
        assert_eq!(p.variable_weight, None);
        assert_eq!(resolve("Bebas Neue", 900, true).family, "Bebas Neue");
        assert_eq!(resolve("No Such Family 123", 400, false).family, "Inter");
        assert_eq!(resolve("", 400, false).family, "Inter");
    }
}
