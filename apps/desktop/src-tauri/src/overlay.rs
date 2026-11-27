//! The viewer overlay (add-viewer-interaction-and-text-fills, design D1/D2): layer geometry for the handles, click to
//! select by the layer's own pixels, and transient previews while a handle is dragged. A drag previews through the
//! playback engine without creating a revision; the UI commits one `update_item` when the pointer is released.

use std::sync::Arc;

use ove_core::ops::{Op, apply_batch};
use ove_core::time::secs_to_micros;
use ove_render::Renderer;
use ove_render::geometry::{FrameGeometry, hit_test, layer_geometry_with, recorded_size};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::engine::EngineMsg;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::lock;
use crate::worker;

/// The canvas height geometry is measured on. Corners go to the UI as fractions of the frame, so this only sets the
/// precision (and matches the GPU preview height).
const GEOMETRY_HEIGHT: u32 = 1080;

fn bad(msg: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidArgument, msg)
}

/// One visible layer in the viewer. Corners are fractions of the frame (0..1), top-left, top-right, bottom-right,
/// bottom-left of the source.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ViewerLayer {
    pub item_id: String,
    pub item_type: String,
    pub index: u32,
    pub quad: Vec<[f64; 2]>,
    pub crop_quad: Vec<[f64; 2]>,
    pub rotation: f64,
    pub selectable: bool,
}

/// The layers visible at a time, bottom first, and the frame size their fractions refer to.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ViewerGeometry {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<ViewerLayer>,
}

fn to_viewer(g: &FrameGeometry) -> ViewerGeometry {
    let (w, h) = (g.width.max(1) as f64, g.height.max(1) as f64);
    let frac = |q: &[[f64; 2]; 4]| q.iter().map(|p| [p[0] / w, p[1] / h]).collect::<Vec<_>>();
    ViewerGeometry {
        width: g.width,
        height: g.height,
        layers: g
            .layers
            .iter()
            .map(|l| ViewerLayer {
                item_id: l.item_id.clone(),
                item_type: l.item_type.clone(),
                index: l.index as u32,
                quad: frac(&l.quad),
                crop_quad: frac(&l.crop_quad),
                rotation: l.rotation,
                selectable: l.selectable,
            })
            .collect(),
    }
}

fn open_doc(state: &AppState) -> AppResult<(ove_core::Document, std::path::PathBuf)> {
    let g = lock(&state.projects.open)?;
    let p = g.as_ref().ok_or_else(|| bad("No project is open."))?;
    Ok((p.doc().clone(), p.bundle().to_path_buf()))
}

fn micros(time: f64) -> i64 {
    secs_to_micros(time.max(0.0)).unwrap_or(0)
}

/// Where each visible layer is at `time` seconds on the open project's timeline.
#[tauri::command]
#[specta::specta]
pub async fn viewer_geometry(app: AppHandle, time: f64) -> AppResult<ViewerGeometry> {
    worker::blocking("viewer_geometry", move || {
        let state = app.state::<AppState>();
        let (doc, bundle) = open_doc(&state)?;
        let mut renderer = Renderer::new(Some(&bundle), GEOMETRY_HEIGHT);
        let g = layer_geometry_with(&doc, micros(time), GEOMETRY_HEIGHT, &mut |l| {
            recorded_size(&doc, l).or_else(|| renderer.source_size(&doc, l))
        });
        Ok(to_viewer(&g))
    })
    .await
}

/// The item whose layer is topmost under the point (`x`, `y`: fractions of the frame) at `time` seconds, judged by
/// the layer's own pixels: a click on a transparent part of a title selects what is below it. `None`: nothing is
/// drawn there.
#[tauri::command]
#[specta::specta]
pub async fn viewer_pick(app: AppHandle, time: f64, x: f64, y: f64) -> AppResult<Option<String>> {
    worker::blocking("viewer_pick", move || {
        let state = app.state::<AppState>();
        let (doc, bundle) = open_doc(&state)?;
        let mut renderer = Renderer::new(Some(&bundle), crate::timeline::PREVIEW_HEIGHT);
        Ok(hit_test(&mut renderer, &doc, micros(time), x, y))
    })
    .await
}

/// A transient change shown while a handle is dragged: fields to set on one item.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ViewerPreview {
    pub item_id: String,
    /// Document field names → values (JSON object text), as `update_item` takes them.
    pub set_json: String,
}

/// Shows `preview` in the viewer without changing the project (no revision), or, with `None`, the project as it is
/// (after a cancelled drag). The viewer re-renders the current frame with it while paused.
#[tauri::command]
#[specta::specta]
pub async fn viewer_preview(app: AppHandle, preview: Option<ViewerPreview>) -> AppResult<()> {
    worker::blocking("viewer_preview", move || {
        let state = app.state::<AppState>();
        let (doc, bundle) = open_doc(&state)?;
        let doc = match preview {
            None => doc,
            Some(p) => {
                let set: Map<String, Value> =
                    serde_json::from_str(&p.set_json).map_err(|e| bad(format!("The preview is not valid: {e}")))?;
                let ops = [Op::UpdateItem { id: p.item_id, set }];
                apply_batch(&doc, &ops).map_err(|e| bad(format!("The preview is not valid: {e}")))?.doc
            }
        };
        let _ = state.engine.send(EngineMsg::PreviewDoc(Arc::new(doc), bundle));
        Ok(())
    })
    .await
}

/// Puts the app window in (or out of) full screen for the full-screen viewer. The viewer itself fills the window in
/// the UI; the native surface follows the viewer rectangle as always.
#[tauri::command]
#[specta::specta]
pub fn viewer_fullscreen(app: AppHandle, on: bool) -> AppResult<()> {
    let window = app.get_webview_window("main").ok_or_else(|| bad("The main window is gone."))?;
    window.set_fullscreen(on).map_err(|e| AppError::new(ErrorCode::Internal, format!("Full screen failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_render::geometry::LayerGeometry;

    #[test]
    fn corners_become_fractions_of_the_frame() {
        let g = FrameGeometry {
            width: 1920,
            height: 1080,
            layers: vec![LayerGeometry {
                item_id: "a".into(),
                item_type: "video".into(),
                index: 0,
                quad: [[0.0, 0.0], [960.0, 0.0], [960.0, 540.0], [0.0, 540.0]],
                crop_quad: [[0.0, 0.0], [480.0, 0.0], [480.0, 540.0], [0.0, 540.0]],
                rotation: 0.0,
                opacity: 1.0,
                selectable: true,
            }],
        };
        let v = to_viewer(&g);
        assert_eq!(v.layers[0].quad[2], [0.5, 0.5]);
        assert_eq!(v.layers[0].crop_quad[1], [0.25, 0.0]);
    }
}
