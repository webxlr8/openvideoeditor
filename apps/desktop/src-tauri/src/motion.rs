//! Motion graphics and fonts for the UI: the font library (`list_fonts`, `font_preview`), built-in motion
//! templates (`list_templates`, `template_thumbnail`) and code-item checking (`check_code`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine as _;
use ove_core::model::Document;
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::lock;

/// A font family the text tools can use.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FontFamily {
    pub family: String,
    /// `bundled` (ships with the app) or `system` (installed on this computer).
    pub source: String,
    /// `sans-serif`, `serif`, `display`, `mono`, `handwriting` or `system`.
    pub category: String,
    pub min_weight: u16,
    pub max_weight: u16,
    pub italic: bool,
}

/// Bundled families first, then installed system families.
#[tauri::command]
#[specta::specta]
pub async fn list_fonts() -> AppResult<Vec<FontFamily>> {
    Ok(ove_graphics::fonts::all_families()
        .into_iter()
        .map(|f| FontFamily {
            family: f.family,
            source: f.source,
            category: f.category,
            min_weight: f.weights.min,
            max_weight: f.weights.max,
            italic: f.italic,
        })
        .collect())
}

fn png_data_url(w: u32, h: u32, rgba: &[u8]) -> Option<String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().ok()?.write_image_data(rgba).ok()?;
    }
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(out)))
}

fn cached(key: String, make: impl FnOnce() -> Option<String>) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().ok()?.get(&key) {
        return Some(v.clone());
    }
    let v = make()?;
    let mut c = cache.lock().ok()?;
    if c.len() > 400 {
        c.clear();
    }
    c.insert(key, v.clone());
    Some(v)
}

/// A white-on-transparent PNG (data URL) of `text` set in `family`, for the font dropdown.
#[tauri::command]
#[specta::specta]
pub async fn font_preview(family: String, text: String, size: u32) -> AppResult<String> {
    let size = size.clamp(8, 96);
    let text: String = text.chars().take(40).collect();
    cached(format!("{family}\u{1}{text}\u{1}{size}"), || {
        let st = ove_render::text::TextStyle {
            size: size as f32,
            align: ove_render::text::Align::Left,
            font: Some(ove_render::fonts::FontSpec { family: family.clone(), weight: 400, italic: false }),
            ..Default::default()
        };
        let img = ove_render::text::render_words(&ove_render::text::words_of(&text), &st);
        png_data_url(img.w, img.h, &img.data)
    })
    .ok_or_else(|| AppError::new(ErrorCode::Internal, "The preview could not be drawn."))
}

/// Built-in motion templates with their prop schemas (JSON array).
#[tauri::command]
#[specta::specta]
pub fn list_templates() -> AppResult<String> {
    serde_json::to_string(&ove_graphics::templates::all())
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))
}

/// A small PNG (data URL) of a template two thirds of the way through, with its default props.
#[tauri::command]
#[specta::specta]
pub async fn template_thumbnail(id: String) -> AppResult<String> {
    let t = ove_graphics::templates::get(&id).ok_or_else(|| AppError::new(ErrorCode::NotFound, "Unknown template."))?;
    cached(format!("template\u{1}{id}"), || {
        let c = ove_graphics::CodeComposition { template: Some(id.clone()), ..Default::default() };
        let p = ove_graphics::prepare(&c, (1920, 1080)).ok()?;
        let none = |_: &str| None;
        let time = ove_graphics::FrameTime { time: t.duration * 0.6, duration: t.duration, fps: 30.0 };
        let r = ove_graphics::render(&p, time, 0.15, &none, ove_graphics::Limits::default()).ok()?;
        png_data_url(r.frame.width, r.frame.height, &r.frame.rgba)
    })
    .ok_or_else(|| AppError::new(ErrorCode::Internal, "The thumbnail could not be drawn."))
}

fn open_doc(state: &AppState) -> Option<(Document, PathBuf)> {
    let g = lock(&state.projects.open).ok()?;
    let p = g.as_ref()?;
    Some((p.doc().clone(), p.bundle().to_path_buf()))
}

/// Checks a code composition (`composition_json`: the `codeComposition` object) for an item lasting `duration`
/// seconds. Returns the check report as JSON: `{ok, diagnostics[{severity, code, message, line, column}], logs,
/// frameMs, width, height}`.
#[tauri::command]
#[specta::specta]
pub async fn check_code(state: State<'_, AppState>, composition_json: String, duration: f64) -> AppResult<String> {
    let value: serde_json::Value = serde_json::from_str(&composition_json)
        .map_err(|e| AppError::new(ErrorCode::InvalidArgument, format!("The composition is not valid JSON: {e}")))?;
    let c = ove_graphics::CodeComposition::from_value(&value)
        .ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "The composition must be an object."))?;
    let opened = open_doc(&state);
    let size =
        opened.as_ref().map(|(d, _)| ove_render::eval::reference_size(&d.project.aspect_ratio)).unwrap_or((1920, 1080));
    let load = |id: &str| {
        let (doc, bundle) = opened.as_ref()?;
        let path = ove_render::sources::media_path(doc, Some(bundle), id)?;
        std::fs::read(path).ok().map(Arc::new)
    };
    let report = ove_graphics::check(&c, size, duration.max(0.1), &load);
    serde_json::to_string(&report).map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))
}
