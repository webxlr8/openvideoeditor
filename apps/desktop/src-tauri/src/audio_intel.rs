//! Audio intelligence in the editor (add-audio-intelligence): the timeline's "Synchronise selection" command lines up
//! the selected recordings against one of them, with the same planner the `sync_items` tool uses, as one revision,
//! and returns a summary per item.

use std::collections::BTreeMap;

use ove_analyze::sync::SyncParams;
use ove_analyze::sync_place::{self, BundleAudio, Mode};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, edit, lock, serial};

/// What to synchronise.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncRequest {
    /// The item the others line up against (usually the camera).
    pub reference: String,
    pub targets: Vec<String>,
    /// auto | timecode | audio
    pub mode: String,
}

/// One item's outcome.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncTargetView {
    pub item: String,
    /// placed | unmatched | refused
    pub status: String,
    /// timecode | audio
    pub method: Option<String>,
    pub confidence: f64,
    /// Timeline move, seconds.
    pub moved_by: Option<f64>,
    pub drift_ppm: Option<f64>,
    pub drift_applied: Option<bool>,
    pub reason: Option<String>,
}

/// The new project state and what happened to each item.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    pub view: ProjectView,
    /// One line for people: how many items moved and why the others did not.
    pub summary: String,
    pub targets: Vec<SyncTargetView>,
}

fn summary_line(plan: &sync_place::SyncPlan) -> String {
    let mut parts = vec![format!(
        "Synchronised {} of {} item{}",
        plan.placed,
        plan.targets.len(),
        if plan.targets.len() == 1 { "" } else { "s" }
    )];
    for t in &plan.targets {
        if t.status == "placed" {
            if let (Some(ppm), Some(true)) = (t.drift_ppm, t.drift_applied) {
                parts.push(format!("{}: drift {ppm:+.0} ppm corrected", t.item));
            }
        } else {
            parts.push(format!("{}: {}", t.item, t.reason.as_deref().unwrap_or(t.status)));
        }
    }
    parts.join(" · ")
}

/// Lines up the targets against the reference as one revision.
#[tauri::command]
#[specta::specta]
pub async fn sync_items(app: AppHandle, request: SyncRequest) -> AppResult<SyncOutcome> {
    serial(&app, "sync_items", move |app, state| {
        let mode = Mode::parse(&request.mode)
            .ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "The sync mode is auto, timecode or audio."))?;
        let plan = {
            let g = lock(&state.projects.open)?;
            let project = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::NotFound, "No project is open."))?;
            let doc = project.doc();
            let files: BTreeMap<_, _> = doc
                .registry
                .iter()
                .map(|(id, e)| {
                    let path = ove_store::media::resolve_media_path(project.bundle(), &e.native_path);
                    (id.clone(), (path, e.fields.get("contentId").and_then(|v| v.as_str()).map(str::to_string)))
                })
                .collect();
            let mut media = BundleAudio::new(project.bundle(), files);
            sync_place::plan(doc, &request.reference, &request.targets, mode, &SyncParams::default(), &mut media)
                .map_err(|e| AppError::new(ErrorCode::InvalidArgument, format!("Cannot synchronise: {e}.")))?
        };
        let summary = summary_line(&plan);
        if plan.placed == 0 {
            return Err(AppError::new(ErrorCode::InvalidArgument, format!("Nothing was synchronised. {summary}")));
        }
        let targets = plan
            .targets
            .iter()
            .map(|t| SyncTargetView {
                item: t.item.clone(),
                status: t.status.to_string(),
                method: t.method.map(str::to_string),
                confidence: t.confidence,
                moved_by: t.moved_by,
                drift_ppm: t.drift_ppm,
                drift_applied: t.drift_applied,
                reason: t.reason.clone(),
            })
            .collect();
        let label = format!("synchronise {} item{}", plan.placed, if plan.placed == 1 { "" } else { "s" });
        let view = if plan.ops.is_empty() {
            let g = lock(&state.projects.open)?;
            let project = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::NotFound, "No project is open."))?;
            crate::projects::view(project)
        } else {
            edit(app, state, plan.ops, &label)?
        };
        Ok(SyncOutcome { view, summary, targets })
    })
    .await
}

/// One music item's beats on the timeline.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ItemBeats {
    pub item: String,
    /// Timeline seconds.
    pub beats: Vec<f64>,
    pub downbeats: Vec<f64>,
}

/// Beats of the audio items, for "Snap to beats".
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TimelineBeats {
    pub items: Vec<ItemBeats>,
    /// True when some item has no cached beats and the beat model is not installed.
    pub model_missing: bool,
}

/// The beats of every audio item in timeline seconds: cached results, else detected with the local beat model (never
/// downloaded). Nothing is written into the project.
#[tauri::command]
#[specta::specta]
pub async fn timeline_beats(app: AppHandle) -> AppResult<TimelineBeats> {
    use ove_tools::tools::audio_intel::{MediaBeats, media_beats};
    serial(&app, "timeline_beats", move |_app, state| {
        let (doc, bundle) = {
            let g = lock(&state.projects.open)?;
            let project = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::NotFound, "No project is open."))?;
            (project.doc().clone(), project.bundle().to_path_buf())
        };
        let mut items = Vec::new();
        let mut model_missing = false;
        let mut by_media: BTreeMap<String, Option<ove_analyze::beats::BeatAnalysis>> = BTreeMap::new();
        for item in doc.project.items.values().filter(|i| i.kind == "audio") {
            let Some(media) = item.media_id() else { continue };
            if !by_media.contains_key(media) {
                let a = match media_beats(&doc, &bundle, media) {
                    Ok(MediaBeats::Ready { analysis, .. }) => Some(analysis),
                    Ok(MediaBeats::NotInstalled { .. }) => {
                        model_missing = true;
                        None
                    }
                    Err(_) => None,
                };
                by_media.insert(media.to_string(), a);
            }
            if let Some(Some(a)) = by_media.get(media) {
                items.push(ItemBeats {
                    item: item.id.clone(),
                    beats: ove_analyze::beats::to_timeline(item, &a.beats),
                    downbeats: ove_analyze::beats::to_timeline(item, &a.downbeats),
                });
            }
        }
        Ok(TimelineBeats { items, model_missing })
    })
    .await
}
