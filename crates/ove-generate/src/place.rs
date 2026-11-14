//! Where generated media goes: a pending placeholder on the timeline while the job runs, then the media registered
//! in the bundle and the placeholder filled in place (same id, so edits made to it meanwhile are kept).

use ove_core::model::Document;
use ove_core::ops::Op;
use ove_core::time::micros_to_json;
use serde_json::{Map, Value, json};

use crate::catalog::{Kind, Model};
use crate::job::{Job, Request};

/// What a request will do, shown before anything is spent.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub kind: Kind,
    pub model: Model,
    pub aspect: String,
    /// Seconds of output (for the estimate and the placeholder length).
    pub seconds: f64,
    pub estimate_usd: Option<f64>,
    pub track_id: String,
    pub start: f64,
    pub item_id: String,
    /// Placeholder (and track) ops: one revision.
    #[serde(skip)]
    pub ops: Vec<Op>,
}

fn us(s: f64) -> i64 {
    (s * 1_000_000.0).round() as i64
}

/// Plans a request on `doc`: model, aspect, length, estimate, track and placeholder.
pub fn plan(doc: &Document, kind: Kind, req: &Request, item_id: &str) -> Result<Plan, String> {
    if req.prompt.trim().is_empty() {
        return Err("the prompt is empty".into());
    }
    let model = crate::catalog::find(req.model.as_deref(), kind)?;
    let aspect = req.aspect.clone().unwrap_or_else(|| doc.project.aspect_ratio.clone());
    if !model.aspects.is_empty() && !model.aspects.contains(&aspect) {
        return Err(format!("{} supports aspect ratios {}; pass --aspect", model.label, model.aspects.join(", ")));
    }
    let seconds = match kind {
        Kind::Image => req.duration.unwrap_or(5.0),
        Kind::Video => {
            let d = req.duration.or_else(|| model.durations.first().copied()).unwrap_or(5.0);
            if !model.durations.is_empty() && !model.durations.iter().any(|x| (x - d).abs() < 1e-6) {
                return Err(format!(
                    "{} makes clips of {} seconds",
                    model.label,
                    model.durations.iter().map(|x| format!("{x}")).collect::<Vec<_>>().join(" or ")
                ));
            }
            d
        }
        Kind::Music => req.duration.unwrap_or(30.0).clamp(3.0, model.max_seconds.max(3.0)),
        Kind::Sfx => req.duration.unwrap_or(5.0).clamp(0.5, 30.0),
        // About 15 characters per second of speech; the real length replaces it when the audio arrives.
        Kind::Voice => (req.prompt.chars().count() as f64 / 15.0).max(1.0),
    };
    if !(seconds.is_finite() && seconds > 0.0) {
        return Err("the duration must be positive".into());
    }
    let chars = req.prompt.chars().count();
    let estimate_usd = crate::catalog::estimate(&model, if kind == Kind::Image { 0.0 } else { seconds }, chars, 1);

    let mut ops = Vec::new();
    let audio = !matches!(kind, Kind::Image | Kind::Video);
    let track_id = match &req.track {
        Some(q) => {
            let t = doc
                .project
                .tracks
                .iter()
                .find(|t| &t.id == q)
                .or_else(|| doc.project.tracks.iter().find(|t| t.name.eq_ignore_ascii_case(q)))
                .ok_or_else(|| format!("no track \"{q}\""))?;
            if (t.kind == "audio") != audio || t.kind == "caption" {
                return Err(format!("track \"{}\" holds {} clips, not {}", t.name, t.kind, kind.item_type()));
            }
            t.id.clone()
        }
        None => {
            let preferred = if audio { "t_audio" } else { "t_overlay" };
            match doc.track(preferred) {
                Some(t) => t.id.clone(),
                None => match doc.project.tracks.iter().find(|t| t.kind == if audio { "audio" } else { "video" }) {
                    Some(t) => t.id.clone(),
                    None => {
                        let (id, name) = if audio { ("t_audio", "Audio") } else { ("t_overlay", "Overlay") };
                        let order = if audio { doc.project.tracks.len() } else { 0 };
                        ops.push(Op::AddTrack { track: json!({"id": id, "name": name, "order": order, "type": if audio { "audio" } else { "video" }}) });
                        id.to_string()
                    }
                },
            }
        }
    };
    let start = match req.start {
        Some(s) if s.is_finite() && s >= 0.0 => s,
        Some(_) => return Err("start must be ≥ 0".into()),
        None => doc.items_on_track(&track_id).iter().map(|i| i.end).max().unwrap_or(0) as f64 / 1e6,
    };
    let name = req.name.clone().unwrap_or_else(|| {
        let p: String = req.prompt.chars().take(40).collect();
        if req.prompt.chars().count() > 40 { format!("{p}…") } else { p }
    });
    ops.push(Op::AddItem {
        item: json!({
            "id": item_id, "trackId": track_id, "type": kind.item_type(),
            "startTime": micros_to_json(us(start)), "endTime": micros_to_json(us(start + seconds)),
            "name": name, "pending": true,
            "pendingDescription": format!("Generating {} with {}: {}", kind.as_str(), model.label, req.prompt),
        }),
    });
    Ok(Plan { kind, model, aspect, seconds, estimate_usd, track_id, start, item_id: item_id.into(), ops })
}

/// What the finished file is, from probing it (the caller probes; this crate does not decode media).
#[derive(Debug, Clone, Default)]
pub struct Probed {
    pub duration_us: Option<i64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub has_audio: bool,
}

/// Registers the output (owned by the bundle) and fills the placeholder: one revision.
pub fn finish_ops(doc: &Document, job: &Job, rel_path: &str, probed: &Probed, now_ms: i64) -> Vec<Op> {
    let media_id = format!("m_{}", job.id.trim_start_matches("g_"));
    let file_name = rel_path.rsplit('/').next().unwrap_or(rel_path).to_string();
    let mut data = Map::new();
    data.insert("type".into(), job.kind.item_type().into());
    data.insert("fileName".into(), file_name.into());
    if let Some(d) = probed.duration_us {
        data.insert("duration".into(), micros_to_json(d));
    }
    if let (Some(w), Some(h)) = (probed.width, probed.height) {
        data.insert("width".into(), w.into());
        data.insert("height".into(), h.into());
    }
    if job.kind != Kind::Image {
        data.insert("hasAudio".into(), (probed.has_audio || job.kind.item_type() == "audio").into());
    }
    data.insert("generated".into(), json!({"jobId": job.id, "model": job.model, "prompt": job.request.prompt}));
    let mut ops = vec![Op::AddMedia {
        media: json!({"id": media_id, "createdAt": now_ms, "addedAt": now_ms, "data": Value::Object(data)}),
        entry: Some(
            json!({"mediaId": media_id, "nativePath": rel_path, "ownedByBundle": true, "probeStatus": "probed"}),
        ),
    }];
    if let Some(item) = job.item_id.as_deref().and_then(|id| doc.item(id)) {
        let mut set = Map::new();
        set.insert("pending".into(), false.into());
        set.insert("mediaItemId".into(), media_id.into());
        // Audio and video take their real length (images keep the placeholder's).
        if let Some(d) = probed.duration_us.filter(|_| job.kind != Kind::Image) {
            set.insert("endTime".into(), micros_to_json(item.start + d));
        }
        ops.push(Op::UpdateItem { id: item.id.clone(), set });
        ops.push(Op::UnsetItemFields { id: item.id.clone(), fields: vec!["pendingDescription".into()] });
    }
    ops
}

/// Removes a failed or cancelled job's placeholder (if it is still pending), so it neither blocks export nor sits
/// under a retry. The job record keeps the error.
pub fn failed_ops(doc: &Document, job: &Job) -> Vec<Op> {
    match job.item_id.as_deref().and_then(|id| doc.item(id)) {
        Some(item) if item.is_pending() => vec![Op::RemoveItems { ids: vec![item.id.clone()] }],
        _ => vec![],
    }
}
