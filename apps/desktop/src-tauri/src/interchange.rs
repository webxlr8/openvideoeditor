//! Timeline exchange with other editors (roadmap change 15): FCPXML, Final Cut Pro 7 / Premiere XML, CMX3600 EDL and
//! OpenTimelineIO, through `ove-interchange`.

use std::path::{Path, PathBuf};

use ove_core::ops::Op;
use ove_interchange::{Format, Rate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, edit, lock};

/// The formats, for the UI: `id` is what the commands take.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TimelineFormat {
    pub id: String,
    pub label: String,
    pub extension: String,
}

const FORMATS: [Format; 4] = [Format::Fcpxml, Format::Xml, Format::Edl, Format::Otio];

#[tauri::command]
#[specta::specta]
pub fn timeline_formats() -> Vec<TimelineFormat> {
    FORMATS
        .iter()
        .map(|f| TimelineFormat { id: f.extension().into(), label: f.label().into(), extension: f.extension().into() })
        .collect()
}

fn format_of(id: &str) -> AppResult<Format> {
    Format::parse(id).map_err(|e| AppError::new(ErrorCode::InvalidArgument, e.to_string()))
}

/// Items of one kind a timeline file leaves out, and why.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OmittedItems {
    /// What was left out, in plain words ("captions", "titles").
    pub kind: String,
    pub count: u32,
    pub reason: String,
}

/// What a timeline file won't carry over: items left out, and properties of the written clips it doesn't keep.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TimelineExportReport {
    pub omitted: Vec<OmittedItems>,
    pub not_carried: Vec<String>,
}

impl From<ove_interchange::ExportReport> for TimelineExportReport {
    fn from(r: ove_interchange::ExportReport) -> Self {
        TimelineExportReport {
            omitted: r
                .omitted
                .into_iter()
                .map(|o| OmittedItems { kind: o.kind, count: o.count as u32, reason: o.reason })
                .collect(),
            not_carried: r.not_carried,
        }
    }
}

/// The written file (`None` when the dialog was cancelled or for a preview) and what it doesn't carry over.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TimelineExport {
    pub path: Option<String>,
    pub report: TimelineExportReport,
}

/// Writes the open project's timeline. `path: None` asks with a save dialog. `preview: true` writes nothing and only
/// returns the report, so the person sees what won't carry over before choosing to continue.
#[tauri::command]
#[specta::specta]
pub async fn export_timeline(
    app: AppHandle,
    format: String,
    fps: f64,
    path: Option<String>,
    preview: bool,
) -> AppResult<TimelineExport> {
    let f = format_of(&format)?;
    let (out, name) = {
        let state = app.state::<AppState>();
        let g = lock(&state.projects.open)?;
        let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open a project first."))?;
        let rate = Rate::from_fps(if fps > 0.0 { fps } else { 30.0 });
        if preview {
            let report = ove_interchange::export_report(p.doc(), Some(p.bundle()), f, rate);
            return Ok(TimelineExport { path: None, report: report.into() });
        }
        (ove_interchange::export_with_report(p.doc(), Some(p.bundle()), f, rate), p.doc().project.name.clone())
    };
    let report: TimelineExportReport = out.report.into();
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => {
            let picked = app
                .dialog()
                .file()
                .add_filter(f.label(), &[f.extension()])
                .set_file_name(format!("{name}.{}", f.extension()))
                .blocking_save_file();
            match picked.and_then(|p| p.into_path().ok()) {
                Some(p) => p,
                None => return Ok(TimelineExport { path: None, report }),
            }
        }
    };
    std::fs::write(&path, out.text)
        .map_err(|e| AppError::new(ErrorCode::Io, format!("Could not write {}: {e}", path.display())))?;
    Ok(TimelineExport { path: Some(path.display().to_string()), report })
}

/// What an import did.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TimelineImport {
    pub project: ProjectView,
    pub tracks: u32,
    pub clips: u32,
    pub media_added: u32,
    /// Clips whose media was not found (added as placeholders).
    pub missing: Vec<String>,
}

/// Probes the media an import adds when the file exists (duration, size, audio; a thumbnail), so it plays at once.
fn probe_new_media(ops: &mut [Op], bundle: &Path) {
    for op in ops {
        let Op::AddMedia { media, entry: Some(entry) } = op else { continue };
        let Some(path) = entry.get("nativePath").and_then(Value::as_str).map(PathBuf::from) else { continue };
        let Ok(info) = ove_media::probe(&path) else { continue };
        let id = media["id"].as_str().unwrap_or_default().to_string();
        let now = media["createdAt"].as_i64().unwrap_or_default();
        let kind = media.pointer("/data/type").and_then(Value::as_str).unwrap_or("video").to_string();
        let (mut m, e, duration_us) = crate::projects::media_records(&path, &info, &id, now);
        if kind == "image" {
            m["data"]["type"] = "image".into();
        }
        if info.video.is_some() {
            let _ = crate::projects::make_thumbnail(
                &path,
                &crate::projects::thumbs_dir(bundle).join(format!("{id}.png")),
                duration_us,
            );
        }
        *media = m;
        *entry = e;
    }
}

/// Adds a timeline file's tracks and clips to the open project as one revision. `path: None` asks with an open
/// dialog (`None` when cancelled).
#[tauri::command]
#[specta::specta]
pub async fn import_timeline(app: AppHandle, path: Option<String>, fps: f64) -> AppResult<Option<TimelineImport>> {
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => {
            let exts: Vec<&str> = FORMATS.iter().map(|f| f.extension()).collect();
            let picked = app.dialog().file().add_filter("Timeline", &exts).blocking_pick_file();
            match picked.and_then(|p| p.into_path().ok()) {
                Some(p) => p,
                None => return Ok(None),
            }
        }
    };
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || import_file(&a, &a.state::<AppState>(), &path, fps).map(Some))
        .await
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?
}

fn import_file(app: &AppHandle, state: &State<'_, AppState>, path: &Path, fps: f64) -> AppResult<TimelineImport> {
    let f = Format::from_path(path)
        .map_err(|_| AppError::new(ErrorCode::Unsupported, "Choose an .fcpxml, .xml, .edl or .otio timeline file."))?;
    let text = std::fs::read_to_string(path)
        .map_err(|e| AppError::new(ErrorCode::Io, format!("Could not read {}: {e}", path.display())))?;
    let tl = ove_interchange::read(&text, f, Rate::from_fps(if fps > 0.0 { fps } else { 30.0 }))
        .map_err(|e| AppError::new(ErrorCode::Corrupt, format!("This timeline can't be imported: {e}.")))?;
    let (doc, bundle) = {
        let g = lock(&state.projects.open)?;
        let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "Open a project first."))?;
        (p.doc().clone(), p.bundle().to_path_buf())
    };
    let (mut ops, summary) = ove_interchange::to_ops(&doc, &tl, ove_core::Clock::now_ms(&ove_core::SystemClock));
    if summary.clips == 0 {
        return Err(AppError::new(ErrorCode::NoMedia, "That timeline has no clips."));
    }
    probe_new_media(&mut ops, &bundle);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let project = edit(app, state, ops, &format!("import timeline {name}"))?;
    Ok(TimelineImport {
        project,
        tracks: summary.tracks as u32,
        clips: summary.clips as u32,
        media_added: summary.media_added as u32,
        missing: summary.missing,
    })
}
