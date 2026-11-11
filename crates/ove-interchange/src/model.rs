//! The neutral timeline every format is written from: tracks bottom-first (V1 = base layer, the convention of NLEs),
//! clips with record range, source in-point and speed, all on an integer frame grid.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use ove_core::model::Document;
use ove_core::time::Micros;

/// A frame rate as a rational (30/1, 30000/1001, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    pub num: u32,
    pub den: u32,
}

impl Rate {
    pub const FPS30: Rate = Rate { num: 30, den: 1 };

    pub fn from_fps(fps: f64) -> Rate {
        for (n, d) in [(24000, 1001), (30000, 1001), (60000, 1001)] {
            if (fps - n as f64 / d as f64).abs() < 0.01 {
                return Rate { num: n, den: d };
            }
        }
        Rate { num: fps.round().max(1.0) as u32, den: 1 }
    }

    pub fn fps(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Integer timebase used by EDL/FCP7 (29.97 → 30).
    pub fn timebase(self) -> u32 {
        (self.fps()).round() as u32
    }

    pub fn ntsc(self) -> bool {
        self.den == 1001
    }

    /// Microseconds → nearest frame.
    pub fn frames(self, us: Micros) -> i64 {
        ((us as f64) * self.num as f64 / (self.den as f64 * 1e6)).round() as i64
    }

    /// Frames → microseconds.
    pub fn micros(self, frames: i64) -> Micros {
        ((frames as f64) * self.den as f64 * 1e6 / self.num as f64).round() as Micros
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    Video,
    Image,
    Audio,
    Title,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Item id in the source document (when exported from one).
    pub id: Option<String>,
    pub kind: ClipKind,
    /// Source file, when the clip plays media.
    pub path: Option<PathBuf>,
    pub media_id: Option<String>,
    /// Record range on the timeline, frames.
    pub start: i64,
    pub end: i64,
    /// Source in-point, frames.
    pub source_in: i64,
    pub speed: f64,
    /// Full media duration, frames (when known).
    pub media_duration: Option<i64>,
    pub has_audio: bool,
    pub has_video: bool,
    /// Title text for title clips.
    pub text: Option<String>,
}

impl Clip {
    pub fn duration(&self) -> i64 {
        self.end - self.start
    }

    /// Source out-point (exclusive), frames.
    pub fn source_out(&self) -> i64 {
        self.source_in + ((self.duration() as f64) * self.speed).round() as i64
    }

    /// Reel/clip name without extension.
    pub fn stem(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub name: String,
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    pub name: String,
    pub rate: Rate,
    pub width: u32,
    pub height: u32,
    /// Bottom-first (V1 is the base layer).
    pub video: Vec<Track>,
    /// A1 first.
    pub audio: Vec<Track>,
}

impl Timeline {
    pub fn duration(&self) -> i64 {
        self.video.iter().chain(&self.audio).flat_map(|t| t.clips.iter().map(|c| c.end)).max().unwrap_or(0)
    }
}

/// Frame size for an aspect ratio at a height (even dimensions).
pub fn frame_size(aspect: &str, height: u32) -> (u32, u32) {
    let (a, b) = aspect
        .split_once(':')
        .and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?)))
        .unwrap_or((16.0, 9.0));
    let w = ((height as f64 * a / b) / 2.0).round() as u32 * 2;
    (w, height)
}

fn resolve(bundle: Option<&Path>, native: &str) -> PathBuf {
    let p = PathBuf::from(native);
    if p.is_absolute() {
        return p;
    }
    match bundle {
        Some(b) => b.join(p),
        None => p,
    }
}

/// Builds the neutral timeline from a document. Overlapping items on one track are spread onto extra tracks so every
/// format sees non-overlapping tracks. `pending`, hidden, caption, code, shader and adjustment items are left out.
pub fn from_document(doc: &Document, bundle: Option<&Path>, rate: Rate) -> Timeline {
    let (width, height) = frame_size(&doc.project.aspect_ratio, 1080);
    let mut video: Vec<Track> = Vec::new();
    let mut audio: Vec<Track> = Vec::new();
    let media_path = |mid: &str| doc.registry.get(mid).map(|e| resolve(bundle, &e.native_path));
    let media_dur = |mid: &str| {
        doc.project
            .media
            .get(mid)
            .and_then(|m| m.data())
            .and_then(|d| d.get("duration"))
            .and_then(ove_core::time::json_to_micros)
    };
    let media_has = |mid: &str, key: &str, default: bool| {
        doc.project
            .media
            .get(mid)
            .and_then(|m| m.data())
            .and_then(|d| d.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(default)
    };
    // Bottom-first: reverse document order (first track = top layer).
    for t in doc.project.tracks.iter().rev() {
        let mut lanes: Vec<Track> = Vec::new();
        let is_audio = t.kind == "audio";
        for it in doc.items_on_track(&t.id) {
            if it.is_pending() || it.fields.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            let kind = match it.kind.as_str() {
                "video" => ClipKind::Video,
                "image" => ClipKind::Image,
                "audio" => ClipKind::Audio,
                "text" => ClipKind::Title,
                _ => continue,
            };
            let mid = it.media_id().map(str::to_string);
            let speed = it.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0);
            let clip = Clip {
                name: if it.name().is_empty() {
                    mid.as_deref()
                        .and_then(|m| doc.project.media.get(m))
                        .and_then(|m| m.file_name().map(str::to_string))
                        .unwrap_or_else(|| it.kind.clone())
                } else {
                    it.name().to_string()
                },
                id: Some(it.id.clone()),
                kind,
                path: mid.as_deref().and_then(media_path),
                media_id: mid.clone(),
                start: rate.frames(it.start),
                end: rate.frames(it.end),
                source_in: rate.frames(it.time_field("trimStart")),
                speed,
                media_duration: mid.as_deref().and_then(media_dur).map(|d| rate.frames(d)),
                has_audio: kind == ClipKind::Audio
                    || (kind == ClipKind::Video && mid.as_deref().is_some_and(|m| media_has(m, "hasAudio", true))),
                has_video: matches!(kind, ClipKind::Video | ClipKind::Image | ClipKind::Title),
                text: it.fields.get("textContent").and_then(Value::as_str).map(str::to_string),
            };
            if clip.end <= clip.start {
                continue;
            }
            match lanes.iter_mut().find(|l| l.clips.last().is_none_or(|c| c.end <= clip.start)) {
                Some(l) => l.clips.push(clip),
                None => lanes.push(Track { name: t.name.clone(), clips: vec![clip] }),
            }
        }
        if lanes.is_empty() {
            lanes.push(Track { name: t.name.clone(), clips: vec![] });
        }
        if is_audio { audio.extend(lanes) } else { video.extend(lanes) }
    }
    // Audio tracks were collected bottom-first too; NLEs list A1 = the first (top) audio track.
    audio.reverse();
    video.retain(|t| !t.clips.is_empty());
    audio.retain(|t| !t.clips.is_empty());
    Timeline { name: doc.project.name.clone(), rate, width, height, video, audio }
}

/// `file://` URL for a path (percent-encoded; Windows drive paths become file:///C:/…).
pub fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = if s.starts_with('/') { s } else { format!("/{s}") };
    let mut out = String::from("file://");
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Path from a `file://` URL (or a plain path).
pub fn url_path(u: &str) -> PathBuf {
    let rest = u.strip_prefix("file://localhost").or_else(|| u.strip_prefix("file://")).unwrap_or(u);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    let s = String::from_utf8_lossy(&out).into_owned();
    // file:///C:/x → C:/x
    let s = if s.len() > 3 && s.as_bytes()[0] == b'/' && s.as_bytes()[2] == b':' { s[1..].to_string() } else { s };
    PathBuf::from(s)
}

/// XML parsing options: a DOCTYPE is allowed (FCPXML and xmeml files carry one); entities are never fetched.
pub fn xml_options<'a>() -> roxmltree::ParsingOptions<'a> {
    roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }
}

/// XML text escaping.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Distinct media used by a timeline, in first-use order: (key, clip).
pub fn distinct_media(t: &Timeline) -> Vec<(String, Clip)> {
    let mut seen = BTreeMap::new();
    let mut out = Vec::new();
    for c in t.video.iter().chain(&t.audio).flat_map(|t| &t.clips) {
        if let Some(p) = &c.path {
            let key = p.display().to_string();
            if seen.insert(key.clone(), ()).is_none() {
                out.push((key, c.clone()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_and_urls() {
        assert_eq!(Rate::from_fps(29.97), Rate { num: 30000, den: 1001 });
        assert_eq!(Rate::FPS30.frames(1_000_000), 30);
        assert_eq!(Rate::FPS30.micros(45), 1_500_000);
        let p = Path::new("/Volumes/Footage/Day 1/take #2.mov");
        let u = file_url(p);
        assert_eq!(u, "file:///Volumes/Footage/Day%201/take%20%232.mov");
        assert_eq!(url_path(&u), p);
        assert_eq!(url_path("file:///C:/Clips/a%20b.mp4"), PathBuf::from("C:/Clips/a b.mp4"));
        assert_eq!(frame_size("9:16", 1080), (608, 1080));
    }
}
