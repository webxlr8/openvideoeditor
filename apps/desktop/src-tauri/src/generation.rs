//! AI generation in the app (roadmap change 14): provider keys, plans with cost estimates, jobs that place a pending
//! placeholder, run on a background thread and fill it when done. Same crate (`ove-generate`) and job files as the
//! `ove generate` command line, so jobs started either way show up here.

use std::path::Path;
use std::sync::Arc;

use ove_core::revision::Source;
use ove_generate::keys::{KeyStore, MemoryKeyStore, OsKeyStore};
use ove_generate::{Job, Kind, Provider, Request, Status};
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{emit, lock};

/// Provider keys: the OS credential store (or memory when `OVE_MEMORY_KEYS` is set, for automated runs).
pub struct Generation {
    keys: Arc<dyn KeyStore>,
}

impl Default for Generation {
    fn default() -> Self {
        let keys: Arc<dyn KeyStore> = if std::env::var_os("OVE_MEMORY_KEYS").is_some() {
            Arc::new(MemoryKeyStore::default())
        } else {
            Arc::new(OsKeyStore)
        };
        Generation { keys }
    }
}

/// A job changed (started, finished, failed). Payload: the job as JSON.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct GenerationUpdated(pub String);

fn bad(m: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidArgument, m)
}

fn kind_of(s: &str) -> AppResult<Kind> {
    Kind::parse(s).ok_or_else(|| bad("Kind is image, video, voice, music or sfx."))
}

fn provider_of(s: &str) -> AppResult<Provider> {
    Provider::parse(s).ok_or_else(|| bad("Unknown provider."))
}

fn request_of(json_text: &str) -> AppResult<Request> {
    serde_json::from_str(json_text).map_err(|e| bad(format!("Bad request: {e}")))
}

/// Models and key status, as JSON `{models, keys}`.
#[tauri::command]
#[specta::specta]
pub fn generation_catalog(g: State<'_, Generation>) -> String {
    json!({"models": ove_generate::catalog::models(), "keys": ove_generate::keys::status(g.keys.as_ref())}).to_string()
}

#[tauri::command]
#[specta::specta]
pub fn set_provider_key(g: State<'_, Generation>, provider: String, key: String) -> AppResult<String> {
    let p = provider_of(&provider)?;
    g.keys.set(p, &key).map_err(bad)?;
    tracing::info!(provider = p.id(), "provider key stored in the system credential store");
    Ok(generation_catalog(g))
}

#[tauri::command]
#[specta::specta]
pub fn clear_provider_key(g: State<'_, Generation>, provider: String) -> AppResult<String> {
    let p = provider_of(&provider)?;
    g.keys.clear(p).map_err(|m| AppError::new(ErrorCode::Io, m))?;
    Ok(generation_catalog(g))
}

/// The plan for a request on the open project: model, aspect, seconds, estimate, track, start (JSON).
#[tauri::command]
#[specta::specta]
pub fn plan_generation(state: State<'_, AppState>, kind: String, request: String) -> AppResult<String> {
    let k = kind_of(&kind)?;
    let req = request_of(&request)?;
    let g = lock(&state.projects.open)?;
    let p = g.as_ref().ok_or_else(|| bad("Open a project first."))?;
    let plan = ove_generate::plan(p.doc(), k, &req, "preview").map_err(bad)?;
    Ok(serde_json::to_string(&plan).unwrap_or_default())
}

/// Starts a job on the open project after the person confirmed the estimate: places the placeholder (one revision)
/// and runs the job in the background. Returns the job as JSON.
#[tauri::command]
#[specta::specta]
pub fn start_generation(
    app: AppHandle,
    state: State<'_, AppState>,
    g: State<'_, Generation>,
    kind: String,
    request: String,
) -> AppResult<String> {
    let k = kind_of(&kind)?;
    let req = request_of(&request)?;
    let now = ove_core::Clock::now_ms(&ove_core::SystemClock);
    let (job, bundle) = {
        let mut guard = lock(&state.projects.open)?;
        let p = guard.as_mut().ok_or_else(|| bad("Open a project first."))?;
        let item_id = format!("i_gen_{now:x}");
        let plan = ove_generate::plan(p.doc(), k, &req, &item_id).map_err(bad)?;
        if g.keys.get(plan.model.provider).is_none() {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                format!("Add your {} API key in Settings → Generation first.", plan.model.provider.label()),
            ));
        }
        let job = ove_generate::new_job(&plan, &req, &ove_generate::job::new_id(now), now);
        p.apply(plan.ops.clone(), Source::Gui, Some(format!("generate {}: placeholder", k.as_str())))?;
        ove_generate::job::save(p.bundle(), &job).map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))?;
        emit(&app, Some(p));
        (job, p.bundle().to_path_buf())
    };
    let text = serde_json::to_string(&job).unwrap_or_default();
    let _ = GenerationUpdated(text.clone()).emit(&app);
    let keys = g.keys.clone();
    let a = app.clone();
    std::thread::Builder::new()
        .name(format!("generate-{}", job.id))
        .spawn(move || run(&a, &bundle, job, keys.as_ref()))
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    Ok(text)
}

fn probe(path: &Path) -> ove_generate::Probed {
    match ove_media::probe(path) {
        Ok(i) => ove_generate::Probed {
            duration_us: i.duration_us,
            width: i.video.as_ref().map(|v| v.width),
            height: i.video.as_ref().map(|v| v.height),
            has_audio: i.audio.is_some(),
        },
        Err(_) => Default::default(),
    }
}

/// Applies ops to the project the job belongs to: the open one, or the bundle itself when another is open.
fn apply_to(app: &AppHandle, bundle: &Path, ops: Vec<ove_core::ops::Op>, summary: String) {
    if ops.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    if let Ok(mut g) = state.projects.open.lock()
        && let Some(p) = g.as_mut()
        && p.bundle() == bundle
    {
        match p.apply(ops, Source::Gui, Some(summary)) {
            Ok(_) => emit(app, Some(p)),
            Err(e) => tracing::warn!(error = %e, "could not place the generated media"),
        }
        return;
    }
    let clock: Arc<dyn ove_core::Clock> = Arc::new(ove_core::SystemClock);
    match ove_store::project::Project::open(bundle, ove_store::project::Mode::Write, clock) {
        Ok(mut p) => {
            if let Err(e) = p.apply(ops, Source::Gui, Some(summary)) {
                tracing::warn!(error = %e, "could not place the generated media");
            }
            let _ = p.close();
        }
        Err(e) => tracing::warn!(error = %e, "project for a finished job is not available"),
    }
}

fn run(app: &AppHandle, bundle: &Path, mut job: Job, keys: &dyn KeyStore) {
    let res = ove_generate::execute(bundle, &mut job, keys, &|| false);
    match res {
        Ok(()) => {
            let rel = job.output.clone().unwrap_or_default();
            let file = bundle.join(&rel);
            let probed = probe(&file);
            let media_id = format!("m_{}", job.id.trim_start_matches("g_"));
            if job.kind != Kind::Voice && job.kind != Kind::Music && job.kind != Kind::Sfx {
                let _ = crate::projects::make_thumbnail(
                    &file,
                    &crate::projects::thumbs_dir(bundle).join(format!("{media_id}.png")),
                    probed.duration_us.unwrap_or(0),
                );
            }
            let doc = {
                let state = app.state::<AppState>();
                let g = state.projects.open.lock().ok();
                g.and_then(|g| g.as_ref().filter(|p| p.bundle() == bundle).map(|p| p.doc().clone()))
            };
            let doc = doc.or_else(|| {
                ove_store::project::Project::open(
                    bundle,
                    ove_store::project::Mode::ReadOnly,
                    Arc::new(ove_core::SystemClock),
                )
                .ok()
                .map(|p| p.doc().clone())
            });
            if let Some(doc) = doc {
                let now = ove_core::Clock::now_ms(&ove_core::SystemClock);
                let ops = ove_generate::finish_ops(&doc, &job, &rel, &probed, now);
                apply_to(
                    app,
                    bundle,
                    ops,
                    format!(
                        "generate {}: {}",
                        job.kind.as_str(),
                        job.request.prompt.chars().take(60).collect::<String>()
                    ),
                );
            }
        }
        Err(e) => {
            tracing::warn!(job = %job.id, code = %e.code, "generation did not finish: {}", e.message);
            let doc = app
                .state::<AppState>()
                .projects
                .open
                .lock()
                .ok()
                .and_then(|g| g.as_ref().filter(|p| p.bundle() == bundle).map(|p| p.doc().clone()));
            if let Some(doc) = doc {
                let ops = ove_generate::place::failed_ops(&doc, &job);
                apply_to(
                    app,
                    bundle,
                    ops,
                    format!(
                        "generate {}: {}",
                        job.kind.as_str(),
                        if job.status == Status::Cancelled { "cancelled" } else { "failed" }
                    ),
                );
            }
        }
    }
    let _ = GenerationUpdated(serde_json::to_string(&job).unwrap_or_default()).emit(app);
}

/// Jobs of the open project, newest first (JSON array).
#[tauri::command]
#[specta::specta]
pub fn generation_jobs(state: State<'_, AppState>) -> AppResult<String> {
    let g = lock(&state.projects.open)?;
    let jobs = g.as_ref().map(|p| ove_generate::job::list(p.bundle())).unwrap_or_default();
    Ok(serde_json::to_string(&jobs).unwrap_or_else(|_| "[]".into()))
}

#[tauri::command]
#[specta::specta]
pub fn cancel_generation(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let g = lock(&state.projects.open)?;
    let p = g.as_ref().ok_or_else(|| bad("Open a project first."))?;
    ove_generate::job::request_cancel(p.bundle(), &id).map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))
}

/// Voices in the ElevenLabs account (JSON array).
#[tauri::command]
#[specta::specta]
pub async fn list_voices(app: AppHandle) -> AppResult<String> {
    let keys = app.state::<Generation>().keys.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (key, _) = keys
            .get(Provider::ElevenLabs)
            .ok_or_else(|| bad("Add your ElevenLabs API key in Settings → Generation."))?;
        let v = ove_generate::providers::elevenlabs::voices(&key, None)
            .map_err(|e| AppError::new(ErrorCode::Unsupported, e.message))?;
        Ok(serde_json::to_string(&v).unwrap_or_else(|_| "[]".into()))
    })
    .await
    .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?
}
