//! Motion-graphics code items (roadmap change 4) and the font library (change 11).
//!
//! A code item's `codeComposition` is either `{source, width?, height?, assets?, props?}` — a TSX module whose
//! default export is a pure function of `{time, duration, progress, frame, fps, width, height, assets, props}`
//! returning JSX — or `{template, props}` naming a built-in template. Each frame:
//!
//! 1. the source is transpiled once with oxc (TypeScript stripped, JSX → `h()` calls) and checked statically;
//! 2. the module runs in a fresh QuickJS sandbox ([`sandbox`]) with memory, stack and CPU limits;
//! 3. the returned element tree becomes a Takumi node ([`tree`]) and is laid out and painted to RGBA.
//!
//! The supported subset is documented in `SUPPORTED.md`.
#![forbid(unsafe_code)]

pub mod diag;
pub mod fonts;
pub mod sandbox;
pub mod templates;
pub mod transpile;
pub mod tree;

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use takumi::prelude::{FontResource, Fonts, ImageSource, Node, RenderOptions, Viewport};

pub use diag::{Diagnostic, Severity};
pub use sandbox::{Limits, RunError};

/// The `codeComposition` field of a code item.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeComposition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Asset name → media id (or `{src: mediaId, …}`). `assets.<name>` in the composition is an image source
    /// for `<img>`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assets: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub props: serde_json::Map<String, Value>,
}

impl CodeComposition {
    /// Reads a composition from an item field value (object, or a JSON string holding one).
    pub fn from_value(v: &Value) -> Option<Self> {
        match v {
            Value::String(s) => serde_json::from_str(s).ok(),
            Value::Object(_) => serde_json::from_value(v.clone()).ok(),
            _ => None,
        }
    }

    /// Asset name → media id, accepting both `name: "id"` and `name: {src: "id"}`.
    pub fn asset_ids(&self) -> BTreeMap<String, String> {
        self.assets
            .iter()
            .filter_map(|(k, v)| {
                let id = v.as_str().or_else(|| v.get("src").and_then(Value::as_str))?;
                (!id.is_empty()).then(|| (k.clone(), id.to_string()))
            })
            .collect()
    }
}

/// A composition ready to render: transpiled once, with resolved props and assets.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub js: Arc<String>,
    pub width: u32,
    pub height: u32,
    pub props: Value,
    /// Asset name → media id.
    pub assets: BTreeMap<String, String>,
    /// Static diagnostics (warnings only; errors make `prepare` fail).
    pub diagnostics: Vec<Diagnostic>,
}

fn transpile_cached(src: &str) -> Arc<transpile::Transpiled> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Arc<transpile::Transpiled>>>> = OnceLock::new();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    let key = h.finish();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(t) = cache.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
        return t.clone();
    }
    let t = Arc::new(transpile::transpile(src));
    let mut c = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if c.len() > 64 {
        c.clear();
    }
    c.insert(key, t.clone());
    t
}

/// Resolves a composition (template or source) and transpiles it. `default_size` is used when the composition
/// does not set `width`/`height` (normally the project's reference frame size).
pub fn prepare(c: &CodeComposition, default_size: (u32, u32)) -> Result<Prepared, Vec<Diagnostic>> {
    let (source, props, mut assets) = match (&c.template, &c.source) {
        (Some(id), _) => {
            let Some(t) = templates::get(id) else {
                return Err(vec![Diagnostic::error("template", format!("unknown template `{id}`"))]);
            };
            let (props, images) = templates::resolve_props(&t, &c.props);
            (t.source.to_string(), props, images.into_iter().collect::<BTreeMap<_, _>>())
        }
        (None, Some(src)) => (src.clone(), Value::Object(c.props.clone()), BTreeMap::new()),
        (None, None) => {
            return Err(vec![Diagnostic::error("empty", "the composition has neither `source` nor `template`")]);
        }
    };
    assets.extend(c.asset_ids());
    let t = transpile_cached(&source);
    if !t.ok() {
        return Err(t.diagnostics.clone());
    }
    let w = c.width.unwrap_or(default_size.0).clamp(16, 8192);
    let h = c.height.unwrap_or(default_size.1).clamp(16, 8192);
    Ok(Prepared { js: Arc::new(t.js.clone()), width: w, height: h, props, assets, diagnostics: t.diagnostics.clone() })
}

/// Item-relative time for one frame.
#[derive(Debug, Clone, Copy)]
pub struct FrameTime {
    /// Seconds since the item starts.
    pub time: f64,
    /// Item duration in seconds.
    pub duration: f64,
    pub fps: f64,
}

/// A rendered layer: straight (not premultiplied) RGBA.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Rendered {
    pub frame: Frame,
    pub diagnostics: Vec<Diagnostic>,
    pub logs: Vec<String>,
    /// Milliseconds spent in the sandbox and in layout + paint.
    pub script_ms: f64,
    pub paint_ms: f64,
}

/// Loads image bytes for a media id.
pub type ImageLoader<'a> = &'a dyn Fn(&str) -> Option<Arc<Vec<u8>>>;

struct FontState {
    fonts: Fonts,
    registered: std::collections::BTreeSet<String>,
}

fn font_state() -> &'static RwLock<FontState> {
    static FONTS: OnceLock<RwLock<FontState>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut fonts = Fonts::default();
        let mut registered = std::collections::BTreeSet::new();
        for f in fonts::FACES {
            if fonts.register(FontResource::new(f.bytes.to_vec())).is_ok() {
                registered.insert(fonts::normalise_family(f.family));
            }
        }
        RwLock::new(FontState { fonts, registered })
    })
}

/// Registers installed system families the tree names that are not bundled (once per family).
fn ensure_families(families: &std::collections::BTreeSet<String>) {
    let missing: Vec<&String> = {
        let s = font_state().read().unwrap_or_else(PoisonError::into_inner);
        families.iter().filter(|f| !s.registered.contains(&fonts::normalise_family(f))).collect()
    };
    if missing.is_empty() {
        return;
    }
    let mut s = font_state().write().unwrap_or_else(PoisonError::into_inner);
    for fam in missing {
        let key = fonts::normalise_family(fam);
        if s.registered.contains(&key) {
            continue;
        }
        s.registered.insert(key.clone());
        for (weight, italic) in [(400, false), (700, false), (400, true)] {
            let r = fonts::resolve(fam, weight, italic);
            if fonts::normalise_family(&r.family) == key {
                let _ = s.fonts.register(FontResource::new(r.data.as_ref().clone()));
            }
        }
    }
}

fn decoded_image(media_id: &str, load: ImageLoader<'_>) -> Option<ImageSource> {
    static CACHE: OnceLock<Mutex<HashMap<String, ImageSource>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(i) = cache.lock().unwrap_or_else(PoisonError::into_inner).get(media_id) {
        return Some(i.clone());
    }
    let bytes = load(media_id)?;
    let img = ImageSource::from_bytes(&bytes).ok()?;
    let mut c = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if c.len() > 32 {
        c.clear();
    }
    c.insert(media_id.to_string(), img.clone());
    Some(img)
}

/// Renders one frame at `scale` × the composition size.
pub fn render(
    p: &Prepared,
    t: FrameTime,
    scale: f32,
    images: ImageLoader<'_>,
    limits: Limits,
) -> Result<Rendered, Vec<Diagnostic>> {
    let fps = if t.fps > 0.0 { t.fps } else { 30.0 };
    let duration = t.duration.max(0.0);
    let asset_values: serde_json::Map<String, Value> =
        p.assets.keys().map(|k| (k.clone(), Value::String(format!("{}{k}", tree::ASSET_PREFIX)))).collect();
    let input = serde_json::json!({
        "time": t.time,
        "duration": duration,
        "progress": if duration > 0.0 { (t.time / duration).clamp(0.0, 1.0) } else { 0.0 },
        "frame": (t.time * fps).floor(),
        "fps": fps,
        "width": p.width,
        "height": p.height,
        "assets": asset_values,
        "props": p.props,
    });
    let t0 = Instant::now();
    let out = sandbox::run(&p.js, &input, limits).map_err(|e| vec![Diagnostic::error(e.code(), e.to_string())])?;
    let script_ms = t0.elapsed().as_secs_f64() * 1e3;

    let t1 = Instant::now();
    let built = tree::build(&out.nodes, p.width, p.height, &|name| p.assets.contains_key(name));
    let mut diagnostics = p.diagnostics.clone();
    diagnostics.extend(built.diagnostics.iter().cloned());
    ensure_families(&built.families);
    let mut imgs: HashMap<Arc<str>, ImageSource> = HashMap::new();
    for name in &built.assets {
        let id = &p.assets[name];
        match decoded_image(id, images) {
            Some(img) => {
                imgs.insert(Arc::from(format!("{}{name}", tree::ASSET_PREFIX)), img);
            }
            None => {
                diagnostics.push(Diagnostic::warning("asset", format!("image for asset `{name}` could not be loaded")))
            }
        }
    }
    let node: Node = serde_json::from_value(built.root).map_err(|e| vec![Diagnostic::error("tree", e.to_string())])?;
    let scale = scale.clamp(0.01, 4.0);
    let ow = ((p.width as f32 * scale).round() as u32).max(1);
    let oh = ((p.height as f32 * scale).round() as u32).max(1);
    let state = font_state().read().unwrap_or_else(PoisonError::into_inner);
    let opts = RenderOptions::builder()
        .viewport(Viewport::new((ow, oh)).with_device_pixel_ratio(scale))
        .node(node)
        .fonts(&state.fonts)
        .images(imgs)
        .time_ms((t.time.max(0.0) * 1000.0) as u64)
        .build();
    let bmp = takumi::render(opts).map_err(|e| vec![Diagnostic::error("paint", e.to_string())])?;
    drop(state);
    let paint_ms = t1.elapsed().as_secs_f64() * 1e3;
    let frame = Frame { width: bmp.width(), height: bmp.height(), rgba: bmp.into_raw() };
    Ok(Rendered { frame, diagnostics, logs: out.logs, script_ms, paint_ms })
}

/// Result of [`check`].
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub ok: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub logs: Vec<String>,
    /// Slowest sampled frame (sandbox + layout + paint at full size), in milliseconds.
    pub frame_ms: f64,
    pub width: u32,
    pub height: u32,
}

/// Checks a composition: static analysis, then renders sample frames (start, middle, near the end) at full
/// size, collecting runtime errors, unsupported elements/styles, console output and timing.
pub fn check(c: &CodeComposition, default_size: (u32, u32), duration: f64, images: ImageLoader<'_>) -> CheckReport {
    let p = match prepare(c, default_size) {
        Ok(p) => p,
        Err(diagnostics) => return CheckReport { ok: false, diagnostics, ..CheckReport::default() },
    };
    let mut report = CheckReport { width: p.width, height: p.height, ..CheckReport::default() };
    let d = duration.max(0.1);
    for time in [0.0, d * 0.5, d * 0.95] {
        let start = Instant::now();
        match render(&p, FrameTime { time, duration: d, fps: 30.0 }, 1.0, images, Limits::default()) {
            Ok(r) => {
                for x in r.diagnostics {
                    if !report.diagnostics.contains(&x) {
                        report.diagnostics.push(x);
                    }
                }
                for l in r.logs {
                    if !report.logs.contains(&l) {
                        report.logs.push(l);
                    }
                }
            }
            Err(e) => {
                for x in e {
                    let x = Diagnostic { message: format!("at {time:.2}s: {}", x.message), ..x };
                    if !report.diagnostics.iter().any(|d| d.code == x.code) {
                        report.diagnostics.push(x);
                    }
                }
            }
        }
        report.frame_ms = report.frame_ms.max(start.elapsed().as_secs_f64() * 1e3);
    }
    report.ok = !report.diagnostics.iter().any(Diagnostic::is_error);
    report
}
