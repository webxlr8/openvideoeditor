//! Built-in motion templates: TSX compositions (`templates/*.tsx`) with a prop schema the UI turns into a form.

use serde::Serialize;
use serde_json::{Value, json};

/// Kind of an editable template prop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PropKind {
    Text,
    Multiline,
    Color,
    Number,
    Boolean,
    Select,
    /// A media id of an image in the project; the composition sees it as `assets.<name>`.
    Image,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: PropKind,
    pub default: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Suggested item duration in seconds.
    pub duration: f64,
    /// Whether the template paints the whole frame (otherwise it overlays video).
    pub full_frame: bool,
    pub props: Vec<PropSpec>,
    #[serde(skip)]
    pub source: &'static str,
}

fn p(name: &'static str, label: &'static str, kind: PropKind, default: Value) -> PropSpec {
    PropSpec { name, label, kind, default, min: None, max: None, step: None, options: Vec::new() }
}

fn num(name: &'static str, label: &'static str, default: f64, min: f64, max: f64, step: f64) -> PropSpec {
    PropSpec { min: Some(min), max: Some(max), step: Some(step), ..p(name, label, PropKind::Number, json!(default)) }
}

fn select(name: &'static str, label: &'static str, default: &'static str, options: &[&'static str]) -> PropSpec {
    PropSpec { options: options.to_vec(), ..p(name, label, PropKind::Select, json!(default)) }
}

use PropKind::*;

/// All built-in templates, in gallery order.
pub fn all() -> Vec<Template> {
    vec![
        Template {
            id: "lower-third",
            name: "Lower third",
            description: "Name and role beside a growing accent bar",
            duration: 5.0,
            full_frame: false,
            props: vec![
                p("name", "Name", Text, json!("Alex Morgan")),
                p("role", "Role", Text, json!("Product designer")),
                p("accent", "Accent", Color, json!("#ffb020")),
                p("color", "Text", Color, json!("#ffffff")),
                p("plate", "Plate", Color, json!("rgba(12,14,22,0.6)")),
                select("side", "Side", "left", &["left", "right"]),
            ],
            source: include_str!("../templates/lower-third.tsx"),
        },
        Template {
            id: "title-card",
            name: "Title card",
            description: "Gradient background with a letter-by-letter title",
            duration: 4.0,
            full_frame: true,
            props: vec![
                p("title", "Title", Text, json!("Chapter One")),
                p("subtitle", "Subtitle", Text, json!("Where it all began")),
                p("color", "Text", Color, json!("#ffffff")),
                p("from", "Gradient start", Color, json!("#1d2b64")),
                p("to", "Gradient end", Color, json!("#7b2ff7")),
            ],
            source: include_str!("../templates/title-card.tsx"),
        },
        Template {
            id: "counter",
            name: "Counter",
            description: "A number counting up to a target",
            duration: 4.0,
            full_frame: false,
            props: vec![
                num("from", "From", 0.0, -1e9, 1e9, 1.0),
                num("to", "To", 12500.0, -1e9, 1e9, 1.0),
                num("seconds", "Count for (s)", 2.0, 0.3, 30.0, 0.1),
                num("decimals", "Decimals", 0.0, 0.0, 4.0, 1.0),
                p("prefix", "Prefix", Text, json!("")),
                p("suffix", "Suffix", Text, json!("+")),
                p("label", "Caption", Text, json!("happy customers")),
                p("color", "Colour", Color, json!("#ffffff")),
            ],
            source: include_str!("../templates/counter.tsx"),
        },
        Template {
            id: "progress-bar",
            name: "Progress bar",
            description: "A bar that fills over the item's duration",
            duration: 10.0,
            full_frame: false,
            props: vec![
                p("label", "Label", Text, json!("")),
                p("color", "Fill", Color, json!("#ff4d6d")),
                p("track", "Track", Color, json!("rgba(255,255,255,0.25)")),
                select("position", "Position", "bottom", &["bottom", "top"]),
                num("thickness", "Thickness", 12.0, 2.0, 80.0, 1.0),
                p("showPercent", "Show percentage", Boolean, json!(false)),
            ],
            source: include_str!("../templates/progress-bar.tsx"),
        },
        Template {
            id: "quote-card",
            name: "Quote card",
            description: "A card with a quote revealed word by word",
            duration: 6.0,
            full_frame: false,
            props: vec![
                p("quote", "Quote", Multiline, json!("Simple things should be simple, complex things should be possible.")),
                p("author", "Attribution", Text, json!("— A wise engineer")),
                p("card", "Card", Color, json!("#fdf7ee")),
                p("color", "Text", Color, json!("#1d1b19")),
                p("accent", "Accent", Color, json!("#d9480f")),
            ],
            source: include_str!("../templates/quote-card.tsx"),
        },
        Template {
            id: "logo-sting",
            name: "Logo sting",
            description: "Logo springs in with a ring burst and tagline",
            duration: 3.0,
            full_frame: true,
            props: vec![
                p("logo", "Logo image", Image, Value::Null),
                p("tagline", "Tagline", Text, json!("MAKE SOMETHING GOOD")),
                p("background", "Background", Color, json!("#0d0f1a")),
                p("accent", "Accent", Color, json!("#3ddc97")),
                p("color", "Text", Color, json!("#ffffff")),
            ],
            source: include_str!("../templates/logo-sting.tsx"),
        },
        Template {
            id: "kinetic-text",
            name: "Kinetic text",
            description: "Words spring up one by one with highlights",
            duration: 4.0,
            full_frame: false,
            props: vec![
                p("text", "Text", Multiline, json!("Ideas are cheap. Shipping them is everything.")),
                p("color", "Text", Color, json!("#ffffff")),
                p("highlight", "Highlight", Color, json!("#ffd23f")),
                num("every", "Highlight every Nth word", 3.0, 0.0, 10.0, 1.0),
                num("size", "Size", 110.0, 30.0, 260.0, 1.0),
                p("uppercase", "Uppercase", Boolean, json!(true)),
            ],
            source: include_str!("../templates/kinetic-text.tsx"),
        },
        Template {
            id: "end-card",
            name: "End card",
            description: "Heading, two next-video frames and a call to action",
            duration: 8.0,
            full_frame: true,
            props: vec![
                p("heading", "Heading", Text, json!("Thanks for watching")),
                p("first", "First frame", Text, json!("Next episode")),
                p("second", "Second frame", Text, json!("Best of the season")),
                p("cta", "Call to action", Text, json!("Subscribe")),
                p("from", "Gradient start", Color, json!("#141e30")),
                p("to", "Gradient end", Color, json!("#243b55")),
                p("accent", "Accent", Color, json!("#ff5e5b")),
                p("color", "Text", Color, json!("#ffffff")),
                p("ctaColor", "Button text", Color, json!("#ffffff")),
            ],
            source: include_str!("../templates/end-card.tsx"),
        },
    ]
}

pub fn get(id: &str) -> Option<Template> {
    all().into_iter().find(|t| t.id == id)
}

/// Template defaults overlaid with `props`; image props are returned separately as asset name → media id.
pub fn resolve_props(t: &Template, props: &serde_json::Map<String, Value>) -> (Value, Vec<(String, String)>) {
    let mut out = serde_json::Map::new();
    let mut images = Vec::new();
    for spec in &t.props {
        let v = props.get(spec.name).cloned().unwrap_or_else(|| spec.default.clone());
        if spec.kind == PropKind::Image {
            if let Some(id) = v.as_str().filter(|s| !s.is_empty()) {
                images.push((spec.name.to_string(), id.to_string()));
            }
            continue;
        }
        out.insert(spec.name.to_string(), v);
    }
    (Value::Object(out), images)
}
