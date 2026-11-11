//! OpenTimelineIO JSON (`.otio`): Timeline.1 → Stack.1 → Track.1 (Video tracks bottom-first, then Audio) → Clip.2 /
//! Gap.1 with ExternalReference.1 media.

use serde_json::{Value, json};

use crate::model::{Clip, ClipKind, Rate, Timeline, Track, file_url, url_path};
use crate::{Error, Result};

fn rt(v: i64, rate: Rate) -> Value {
    json!({"OTIO_SCHEMA": "RationalTime.1", "rate": rate.fps(), "value": v as f64})
}

fn range(start: i64, dur: i64, rate: Rate) -> Value {
    json!({"OTIO_SCHEMA": "TimeRange.1", "start_time": rt(start, rate), "duration": rt(dur, rate)})
}

fn track_json(t: &Track, kind: &str, rate: Rate) -> Value {
    let mut children = Vec::new();
    let mut cursor = 0;
    for c in &t.clips {
        if c.start > cursor {
            children
                .push(json!({"OTIO_SCHEMA": "Gap.1", "name": "", "source_range": range(0, c.start - cursor, rate)}));
        }
        let media = match &c.path {
            Some(p) => json!({
                "OTIO_SCHEMA": "ExternalReference.1",
                "target_url": file_url(p),
                "available_range": c.media_duration.map(|d| range(0, d, rate)).unwrap_or(Value::Null),
            }),
            None => json!({"OTIO_SCHEMA": "MissingReference.1"}),
        };
        let mut clip = json!({
            "OTIO_SCHEMA": "Clip.2",
            "name": c.name,
            "source_range": range(c.source_in, c.duration(), rate),
            "media_references": {"DEFAULT_MEDIA": media},
            "active_media_reference_key": "DEFAULT_MEDIA",
            "metadata": {},
        });
        if (c.speed - 1.0).abs() > 1e-6 {
            clip["effects"] = json!([{"OTIO_SCHEMA": "LinearTimeWarp.1", "name": "", "effect_name": "LinearTimeWarp", "time_scalar": c.speed}]);
        }
        if let Some(text) = &c.text {
            clip["metadata"] = json!({"ove": {"title": text}});
        }
        children.push(clip);
        cursor = c.end;
    }
    json!({"OTIO_SCHEMA": "Track.1", "name": t.name, "kind": kind, "children": children})
}

pub fn write(t: &Timeline) -> String {
    let mut tracks: Vec<Value> = t.video.iter().map(|tr| track_json(tr, "Video", t.rate)).collect();
    tracks.extend(t.audio.iter().map(|tr| track_json(tr, "Audio", t.rate)));
    let v = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": t.name,
        "global_start_time": rt(0, t.rate),
        "metadata": {"ove": {"width": t.width, "height": t.height}},
        "tracks": {"OTIO_SCHEMA": "Stack.1", "name": "tracks", "children": tracks},
    });
    let mut s = serde_json::to_string_pretty(&v).expect("json");
    s.push('\n');
    s
}

fn time_of(v: &Value) -> Option<(f64, f64)> {
    Some((v.get("value")?.as_f64()?, v.get("rate")?.as_f64()?))
}

fn frames(v: &Value, rate: Rate) -> i64 {
    match time_of(v) {
        Some((value, r)) if r > 0.0 => (value / r * rate.fps()).round() as i64,
        _ => 0,
    }
}

/// Speed of a LinearTimeWarp, or of an FCP7 "Time Remap" effect kept as metadata by the FCP XML adapter.
fn effect_speed(e: &Value) -> Option<f64> {
    if let Some(s) = e.get("time_scalar").and_then(Value::as_f64) {
        return Some(s);
    }
    let fx = e.pointer("/metadata/fcp_xml")?;
    if fx.get("effectid").and_then(Value::as_str) != Some("timeremap") {
        return None;
    }
    let params: Vec<&Value> = match fx.get("parameter")? {
        Value::Array(a) => a.iter().collect(),
        p => vec![p],
    };
    let speed = params.iter().find(|p| p.get("parameterid").and_then(Value::as_str) == Some("speed"))?;
    let v = speed.get("value")?;
    let pct = v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())?;
    (pct > 0.0).then_some(pct / 100.0)
}

pub fn read(text: &str, rate_hint: Option<Rate>) -> Result<Timeline> {
    let v: Value = serde_json::from_str(text).map_err(|e| Error::Parse(format!("not OTIO JSON: {e}")))?;
    if !v.get("OTIO_SCHEMA").and_then(Value::as_str).is_some_and(|s| s.starts_with("Timeline")) {
        return Err(Error::Parse("the file is not an OpenTimelineIO timeline".into()));
    }
    let stack = v
        .pointer("/tracks/children")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Parse("timeline has no tracks".into()))?;
    // Rate: hint, else the first clip's source rate.
    let rate = rate_hint.unwrap_or_else(|| {
        stack
            .iter()
            .flat_map(|t| t.get("children").and_then(Value::as_array).into_iter().flatten())
            .find_map(|c| c.pointer("/source_range/duration/rate").and_then(Value::as_f64))
            .map(Rate::from_fps)
            .unwrap_or(Rate::FPS30)
    });
    let mut out = Timeline {
        name: v.get("name").and_then(Value::as_str).unwrap_or("Imported OTIO").to_string(),
        rate,
        width: v.pointer("/metadata/ove/width").and_then(Value::as_u64).unwrap_or(1920) as u32,
        height: v.pointer("/metadata/ove/height").and_then(Value::as_u64).unwrap_or(1080) as u32,
        video: vec![],
        audio: vec![],
    };
    for tr in stack {
        let kind = tr.get("kind").and_then(Value::as_str).unwrap_or("Video");
        let mut track = Track { name: tr.get("name").and_then(Value::as_str).unwrap_or("").to_string(), clips: vec![] };
        let mut cursor = 0;
        for c in tr.get("children").and_then(Value::as_array).into_iter().flatten() {
            let schema = c.get("OTIO_SCHEMA").and_then(Value::as_str).unwrap_or("");
            let dur = c.pointer("/source_range/duration").map(|d| frames(d, rate)).unwrap_or(0);
            if schema.starts_with("Gap") {
                cursor += dur;
                continue;
            }
            if schema.starts_with("Transition") {
                continue;
            }
            if !schema.starts_with("Clip") {
                cursor += dur;
                continue;
            }
            let key = c.get("active_media_reference_key").and_then(Value::as_str).unwrap_or("DEFAULT_MEDIA");
            let media = c.pointer(&format!("/media_references/{key}")).or_else(|| c.get("media_reference"));
            let path = media.and_then(|m| m.get("target_url")).and_then(Value::as_str).map(url_path);
            let speed =
                c.get("effects").and_then(Value::as_array).and_then(|e| e.iter().find_map(effect_speed)).unwrap_or(1.0);
            let text = c.pointer("/metadata/ove/title").and_then(Value::as_str).map(str::to_string);
            let is_audio = kind == "Audio";
            track.clips.push(Clip {
                name: c.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                id: None,
                kind: if text.is_some() {
                    ClipKind::Title
                } else if is_audio {
                    ClipKind::Audio
                } else {
                    ClipKind::Video
                },
                path,
                media_id: None,
                start: cursor,
                end: cursor + dur,
                source_in: c.pointer("/source_range/start_time").map(|s| frames(s, rate)).unwrap_or(0),
                speed,
                media_duration: media.and_then(|m| m.pointer("/available_range/duration")).map(|d| frames(d, rate)),
                has_audio: is_audio,
                has_video: !is_audio,
                text,
            });
            cursor += dur;
        }
        if kind == "Audio" { out.audio.push(track) } else { out.video.push(track) }
    }
    Ok(out)
}
