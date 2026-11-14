//! The 21 caption styles (docs/spec/presets.md, "Caption styles"). Each is our own design: font, size and weight,
//! colours, how the spoken word is marked, placement, case, spacing and the animation that paces the words.
//! Sizes are in reference pixels (the project's reference canvas, e.g. 1080 × 1920 for 9:16).

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anim {
    /// The whole caption is shown; the spoken word is marked.
    Word,
    /// As `Word`, and the spoken word pops (scales up with a little overshoot).
    WordPop,
    /// The spoken word is punched in with a quick pulse.
    Pulse,
    /// Words appear as they are spoken, rising into place.
    Reveal,
    /// Short groups of words, one per line, each appearing as it is spoken.
    Stacked,
    /// Every word but the spoken one is dimmed.
    Spotlight,
}

impl Anim {
    pub fn id(self) -> &'static str {
        match self {
            Anim::Word => "word",
            Anim::WordPop => "word-pop",
            Anim::Pulse => "pulse",
            Anim::Reveal => "reveal",
            Anim::Stacked => "stacked",
            Anim::Spotlight => "spotlight",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    /// The spoken word keeps the text colour.
    None,
    /// The spoken word changes colour.
    Color([u8; 4]),
    /// The spoken word sits on a rounded box (fill, text colour).
    Box([u8; 4], [u8; 4]),
    /// A hand-drawn ring around the spoken word.
    Ring([u8; 4]),
    /// A bar under the spoken word.
    Underline([u8; 4]),
}

impl Mark {
    pub fn color(self) -> Option<[u8; 4]> {
        match self {
            Mark::None => None,
            Mark::Color(c) | Mark::Box(c, _) | Mark::Ring(c) | Mark::Underline(c) => Some(c),
        }
    }

    /// The same mark in another colour (the item's highlight colour overrides the preset's).
    pub fn with_color(self, c: [u8; 4]) -> Mark {
        match self {
            Mark::None | Mark::Color(_) => Mark::Color(c),
            Mark::Box(_, t) => Mark::Box(c, t),
            Mark::Ring(_) => Mark::Ring(c),
            Mark::Underline(_) => Mark::Underline(c),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Top,
    Center,
    Bottom,
}

impl Position {
    pub fn parse(s: &str) -> Option<Position> {
        Some(match s {
            "top" => Position::Top,
            "center" | "middle" => Position::Center,
            "bottom" => Position::Bottom,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Position::Top => "top",
            Position::Center => "center",
            Position::Bottom => "bottom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    AsWritten,
    Upper,
    Lower,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
}

/// A box behind the whole caption (or behind each word for `per_word`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Panel {
    pub fill: [u8; 4],
    /// Hard offset shadow (colour, dx, dy in em).
    pub shadow: Option<([u8; 4], f32, f32)>,
    /// Corner radius in em.
    pub radius: f32,
    /// One box per word, tilted alternately (cut-out paper), cycling through `fill` and `alt`.
    pub per_word: bool,
    pub alt: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptionPreset {
    pub id: &'static str,
    pub label: &'static str,
    /// Font family class: sans, serif, display, script, condensed. Rendered with the bundled faces that fit.
    pub family: &'static str,
    pub size: f32,
    pub weight: u16,
    pub color: [u8; 4],
    pub mark: Mark,
    pub position: Position,
    /// Vertical offset from the position's anchor, in reference pixels (negative = up).
    pub offset_y: f32,
    pub align: Align,
    pub line_height: f32,
    /// In em.
    pub letter_spacing: f32,
    pub case: Case,
    pub anim: Anim,
    /// How long a word takes to appear or pop, in milliseconds.
    pub reveal_ms: u32,
    /// (colour, width in em)
    pub stroke: Option<([u8; 4], f32)>,
    /// (colour, dx, dy in em)
    pub shadow: Option<([u8; 4], f32, f32)>,
    pub panel: Option<Panel>,
    /// Words per group for stacked styles.
    pub group: usize,
    /// In stacked groups the longest word is drawn this much larger (1 = no emphasis).
    pub emphasis: f32,
    /// Opacity of words that are not being spoken (spotlight and quiet styles).
    pub dim: f32,
    /// Wrap width as a fraction of the frame width.
    pub max_width: f32,
    /// Font scale on landscape canvases (big stacked styles are sized for vertical video).
    pub landscape_scale: f32,
    /// Drawn behind the subject when a subject mask is available; otherwise in front.
    pub behind_subject: bool,
}

const fn rgb(r: u8, g: u8, b: u8) -> [u8; 4] {
    [r, g, b, 255]
}

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    [r, g, b, a]
}

const WHITE: [u8; 4] = rgb(255, 255, 255);
const INK: [u8; 4] = rgb(17, 17, 17);

const BASE: CaptionPreset = CaptionPreset {
    id: "",
    label: "",
    family: "sans",
    size: 56.0,
    weight: 600,
    color: WHITE,
    mark: Mark::None,
    position: Position::Bottom,
    offset_y: 0.0,
    align: Align::Center,
    line_height: 1.15,
    letter_spacing: 0.0,
    case: Case::AsWritten,
    anim: Anim::Word,
    reveal_ms: 160,
    stroke: None,
    shadow: None,
    panel: None,
    group: 3,
    emphasis: 1.0,
    dim: 1.0,
    max_width: 0.84,
    landscape_scale: 1.0,
    behind_subject: false,
};

/// Stacked, behind-the-subject title styles share this base.
const STACK: CaptionPreset = CaptionPreset {
    position: Position::Center,
    anim: Anim::Stacked,
    line_height: 0.98,
    reveal_ms: 140,
    group: 3,
    landscape_scale: 0.62,
    behind_subject: true,
    ..BASE
};

pub const DEFAULT_PRESET: &str = "butter-serif";

pub static PRESETS: [CaptionPreset; 21] = [
    CaptionPreset {
        id: "landmark",
        label: "Landmark",
        family: "display",
        size: 118.0,
        weight: 400,
        mark: Mark::Color(rgb(242, 193, 78)),
        case: Case::Upper,
        letter_spacing: 0.02,
        line_height: 0.92,
        shadow: Some((rgba(0, 0, 0, 150), 0.0, 0.04)),
        ..STACK
    },
    CaptionPreset {
        id: "margin-type",
        label: "Margin Type",
        family: "sans",
        size: 108.0,
        weight: 700,
        color: rgb(244, 241, 234),
        mark: Mark::Color(rgb(255, 107, 90)),
        case: Case::Lower,
        align: Align::Left,
        letter_spacing: -0.02,
        shadow: Some((rgba(0, 0, 0, 120), 0.0, 0.03)),
        ..STACK
    },
    CaptionPreset {
        id: "poster-stack",
        label: "Poster Stack",
        family: "condensed",
        size: 104.0,
        weight: 800,
        mark: Mark::Color(rgb(255, 225, 77)),
        case: Case::Upper,
        emphasis: 1.4,
        line_height: 0.9,
        shadow: Some((rgba(0, 0, 0, 200), 0.05, 0.05)),
        ..STACK
    },
    CaptionPreset {
        id: "soft-script",
        label: "Soft Script",
        family: "script",
        size: 112.0,
        weight: 700,
        color: rgb(255, 247, 240),
        mark: Mark::Color(rgb(255, 179, 199)),
        group: 2,
        shadow: Some((rgba(60, 20, 40, 150), 0.0, 0.05)),
        ..STACK
    },
    CaptionPreset {
        id: "chrome-italic",
        label: "Chrome Italic",
        family: "display",
        size: 116.0,
        weight: 900,
        color: rgb(230, 235, 242),
        mark: Mark::Color(rgb(154, 215, 255)),
        case: Case::Upper,
        group: 2,
        stroke: Some((rgb(20, 28, 40), 0.04)),
        shadow: Some((rgba(0, 0, 0, 160), 0.03, 0.06)),
        ..STACK
    },
    CaptionPreset {
        id: "scrapbook-cutout",
        label: "Scrapbook Cutout",
        family: "display",
        size: 106.0,
        weight: 900,
        color: INK,
        mark: Mark::Box(rgb(255, 90, 78), WHITE),
        case: Case::Upper,
        line_height: 1.12,
        panel: Some(Panel {
            fill: rgb(255, 243, 214),
            shadow: Some((rgba(0, 0, 0, 90), 0.03, 0.05)),
            radius: 0.04,
            per_word: true,
            alt: rgb(232, 201, 160),
        }),
        ..STACK
    },
    CaptionPreset {
        id: "marker-circle",
        label: "Marker Circle",
        family: "sans",
        size: 118.0,
        weight: 700,
        mark: Mark::Ring(rgb(255, 59, 48)),
        line_height: 1.08,
        shadow: Some((rgba(0, 0, 0, 140), 0.0, 0.04)),
        ..STACK
    },
    CaptionPreset {
        id: "neo-brutal",
        label: "Neo Brutal",
        family: "condensed",
        size: 108.0,
        weight: 400,
        color: INK,
        mark: Mark::Color(rgb(255, 61, 127)),
        case: Case::Upper,
        line_height: 1.0,
        panel: Some(Panel {
            fill: rgb(255, 230, 0),
            shadow: Some((INK, 0.08, 0.08)),
            radius: 0.0,
            per_word: false,
            alt: rgb(255, 230, 0),
        }),
        ..STACK
    },
    CaptionPreset {
        id: "hero-world-wall",
        label: "Hero Wall",
        family: "display",
        size: 104.0,
        weight: 400,
        mark: Mark::Color(rgb(124, 242, 200)),
        case: Case::Upper,
        letter_spacing: 0.08,
        emphasis: 1.3,
        shadow: Some((rgba(0, 0, 0, 170), 0.0, 0.06)),
        ..STACK
    },
    CaptionPreset {
        id: "Cascade",
        label: "Cascade",
        family: "sans",
        size: 50.0,
        weight: 500,
        position: Position::Center,
        mark: Mark::Color(rgb(165, 180, 255)),
        anim: Anim::Reveal,
        reveal_ms: 260,
        shadow: Some((rgba(0, 0, 0, 150), 0.0, 0.06)),
        max_width: 0.7,
        ..BASE
    },
    CaptionPreset {
        id: "rapid-reveal",
        label: "Rapid Reveal",
        family: "condensed",
        size: 96.0,
        weight: 800,
        position: Position::Center,
        mark: Mark::Color(rgb(255, 212, 0)),
        case: Case::Upper,
        anim: Anim::Reveal,
        reveal_ms: 80,
        line_height: 1.0,
        stroke: Some((INK, 0.07)),
        landscape_scale: 0.75,
        ..BASE
    },
    CaptionPreset {
        id: "stacked-emphasis",
        label: "Stacked Emphasis",
        family: "sans",
        size: 56.0,
        weight: 700,
        position: Position::Center,
        mark: Mark::Color(rgb(79, 209, 197)),
        anim: Anim::Stacked,
        group: 4,
        emphasis: 1.6,
        line_height: 1.0,
        shadow: Some((rgba(0, 0, 0, 160), 0.0, 0.05)),
        ..BASE
    },
    CaptionPreset {
        id: "word-pop",
        label: "Word Pop",
        family: "sans",
        size: 88.0,
        weight: 600,
        position: Position::Center,
        mark: Mark::Color(rgb(60, 255, 122)),
        anim: Anim::WordPop,
        reveal_ms: 180,
        stroke: Some((INK, 0.08)),
        landscape_scale: 0.8,
        ..BASE
    },
    CaptionPreset {
        id: "pulse-cut",
        label: "Pulse Cut",
        family: "condensed",
        size: 62.0,
        weight: 850,
        mark: Mark::Box(rgb(255, 45, 85), WHITE),
        case: Case::Upper,
        anim: Anim::Pulse,
        reveal_ms: 140,
        stroke: Some((INK, 0.05)),
        ..BASE
    },
    CaptionPreset {
        id: "beast-mode",
        label: "Beast Mode",
        family: "display",
        size: 72.0,
        weight: 400,
        position: Position::Center,
        color: rgb(255, 250, 230),
        mark: Mark::Color(rgb(57, 255, 20)),
        case: Case::Upper,
        anim: Anim::WordPop,
        reveal_ms: 120,
        stroke: Some((INK, 0.11)),
        shadow: Some((INK, 0.04, 0.07)),
        ..BASE
    },
    CaptionPreset {
        id: "butter-serif",
        label: "Butter Serif",
        family: "serif",
        size: 52.0,
        weight: 400,
        color: rgb(255, 244, 214),
        mark: Mark::Color(rgb(255, 209, 102)),
        shadow: Some((rgba(30, 20, 0, 170), 0.0, 0.06)),
        stroke: Some((rgba(30, 20, 0, 120), 0.03)),
        ..BASE
    },
    CaptionPreset {
        id: "minimal-modern",
        label: "Minimal Modern",
        family: "sans",
        size: 46.0,
        weight: 300,
        mark: Mark::Underline(WHITE),
        letter_spacing: 0.01,
        dim: 0.72,
        shadow: Some((rgba(0, 0, 0, 110), 0.0, 0.04)),
        ..BASE
    },
    CaptionPreset {
        id: "spotlight",
        label: "Spotlight",
        family: "sans",
        size: 60.0,
        weight: 700,
        position: Position::Center,
        mark: Mark::Box(WHITE, INK),
        anim: Anim::Spotlight,
        dim: 0.38,
        shadow: Some((rgba(0, 0, 0, 140), 0.0, 0.05)),
        ..BASE
    },
    CaptionPreset {
        id: "classic",
        label: "Classic",
        family: "sans",
        size: 40.0,
        weight: 500,
        panel: Some(Panel {
            fill: rgba(0, 0, 0, 160),
            shadow: None,
            radius: 0.18,
            per_word: false,
            alt: rgba(0, 0, 0, 160),
        }),
        ..BASE
    },
    CaptionPreset {
        id: "subtitle",
        label: "Subtitle",
        family: "sans",
        size: 40.0,
        weight: 500,
        color: rgb(255, 246, 196),
        stroke: Some((INK, 0.07)),
        shadow: Some((rgba(0, 0, 0, 160), 0.0, 0.05)),
        ..BASE
    },
    CaptionPreset {
        id: "bold",
        label: "Bold",
        family: "sans",
        size: 60.0,
        weight: 800,
        mark: Mark::Color(rgb(255, 212, 0)),
        stroke: Some((INK, 0.08)),
        ..BASE
    },
];

/// The preset with this id (case-insensitive), if any.
pub fn preset(id: &str) -> Option<&'static CaptionPreset> {
    PRESETS.iter().find(|p| p.id.eq_ignore_ascii_case(id))
}

pub fn hex(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}

impl CaptionPreset {
    /// A summary for the UI (gallery previews) and the assistant.
    pub fn to_json(&self) -> Value {
        let mark = match self.mark {
            Mark::None => json!({"kind": "none"}),
            Mark::Color(c) => json!({"kind": "color", "color": hex(c)}),
            Mark::Box(c, t) => json!({"kind": "box", "color": hex(c), "text": hex(t)}),
            Mark::Ring(c) => json!({"kind": "ring", "color": hex(c)}),
            Mark::Underline(c) => json!({"kind": "underline", "color": hex(c)}),
        };
        json!({
            "id": self.id,
            "label": self.label,
            "family": self.family,
            "fontSize": self.size,
            "fontWeight": self.weight,
            "textColor": hex(self.color),
            "highlightColor": self.mark.color().map(hex),
            "mark": mark,
            "position": self.position.id(),
            "align": match self.align { Align::Left => "left", Align::Center => "center" },
            "uppercase": self.case == Case::Upper,
            "lowercase": self.case == Case::Lower,
            "letterSpacing": self.letter_spacing,
            "animation": self.anim.id(),
            "revealMs": self.reveal_ms,
            "stroke": self.stroke.map(|(c, w)| json!({"color": hex(c), "width": w})),
            "shadow": self.shadow.map(|(c, x, y)| json!({"color": hex(c), "x": x, "y": y})),
            "panel": self.panel.map(|p| json!({"color": hex(p.fill), "perWord": p.per_word, "radius": p.radius})),
            "composition": if self.behind_subject { "behind-subject" } else { "front" },
        })
    }
}

/// All presets as JSON, in gallery order.
pub fn presets_json() -> Value {
    Value::Array(PRESETS.iter().map(CaptionPreset::to_json).collect())
}
