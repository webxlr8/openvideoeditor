//! The Audio panel's Duck popover (design D7): lowers music under dialogue by writing volume keyframes on the music
//! items, as one revision, with the same planner the `duck_audio` tool uses. A preview returns what would change.

use ove_analyze::duck::{self, DuckParams, Mode, SpeechSource};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, edit, lock, serial};

/// What to duck, under what, and how.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DuckRequest {
    /// Music item ids or track ids/names.
    pub music: Vec<String>,
    /// Dialogue item ids or track ids/names.
    pub under: Vec<String>,
    /// dB during speech (-60 to 0).
    pub depth: f64,
    /// Seconds.
    pub attack: f64,
    pub release: f64,
    pub hold: f64,
    /// transcript | level | auto
    pub source: String,
    /// replace | merge
    pub mode: String,
}

/// What a duck would change.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DuckPreview {
    /// transcript | level
    pub source: String,
    /// Lines of speech found.
    pub lines: u32,
    /// Speech, seconds, as [start, end] pairs.
    pub speech: Vec<Vec<f64>>,
    pub music_items: u32,
    pub keyframes: u32,
}

fn params(r: &DuckRequest) -> AppResult<(DuckParams, SpeechSource, Mode)> {
    let bad = |m: String| AppError::new(ErrorCode::InvalidArgument, m);
    if !(-60.0..=0.0).contains(&r.depth) {
        return Err(bad(format!("The depth must be between -60 and 0 dB, not {}.", r.depth)));
    }
    let us = |s: f64, what: &str| {
        if (0.0..=10.0).contains(&s) {
            Ok((s * 1e6).round() as i64)
        } else {
            Err(bad(format!("The {what} must be 0–10 s.")))
        }
    };
    let p = DuckParams {
        depth_db: r.depth,
        attack_us: us(r.attack, "attack")?,
        release_us: us(r.release, "release")?,
        hold_us: us(r.hold, "hold")?,
    };
    let source = match r.source.as_str() {
        "transcript" => SpeechSource::Transcript,
        "level" => SpeechSource::Level,
        _ => SpeechSource::Auto,
    };
    let mode = if r.mode == "merge" { Mode::Merge } else { Mode::Replace };
    Ok((p, source, mode))
}

fn plan(state: &crate::AppState, r: &DuckRequest) -> AppResult<duck::DuckPlan> {
    let (p, source, mode) = params(r)?;
    let g = lock(&state.projects.open)?;
    let project = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::NotFound, "No project is open."))?;
    let message = |e: duck::DuckError| {
        let text = match &e {
            duck::DuckError::NoSpeech => "No speech was found in the dialogue, so nothing was changed.".to_string(),
            duck::DuckError::NoTranscript => {
                "The dialogue has no transcript yet. Choose “From the level” or caption it first.".to_string()
            }
            other => format!("Cannot duck: {other}."),
        };
        AppError::new(ErrorCode::InvalidArgument, text)
    };
    duck::plan(project.doc(), Some(project.bundle()), &r.music, &r.under, &p, source, mode).map_err(message)
}

/// What a duck would change, without changing anything.
#[tauri::command]
#[specta::specta]
pub async fn duck_preview(app: AppHandle, request: DuckRequest) -> AppResult<DuckPreview> {
    serial(&app, "duck_preview", move |_app, state| {
        let p = plan(state, &request)?;
        Ok(DuckPreview {
            source: p.source.to_string(),
            lines: p.speech.len() as u32,
            speech: p.speech.iter().map(|(a, b)| vec![*a, *b]).collect(),
            music_items: p.items.len() as u32,
            keyframes: p.items.iter().map(|i| i.keyframes.len() as u32).sum(),
        })
    })
    .await
}

/// Ducks the music under the dialogue as one revision.
#[tauri::command]
#[specta::specta]
pub async fn duck_audio(app: AppHandle, request: DuckRequest) -> AppResult<ProjectView> {
    serial(&app, "duck_audio", move |app, state| {
        let p = plan(state, &request)?;
        edit(app, state, p.ops, &format!("duck music {} dB under dialogue", request.depth))
    })
    .await
}
