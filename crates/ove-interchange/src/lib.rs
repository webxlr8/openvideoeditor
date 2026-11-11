//! Timeline interchange with other editors (roadmap change 15): FCPXML (Final Cut Pro, DaVinci Resolve), Final Cut
//! Pro 7 XML (Premiere Pro), CMX3600 EDL and OpenTimelineIO. Export reads the project document; import produces one
//! batch of document ops that adds the imported tracks, media and clips to a project.

pub mod edl;
pub mod fcpxml;
pub mod model;
pub mod otio;
pub mod xmeml;

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value, json};

use ove_core::model::Document;
use ove_core::ops::Op;
use ove_core::time::micros_to_json;

pub use model::{Clip, ClipKind, Rate, Timeline, Track};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Parse(String),
    #[error("unknown format: {0} (use fcpxml, xml, edl or otio)")]
    Format(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// FCPXML 1.11 — Final Cut Pro, DaVinci Resolve.
    Fcpxml,
    /// Final Cut Pro 7 XML — Premiere Pro, Resolve.
    Xml,
    /// CMX3600 EDL.
    Edl,
    /// OpenTimelineIO JSON.
    Otio,
}

impl Format {
    pub fn parse(s: &str) -> Result<Format> {
        match s.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "fcpxml" | "fcpxmld" => Ok(Format::Fcpxml),
            "xml" | "xmeml" | "premiere" | "fcp7" => Ok(Format::Xml),
            "edl" => Ok(Format::Edl),
            "otio" | "json" => Ok(Format::Otio),
            other => Err(Error::Format(other.to_string())),
        }
    }

    /// From a file name's extension.
    pub fn from_path(p: &Path) -> Result<Format> {
        Format::parse(&p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default())
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Fcpxml => "fcpxml",
            Format::Xml => "xml",
            Format::Edl => "edl",
            Format::Otio => "otio",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Fcpxml => "FCPXML (Final Cut Pro, DaVinci Resolve)",
            Format::Xml => "XML (Premiere Pro, Final Cut Pro 7)",
            Format::Edl => "EDL (CMX3600)",
            Format::Otio => "OpenTimelineIO",
        }
    }
}

/// Writes a project's timeline in a format. `bundle` resolves media owned by the bundle.
pub fn export(doc: &Document, bundle: Option<&Path>, format: Format, rate: Rate) -> String {
    let t = model::from_document(doc, bundle, rate);
    match format {
        Format::Fcpxml => fcpxml::write(&t),
        Format::Xml => xmeml::write(&t),
        Format::Edl => edl::write(&t),
        Format::Otio => otio::write(&t),
    }
}

/// Reads a timeline file.
pub fn read(text: &str, format: Format, rate: Rate) -> Result<Timeline> {
    match format {
        Format::Fcpxml => fcpxml::read(text),
        Format::Xml => xmeml::read(text),
        Format::Edl => edl::read(text, rate),
        Format::Otio => otio::read(text, None),
    }
}

fn media_type(p: &Path, kind: ClipKind) -> &'static str {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "webp" | "heic" | "bmp" | "gif" => "image",
        "wav" | "mp3" | "m4a" | "aac" | "flac" | "aif" | "aiff" | "ogg" | "opus" | "caf" => "audio",
        _ if kind == ClipKind::Audio => "audio",
        _ => "video",
    }
}

/// What an import will do.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportSummary {
    pub tracks: usize,
    pub clips: usize,
    pub media_added: usize,
    /// Clips whose media could not be found (added as pending placeholders).
    pub missing: Vec<String>,
}

/// Turns an imported timeline into one batch of ops on `doc`: new tracks (video above the existing ones, audio
/// below), media registered by path (reusing entries with the same path), clips as items. EDL clips without a path
/// are matched by file name against the project's media; unmatched clips become pending placeholders.
pub fn to_ops(doc: &Document, t: &Timeline, now_ms: i64) -> (Vec<Op>, ImportSummary) {
    let mut ops = Vec::new();
    let mut summary = ImportSummary::default();
    let us = |f: i64| t.rate.micros(f);
    let mut by_path: BTreeMap<String, String> =
        doc.registry.values().map(|e| (e.native_path.clone(), e.media_id.clone())).collect();
    let by_name: BTreeMap<String, String> =
        doc.project.media.values().filter_map(|m| m.file_name().map(|n| (n.to_lowercase(), m.id.clone()))).collect();
    let tag = format!("{:x}", now_ms & 0xff_ffff);
    let taken: std::collections::BTreeSet<String> = doc.project.tracks.iter().map(|t| t.id.clone()).collect();
    let mut n_media = 0;
    let mut n_item = 0;

    let add_track = |name: String, kind: &str, order: usize, ops: &mut Vec<Op>| -> String {
        let mut id = format!("t_imp_{tag}_{order}_{kind}");
        while taken.contains(&id) {
            id.push('x');
        }
        ops.push(Op::AddTrack { track: json!({"id": id, "name": name, "order": order, "type": kind}) });
        id
    };

    // Video: V1 lowest of the imported set but above existing tracks: insert top-first at order 0.
    let mut video_ids = Vec::new();
    for (i, tr) in t.video.iter().enumerate().rev() {
        let order = t.video.len() - 1 - i;
        video_ids.push((
            i,
            add_track(
                format!("Imported {}", if tr.name.is_empty() { format!("V{}", i + 1) } else { tr.name.clone() }),
                "video",
                order,
                &mut ops,
            ),
        ));
    }
    video_ids.sort();
    let mut audio_ids = Vec::new();
    for (i, tr) in t.audio.iter().enumerate() {
        let order = doc.project.tracks.len() + t.video.len() + i;
        audio_ids.push(add_track(
            format!("Imported {}", if tr.name.is_empty() { format!("A{}", i + 1) } else { tr.name.clone() }),
            "audio",
            order,
            &mut ops,
        ));
    }
    summary.tracks = video_ids.len() + audio_ids.len();

    let tracks = t
        .video
        .iter()
        .zip(video_ids.iter().map(|(_, id)| id.clone()))
        .chain(t.audio.iter().zip(audio_ids.iter().cloned()));
    for (tr, track_id) in tracks {
        for c in &tr.clips {
            n_item += 1;
            let mut item = Map::new();
            item.insert("id".into(), format!("i_imp_{tag}_{n_item}").into());
            item.insert("trackId".into(), track_id.clone().into());
            item.insert("startTime".into(), micros_to_json(us(c.start)));
            item.insert("endTime".into(), micros_to_json(us(c.end)));
            item.insert("name".into(), c.name.clone().into());
            match c.kind {
                ClipKind::Title => {
                    item.insert("type".into(), "text".into());
                    item.insert("textContent".into(), c.text.clone().unwrap_or_default().into());
                }
                _ => {
                    let media_id = match &c.path {
                        Some(p) => {
                            let key = p.display().to_string();
                            match by_path.get(&key) {
                                Some(id) => Some(id.clone()),
                                None => {
                                    n_media += 1;
                                    let id = format!("m_imp_{tag}_{n_media}");
                                    let mut data = Map::new();
                                    data.insert("type".into(), media_type(p, c.kind).into());
                                    data.insert(
                                        "fileName".into(),
                                        p.file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_default()
                                            .into(),
                                    );
                                    if let Some(d) = c.media_duration {
                                        data.insert("duration".into(), micros_to_json(us(d)));
                                    }
                                    ops.push(Op::AddMedia {
                                        media: json!({"id": id, "createdAt": now_ms, "addedAt": now_ms, "data": Value::Object(data)}),
                                        entry: Some(json!({"mediaId": id, "nativePath": key, "ownedByBundle": false, "probeStatus": "awaiting-probe"})),
                                    });
                                    by_path.insert(key, id.clone());
                                    summary.media_added += 1;
                                    Some(id)
                                }
                            }
                        }
                        None => by_name.get(&c.name.to_lowercase()).cloned(),
                    };
                    let kind = match (&c.path, c.kind) {
                        (Some(p), k) => media_type(p, k),
                        (None, ClipKind::Audio) => "audio",
                        _ => "video",
                    };
                    item.insert("type".into(), kind.into());
                    match media_id {
                        Some(id) => {
                            item.insert("mediaItemId".into(), id.into());
                        }
                        None => {
                            item.insert("pending".into(), true.into());
                            item.insert("pendingDescription".into(), format!("Missing media: {}", c.name).into());
                            summary.missing.push(c.name.clone());
                        }
                    }
                    if c.source_in > 0 {
                        item.insert("trimStart".into(), micros_to_json(us(c.source_in)));
                    }
                    if (c.speed - 1.0).abs() > 1e-6 && kind != "image" {
                        item.insert("playbackSpeed".into(), json!((c.speed * 1000.0).round() / 1000.0));
                    }
                }
            }
            ops.push(Op::AddItem { item: Value::Object(item) });
            summary.clips += 1;
        }
    }
    (ops, summary)
}
