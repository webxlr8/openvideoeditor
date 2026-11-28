//! add-semantic-media-search: `inspect_media` with `overview` on a 10-minute video gives one sheet of at most 36
//! labelled tiles from the shot list, without decoding the whole file again (the index is built first with a stand-in
//! encoder). Needs the `media` feature and the ffmpeg CLI for the fixture.
#![cfg(feature = "media")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use ove_core::revision::Source;
use ove_tools::{BundleCtx, Surface};
use ove_vision::index::{DIM, Provenance};
use serde_json::json;

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
}

struct Flat;

impl ove_vision::model::Encoder for Flat {
    fn images(&mut self, p: &[Vec<f32>]) -> ove_vision::Result<Vec<Vec<f32>>> {
        Ok(p.iter().map(|_| (0..DIM).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect()).collect())
    }
    fn text(&mut self, _: &str) -> ove_vision::Result<Vec<f32>> {
        Ok(vec![0.0; DIM])
    }
    fn provenance(&self) -> Provenance {
        Provenance { model: "flat".into(), model_hash: "0".into(), sampler: 1, preprocessing: "t".into() }
    }
}

#[test]
fn a_ten_minute_overview_is_one_sheet_of_at_most_36_tiles() {
    let Some(ff) = ffmpeg() else {
        eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
        return;
    };
    let d = tempfile::tempdir().unwrap();
    let clip = d.path().join("long.mp4");
    // 10 minutes: a moving test card with a cut to a fractal at 5 minutes, small and at 10 fps.
    let st = Command::new(ff)
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-t", "300", "-i", "testsrc2=s=192x108:r=10"])
        .args(["-f", "lavfi", "-t", "300", "-i", "mandelbrot=s=192x108:r=10"])
        .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0", "-pix_fmt", "yuv420p", "-c:v", "libx264"])
        .args(["-preset", "ultrafast", "-g", "50"])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(st.success());
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Long").unwrap();
    let cid = ove_store::media::content_id(&clip).unwrap();
    let ops: Vec<ove_core::ops::Op> = serde_json::from_value(json!([
        {"op": "addMedia", "media": {"id": "m", "data": {"type": "video", "fileName": "long.mp4", "duration": 600}},
         "entry": {"mediaId": "m", "nativePath": clip.to_string_lossy(), "ownedByBundle": false, "contentId": cid}}
    ]))
    .unwrap();
    p.apply(ops, Source::Gui, None).unwrap();
    let bundle = p.bundle().to_path_buf();
    p.close().unwrap();
    let built = ove_vision::indexer::ensure_index(
        &bundle,
        &clip,
        &cid,
        ove_vision::indexer::MediaKind::Video,
        &mut Flat,
        &mut |_| true,
    )
    .unwrap();
    assert_eq!(built.shots, 2, "{built:?}");
    assert!(built.rows > 36, "{built:?}");

    let mut ctx = BundleCtx::open(Some(&lib), Surface::Mcp, Source::Mcp).unwrap();
    let t0 = std::time::Instant::now();
    let r = ove_tools::call(
        &mut ctx,
        "inspect_media",
        json!({"project": "Long", "media": "long", "overview": true}).as_object().unwrap(),
    )
    .unwrap();
    let took = t0.elapsed();
    let sheet = &r.json["sheet"];
    eprintln!("overview of 10 minutes: {} tiles in {took:?}", sheet["tiles"]);
    assert_eq!(sheet["from"], "shots");
    let times: Vec<f64> = sheet["times"].as_array().unwrap().iter().map(|t| t.as_f64().unwrap()).collect();
    assert_eq!(times.len(), 36);
    assert!(times.iter().any(|&t| t < 300.0) && times.iter().any(|&t| t >= 300.0), "both shots are on the sheet");
    assert_eq!(r.images.len(), 1, "one sheet");
    assert_eq!(r.json["shots"], 2);
    let _ = Path::new(&bundle);
}
