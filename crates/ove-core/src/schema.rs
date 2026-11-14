//! The field registry: one declarative description of every known field of tracks, items, media entries and
//! the project. Validation, normalisation (default omission), the generated JSON Schema, the schema document,
//! the projection's attribute order and the generated `timeline.d.ts` typings are all derived from it, so there is
//! one source of truth (design D5/D8).

use serde_json::{Map, Value};

use crate::time::{json_to_micros, micros_to_json};

/// The kind of value a field holds.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// Any string.
    Str,
    /// `true` / `false`.
    Bool,
    /// A finite number, optionally range-checked (inclusive).
    Num { min: Option<f64>, max: Option<f64> },
    /// An integer, optionally range-checked (inclusive).
    Int { min: Option<i64>, max: Option<i64> },
    /// Seconds, finite and non-negative, stored at microsecond precision.
    Time,
    /// Seconds that may be negative (offsets), stored at microsecond precision.
    SignedTime,
    /// One of a fixed list of strings.
    Enum(&'static [&'static str]),
    /// An object with known keys. `open` objects accept extra keys silently (preset parameters); closed objects
    /// keep unknown keys but report a warning.
    Obj { fields: &'static [Field], open: bool },
    /// An array whose elements all have one kind.
    Arr(&'static Kind),
    /// Any JSON value (validated by a later change that owns it).
    Any,
}

/// A default value. Fields equal to their default are omitted from projections.
#[derive(Debug, Clone, Copy)]
pub enum Def {
    Num(f64),
    Bool(bool),
    Str(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub name: &'static str,
    pub kind: Kind,
    pub default: Option<Def>,
    pub doc: &'static str,
}

const fn f(name: &'static str, kind: Kind, doc: &'static str) -> Field {
    Field { name, kind, default: None, doc }
}
const fn fd(name: &'static str, kind: Kind, default: Def, doc: &'static str) -> Field {
    Field { name, kind, default: Some(default), doc }
}
const fn num(min: f64, max: f64) -> Kind {
    Kind::Num { min: Some(min), max: Some(max) }
}
const NUM: Kind = Kind::Num { min: None, max: None };
const NONNEG: Kind = Kind::Num { min: Some(0.0), max: None };

// ---------------------------------------------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------------------------------------------

pub const TRACK_TYPES: &[&str] = &["video", "audio", "caption", "adjustment", "layout"];
pub const ITEM_TYPES: &[&str] = &["video", "image", "audio", "text", "caption", "code", "adjustment", "shader"];
pub const ASPECT_RATIOS: &[&str] = &["16:9", "4:3", "3:4", "9:16", "1:1", "4:5"];
pub const BLEND_MODES: &[&str] = &[
    "normal",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color-dodge",
    "color-burn",
    "hard-light",
    "soft-light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
];
pub const EASINGS: &[&str] = &["linear", "ease-in", "ease-out", "ease-in-out"];
pub const KEYFRAME_PROPERTIES: &[&str] = &["x", "y", "scale", "rotation", "opacity", "gaussianBlur", "borderRadius"];
pub const PLAYBACK_DIRECTIONS: &[&str] = &["forward", "reverse"];
pub const TEXT_ALIGNS: &[&str] = &["left", "center", "right", "justify"];
pub const FONT_STYLES: &[&str] = &["normal", "italic"];
pub const CAPTION_MODES: &[&str] = &["front", "behind-subject"];
pub const CAPTION_FALLBACKS: &[&str] = &["front", "hide"];
pub const PROBE_STATUSES: &[&str] = &["awaiting-probe", "probed", "failed"];
pub const LUT_STAGES: &[&str] = &["input", "look"];
pub const SHADER_REGIONS: &[&str] = &["all", "subject", "background"];
pub const TRANSITION_DIRECTIONS: &[&str] = &["left", "right", "up", "down"];
pub const LAYOUT_PRESETS: &[&str] = &[
    "split-horizontal",
    "split-vertical",
    "3up-horizontal",
    "3up-vertical",
    "4up-horizontal",
    "4up-vertical",
    "grid-2x2",
    "pip-bottom-right",
    "pip-bottom-left",
    "pip-top-right",
    "pip-top-left",
    "hero-top",
    "hero-bottom",
    "hero-left",
    "hero-right",
];

// ---------------------------------------------------------------------------------------------------------------
// Nested objects
// ---------------------------------------------------------------------------------------------------------------

const TRANSFORM: &[Field] = &[
    fd("x", NUM, Def::Num(0.0), "Horizontal offset in pixels from the frame centre."),
    fd("y", NUM, Def::Num(0.0), "Vertical offset in pixels from the frame centre."),
    fd("scale", Kind::Num { min: Some(0.0), max: None }, Def::Num(1.0), "Uniform scale factor."),
    fd("rotation", NUM, Def::Num(0.0), "Rotation in degrees, clockwise."),
    fd("flipX", Kind::Bool, Def::Bool(false), "Mirror horizontally."),
];

const CROP: &[Field] = &[
    fd("top", num(0.0, 100.0), Def::Num(0.0), "Percent of height removed from the top."),
    fd("right", num(0.0, 100.0), Def::Num(0.0), "Percent of width removed from the right."),
    fd("bottom", num(0.0, 100.0), Def::Num(0.0), "Percent of height removed from the bottom."),
    fd("left", num(0.0, 100.0), Def::Num(0.0), "Percent of width removed from the left."),
];

const KEYFRAME: &[Field] =
    &[f("time", Kind::Time, "Seconds from the item's start."), f("value", NUM, "Property value at that time.")];
const KEYFRAME_KIND: Kind = Kind::Obj { fields: KEYFRAME, open: false };
const KEYFRAME_TRACK: &[Field] = &[
    f("property", Kind::Enum(KEYFRAME_PROPERTIES), "Animated property."),
    f("keyframes", Kind::Arr(&KEYFRAME_KIND), "Keyframes in time order."),
    fd("easing", Kind::Enum(EASINGS), Def::Str("linear"), "Interpolation between keyframes."),
];
const KEYFRAME_TRACK_KIND: Kind = Kind::Obj { fields: KEYFRAME_TRACK, open: false };

const TRANSITION: &[Field] = &[
    f("type", Kind::Str, "Built-in transition type."),
    f("presetId", Kind::Str, "Transition preset id."),
    f("duration", Kind::Time, "Seconds."),
    f("easing", Kind::Enum(EASINGS), "Easing curve."),
    f("direction", Kind::Enum(TRANSITION_DIRECTIONS), "Direction of travel for wipe, push and fly."),
];

const LAYOUT: &[Field] = &[
    f("preset", Kind::Enum(LAYOUT_PRESETS), "Layout preset."),
    fd("slot", Kind::Num { min: Some(0.0), max: None }, Def::Num(0.0), "Slot index within the preset."),
];

const EFFECT: &[Field] = &[f("id", Kind::Str, "Effect instance id."), f("type", Kind::Str, "Effect preset type.")];
const EFFECT_KIND: Kind = Kind::Obj { fields: EFFECT, open: true };

const CAPTION_COMPOSITION: &[Field] = &[
    fd("mode", Kind::Enum(CAPTION_MODES), Def::Str("front"), "Draw in front of or behind the subject."),
    fd("fallback", Kind::Enum(CAPTION_FALLBACKS), Def::Str("front"), "When no subject matte is available."),
];

const FADE: &[Field] =
    &[f("duration", Kind::Time, "Seconds."), fd("easing", Kind::Enum(EASINGS), Def::Str("linear"), "Fade curve.")];

const COLOR_GRADING: &[Field] = &[
    fd("exposure", NUM, Def::Num(0.0), "Exposure adjustment."),
    fd("brightness", NUM, Def::Num(0.0), "Brightness adjustment."),
    fd("contrast", NUM, Def::Num(0.0), "Contrast adjustment."),
    fd("highlights", NUM, Def::Num(0.0), "Highlights adjustment."),
    fd("shadows", NUM, Def::Num(0.0), "Shadows adjustment."),
    fd("temperature", NUM, Def::Num(0.0), "White-balance temperature shift."),
    fd("tint", NUM, Def::Num(0.0), "White-balance tint shift."),
    fd("saturation", NUM, Def::Num(0.0), "Saturation adjustment."),
    fd("vibrance", NUM, Def::Num(0.0), "Vibrance adjustment."),
    fd("gamma", NUM, Def::Num(0.0), "Gamma adjustment."),
];
const LUT: &[Field] = &[
    f("preset", Kind::Str, "LUT preset id."),
    f("file", Kind::Str, "Path to a .cube file (copied into the bundle on apply)."),
    fd("intensity", num(0.0, 1.0), Def::Num(1.0), "Blend amount."),
    fd("stage", Kind::Enum(LUT_STAGES), Def::Str("look"), "Where in the grade the LUT applies."),
];
const COLOR_GRADE: &[Field] = &[
    f("wheels", Kind::Any, "Lift/gamma/gain/offset wheels, each {r,g,b,y}."),
    f("curves", Kind::Any, "master, red, green, blue, hueVsSat, hueVsHue, hueVsLum, lumVsSat curves."),
    f("saturation", NUM, "Global saturation."),
    f("lut", Kind::Obj { fields: LUT, open: false }, "LUT applied in the grade."),
    f("match", Kind::Any, "Reference match; fitted on apply and replaced by concrete values."),
];

const TEXT_ANIMATION: &[Field] = &[f("preset", Kind::Str, "Text animation preset id.")];

const WORD_TIMING: &[Field] =
    &[f("word", Kind::Str, "The word."), f("start", Kind::Time, "Seconds."), f("end", Kind::Time, "Seconds.")];
const WORD_TIMING_KIND: Kind = Kind::Obj { fields: WORD_TIMING, open: false };
const CAPTION_SEGMENT: &[Field] = &[
    f("id", Kind::Str, "Segment id."),
    f("text", Kind::Str, "Segment text."),
    f("startTime", Kind::Time, "Seconds, item-relative."),
    f("endTime", Kind::Time, "Seconds, item-relative."),
    f("wordTimings", Kind::Arr(&WORD_TIMING_KIND), "Per-word timings."),
];
const CAPTION_SEGMENT_KIND: Kind = Kind::Obj { fields: CAPTION_SEGMENT, open: false };

const CODE_COMPOSITION: &[Field] = &[
    f("source", Kind::Str, "JSX source: a pure function of {time, width, height, assets}."),
    f("template", Kind::Str, "Template id (instead of source)."),
    f("props", Kind::Any, "Template props."),
    f("width", Kind::Num { min: Some(1.0), max: None }, "Composition width in pixels."),
    f("height", Kind::Num { min: Some(1.0), max: None }, "Composition height in pixels."),
    f("assets", Kind::Any, "Named assets: name → src or {src, in, loop}."),
];

const SHADER_COMPOSITION: &[Field] = &[
    f("source", Kind::Str, "Shader source."),
    f("template", Kind::Str, "Shader template id (instead of source)."),
    f("params", Kind::Any, "Parameter values."),
    f("controls", Kind::Any, "Control definitions {label, min, max, step, format, toggle}."),
    f("amount", NUM, "Overall amount."),
    f("fadeIn", Kind::Time, "Seconds."),
    f("fadeOut", Kind::Time, "Seconds."),
    f("region", Kind::Enum(SHADER_REGIONS), "Where the shader applies."),
    f("defaults", Kind::Any, "Template defaults."),
];

// ---------------------------------------------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------------------------------------------

/// Typed head fields every item has. They are stored typed in [`crate::model::Item`], not in the field map.
pub const ITEM_HEAD: &[Field] = &[
    f("id", Kind::Str, "Unique item id."),
    f("trackId", Kind::Str, "Id of the track the item is on."),
    f("type", Kind::Enum(ITEM_TYPES), "Item type."),
    f("startTime", Kind::Time, "Timeline start, seconds."),
    f("endTime", Kind::Time, "Timeline end, seconds (greater than startTime)."),
];

const IDENTITY: &[Field] = &[fd("name", Kind::Str, Def::Str(""), "Display name.")];

const TIMING: &[Field] = &[
    fd("trimStart", Kind::Time, Def::Num(0.0), "Seconds trimmed from the start of the source."),
    fd("trimEnd", Kind::Time, Def::Num(0.0), "Seconds trimmed from the end of the source."),
];

const MEDIA: &[Field] = &[
    f("mediaItemId", Kind::Str, "Id of the media item the item plays."),
    fd("pending", Kind::Bool, Def::Bool(false), "Placeholder awaiting generated or supplied media."),
    f("pendingDescription", Kind::Str, "What the pending placeholder should become."),
];

const LOOK: &[Field] = &[
    f("transform", Kind::Obj { fields: TRANSFORM, open: false }, "Position, scale, rotation, flip."),
    f("crop", Kind::Obj { fields: CROP, open: false }, "Crop in percent per edge."),
    fd("opacity", num(0.0, 1.0), Def::Num(1.0), "Opacity."),
    fd("hidden", Kind::Bool, Def::Bool(false), "Hidden items render nothing."),
    fd("zIndex", NUM, Def::Num(0.0), "Stacking order within the track."),
    fd("blendMode", Kind::Enum(BLEND_MODES), Def::Str("normal"), "Blend mode."),
    fd("borderRadius", NONNEG, Def::Num(0.0), "Corner radius in pixels."),
    fd("gaussianBlur", NONNEG, Def::Num(0.0), "Blur radius in pixels."),
    f("keyframeTracks", Kind::Arr(&KEYFRAME_TRACK_KIND), "Animated properties."),
    f("transitionIn", Kind::Obj { fields: TRANSITION, open: true }, "Transition at the start."),
    f("transitionOut", Kind::Obj { fields: TRANSITION, open: true }, "Transition at the end."),
    f("effects", Kind::Arr(&EFFECT_KIND), "Effect stack."),
    f("layout", Kind::Obj { fields: LAYOUT, open: false }, "Layout slot the item fills (x, y, w, h from the preset)."),
    f("captionComposition", Kind::Obj { fields: CAPTION_COMPOSITION, open: false }, "Caption depth compositing."),
    fd("autoReframe", Kind::Bool, Def::Bool(true), "Fill the frame height with a centred crop when not transformed."),
];

const GRADE: &[Field] = &[
    f("colorGrading", Kind::Obj { fields: COLOR_GRADING, open: false }, "Basic colour adjustments."),
    f("colorGrade", Kind::Obj { fields: COLOR_GRADE, open: false }, "Wheels, curves, LUT and match."),
];

const SOUND: &[Field] = &[
    fd("volume", num(0.0, 200.0), Def::Num(100.0), "Volume in percent."),
    fd("muted", Kind::Bool, Def::Bool(false), "Muted."),
    f("audioFadeIn", Kind::Obj { fields: FADE, open: false }, "Audio fade at the start."),
    f("audioFadeOut", Kind::Obj { fields: FADE, open: false }, "Audio fade at the end."),
    fd("playbackSpeed", num(0.01, 100.0), Def::Num(1.0), "Playback speed factor."),
    fd("playbackDirection", Kind::Enum(PLAYBACK_DIRECTIONS), Def::Str("forward"), "Playback direction."),
];

const TEXT: &[Field] = &[
    f("textContent", Kind::Str, "The text."),
    f("fontFamily", Kind::Str, "Font family."),
    f("fontSize", Kind::Num { min: Some(0.0), max: None }, "Font size in pixels."),
    f("fontWeight", Kind::Any, "Font weight (number or keyword)."),
    fd("fontStyle", Kind::Enum(FONT_STYLES), Def::Str("normal"), "Font style."),
    f("color", Kind::Str, "Text colour."),
    fd("textAlign", Kind::Enum(TEXT_ALIGNS), Def::Str("center"), "Alignment."),
    f("lineHeight", NONNEG, "Line height multiplier."),
    f("letterSpacing", NUM, "Letter spacing in pixels."),
    f("strokeColor", Kind::Str, "Outline colour."),
    f("strokeWidth", NONNEG, "Outline width in pixels."),
    f("shadowColor", Kind::Str, "Shadow colour."),
    f("shadowBlur", NONNEG, "Shadow blur in pixels."),
    f("shadowOffsetX", NUM, "Shadow x offset in pixels."),
    f("shadowOffsetY", NUM, "Shadow y offset in pixels."),
    f("backgroundColor", Kind::Str, "Background box colour."),
    f("backgroundOpacity", num(0.0, 1.0), "Background box opacity."),
    f("backgroundPadding", NONNEG, "Background box padding in pixels."),
    f("textAnimation", Kind::Obj { fields: TEXT_ANIMATION, open: true }, "Text animation preset and parameters."),
];

const CAPTION: &[Field] = &[
    f("captionText", Kind::Str, "Full caption text."),
    f("captionSegments", Kind::Arr(&CAPTION_SEGMENT_KIND), "Timed segments with word timings."),
    f("captionPresetId", Kind::Str, "Caption style preset."),
    f("captionPosition", Kind::Any, "Caption position."),
    f("captionFontSize", Kind::Num { min: Some(0.0), max: None }, "Font size in pixels."),
    f("captionFontFamily", Kind::Str, "Font family."),
    f("captionTextColor", Kind::Str, "Text colour."),
    f("captionHighlightColor", Kind::Str, "Active-word highlight colour."),
];

const CODE: &[Field] = &[f(
    "codeComposition",
    Kind::Obj { fields: CODE_COMPOSITION, open: false },
    "Motion-graphics composition (pure JSX of {time, width, height, assets}).",
)];

const ADJUSTMENT: &[Field] = &[
    f("adjustmentKind", Kind::Str, "Kind of adjustment layer."),
    f("lutPresetId", Kind::Str, "LUT preset id."),
    fd("lutIntensity", num(0.0, 1.0), Def::Num(1.0), "LUT blend amount."),
];

const SHADER: &[Field] =
    &[f("shaderComposition", Kind::Obj { fields: SHADER_COMPOSITION, open: false }, "Shader layer definition.")];

fn build_item_fields(item_type: &str) -> Vec<&'static Field> {
    let mut groups: Vec<&'static [Field]> = vec![IDENTITY, TIMING, MEDIA];
    match item_type {
        "video" => groups.extend([LOOK, GRADE, SOUND]),
        "image" => groups.extend([LOOK, GRADE]),
        "audio" => groups.extend([LOOK, SOUND]),
        "text" => groups.extend([TEXT, LOOK]),
        "caption" => groups.extend([CAPTION, LOOK]),
        "code" => groups.extend([CODE, LOOK]),
        "adjustment" => groups.extend([ADJUSTMENT, LOOK, GRADE]),
        "shader" => groups.extend([SHADER, LOOK]),
        _ => groups.push(LOOK),
    }
    groups.into_iter().flat_map(|g| g.iter()).collect()
}

struct FieldTables {
    lists: std::collections::HashMap<&'static str, Vec<&'static Field>>,
    fallback: Vec<&'static Field>,
    index: std::collections::HashMap<(&'static str, &'static str), &'static Field>,
}

fn tables() -> &'static FieldTables {
    static T: std::sync::OnceLock<FieldTables> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut lists = std::collections::HashMap::new();
        let mut index = std::collections::HashMap::new();
        for t in ITEM_TYPES {
            let l = build_item_fields(t);
            for f in &l {
                index.insert((*t, f.name), *f);
            }
            lists.insert(*t, l);
        }
        FieldTables { lists, fallback: build_item_fields(""), index }
    })
}

/// Known non-head fields of an item type, in projection attribute order: identity, timing, media, content, look,
/// sound. Unknown fields follow, sorted by key.
pub fn item_fields(item_type: &str) -> &'static [&'static Field] {
    let t = tables();
    t.lists.get(item_type).unwrap_or(&t.fallback)
}

/// Looks up a known field of an item type.
pub fn item_field(item_type: &str, name: &str) -> Option<&'static Field> {
    let t = tables();
    match t.lists.get_key_value(item_type) {
        Some((k, _)) => t.index.get(&(*k, name)).copied(),
        None => t.fallback.iter().find(|f| f.name == name).copied(),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tracks, media, project
// ---------------------------------------------------------------------------------------------------------------

pub const TRACK_HEAD: &[Field] = &[
    f("id", Kind::Str, "Unique track id."),
    f("name", Kind::Str, "Display name."),
    f("order", Kind::Int { min: Some(0), max: None }, "Position; 0 is the top layer. Unique and contiguous."),
    f("type", Kind::Enum(TRACK_TYPES), "Track type."),
];

pub const TRACK_FIELDS: &[Field] = &[
    fd("locked", Kind::Bool, Def::Bool(false), "Locked tracks cannot be edited in the GUI."),
    fd("visible", Kind::Bool, Def::Bool(true), "Hidden tracks render nothing."),
    fd("muted", Kind::Bool, Def::Bool(false), "Muted tracks play no audio."),
];

pub fn track_field(name: &str) -> Option<&'static Field> {
    TRACK_FIELDS.iter().find(|f| f.name == name)
}

/// Registry entry (top-level `media` array): where a media file lives and its last known state.
pub const REGISTRY_FIELDS: &[Field] = &[
    f("mediaId", Kind::Str, "Id of the media item."),
    f("nativePath", Kind::Str, "Absolute path of the file, or relative to the bundle when owned."),
    f("ownedByBundle", Kind::Bool, "The file lives in the bundle's media/ folder."),
    fd("probeStatus", Kind::Enum(PROBE_STATUSES), Def::Str("awaiting-probe"), "Probe state."),
    f("size", Kind::Int { min: Some(0), max: None }, "Last known file size in bytes."),
    f("modifiedAt", Kind::Int { min: None, max: None }, "Last known modification time, ms since the epoch."),
];

/// Media item (`project.media`): what a media file is. `data` holds probe results owned by the media change.
pub const MEDIA_ITEM_FIELDS: &[Field] = &[
    f("id", Kind::Str, "Unique media id."),
    f("createdAt", Kind::Int { min: None, max: None }, "ms since the epoch."),
    f("addedAt", Kind::Int { min: None, max: None }, "ms since the epoch."),
    f("data", Kind::Any, "type, fileName, format, size, duration, width, height, frameRate, hasAudio, statuses, …"),
];

pub const PROJECT_FIELDS: &[Field] = &[
    f("id", Kind::Str, "Project id."),
    f("name", Kind::Str, "Project name (free text; the bundle folder never changes on rename)."),
    f("createdAt", Kind::Int { min: None, max: None }, "ms since the epoch."),
    f("updatedAt", Kind::Int { min: None, max: None }, "ms since the epoch."),
    f("stateUpdatedAt", Kind::Int { min: Some(0), max: None }, "Current revision (rev), ms-based."),
    f("aspectRatio", Kind::Enum(ASPECT_RATIOS), "Frame aspect ratio."),
    f("media", Kind::Any, "Media items."),
    f("timelineItems", Kind::Any, "Items."),
    f("tracks", Kind::Any, "Tracks."),
];

// ---------------------------------------------------------------------------------------------------------------
// Validation and normalisation
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum Severity {
    Error,
    Warning,
}

/// A problem found in a value. Errors violate invariants; warnings are informational.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

impl Issue {
    fn err(code: &'static str, path: &str, message: String) -> Self {
        Issue { severity: Severity::Error, code, path: path.to_string(), message }
    }
}

/// Validates a value against a kind. Pushes errors (invariant violations) and warnings (unknown keys in closed
/// objects) to `out`.
pub fn validate(kind: &Kind, v: &Value, path: &str, out: &mut Vec<Issue>) {
    match kind {
        Kind::Any => {}
        Kind::Str => {
            if !v.is_string() {
                out.push(Issue::err("value.type", path, format!("{path} must be a string")));
            }
        }
        Kind::Bool => {
            if !v.is_boolean() {
                out.push(Issue::err("value.type", path, format!("{path} must be true or false")));
            }
        }
        Kind::Num { min, max } => match v.as_f64() {
            None => out.push(Issue::err("value.type", path, format!("{path} must be a number"))),
            Some(x) if !x.is_finite() => out.push(Issue::err("value.finite", path, format!("{path} must be finite"))),
            Some(x) => range(x, *min, *max, path, out),
        },
        Kind::Int { min, max } => match v.as_i64() {
            None => out.push(Issue::err("value.type", path, format!("{path} must be an integer"))),
            Some(x) => range(x as f64, min.map(|m| m as f64), max.map(|m| m as f64), path, out),
        },
        Kind::Time | Kind::SignedTime => match v.as_f64() {
            None => out.push(Issue::err("value.type", path, format!("{path} must be a number of seconds"))),
            Some(x) if !x.is_finite() || json_to_micros(v).is_none() => {
                out.push(Issue::err("value.finite", path, format!("{path} must be a finite number of seconds")))
            }
            Some(x) if matches!(kind, Kind::Time) && x < 0.0 => {
                out.push(Issue::err("value.nonNegative", path, format!("{path} must not be negative")))
            }
            Some(_) => {}
        },
        Kind::Enum(values) => match v.as_str() {
            Some(s) if values.contains(&s) => {}
            _ => out.push(Issue::err(
                "enum.known",
                path,
                format!("{path} must be one of {}, got {}", values.join(", "), short(v)),
            )),
        },
        Kind::Arr(elem) => match v.as_array() {
            None => out.push(Issue::err("value.type", path, format!("{path} must be an array"))),
            Some(a) => {
                for (i, e) in a.iter().enumerate() {
                    validate(elem, e, &format!("{path}[{i}]"), out);
                }
            }
        },
        Kind::Obj { fields, open } => match v.as_object() {
            None => out.push(Issue::err("value.type", path, format!("{path} must be an object"))),
            Some(o) => validate_object(fields, *open, o, path, out),
        },
    }
}

fn validate_object(fields: &[Field], open: bool, o: &Map<String, Value>, path: &str, out: &mut Vec<Issue>) {
    for (k, val) in o {
        match fields.iter().find(|f| f.name == k) {
            Some(field) => validate(&field.kind, val, &format!("{path}.{k}"), out),
            None if open => {}
            None => out.push(Issue {
                severity: Severity::Warning,
                code: "object.key.unknown",
                path: format!("{path}.{k}"),
                message: format!("unknown key `{k}` in {path}; it is kept but has no effect"),
            }),
        }
    }
}

fn range(x: f64, min: Option<f64>, max: Option<f64>, path: &str, out: &mut Vec<Issue>) {
    if let Some(m) = min
        && x < m
    {
        let code = if m == 0.0 { "value.nonNegative" } else { "value.range" };
        out.push(Issue::err(code, path, format!("{path} must be ≥ {m}, got {x}")));
    }
    if let Some(m) = max
        && x > m
    {
        out.push(Issue::err("value.range", path, format!("{path} must be ≤ {m}, got {x}")));
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 40 { format!("{}…", &s[..s.floor_char_boundary(40)]) } else { s }
}

/// Normalises a value of a kind: times rounded to microseconds and written canonically, recursively. Values that do
/// not match their kind are returned unchanged (validation reports them).
pub fn normalise(kind: &Kind, v: &Value) -> Value {
    match (kind, v) {
        (Kind::Time | Kind::SignedTime, Value::Number(_)) => json_to_micros(v).map(micros_to_json).unwrap_or(v.clone()),
        (Kind::Arr(elem), Value::Array(a)) => Value::Array(a.iter().map(|e| normalise(elem, e)).collect()),
        (Kind::Obj { fields, .. }, Value::Object(o)) => Value::Object(
            o.iter()
                .map(|(k, val)| {
                    let n = match fields.iter().find(|f| f.name == k) {
                        Some(field) => normalise(&field.kind, val),
                        None => val.clone(),
                    };
                    (k.clone(), n)
                })
                .collect(),
        ),
        _ => v.clone(),
    }
}

/// True when a value equals the field's default (or, for objects, when every key is at its default).
pub fn is_default(field: &Field, v: &Value) -> bool {
    if let Some(d) = field.default {
        return def_eq(d, v);
    }
    match (&field.kind, v) {
        (Kind::Obj { .. }, Value::Object(o)) => {
            strip_defaults(&field.kind, v).as_object().is_some_and(|m| m.is_empty()) || o.is_empty()
        }
        _ => false,
    }
}

fn def_eq(d: Def, v: &Value) -> bool {
    match (d, v) {
        (Def::Num(x), Value::Number(n)) => n.as_f64() == Some(x),
        (Def::Bool(b), Value::Bool(c)) => b == *c,
        (Def::Str(s), Value::String(t)) => s == t,
        _ => false,
    }
}

/// Removes default-valued keys from known objects (recursively one level of known object fields). Arrays and
/// unknown keys are kept as they are.
pub fn strip_defaults(kind: &Kind, v: &Value) -> Value {
    match (kind, v) {
        (Kind::Obj { fields, .. }, Value::Object(o)) => {
            let mut m = Map::new();
            for (k, val) in o {
                match fields.iter().find(|f| f.name == k) {
                    Some(field) if field.default.is_some_and(|d| def_eq(d, val)) => {}
                    Some(field) => {
                        let s = strip_defaults(&field.kind, val);
                        let empty_obj =
                            matches!(field.kind, Kind::Obj { .. }) && s.as_object().is_some_and(|m| m.is_empty());
                        if !empty_obj {
                            m.insert(k.clone(), s);
                        }
                    }
                    None => {
                        m.insert(k.clone(), val.clone());
                    }
                }
            }
            Value::Object(m)
        }
        _ => v.clone(),
    }
}

/// Fills in the field's default when the value is absent.
pub fn default_value(field: &Field) -> Option<Value> {
    field.default.map(|d| match d {
        Def::Num(x) => {
            if x.fract() == 0.0 {
                Value::from(x as i64)
            } else {
                Value::from(x)
            }
        }
        Def::Bool(b) => Value::Bool(b),
        Def::Str(s) => Value::String(s.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_item_type_has_fields() {
        for t in ITEM_TYPES {
            let fields = item_fields(t);
            assert!(fields.iter().any(|f| f.name == "transform"), "{t}");
            let mut names: Vec<_> = fields.iter().map(|f| f.name).collect();
            names.sort();
            let n = names.len();
            names.dedup();
            assert_eq!(n, names.len(), "duplicate field in {t}");
        }
        assert!(item_field("video", "volume").is_some());
        assert!(item_field("image", "volume").is_none());
        assert!(item_field("text", "textContent").is_some());
    }

    #[test]
    fn validation_codes() {
        let mut out = vec![];
        validate(&item_field("video", "opacity").unwrap().kind, &json!(2), "o", &mut out);
        assert_eq!(out[0].code, "value.range");
        out.clear();
        validate(&item_field("video", "blendMode").unwrap().kind, &json!("glow"), "b", &mut out);
        assert_eq!(out[0].code, "enum.known");
        out.clear();
        validate(&item_field("video", "trimStart").unwrap().kind, &json!(-1), "t", &mut out);
        assert_eq!(out[0].code, "value.nonNegative");
        out.clear();
        validate(&item_field("video", "transform").unwrap().kind, &json!({"x": 0, "scael": 2}), "t", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Warning);
        assert_eq!(out[0].code, "object.key.unknown");
    }

    #[test]
    fn defaults() {
        let t = item_field("video", "transform").unwrap();
        assert!(is_default(t, &json!({"x": 0, "scale": 1})));
        assert!(!is_default(t, &json!({"x": 3})));
        assert_eq!(strip_defaults(&t.kind, &json!({"x": 0, "y": 4, "zz": 1})), json!({"y": 4, "zz": 1}));
        assert!(is_default(item_field("video", "volume").unwrap(), &json!(100)));
        assert!(is_default(item_field("video", "volume").unwrap(), &json!(100.0)));
    }
}
