//! What an export leaves behind, so the person can see it before writing the file: items a format can't hold
//! (`omitted`) and properties of the written clips that no format here carries (`not_carried`).

use serde_json::{Value, json};

use ove_core::model::{Document, Item};

use crate::Format;
use crate::model::{ClipKind, Timeline};

/// Items of one kind left out of the file, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Omitted {
    /// What was left out, in plain words ("captions", "titles", "video clips above V1").
    pub kind: String,
    pub count: usize,
    pub reason: String,
}

/// The fidelity report of one export.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportReport {
    pub omitted: Vec<Omitted>,
    /// Properties used by the written clips that the file does not carry, in plain words ("effects", "crop").
    pub not_carried: Vec<String>,
}

impl ExportReport {
    /// Nothing is lost.
    pub fn is_empty(&self) -> bool {
        self.omitted.is_empty() && self.not_carried.is_empty()
    }

    /// Adds `count` items of `kind` (merged with an entry of the same kind and reason).
    pub fn omit(&mut self, kind: &str, count: usize, reason: &str) {
        if count == 0 {
            return;
        }
        match self.omitted.iter_mut().find(|o| o.kind == kind && o.reason == reason) {
            Some(o) => o.count += count,
            None => self.omitted.push(Omitted { kind: kind.into(), count, reason: reason.into() }),
        }
    }

    pub fn not_carried(&mut self, property: &str) {
        if !self.not_carried.iter().any(|p| p == property) {
            self.not_carried.push(property.into());
        }
    }

    /// `{"omitted": [{"kind", "count", "reason"}], "notCarried": [...]}`.
    pub fn to_json(&self) -> Value {
        json!({
            "omitted": self.omitted.iter().map(|o| json!({"kind": o.kind, "count": o.count, "reason": o.reason})).collect::<Vec<_>>(),
            "notCarried": self.not_carried,
        })
    }

    /// One line per entry, for the command line.
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> =
            self.omitted.iter().map(|o| format!("left out: {} ({}): {}", o.kind, o.count, o.reason)).collect();
        if !self.not_carried.is_empty() {
            out.push(format!("not carried: {}", self.not_carried.join(", ")));
        }
        out
    }
}

/// Items `from_document` does not write, by item type: (what to call them, why).
pub(crate) fn unsupported_kind(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "caption" => ("captions", "timeline files have no caption track; captions stay in the project"),
        "code" => ("motion graphics", "they are drawn by OpenVideoEditor and have no equivalent in other editors"),
        "shader" => ("shader layers", "they are drawn by OpenVideoEditor and have no equivalent in other editors"),
        "adjustment" => ("adjustment layers", "adjustment layers are not written to timeline files yet"),
        _ => ("other items", "this item type has no equivalent in timeline files"),
    }
}

pub(crate) const PENDING: (&str, &str) = ("placeholders", "they have no media yet");
pub(crate) const HIDDEN: (&str, &str) = ("hidden items", "hidden items are not written");
pub(crate) const TOO_SHORT: (&str, &str) = ("clips shorter than one frame", "they vanish at this frame rate");

/// Properties of written items that no format here carries: (document field, plain name, is it set?).
type Check = fn(&Value) -> bool;
const PROPERTIES: &[(&str, &str, Check)] = &[
    ("effects", "effects", non_empty_array),
    ("transform", "position, scale and rotation", non_empty_object),
    ("crop", "crop", non_empty_object),
    ("opacity", "opacity", |v| v.as_f64().is_some_and(|o| (o - 1.0).abs() > 1e-6)),
    ("keyframeTracks", "keyframes", non_empty_array),
    ("transitionIn", "transitions", non_empty_object),
    ("transitionOut", "transitions", non_empty_object),
    ("volume", "volume", |v| v.as_f64().is_some_and(|x| (x - 100.0).abs() > 1e-6)),
    ("muted", "muted clips", |v| v.as_bool() == Some(true)),
    ("audioFadeIn", "audio fades", non_empty_object),
    ("audioFadeOut", "audio fades", non_empty_object),
    ("colorGrading", "colour grading", non_empty_object),
    ("colorGrade", "colour grading", non_empty_object),
    ("blendMode", "blend modes", |v| v.as_str().is_some_and(|s| s != "normal")),
    ("borderRadius", "rounded corners", positive),
    ("gaussianBlur", "blur", positive),
    ("playbackDirection", "reverse playback", |v| v.as_str().is_some_and(|s| s != "forward")),
    ("layout", "layout slots", non_empty_object),
    ("textAnimation", "text animation", |v| {
        v.get("preset").and_then(Value::as_str).is_some_and(|p| p != "none") || v.get("out").is_some()
    }),
];

/// Title look: none of the formats keeps the font, size or colours (titles are written in a default style).
const TITLE_STYLE: &[&str] = &[
    "fontFamily",
    "fontSize",
    "fontWeight",
    "fontStyle",
    "color",
    "textAlign",
    "lineHeight",
    "letterSpacing",
    "strokeColor",
    "strokeWidth",
    "shadowColor",
    "backgroundColor",
];

fn non_empty_array(v: &Value) -> bool {
    v.as_array().is_some_and(|a| !a.is_empty())
}

fn non_empty_object(v: &Value) -> bool {
    v.as_object().is_some_and(|o| !o.is_empty())
}

fn positive(v: &Value) -> bool {
    v.as_f64().is_some_and(|x| x > 0.0)
}

/// Records the properties a written item uses that no format carries.
pub(crate) fn note_properties(report: &mut ExportReport, doc: &Document, it: &Item) {
    for (field, name, set) in PROPERTIES {
        if it.fields.get(*field).is_some_and(set) {
            report.not_carried(name);
        }
    }
    if it.kind == "text" && TITLE_STYLE.iter().any(|k| it.fields.contains_key(*k)) {
        report.not_carried("title styling (font, size, colours)");
    }
    // Footage fills a non-16:9 frame with a centred crop unless it was placed by hand; other editors fit it.
    let visual = matches!(it.kind.as_str(), "video" | "image");
    let auto_reframe = it.fields.get("autoReframe").and_then(Value::as_bool).unwrap_or(true);
    if visual && auto_reframe && !it.fields.contains_key("transform") && doc.project.aspect_ratio != "16:9" {
        report.not_carried("fill-the-frame crop");
    }
}

/// Track states the formats don't write: hidden and muted tracks are written as normal tracks.
pub(crate) fn note_track(report: &mut ExportReport, t: &ove_core::model::Track, has_items: bool) {
    if !has_items {
        return;
    }
    if t.fields.get("visible").and_then(Value::as_bool) == Some(false) {
        report.not_carried("hidden tracks (written as visible)");
    }
    if t.fields.get("muted").and_then(Value::as_bool) == Some(true) {
        report.not_carried("muted tracks (written as audible)");
    }
}

/// What a format drops from the neutral timeline.
pub(crate) fn note_format(report: &mut ExportReport, t: &Timeline, format: Format) {
    let titles = |tracks: &[crate::model::Track]| {
        tracks.iter().flat_map(|tr| &tr.clips).filter(|c| c.kind == ClipKind::Title).count()
    };
    let unlinked = |tracks: &[crate::model::Track]| {
        tracks.iter().flat_map(|tr| &tr.clips).filter(|c| c.kind != ClipKind::Title && c.path.is_none()).count()
    };
    let unlinked_reason = "their media file is not linked in the project";
    match format {
        Format::Edl => {
            let upper = t.video.iter().skip(1);
            report.omit(
                "video clips above the main track",
                upper.flat_map(|tr| &tr.clips).filter(|c| c.kind != ClipKind::Title && c.path.is_some()).count(),
                "an EDL has one video track (V1)",
            );
            report.omit("titles", titles(&t.video), "an EDL has no titles");
            report.omit(
                "audio clips after the first audio track",
                t.audio.iter().skip(1).flat_map(|tr| &tr.clips).filter(|c| c.path.is_some()).count(),
                "an EDL holds the main video's sound and one audio track (A1)",
            );
            let v1_unlinked = t.video.first().map_or(0, |tr| unlinked(std::slice::from_ref(tr)));
            let a1_unlinked = t.audio.first().map_or(0, |tr| unlinked(std::slice::from_ref(tr)));
            report.omit("clips", v1_unlinked + a1_unlinked, unlinked_reason);
            if t.rate.ntsc() {
                report.not_carried("drop-frame timecode (the EDL is written non-drop frame)");
            }
        }
        Format::Fcpxml => {
            report.omit("clips", unlinked(&t.video) + unlinked(&t.audio), unlinked_reason);
        }
        // Audio clips without a file are still written (as offline clips); video ones are not.
        Format::Xml => report.omit("clips", unlinked(&t.video), unlinked_reason),
        Format::Otio => {
            report.omit(
                "titles",
                titles(&t.video),
                "OpenTimelineIO has no titles: they are written as empty clips, their text kept only for OpenVideoEditor",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_merge_and_serialise() {
        let mut r = ExportReport::default();
        assert!(r.is_empty());
        r.omit("captions", 2, "no caption track");
        r.omit("captions", 1, "no caption track");
        r.omit("titles", 0, "ignored: nothing left out");
        r.not_carried("effects");
        r.not_carried("effects");
        r.not_carried("crop");
        assert!(!r.is_empty());
        assert_eq!(r.omitted, vec![Omitted { kind: "captions".into(), count: 3, reason: "no caption track".into() }]);
        assert_eq!(r.not_carried, vec!["effects".to_string(), "crop".to_string()]);
        assert_eq!(
            r.to_json(),
            json!({"omitted": [{"kind": "captions", "count": 3, "reason": "no caption track"}], "notCarried": ["effects", "crop"]})
        );
        assert_eq!(r.lines(), vec!["left out: captions (3): no caption track", "not carried: effects, crop"]);
    }

    #[test]
    fn defaults_are_not_reported() {
        let set = |field: &str, v: Value| PROPERTIES.iter().find(|p| p.0 == field).map(|p| (p.2)(&v)).unwrap();
        assert!(!set("opacity", json!(1.0)));
        assert!(set("opacity", json!(0.5)));
        assert!(!set("volume", json!(100)));
        assert!(!set("effects", json!([])));
        assert!(!set("blendMode", json!("normal")));
        assert!(!set("playbackDirection", json!("forward")));
        assert!(set("playbackDirection", json!("reverse")));
        assert!(!set("textAnimation", json!({"preset": "none"})));
        assert!(set("textAnimation", json!({"preset": "rise"})));
    }
}
