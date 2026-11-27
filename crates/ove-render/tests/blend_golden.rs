//! Golden render of every schema blend mode through the full renderer (no media files needed).
//!
//! The fixture is a 4×4 grid on a 16:9 canvas. Behind each cell is the same backdrop: a horizontal ramp from black
//! through red, yellow and blue to white. Over each cell's middle lies one layer with one blend mode, in schema order
//! (row by row): a vertical ramp from white through magenta, green and orange to black. The cell's margins show the
//! plain backdrop, so a review by eye can compare each mode with its inputs.
//!
//! The frame is compared with `tests/golden/blend-modes-320x180.pam` with a tolerance of one 8-bit step. A second test
//! renders each cell again with `normal` and requires a visible difference, so the check fails if any mode falls back
//! to Normal. Regenerate the golden file with `OVE_UPDATE_GOLDEN=1` after checking the frame by eye;
//! `OVE_GOLDEN_DUMP=<dir>` also writes the current frame there.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

mod golden_util;

use golden_util::{TOLERANCE, golden_or_bless, max_diff, write_pam};
use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_core::schema::BLEND_MODES;
use ove_render::{Renderer, Rgba};
use serde_json::json;

const GOLDEN: &str = "blend-modes-320x180.pam";
/// Output frame height; 16:9 gives 320×180, so each cell is 80×45 pixels.
const OUT_H: u32 = 180;
const COLS: usize = 4;
/// Cell size and layer inset in the composition's reference frame (1920×1080).
const CELL_W: u32 = 480;
const CELL_H: u32 = 270;
const INSET_X: u32 = 40;
const INSET_Y: u32 = 30;

const BACKDROP: &str = "export default function Backdrop() {
  const cells = [];
  for (let i = 0; i < 16; i++) {
    cells.push(<div style={{ position: 'absolute', left: (i % 4) * 480, top: Math.floor(i / 4) * 270, width: 480,
      height: 270, backgroundImage: 'linear-gradient(90deg, #000000, #e0402a, #f2c94c, #2e6fd7, #ffffff)' }} />);
  }
  return <Fill>{cells}</Fill>;
}";

/// The source layer of cell `i`: a vertical ramp inset in the cell.
fn layer_source(i: usize) -> String {
    let (x, y) = ((i % COLS) as u32 * CELL_W + INSET_X, (i / COLS) as u32 * CELL_H + INSET_Y);
    let (w, h) = (CELL_W - 2 * INSET_X, CELL_H - 2 * INSET_Y);
    format!(
        "export default function Layer() {{
  return <Fill><div style={{{{ position: 'absolute', left: {x}, top: {y}, width: {w}, height: {h},
    backgroundImage: 'linear-gradient(180deg, #ffffff, #d63fb8, #3fbf5a, #f08a24, #000000)' }}}} /></Fill>;
}}"
    )
}

/// The fixture document; `modes[i]` is the blend mode of cell `i`'s layer.
fn doc(modes: &[&str]) -> Document {
    let n = modes.len();
    let mut ops = vec![Op::SetAspectRatio { aspect_ratio: "16:9".into() }];
    // Track order 0 is the top: the cell layers sit above the backdrop.
    for (i, mode) in modes.iter().enumerate() {
        let track = format!("l{i}");
        ops.push(Op::AddTrack { track: json!({"id": track, "name": mode, "order": i, "type": "video"}) });
        ops.push(Op::AddItem {
            item: json!({"id": format!("c{i}"), "trackId": track, "type": "code", "startTime": 0, "endTime": 2,
                "blendMode": mode, "codeComposition": {"source": layer_source(i)}}),
        });
    }
    ops.push(Op::AddTrack { track: json!({"id": "bg", "name": "Backdrop", "order": n, "type": "video"}) });
    ops.push(Op::AddItem {
        item: json!({"id": "bg", "trackId": "bg", "type": "code", "startTime": 0, "endTime": 2,
            "codeComposition": {"source": BACKDROP}}),
    });
    apply_batch(&Document::new("p", "Blend modes", 1), &ops).unwrap().doc
}

fn render(d: &Document) -> Rgba {
    Renderer::new(None, OUT_H).render(d, 1_000_000)
}

/// The pixels of cell `i`'s layer area (inside the inset, one pixel in from the edge), as RGBA.
fn cell(img: &Rgba, i: usize) -> Vec<u8> {
    let k = img.h as f64 / 1080.0;
    let x0 = (((i % COLS) as u32 * CELL_W + INSET_X) as f64 * k).ceil() as u32 + 1;
    let y0 = (((i / COLS) as u32 * CELL_H + INSET_Y) as f64 * k).ceil() as u32 + 1;
    let x1 = ((((i % COLS) as u32 + 1) * CELL_W - INSET_X) as f64 * k).floor() as u32 - 1;
    let y1 = ((((i / COLS) as u32 + 1) * CELL_H - INSET_Y) as f64 * k).floor() as u32 - 1;
    let mut out = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            out.extend_from_slice(&img.px(x, y));
        }
    }
    out
}

#[test]
fn every_blend_mode_matches_its_golden_frame() {
    assert_eq!(BLEND_MODES.len(), 16, "the grid has 16 cells: extend the fixture for new modes");
    let img = render(&doc(BLEND_MODES));
    assert_eq!((img.w, img.h), (320, 180));
    if let Some(dir) = std::env::var_os("OVE_GOLDEN_DUMP") {
        write_pam(&std::path::PathBuf::from(dir).join(GOLDEN), img.w, img.h, &img.data).unwrap();
    }
    let (gw, gh, want) = golden_or_bless(GOLDEN, img.w, img.h, &img.data);
    assert_eq!((gw, gh), (img.w, img.h));
    let golden = Rgba { w: gw, h: gh, data: want };
    let failed: Vec<String> = BLEND_MODES
        .iter()
        .enumerate()
        .filter_map(|(i, mode)| {
            let d = max_diff(&cell(&img, i), &cell(&golden, i));
            (d > TOLERANCE).then(|| format!("{mode} (max channel difference {d})"))
        })
        .collect();
    assert!(failed.is_empty(), "blend output changed: {}", failed.join(", "));
    let whole = max_diff(&img.data, &golden.data);
    assert!(whole <= TOLERANCE, "the frame outside the layers changed (max channel difference {whole})");
}

#[test]
fn no_blend_mode_renders_as_normal() {
    let img = render(&doc(BLEND_MODES));
    let normal = render(&doc(&["normal"; 16]));
    let same: Vec<&str> = BLEND_MODES
        .iter()
        .enumerate()
        .filter(|(_, mode)| **mode != "normal")
        // A mode that fell back to Normal gives the same cell as the Normal render.
        .filter(|(i, _)| max_diff(&cell(&img, *i), &cell(&normal, *i)) <= 8)
        .map(|(_, mode)| *mode)
        .collect();
    assert!(same.is_empty(), "these modes render as normal: {}", same.join(", "));
    assert!(max_diff(&cell(&img, 0), &cell(&normal, 0)) <= TOLERANCE, "the normal cell matches the normal render");
}
