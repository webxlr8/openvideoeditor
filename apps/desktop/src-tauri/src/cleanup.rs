//! Clean up voice (add-ai-generation 1.5): removes background noise from a clip's speech, on this computer
//! (`ove_analyze::clean_voice`, RNNoise). Nothing is uploaded and nothing is billed.
//!
//! - The cleaned sound is a new media item (`<media id>-clean`, a WAV in the bundle's `media/cleaned/`), made once
//!   per source and reused by every clip of that source.
//! - An audio clip switches to the cleaned media; its trims and speed still fit, because the cleaned file has the
//!   source's length and timing.
//! - A video clip keeps its picture and is muted; a new audio clip with the same timing plays the cleaned sound, on
//!   the first audio track with room (a new "Clean voice" track when none has).
//! - Everything is one revision, so one Undo puts the clip back.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use ove_core::ops::Op;
use ove_core::time::{Micros, micros_to_json};
use ove_core::{Clock, Document};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use specta::Type;
use tauri::AppHandle;
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::ProjectView;

fn bad(msg: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidArgument, msg)
}

/// The cleaned media's id for a source media id.
pub fn clean_media_id(media_id: &str) -> String {
    format!("{media_id}-clean")
}

/// Item fields that carry over to the audio clip that replaces a video clip's sound.
const SOUND_FIELDS: &[&str] =
    &["trimStart", "trimEnd", "volume", "audioFadeIn", "audioFadeOut", "playbackSpeed", "playbackDirection"];

/// The ops that put the cleaned media on the timeline for `item_id` (see the module docs). `add_media` is the
/// AddMedia op when the cleaned media is not in the project yet; `new_id` makes item and track ids.
pub fn cleanup_ops(
    doc: &Document,
    item_id: &str,
    clean_id: &str,
    add_media: Option<Op>,
    new_id: &mut dyn FnMut() -> String,
) -> AppResult<Vec<Op>> {
    let item = doc.item(item_id).ok_or_else(|| bad("That clip is not in the project."))?;
    let mut ops: Vec<Op> = add_media.into_iter().collect();
    let mut set = Map::new();
    match item.kind.as_str() {
        "audio" => {
            set.insert("mediaItemId".into(), clean_id.into());
            ops.push(Op::UpdateItem { id: item.id.clone(), set });
        }
        "video" => {
            set.insert("muted".into(), true.into());
            ops.push(Op::UpdateItem { id: item.id.clone(), set });
            let free =
                |track: &str| doc.items_on_track(track).iter().all(|i| i.end <= item.start || i.start >= item.end);
            let track = match doc.project.tracks.iter().find(|t| t.kind == "audio" && free(&t.id)) {
                Some(t) => t.id.clone(),
                None => {
                    let id = format!("t_{}", new_id().trim_start_matches("i_"));
                    let order = doc.project.tracks.len();
                    ops.push(Op::AddTrack {
                        track: json!({"id": id, "name": "Clean voice", "order": order, "type": "audio"}),
                    });
                    id
                }
            };
            let name = item.fields.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or("Clip");
            let mut audio = json!({
                "id": new_id(),
                "trackId": track,
                "type": "audio",
                "startTime": micros_to_json(item.start),
                "endTime": micros_to_json(item.end),
                "mediaItemId": clean_id,
                "name": format!("{name} (clean voice)"),
            });
            for k in SOUND_FIELDS {
                if let Some(v) = item.fields.get(*k) {
                    audio[*k] = v.clone();
                }
            }
            ops.push(Op::AddItem { item: audio });
        }
        _ => return Err(bad("Clean up voice works on video and audio clips.")),
    }
    Ok(ops)
}

/// The AddMedia op for a cleaned file at `rel` (bundle-relative) of `seconds`, made from `source`.
fn add_media_op(clean_id: &str, source: &str, rel: &str, seconds: f64, now: i64) -> Op {
    let file_name = rel.rsplit('/').next().unwrap_or(rel);
    Op::AddMedia {
        media: json!({"id": clean_id, "createdAt": now, "addedAt": now, "data": {
            "type": "audio",
            "fileName": file_name,
            "duration": micros_to_json((seconds * 1e6).round() as Micros),
            "hasAudio": true,
            "cleanedFrom": source,
        }}),
        entry: Some(json!({"mediaId": clean_id, "nativePath": rel, "ownedByBundle": true, "probeStatus": "probed"})),
    }
}

/// A free bundle-relative path under `media/cleaned/` for a source file.
fn output_path(bundle: &Path, source: &Path) -> (String, PathBuf) {
    let stem: String = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(60)
        .collect();
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem}-clean.wav") } else { format!("{stem}-clean-{n}.wav") };
        let rel = format!("media/cleaned/{name}");
        let abs = bundle.join("media").join("cleaned").join(&name);
        if !abs.exists() {
            return (rel, abs);
        }
        n += 1;
    }
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioCleaned {
    pub view: ProjectView,
    /// Seconds of audio cleaned (0 when an earlier clean-up of the same source was reused).
    pub seconds: f64,
    pub reused: bool,
}

/// Progress (0–1) of cleaning up the sound of clip `id`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct AudioCleanupProgress {
    pub id: String,
    pub progress: f64,
}

/// Removes background noise from the speech of a video or audio clip (see the module docs).
#[tauri::command]
#[specta::specta]
pub async fn clean_up_audio(app: AppHandle, id: String) -> AppResult<AudioCleaned> {
    crate::projects::serial(&app, "clean_up_audio", move |app, state| {
        let (doc, bundle) = {
            let g = crate::projects::lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| bad("No project is open."))?;
            (p.doc().clone(), p.bundle().to_path_buf())
        };
        let item = doc.item(&id).ok_or_else(|| bad("That clip is not in the project."))?;
        if item.fields.get("muted").and_then(Value::as_bool) == Some(true) {
            return Err(bad("This clip is muted, so there is no sound to clean up."));
        }
        let clip = crate::captions::source_clips(&doc, &bundle, std::slice::from_ref(&id))?
            .into_iter()
            .next()
            .ok_or_else(|| bad("That clip has no sound."))?;
        if doc.project.media.get(&clip.media_id).and_then(|m| m.data()).and_then(|d| d.get("cleanedFrom")).is_some() {
            return Err(bad("This clip's sound is already cleaned up."));
        }
        let clean_id = clean_media_id(&clip.media_id);
        let have = doc.project.media.contains_key(&clean_id)
            && ove_render::sources::media_path(&doc, Some(&bundle), &clean_id).is_some_and(|p| p.exists());
        let (add, seconds) = if have {
            (None, 0.0)
        } else {
            let (rel, abs) = output_path(&bundle, &clip.path);
            std::fs::create_dir_all(abs.parent().unwrap_or(&bundle))
                .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
            let mut last = -1.0;
            let mut report = |p: f64| {
                if p - last >= 0.02 || p >= 1.0 {
                    last = p;
                    let _ = AudioCleanupProgress { id: id.clone(), progress: p }.emit(app);
                }
            };
            let done = ove_analyze::clean_voice(&clip.path, &abs, &AtomicBool::new(false), &mut report)
                .map_err(|e| AppError::new(ErrorCode::Internal, format!("Could not clean up the sound: {e}")))?;
            let now = ove_core::SystemClock.now_ms();
            // An earlier clean-up whose file was deleted: point the media at the new file instead of adding it.
            let op = if doc.project.media.contains_key(&clean_id) {
                let mut entry_set = Map::new();
                entry_set.insert("nativePath".into(), rel.into());
                entry_set.insert("ownedByBundle".into(), true.into());
                Op::UpdateMedia { id: clean_id.clone(), set: Map::new(), unset: vec![], entry_set, entry_unset: vec![] }
            } else {
                add_media_op(&clean_id, &clip.media_id, &rel, done.seconds, now)
            };
            (Some(op), done.seconds)
        };
        let ops = cleanup_ops(&doc, &id, &clean_id, add, &mut crate::edits::new_item_id)?;
        let view = crate::projects::edit(app, state, ops, "clean up voice")?;
        Ok(AudioCleaned { view, seconds, reused: have })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::revision::Source;
    use std::sync::Arc;

    fn doc() -> (tempfile::TempDir, ove_store::Project) {
        let d = tempfile::tempdir().unwrap();
        let lib = ove_store::Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
        let mut p = lib.create("Clean").unwrap();
        let ops: Vec<Op> = serde_json::from_value(json!([
            {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 0, "type": "video"}},
            {"op": "addTrack", "track": {"id": "t_music", "name": "Music", "order": 1, "type": "audio"}},
            {"op": "addMedia", "media": {"id": "m", "data": {"type": "video", "fileName": "talk.mp4", "duration": 60, "hasAudio": true}}},
            {"op": "addMedia", "media": {"id": "v", "data": {"type": "audio", "fileName": "vo.wav", "duration": 60}}},
            {"op": "addItem", "item": {"id": "clip", "trackId": "t_main", "type": "video", "mediaItemId": "m", "startTime": 2, "endTime": 10, "trimStart": 1, "volume": 80, "playbackSpeed": 1.5, "name": "Talk"}},
            {"op": "addItem", "item": {"id": "music", "trackId": "t_music", "type": "audio", "mediaItemId": "v", "startTime": 0, "endTime": 4}},
            {"op": "addItem", "item": {"id": "vo", "trackId": "t_music", "type": "audio", "mediaItemId": "v", "startTime": 20, "endTime": 30, "trimStart": 3}},
        ]))
        .unwrap();
        p.apply(ops, Source::Gui, None).unwrap();
        (d, p)
    }

    fn ids() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("i_new{n}")
        }
    }

    #[test]
    fn a_video_clip_is_muted_and_its_clean_voice_plays_alongside() {
        let (_d, mut p) = doc();
        let add = add_media_op("m-clean", "m", "media/cleaned/talk-clean.wav", 60.0, 1);
        let ops = cleanup_ops(p.doc(), "clip", "m-clean", Some(add), &mut ids()).unwrap();
        p.apply(ops, Source::Gui, None).unwrap();
        let d = p.doc();
        assert_eq!(d.item("clip").unwrap().fields["muted"], json!(true));
        // The music track is busy from 0 to 4 s, across the clip: a new audio track at the bottom.
        let voice = d.project.items.values().find(|i| i.media_id() == Some("m-clean")).unwrap();
        assert_ne!(voice.track_id, "t_music");
        assert_eq!(d.track(&voice.track_id).unwrap().kind, "audio");
        assert_eq!((voice.start, voice.end), (2_000_000, 10_000_000));
        assert_eq!(voice.time_field("trimStart"), 1_000_000);
        assert_eq!(voice.fields["volume"], json!(80));
        assert_eq!(voice.fields["playbackSpeed"], json!(1.5));
        assert_eq!(voice.fields["name"], json!("Talk (clean voice)"));
        let media = d.project.media.get("m-clean").and_then(|m| m.data()).unwrap();
        assert_eq!(media["cleanedFrom"], json!("m"));
        // One revision: one undo puts it all back.
        p.undo().unwrap();
        assert!(!p.doc().project.media.contains_key("m-clean"));
        assert!(p.doc().item("clip").unwrap().fields.get("muted").is_none());
    }

    #[test]
    fn an_audio_clip_switches_to_the_clean_media_and_keeps_its_trim() {
        let (_d, mut p) = doc();
        let add = add_media_op("v-clean", "v", "media/cleaned/vo-clean.wav", 60.0, 1);
        let ops = cleanup_ops(p.doc(), "vo", "v-clean", Some(add), &mut ids()).unwrap();
        p.apply(ops, Source::Gui, None).unwrap();
        let vo = p.doc().item("vo").unwrap();
        assert_eq!(vo.media_id(), Some("v-clean"));
        assert_eq!(vo.time_field("trimStart"), 3_000_000);
        assert!(cleanup_ops(p.doc(), "nope", "x", None, &mut ids()).is_err());
    }

    #[test]
    fn a_free_audio_track_is_reused() {
        let (_d, mut p) = doc();
        let free: Vec<Op> = serde_json::from_value(json!([{"op": "removeItems", "ids": ["music"]}])).unwrap();
        p.apply(free, Source::Gui, None).unwrap();
        let ops = cleanup_ops(p.doc(), "clip", "m-clean", None, &mut ids()).unwrap();
        assert!(!ops.iter().any(|o| matches!(o, Op::AddTrack { .. })));
        assert!(ops.iter().any(|o| matches!(o, Op::AddItem { item } if item["trackId"] == "t_music")));
    }

    #[test]
    fn output_names_do_not_collide() {
        let d = tempfile::tempdir().unwrap();
        let (rel, abs) = output_path(d.path(), Path::new("C:/x/My talk!.mp4"));
        assert_eq!(rel, "media/cleaned/My-talk--clean.wav");
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, b"x").unwrap();
        assert_eq!(output_path(d.path(), Path::new("My talk!.mp4")).0, "media/cleaned/My-talk--clean-2.wav");
    }
}
