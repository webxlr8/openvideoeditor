//! Element tree (plain data from the sandbox) → Takumi node JSON, with diagnostics for unsupported elements,
//! props and style properties. Unsupported things are dropped with a diagnostic; the rest still renders.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

use serde_json::{Map, Value, json};
use takumi::prelude::Style;

use crate::diag::Diagnostic;

/// Prefix of image sources that refer to a composition asset: `assets.logo` is the string `asset:logo`.
pub const ASSET_PREFIX: &str = "asset:";

/// Block-level elements rendered as containers.
const BLOCK: &[&str] = &[
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "figure",
    "figcaption",
    "blockquote",
    "ul",
    "ol",
    "li",
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
];
/// Inline elements rendered as inline containers.
const INLINE: &[&str] =
    &["span", "strong", "b", "em", "i", "u", "s", "small", "code", "mark", "sub", "sup", "label", "a"];

fn preset(tag: &str) -> Option<Value> {
    let heading = |em: f32| json!({"display": "block", "fontSize": format!("{em}em"), "fontWeight": 700});
    Some(match tag {
        "h1" => heading(2.0),
        "h2" => heading(1.5),
        "h3" => heading(1.17),
        "h4" => heading(1.0),
        "h5" => heading(0.83),
        "h6" => heading(0.67),
        "strong" | "b" => json!({"display": "inline", "fontWeight": 700}),
        "em" | "i" => json!({"display": "inline", "fontStyle": "italic"}),
        "u" => json!({"display": "inline", "textDecoration": "underline"}),
        "s" => json!({"display": "inline", "textDecoration": "line-through"}),
        "code" => json!({"display": "inline", "fontFamily": "JetBrains Mono"}),
        "sub" | "sup" | "small" => json!({"display": "inline", "fontSize": "0.8em"}),
        t if INLINE.contains(&t) => json!({"display": "inline"}),
        _ => return None,
    })
}

/// A built Takumi tree with what it needs from the host.
#[derive(Debug, Clone)]
pub struct Built {
    pub root: Value,
    /// Asset names referenced by `<img src={assets.name}>`.
    pub assets: BTreeSet<String>,
    /// Font families named in styles.
    pub families: BTreeSet<String>,
    pub diagnostics: Vec<Diagnostic>,
}

struct Builder<'a> {
    known_assets: &'a dyn Fn(&str) -> bool,
    assets: BTreeSet<String>,
    families: BTreeSet<String>,
    diags: Vec<Diagnostic>,
}

impl Builder<'_> {
    fn diag(&mut self, d: Diagnostic) {
        if !self.diags.contains(&d) && self.diags.len() < 100 {
            self.diags.push(d);
        }
    }

    fn node(&mut self, v: &Value) -> Option<Value> {
        match v {
            Value::String(s) => Some(json!({"type": "text", "text": s})),
            Value::Object(o) => self.element(o),
            _ => None,
        }
    }

    fn children(&mut self, o: &Map<String, Value>) -> Vec<Value> {
        o.get("children")
            .and_then(Value::as_array)
            .map(|c| c.iter().filter_map(|c| self.node(c)).collect())
            .unwrap_or_default()
    }

    fn element(&mut self, o: &Map<String, Value>) -> Option<Value> {
        let tag = o.get("tag").and_then(Value::as_str).unwrap_or("div").to_ascii_lowercase();
        let empty = Map::new();
        let props = o.get("props").and_then(Value::as_object).unwrap_or(&empty);
        for d in o.get("dropped").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            self.diag(Diagnostic::warning("prop", format!("`{d}` on <{tag}> is ignored")));
        }
        let style = self.style(&tag, props.get("style"));
        let mut node = Map::new();
        node.insert("tagName".into(), json!(tag));
        if let Some(s) = style {
            node.insert("style".into(), s);
        }
        if let Some(tw) = props.get("tw").and_then(Value::as_str) {
            node.insert("tw".into(), json!(tw));
        }
        if let Some(p) = preset(&tag) {
            node.insert("preset".into(), p);
        }
        if tag == "img" {
            let src = props.get("src").and_then(Value::as_str).unwrap_or("");
            let Some(name) = src.strip_prefix(ASSET_PREFIX) else {
                let why = if src.is_empty() {
                    "<img> needs `src={assets.name}`".to_string()
                } else {
                    format!(
                        "<img src=\"{}\"> is not allowed: images come from `assets` (no network or files)",
                        short(src)
                    )
                };
                self.diag(Diagnostic::error("asset", why));
                return None;
            };
            if !(self.known_assets)(name) {
                self.diag(Diagnostic::error("asset", format!("asset `{name}` is not attached to this composition")));
                return None;
            }
            self.assets.insert(name.to_string());
            node.insert("type".into(), json!("image"));
            node.insert("src".into(), json!(src));
            for k in ["width", "height"] {
                if let Some(n) = props.get(k).and_then(Value::as_f64) {
                    node.insert(k.into(), json!(n));
                }
            }
            return Some(Value::Object(node));
        }
        if tag == "br" {
            return Some(json!({"type": "text", "text": "\n"}));
        }
        if !BLOCK.contains(&tag.as_str()) && !INLINE.contains(&tag.as_str()) {
            let hint = match tag.as_str() {
                "svg" | "path" | "circle" | "rect" => {
                    "draw shapes with styled <div>s (borderRadius, gradients) or an image asset"
                }
                "video" | "audio" | "iframe" | "canvas" | "script" | "style" | "link" => {
                    "not available in compositions"
                }
                _ => "it renders as a <div>",
            };
            self.diag(Diagnostic::warning("tag", format!("<{tag}> is not supported: {hint}")));
            if matches!(tag.as_str(), "script" | "style" | "link" | "iframe" | "video" | "audio" | "canvas") {
                return None;
            }
            node.insert("tagName".into(), json!("div"));
        }
        node.insert("type".into(), json!("container"));
        node.insert("children".into(), Value::Array(self.children(o)));
        Some(Value::Object(node))
    }

    fn style(&mut self, tag: &str, style: Option<&Value>) -> Option<Value> {
        let s = match style? {
            Value::Object(s) => s,
            Value::Null => return None,
            _ => {
                self.diag(Diagnostic::warning("style", format!("`style` on <{tag}> must be an object")));
                return None;
            }
        };
        let mut out = Map::new();
        for (k, v) in s {
            match property(k, v) {
                Property::Ok => {
                    if k == "fontFamily"
                        && let Some(f) = v.as_str()
                    {
                        for fam in f.split(',') {
                            let fam = fam.trim().trim_matches(|c| c == '"' || c == '\'');
                            if !fam.is_empty() {
                                self.families.insert(fam.to_string());
                            }
                        }
                    }
                    out.insert(k.clone(), v.clone());
                }
                Property::Unknown => {
                    self.diag(Diagnostic::warning("style", format!("style property `{k}` is not supported")))
                }
                Property::Invalid(e) => {
                    self.diag(Diagnostic::warning("style", format!("`{k}: {}`: {e}", short(&v.to_string()))))
                }
            }
        }
        Some(Value::Object(out))
    }
}

fn short(s: &str) -> String {
    if s.chars().count() > 60 { format!("{}…", s.chars().take(60).collect::<String>()) } else { s.to_string() }
}

enum Property {
    Ok,
    Unknown,
    Invalid(String),
}

/// Whether Takumi understands a style property and value. Property names are cached; values are parsed.
fn property(key: &str, value: &Value) -> Property {
    static KNOWN: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    static EMPTY: OnceLock<String> = OnceLock::new();
    let empty = EMPTY.get_or_init(|| format!("{:?}", Style::default()));
    let mut one = Map::new();
    one.insert(key.to_string(), value.clone());
    let parsed = serde_json::from_value::<Style>(Value::Object(one));
    let known = KNOWN.get_or_init(Default::default);
    match parsed {
        Err(e) => {
            let msg = e.to_string();
            let msg = msg.split(';').next().unwrap_or(&msg).to_string();
            Property::Invalid(msg)
        }
        Ok(style) => {
            let mut k = known.lock().expect("style cache");
            let is_known = *k.entry(key.to_string()).or_insert_with(|| format!("{style:?}") != *empty);
            if is_known { Property::Ok } else { Property::Unknown }
        }
    }
}

/// Builds the Takumi root for the composition's top-level nodes at `width`×`height` composition units.
pub fn build(nodes: &Value, width: u32, height: u32, known_assets: &dyn Fn(&str) -> bool) -> Built {
    let mut b = Builder { known_assets, assets: BTreeSet::new(), families: BTreeSet::new(), diags: Vec::new() };
    let children: Vec<Value> =
        nodes.as_array().map(|a| a.iter().filter_map(|n| b.node(n)).collect()).unwrap_or_default();
    let root = json!({
        "type": "container",
        "tagName": "div",
        "style": {
            "width": width, "height": height, "position": "relative", "overflow": "hidden",
            "fontFamily": crate::fonts::DEFAULT_FAMILY, "fontSize": 48, "color": "#ffffff",
        },
        "children": children,
    });
    Built { root, assets: b.assets, families: b.families, diagnostics: b.diags }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_elements_and_reports_problems() {
        let nodes = json!([
            {"tag": "div", "props": {"style": {"opacity": 0.5, "fooBar": 1, "width": "banana"}}, "children": ["hi", {"tag": "b", "props": {}, "children": ["x"], "dropped": []}], "dropped": ["onClick"]},
            {"tag": "img", "props": {"src": "asset:logo", "width": 10}, "children": [], "dropped": []},
            {"tag": "img", "props": {"src": "https://example.com/a.png"}, "children": [], "dropped": []},
            {"tag": "img", "props": {"src": "asset:missing"}, "children": [], "dropped": []},
            {"tag": "svg", "props": {}, "children": [], "dropped": []},
            {"tag": "script", "props": {}, "children": ["x"], "dropped": []},
        ]);
        let b = build(&nodes, 1920, 1080, &|n| n == "logo");
        let kids = b.root["children"].as_array().unwrap();
        assert_eq!(kids.len(), 3, "{kids:?}");
        assert_eq!(kids[0]["style"], json!({"opacity": 0.5}));
        assert_eq!(kids[0]["children"][1]["preset"]["fontWeight"], 700);
        assert_eq!(kids[1]["type"], "image");
        assert_eq!(kids[2]["tagName"], "div");
        assert!(b.assets.contains("logo"));
        let msgs: Vec<_> = b.diagnostics.iter().map(|d| format!("{}:{}", d.code, d.message)).collect();
        for needle in ["fooBar", "banana", "onClick", "example.com", "missing", "<svg>"] {
            assert!(msgs.iter().any(|m| m.contains(needle)), "{needle} in {msgs:?}");
        }
        // The whole tree deserialises as a Takumi node.
        serde_json::from_value::<takumi::prelude::Node>(b.root).unwrap();
    }
}
