//! `look` gives the same receipt and the same image bytes in-app and over MCP, with and without the grid
//! (`cargo test -p ove-tools --features media`).
#![cfg(feature = "media")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::sync::Arc;

use ove_core::ops::Op;
use ove_core::revision::Source;
use ove_store::project::{Mode, Project};
use ove_tools::{AppCtx, BundleCtx, ProjectHost, Surface};
use serde_json::json;

#[test]
fn look_is_the_same_in_app_and_over_mcp() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Demo").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t", "name": "Text", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "title", "trackId": "t", "type": "text", "startTime": 0, "endTime": 4,
            "textContent": "Title", "fontSize": 96, "color": "#ffffff", "transform": {"x": -320, "y": 0}}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();

    for grid in [false, true] {
        let args = json!({"project": "Demo", "at": "1", "width": 320, "grid": grid});
        let mut mcp = BundleCtx::open(Some(&lib), Surface::Mcp, Source::Mcp).unwrap();
        let over_mcp = ove_tools::call(&mut mcp, "look", args.as_object().unwrap()).unwrap();
        let mut held = Project::open(&bundle, Mode::Write, Arc::new(ove_core::SystemClock)).unwrap();
        let mut host = ProjectHost { project: &mut held };
        let mut app = AppCtx::assistant(&mut host);
        let in_app = ove_tools::call(&mut app, "look", args.as_object().unwrap()).unwrap();
        assert_eq!(over_mcp.json, in_app.json, "grid {grid}");
        assert_eq!(over_mcp.images.len(), 1);
        assert_eq!(over_mcp.images, in_app.images, "same image bytes (grid {grid})");
        assert_eq!(over_mcp.json.get("grid").is_some(), grid);
        held.close().unwrap();
    }
}

#[test]
fn a_timeline_storyboard_names_the_visible_items() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Board").unwrap();
    let ops: Vec<Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "t", "name": "Text", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "first", "trackId": "t", "type": "text", "startTime": 0, "endTime": 5,
            "textContent": "One", "fontSize": 120, "color": "#ffffff"}},
        {"op": "addItem", "item": {"id": "second", "trackId": "t", "type": "text", "startTime": 5, "endTime": 10,
            "textContent": "Two", "fontSize": 120, "color": "#ff0000", "transform": {"x": 400, "y": 200}}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    p.close().unwrap();
    let mut mcp = BundleCtx::open(Some(&lib), Surface::Mcp, Source::Mcp).unwrap();
    let r = ove_tools::call(
        &mut mcp,
        "look",
        json!({"project": "Board", "storyboard": true, "width": 160}).as_object().unwrap(),
    )
    .unwrap();
    let tiles = r.json["tiles"].as_array().unwrap();
    assert!(!tiles.is_empty() && tiles.len() <= 36, "{}", r.json);
    assert!(tiles.iter().any(|t| t["items"] == json!(["first"])), "{}", r.json);
    assert!(tiles.iter().any(|t| t["items"] == json!(["second"])), "{}", r.json);
    assert_eq!(tiles[0]["timecode"], "0:00:00.0");
    assert_eq!(r.images.len(), 1, "one sheet");
}
