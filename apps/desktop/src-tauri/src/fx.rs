//! Effects, transitions, layouts and colour (change 12): catalogues for the panels, effect and look thumbnails,
//! `.cube` import, reference colour match and scopes. Edits themselves go through the generic item commands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use base64::Engine as _;
use ove_core::ops::{Op, apply_batch};
use ove_core::time::secs_to_micros;
use ove_render::color::{self, LUT_PRESETS, Lut, apply_lut, preset_lut};
use ove_render::{Renderer, Rgba, effects, layouts, transitions};
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::projects::{ProjectView, edit, lock, serial};
use crate::worker;

fn bad(msg: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidArgument, msg)
}

/// Every catalogue the panels and inspector need, as JSON: effects (with parameter contracts), transitions,
/// easings, directions, layouts (with slot rectangles) and LUT looks.
#[tauri::command]
#[specta::specta]
pub fn fx_catalog() -> AppResult<String> {
    Ok(catalog().to_string())
}

fn catalog() -> Value {
    let luts: Vec<Value> = LUT_PRESETS.iter().map(|(id, label)| json!({"id": id, "label": label})).collect();
    tidy(json!({
        "effects": effects::EFFECTS,
        "transitions": transitions::TRANSITIONS,
        "easings": transitions::EASINGS,
        "directions": transitions::DIRECTIONS,
        "layouts": layouts::LAYOUTS,
        "luts": luts,
    }))
}

/// Catalogue values are f32: print them as the short decimals they were written as.
fn tidy(v: Value) -> Value {
    match v {
        Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap_or(0.0);
            json!((x * 1e5).round() / 1e5)
        }
        Value::Array(a) => Value::Array(a.into_iter().map(tidy).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, tidy(v))).collect()),
        v => v,
    }
}

/// The catalogue as the browser mock reads it (`ui/src/fx-catalog.json`, written by `export-bindings`).
pub fn catalog_file() -> (PathBuf, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/src/fx-catalog.json");
    (
        path,
        serde_json::to_string_pretty(&catalog()).unwrap_or_default()
            + "
",
    )
}

fn png_data_url(img: &Rgba) -> Option<String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.w, img.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().ok()?.write_image_data(&img.data).ok()?;
    }
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(out)))
}

fn thumb_cache() -> &'static Mutex<HashMap<String, String>> {
    static C: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

const THUMB: (u32, u32) = (160, 90);

/// A thumbnail (PNG data URL) of an effect at its defaults, or of a LUT look, over a built-in sample frame.
#[tauri::command]
#[specta::specta]
pub async fn fx_preview(kind: String, id: String) -> AppResult<String> {
    worker::blocking("fx_preview", move || {
        let key = format!("{kind}:{id}");
        if let Some(u) = lock(thumb_cache())?.get(&key) {
            return Ok(u.clone());
        }
        let img = match kind.as_str() {
            "effect" => effects::preview(&id, THUMB.0, THUMB.1).ok_or_else(|| bad(format!("Unknown effect {id}.")))?,
            "lut" => {
                let lut = preset_lut(&id).ok_or_else(|| bad(format!("Unknown look {id}.")))?;
                let mut img = effects::sample_image(THUMB.0, THUMB.1);
                apply_lut(&mut img, &lut, 1.0);
                img
            }
            _ => return Err(bad("Previews exist for effects and looks.")),
        };
        let url =
            png_data_url(&img).ok_or_else(|| AppError::new(ErrorCode::Internal, "Could not encode the preview."))?;
        lock(thumb_cache())?.insert(key, url.clone());
        Ok(url)
    })
    .await
}

fn bundle(state: &AppState) -> AppResult<PathBuf> {
    let g = lock(&state.projects.open)?;
    Ok(g.as_ref().ok_or_else(|| bad("No project is open."))?.bundle().to_path_buf())
}

/// Copies a `.cube` file into the project bundle (`luts/`), after checking it parses. Returns the bundle-relative
/// path to store in `colorGrade.lut.file`. With no path, asks with a file dialog (None when cancelled).
#[tauri::command]
#[specta::specta]
pub async fn import_lut(app: AppHandle, path: Option<String>) -> AppResult<Option<String>> {
    worker::blocking("import_lut", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        let src = match path {
            Some(p) => PathBuf::from(p),
            None => {
                let Some(f) = app.dialog().file().add_filter("Cube LUT", &["cube"]).blocking_pick_file() else {
                    return Ok(None);
                };
                f.into_path().map_err(|_| bad("That file cannot be opened."))?
            }
        };
        Ok(Some(copy_lut(&bundle(state)?, &src)?))
    })
    .await
}

fn copy_lut(bundle: &Path, src: &Path) -> AppResult<String> {
    let text = std::fs::read_to_string(src).map_err(|e| bad(format!("Cannot read {}: {e}", src.display())))?;
    Lut::parse_cube(&text).map_err(|e| bad(format!("Not a usable .cube LUT: {e}")))?;
    let stem: String = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("look")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let dir = bundle.join("luts");
    std::fs::create_dir_all(&dir).map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    let mut name = format!("{stem}.cube");
    let mut n = 1;
    while dir.join(&name).exists() && std::fs::read_to_string(dir.join(&name)).ok().as_deref() != Some(&text) {
        n += 1;
        name = format!("{stem}-{n}.cube");
    }
    std::fs::write(dir.join(&name), &text).map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    Ok(format!("luts/{name}"))
}

const MATCH_HEIGHT: u32 = 270;

/// Fits an item's colour to the frame at `reference_time` (seconds): Lab mean/std transfer, stored as concrete
/// gain and offset wheels in the item's `colorGrade` (any pending `match` is replaced by those values).
#[tauri::command]
#[specta::specta]
pub async fn match_color(
    app: AppHandle,
    item_id: String,
    reference_time: f64,
    strength: Option<f64>,
) -> AppResult<ProjectView> {
    serial(&app, "match_color", move |app, state| {
        let (doc, bundle) = {
            let g = lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| bad("No project is open."))?;
            (p.doc().clone(), p.bundle().to_path_buf())
        };
        let item =
            doc.item(&item_id).cloned().ok_or_else(|| AppError::new(ErrorCode::NotFound, "That item is gone."))?;
        if !matches!(item.kind.as_str(), "video" | "image") {
            return Err(bad("Colour match works on video and image items."));
        }
        let mut r = Renderer::new(Some(&bundle), MATCH_HEIGHT);
        let reference = r.render(&doc, secs_to_micros(reference_time.max(0.0)).unwrap_or(0));
        // The item alone, ungraded, at its middle.
        let mut ops: Vec<Op> = doc
            .project
            .items
            .keys()
            .filter(|id| **id != item_id)
            .map(|id| Op::UpdateItem {
                id: id.clone(),
                set: json!({"hidden": true}).as_object().cloned().unwrap_or_default(),
            })
            .collect();
        ops.push(Op::UnsetItemFields { id: item_id.clone(), fields: vec!["colorGrade".into(), "effects".into()] });
        let alone = apply_batch(&doc, &ops).map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?.doc;
        let target = r.render(&alone, (item.start + item.end) / 2);
        let wheels = color::fit_match(&reference, &target, strength.unwrap_or(1.0) as f32);
        let mut grade: Map<String, Value> =
            item.fields.get("colorGrade").and_then(Value::as_object).cloned().unwrap_or_default();
        grade.remove("match");
        grade.insert("wheels".into(), wheels);
        let set = json!({ "colorGrade": grade }).as_object().cloned().unwrap_or_default();
        edit(app, state, vec![Op::UpdateItem { id: item_id, set }], "match colour")
    })
    .await
}

/// Histograms (red, green, blue, luma; 64 bins, 0..1) of the timeline frame at `time` seconds.
#[tauri::command]
#[specta::specta]
pub async fn color_scopes(app: AppHandle, time: f64) -> AppResult<Vec<Vec<f32>>> {
    worker::blocking("color_scopes", move || {
        let state = app.state::<AppState>();
        let state = &*state;
        let (doc, bundle) = {
            let g = lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| bad("No project is open."))?;
            (p.doc().clone(), p.bundle().to_path_buf())
        };
        let frame = Renderer::new(Some(&bundle), 180).render(&doc, secs_to_micros(time.max(0.0)).unwrap_or(0));
        Ok(color::histogram(&frame).into_iter().collect())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_everything() {
        let v: Value = serde_json::from_str(&fx_catalog().unwrap()).unwrap();
        assert_eq!(v["effects"].as_array().unwrap().len(), 41);
        assert_eq!(v["layouts"].as_array().unwrap().len(), 15);
        assert_eq!(v["luts"].as_array().unwrap().len(), 9);
        assert!(v["transitions"].as_array().unwrap().len() >= 23);
        assert_eq!(v["effects"][0]["params"][0]["kind"]["kind"], "color");
    }

    #[test]
    fn lut_import_validates_and_dedupes() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("My Look.cube");
        std::fs::write(&src, Lut::from_fn(2, |c| c).to_cube("t", "")).unwrap();
        let bundle = d.path().join("b");
        assert_eq!(copy_lut(&bundle, &src).unwrap(), "luts/My-Look.cube");
        assert_eq!(copy_lut(&bundle, &src).unwrap(), "luts/My-Look.cube", "same file reuses the copy");
        std::fs::write(&src, Lut::from_fn(3, |c| c).to_cube("t", "")).unwrap();
        assert_eq!(copy_lut(&bundle, &src).unwrap(), "luts/My-Look-2.cube");
        std::fs::write(&src, "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
        assert!(copy_lut(&bundle, &src).is_err());
    }

    #[test]
    fn previews_are_png_data_urls() {
        let img = effects::preview("duotone", 16, 9).unwrap();
        assert!(png_data_url(&img).unwrap().starts_with("data:image/png;base64,"));
    }
}
