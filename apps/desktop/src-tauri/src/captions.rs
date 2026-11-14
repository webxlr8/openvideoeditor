//! Captions (roadmap change 10): speech-model status and download, auto-captions from clip audio (offline
//! transcription, cached per media file in the bundle), and the caption style list. A captions run is one revision:
//! caption items on a Captions track, aligned to timeline time through each clip's trim and speed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use ove_core::Document;
use ove_core::ops::Op;
use ove_core::time::{Micros, micros_to_json};
use ove_transcribe::{Model, SegmentOptions, Transcript, Word, models};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::AppState;
use crate::edits::new_item_id;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, edit, lock, serial};
use crate::worker::{self, relock};

fn bad(msg: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidArgument, msg)
}

/// A speech model and whether it is on this computer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CaptionModel {
    pub id: String,
    pub label: String,
    pub size_mb: u32,
    pub installed: bool,
    pub default: bool,
}

/// Progress of a captions job (model download, transcription, applying); `done` ends it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct CaptionJobProgress {
    /// download | audio | transcribe | apply | done
    pub stage: String,
    pub fraction: f64,
    pub message: String,
    pub done: bool,
    pub error: Option<String>,
    pub cancelled: bool,
    /// Caption items created (when done).
    pub captions: Option<u32>,
    pub language: Option<String>,
    pub seconds_taken: Option<f64>,
}

/// What to caption.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AutoCaptionRequest {
    /// Clips to transcribe; empty = every clip on the Main track (or every clip with sound).
    pub item_ids: Vec<String>,
    /// ISO 639-1 code, or None / "auto" to detect.
    pub language: Option<String>,
    /// tiny | base | small
    pub model: String,
    /// Caption style for the new items.
    pub preset_id: Option<String>,
    /// Replace captions already on the Captions track in the captioned range.
    pub replace: bool,
}

fn data_dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("openvideoeditor"))
}

fn job() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    static J: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
    J.get_or_init(|| Mutex::new(None))
}

/// Claims the single captions job slot.
fn start_job() -> AppResult<Arc<AtomicBool>> {
    let mut slot = relock(job());
    if slot.is_some() {
        return Err(bad("Captions are already being made. Wait for them to finish or cancel."));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *slot = Some(cancel.clone());
    Ok(cancel)
}

fn end_job() {
    *relock(job()) = None;
}

/// The speech models (none ship with the app; each downloads on first use).
#[tauri::command]
#[specta::specta]
pub async fn caption_models(app: AppHandle) -> AppResult<Vec<CaptionModel>> {
    worker::blocking("caption_models", move || {
        let dir = models::model_dir(&data_dir(&app));
        Ok(Model::ALL
            .iter()
            .map(|m| CaptionModel {
                id: m.id().into(),
                label: m.label().into(),
                size_mb: (m.size_bytes() / 1_000_000) as u32,
                installed: models::is_installed(&dir, *m),
                default: *m == Model::Base,
            })
            .collect())
    })
    .await
}

/// The caption styles as JSON (`[{id, label, fontSize, textColor, highlightColor, animation, …}]`).
#[tauri::command]
#[specta::specta]
pub fn caption_presets() -> String {
    ove_render::captions_presets::presets_json().to_string()
}

/// Stops the running captions job (download or transcription).
#[tauri::command]
#[specta::specta]
pub fn cancel_captions() -> AppResult<()> {
    if let Some(c) = relock(job()).as_ref() {
        c.store(true, Ordering::SeqCst);
    }
    Ok(())
}

fn progress(app: &AppHandle, stage: &str, fraction: f64, message: impl Into<String>) {
    let _ =
        CaptionJobProgress { stage: stage.into(), fraction, message: message.into(), ..Default::default() }.emit(app);
}

fn ensure(app: &AppHandle, model: Model, cancel: &AtomicBool) -> Result<PathBuf, ove_transcribe::TranscribeError> {
    let dir = models::model_dir(&data_dir(app));
    let label = model.label();
    let mut last = -1i64;
    models::ensure_model(
        &dir,
        model,
        &mut |done, total| {
            let permille = (done * 1000 / total.max(1)) as i64;
            if permille != last {
                last = permille;
                progress(
                    app,
                    "download",
                    done as f64 / total.max(1) as f64,
                    format!("Downloading the {label} speech model ({} of {} MB)", done / 1_000_000, total / 1_000_000),
                );
            }
        },
        cancel,
    )
}

/// Downloads a speech model in the background (progress as `captionJobProgress`).
#[tauri::command]
#[specta::specta]
pub fn download_caption_model(app: AppHandle, model: String) -> AppResult<()> {
    let m = Model::parse(&model).ok_or_else(|| bad(format!("Unknown speech model {model:?}.")))?;
    let cancel = start_job()?;
    crate::worker::spawn_contained("caption model download", move || {
        let end = match ensure(&app, m, &cancel) {
            Ok(_) => CaptionJobProgress { stage: "done".into(), fraction: 1.0, done: true, ..Default::default() },
            Err(e) => finished_with(e),
        };
        end_job();
        let _ = end.emit(&app);
    })
    .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    Ok(())
}

fn finished_with(e: ove_transcribe::TranscribeError) -> CaptionJobProgress {
    let cancelled = matches!(e, ove_transcribe::TranscribeError::Cancelled);
    CaptionJobProgress {
        stage: "done".into(),
        done: true,
        cancelled,
        error: (!cancelled).then(|| e.to_string()),
        ..Default::default()
    }
}

/// A clip whose sound is captioned: the timeline range it occupies and the part of its media it plays.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceClip {
    pub item_id: String,
    pub media_id: String,
    pub path: PathBuf,
    pub start: Micros,
    pub end: Micros,
    pub trim: Micros,
    pub speed: f64,
    pub reverse: bool,
}

/// The clips to caption: the given ids, else everything on the Main track, else every clip with sound.
pub fn source_clips(doc: &Document, bundle: &Path, ids: &[String]) -> AppResult<Vec<SourceClip>> {
    let to_clip = |i: &ove_core::Item| -> Option<SourceClip> {
        if !matches!(i.kind.as_str(), "video" | "audio")
            || i.fields.get("pending").and_then(Value::as_bool) == Some(true)
        {
            return None;
        }
        let media_id = i.media_id()?.to_string();
        let data = doc.project.media.get(&media_id).and_then(|m| m.data());
        if data.and_then(|d| d.get("type")).and_then(Value::as_str) == Some("image") {
            return None;
        }
        if data.and_then(|d| d.get("hasAudio")).and_then(Value::as_bool) == Some(false) {
            return None;
        }
        Some(SourceClip {
            item_id: i.id.clone(),
            path: ove_render::sources::media_path(doc, Some(bundle), &media_id)?,
            media_id,
            start: i.start,
            end: i.end,
            trim: i.time_field("trimStart"),
            speed: i.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01),
            reverse: i.fields.get("playbackDirection").and_then(Value::as_str) == Some("reverse"),
        })
    };
    let mut clips: Vec<SourceClip> = if ids.is_empty() {
        let main: Vec<SourceClip> = doc.items_on_track("t_main").into_iter().filter_map(to_clip).collect();
        if main.is_empty() { doc.project.items.values().filter_map(|i| to_clip(i)).collect() } else { main }
    } else {
        ids.iter().filter_map(|id| doc.item(id)).filter_map(to_clip).collect()
    };
    clips.sort_by_key(|c| (c.start, c.item_id.clone()));
    if clips.is_empty() {
        return Err(bad(if ids.is_empty() {
            "There is no clip with sound to caption. Add a video or audio clip first."
        } else {
            "The selected clips have no sound to caption."
        }));
    }
    Ok(clips)
}

/// The transcript's words that play in a clip, moved to timeline time (trim, speed and direction applied).
pub fn timeline_words(clip: &SourceClip, transcript: &Transcript) -> Vec<Word> {
    let used = ((clip.end - clip.start) as f64 * clip.speed) as Micros;
    let (lo, hi) = (clip.trim, clip.trim + used);
    let to_tl = |src: Micros| -> Micros {
        let rel = if clip.reverse { hi - src } else { src - lo };
        clip.start + (rel as f64 / clip.speed) as Micros
    };
    let mut out: Vec<Word> = transcript
        .words
        .iter()
        // Keep words mostly inside the used range.
        .filter(|w| {
            let mid = (w.start_us + w.end_us) / 2;
            mid >= lo && mid < hi
        })
        .map(|w| {
            let (a, b) = (to_tl(w.start_us.clamp(lo, hi)), to_tl(w.end_us.clamp(lo, hi)));
            Word {
                text: w.text.clone(),
                start_us: a.min(b),
                end_us: a.max(b).max(a.min(b) + 20_000),
                confidence: w.confidence,
            }
        })
        .collect();
    if clip.reverse {
        out.reverse();
    }
    out
}

/// Ops that put one caption item per caption segment on the Captions track. Returns the ops and the item count.
pub fn caption_ops(doc: &Document, words: &[Word], preset_id: Option<&str>, replace: bool) -> (Vec<Op>, usize) {
    let segments = ove_transcribe::segment(words, &SegmentOptions::default());
    if segments.is_empty() {
        return (Vec::new(), 0);
    }
    let (lo, hi) = (segments[0].start_us, segments[segments.len() - 1].end_us);
    let mut ops = Vec::new();
    let caption_track = doc.project.tracks.iter().filter(|t| t.kind == "caption").min_by_key(|t| t.order);
    let overlapping = |track: &str| -> Vec<String> {
        doc.items_on_track(track).iter().filter(|i| i.start < hi && i.end > lo).map(|i| i.id.clone()).collect()
    };
    let track_id = match caption_track {
        Some(t) => {
            let clash = overlapping(&t.id);
            if clash.is_empty() {
                t.id.clone()
            } else if replace {
                ops.push(Op::RemoveItems { ids: clash });
                t.id.clone()
            } else {
                let id = format!("t_{}", new_item_id().trim_start_matches("i_"));
                ops.push(Op::AddTrack {
                    track: json!({"id": id, "name": "Captions 2", "order": 0, "type": "caption"}),
                });
                id
            }
        }
        None => {
            let id = "t_captions".to_string();
            let id =
                if doc.track(&id).is_some() { format!("t_{}", new_item_id().trim_start_matches("i_")) } else { id };
            ops.push(Op::AddTrack { track: json!({"id": id, "name": "Captions", "order": 0, "type": "caption"}) });
            id
        }
    };
    let n = segments.len();
    for (i, s) in segments.iter().enumerate() {
        // Hold a caption until the next one (up to half a second) so short gaps do not flash.
        let next = segments.get(i + 1).map(|x| x.start_us).unwrap_or(Micros::MAX);
        let end = (s.end_us + 500_000).min(next).max(s.start_us + 200_000);
        let start = s.start_us;
        let rel = |t: Micros| micros_to_json(t - start);
        let timings: Vec<Value> =
            s.words.iter().map(|w| json!({"word": w.text, "start": rel(w.start_us), "end": rel(w.end_us)})).collect();
        let mut item = json!({
            "id": new_item_id(),
            "trackId": track_id,
            "type": "caption",
            "startTime": micros_to_json(start),
            "endTime": micros_to_json(end),
            "name": "Caption",
            "captionText": s.text,
            "captionSegments": [{"id": "s1", "text": s.text, "startTime": 0, "endTime": rel(end), "wordTimings": timings}],
        });
        if let Some(p) = preset_id {
            item["captionPresetId"] = p.into();
        }
        ops.push(Op::AddItem { item });
    }
    (ops, n)
}

/// Transcribes one media file (or takes the bundle's cached transcript).
fn transcript_for(
    app: &AppHandle,
    bundle: &Path,
    (model_path, model): (&Path, Model),
    clip: &SourceClip,
    language: Option<&str>,
    cancel: &Arc<AtomicBool>,
    (index, count): (usize, usize),
) -> Result<Transcript, ove_transcribe::TranscribeError> {
    let key = ove_transcribe::cache::key(&clip.path, model.id(), language);
    if let Some(t) = ove_transcribe::cache::load(bundle, &key) {
        return Ok(t);
    }
    let name = clip.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if cancel.load(Ordering::SeqCst) {
        return Err(ove_transcribe::TranscribeError::Cancelled);
    }
    let opts = ove_transcribe::Options {
        language: language.map(str::to_string),
        model_name: model.id().into(),
        ..Default::default()
    };
    let app2 = app.clone();
    let label = if count > 1 {
        format!("Transcribing {name} ({} of {count})", index + 1)
    } else {
        format!("Transcribing {name}")
    };
    let label2 = label.clone();
    progress(app, "transcribe", index as f64 / count as f64, label);
    // Decoded and recognised window by window, so a long recording never needs all of its audio in memory.
    let t = ove_transcribe::transcribe_file(
        model_path,
        &clip.path,
        &opts,
        move |pct| progress(&app2, "transcribe", (index as f64 + pct as f64 / 100.0) / count as f64, label2.clone()),
        cancel.clone(),
    )?;
    ove_transcribe::cache::store(bundle, &key, &t);
    Ok(t)
}

/// Makes captions from the sound of clips, in the background: downloads the model if needed, transcribes (offline),
/// and adds the caption items as one revision. Progress and the result arrive as `captionJobProgress` events.
#[tauri::command]
#[specta::specta]
pub async fn auto_caption(app: AppHandle, request: AutoCaptionRequest) -> AppResult<()> {
    worker::blocking("auto_caption", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        let model =
            Model::parse(&request.model).ok_or_else(|| bad(format!("Unknown speech model {:?}.", request.model)))?;
        let (doc, bundle) = {
            let g = lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| bad("Open a project first."))?;
            (p.doc().clone(), p.bundle().to_path_buf())
        };
        let clips = source_clips(&doc, &bundle, &request.item_ids)?;
        let cancel = start_job()?;
        let language = request.language.clone().filter(|l| !l.is_empty() && l != "auto");
        crate::worker::spawn_contained("captions", move || {
            let started = std::time::Instant::now();
            let run = || -> Result<(usize, String), ove_transcribe::TranscribeError> {
                let model_path = ensure(&app, model, &cancel)?;
                let mut words = Vec::new();
                let mut detected = String::new();
                for (i, clip) in clips.iter().enumerate() {
                    match transcript_for(
                        &app,
                        &bundle,
                        (&model_path, model),
                        clip,
                        language.as_deref(),
                        &cancel,
                        (i, clips.len()),
                    ) {
                        Ok(t) => {
                            if detected.is_empty() {
                                detected = t.language.clone();
                            }
                            words.extend(timeline_words(clip, &t));
                        }
                        // A clip without sound is skipped; the others still get captions.
                        Err(ove_transcribe::TranscribeError::NoAudio) => continue,
                        Err(e) => return Err(e),
                    }
                }
                words.sort_by_key(|w| w.start_us);
                progress(&app, "apply", 1.0, "Adding captions");
                let state = app.state::<AppState>();
                // The project may have changed while transcribing: build the ops on the current document.
                let doc = {
                    let g = lock(&state.projects.open).map_err(|e| ove_transcribe::TranscribeError::Io(e.message))?;
                    match g.as_ref() {
                        Some(p) if p.bundle() == bundle => p.doc().clone(),
                        _ => return Err(ove_transcribe::TranscribeError::Io("The project was closed.".into())),
                    }
                };
                let (ops, n) = caption_ops(&doc, &words, request.preset_id.as_deref(), request.replace);
                if n > 0 {
                    edit(&app, &state, ops, "auto captions")
                        .map_err(|e| ove_transcribe::TranscribeError::Io(e.message))?;
                }
                Ok((n, detected))
            };
            let end = match run() {
                Ok((n, language)) => {
                    tracing::info!(captions = n, seconds = started.elapsed().as_secs_f64(), "auto captions finished");
                    CaptionJobProgress {
                        stage: "done".into(),
                        fraction: 1.0,
                        done: true,
                        captions: Some(n as u32),
                        language: (!language.is_empty()).then_some(language),
                        seconds_taken: Some(started.elapsed().as_secs_f64()),
                        message: if n == 0 { "No speech was found.".into() } else { String::new() },
                        ..Default::default()
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "auto captions did not finish");
                    finished_with(e)
                }
            };
            end_job();
            let _ = end.emit(&app);
        })
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
        Ok(())
    })
    .await
}

/// Applies a caption style to caption items (all of them when `item_ids` is empty), clearing per-item overrides of
/// size and colours so the style shows as designed. One revision.
#[tauri::command]
#[specta::specta]
pub async fn apply_caption_style(app: AppHandle, preset_id: String, item_ids: Vec<String>) -> AppResult<ProjectView> {
    serial(&app, "apply_caption_style", move |app, state| {
        let preset = ove_render::captions_presets::preset(&preset_id)
            .ok_or_else(|| bad(format!("Unknown caption style {preset_id:?}.")))?;
        let ids: Vec<String> = {
            let g = lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| bad("Open a project first."))?;
            let doc = p.doc();
            doc.project
                .items
                .values()
                .filter(|i| i.kind == "caption" && (item_ids.is_empty() || item_ids.contains(&i.id)))
                .map(|i| i.id.clone())
                .collect()
        };
        if ids.is_empty() {
            return Err(bad("There are no captions to style yet."));
        }
        let ops = style_ops(&ids, preset.id);
        edit(app, state, ops, &format!("caption style {}", preset.label.to_lowercase()))
    })
    .await
}

pub fn style_ops(ids: &[String], preset_id: &str) -> Vec<Op> {
    let overrides = ["captionFontSize", "captionTextColor", "captionHighlightColor", "captionPosition"];
    ids.iter()
        .flat_map(|id| {
            let mut set = serde_json::Map::new();
            set.insert("captionPresetId".into(), preset_id.into());
            [
                Op::UnsetItemFields { id: id.clone(), fields: overrides.iter().map(|s| s.to_string()).collect() },
                Op::UpdateItem { id: id.clone(), set },
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::ops::apply_batch;

    fn doc() -> Document {
        let mut ops = crate::projects::default_tracks();
        ops.push(Op::AddMedia {
            media: json!({"id": "m", "data": {"type": "video", "fileName": "talk.mp4", "duration": 60, "hasAudio": true}}),
            entry: Some(json!({"mediaId": "m", "nativePath": "/media/talk.mp4", "ownedByBundle": false})),
        });
        // Plays source 2 s – 8 s at double speed from timeline 10 s to 13 s.
        ops.push(Op::AddItem {
            item: json!({"id": "v", "trackId": "t_main", "type": "video", "startTime": 10, "endTime": 13,
                "mediaItemId": "m", "trimStart": 2, "playbackSpeed": 2}),
        });
        ops.push(Op::AddItem {
            item: json!({"id": "old", "trackId": "t_captions", "type": "caption", "startTime": 10, "endTime": 11,
                "captionText": "old"}),
        });
        apply_batch(&Document::new("p", "p", 1), &ops).unwrap().doc
    }

    fn w(t: &str, s: f64, e: f64) -> Word {
        Word { text: t.into(), start_us: (s * 1e6) as i64, end_us: (e * 1e6) as i64, confidence: 0.9 }
    }

    fn transcript() -> Transcript {
        Transcript {
            language: "en".into(),
            model: "base".into(),
            words: vec![
                w("Before", 0.5, 1.0),
                w("Hello", 2.5, 3.0),
                w("there.", 3.0, 3.5),
                w("Second", 6.0, 6.6),
                w("line", 6.6, 7.0),
                w("After", 9.0, 9.5),
            ],
            duration_us: 60_000_000,
        }
    }

    #[test]
    fn words_follow_trim_and_speed() {
        let d = doc();
        let clips = source_clips(&d, Path::new("/bundle"), &[]).unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!((clips[0].trim, clips[0].speed), (2_000_000, 2.0));
        let words = timeline_words(&clips[0], &transcript());
        let texts: Vec<_> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Hello", "there.", "Second", "line"]);
        // Source 2.5 s → 10 s + 0.5 s / 2.
        assert_eq!(words[0].start_us, 10_250_000);
        assert_eq!(words[3].end_us, 12_500_000);
        let rev = SourceClip { reverse: true, ..clips[0].clone() };
        let r = timeline_words(&rev, &transcript());
        assert_eq!(r[0].text, "line");
        assert!(r.windows(2).all(|p| p[0].start_us <= p[1].start_us));
    }

    #[test]
    fn captions_are_one_revision_on_the_captions_track() {
        let d = doc();
        let clips = source_clips(&d, Path::new("/bundle"), &["v".into()]).unwrap();
        let words = timeline_words(&clips[0], &transcript());
        let (ops, n) = caption_ops(&d, &words, Some("word-pop"), true);
        assert_eq!(n, 2, "a sentence break makes two captions");
        let after = apply_batch(&d, &ops).unwrap().doc;
        let caps = after.items_on_track("t_captions");
        assert_eq!(caps.len(), 2, "the overlapping old caption was replaced");
        assert_eq!(caps[0].start, 10_250_000);
        assert_eq!(caps[0].fields["captionText"], "Hello there.");
        assert_eq!(caps[0].fields["captionPresetId"], "word-pop");
        let timings = &caps[1].fields["captionSegments"][0]["wordTimings"];
        assert_eq!(timings[0]["word"], "Second");
        assert_eq!(timings[0]["start"], 0.0, "word times are relative to the item");
        // Without replace, the new captions go on a new track and the old one stays.
        let (ops, _) = caption_ops(&d, &words, None, false);
        let after = apply_batch(&d, &ops).unwrap().doc;
        assert_eq!(after.items_on_track("t_captions").len(), 1);
        assert_eq!(after.project.tracks.iter().filter(|t| t.kind == "caption").count(), 2);
        // Styling clears overrides.
        let styled = apply_batch(&after, &style_ops(&["old".into()], "classic")).unwrap().doc;
        assert_eq!(styled.item("old").unwrap().fields["captionPresetId"], "classic");
    }

    #[test]
    fn nothing_to_caption_is_explained() {
        let d = apply_batch(&Document::new("p", "p", 1), &crate::projects::default_tracks()).unwrap().doc;
        let e = source_clips(&d, Path::new("/b"), &[]).unwrap_err();
        assert!(e.message.contains("no clip with sound"));
    }
}
