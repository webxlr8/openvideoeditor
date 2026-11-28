//! A whole project rendered by the timeline renderer on the CPU backend and on the GPU backend (task 3.2): clips with
//! every one of the 41 effects (the GPU backend runs effects without a GPU version on the CPU for that layer), blend
//! modes, transforms with crop and rounded corners, the basic grade, a grade with wheels and a LUT, transitions,
//! a shader item and text. Frames are compared with the parity tolerance.
//!
//! Needs the ffmpeg CLI to make the clips (`OVE_FIXTURE_FFMPEG` or `ffmpeg` on PATH) and a GPU adapter (software
//! with `OVE_GPU_SOFTWARE=1`); skipped with a message otherwise.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_gpu::compositor::GpuBackend;
use ove_gpu::{GpuContext, GpuOptions};
use ove_render::Renderer;
use ove_render::effects::EFFECTS;
use ove_render::parity::{TOLERANCE, compare};
use serde_json::json;

fn ffmpeg() -> Option<String> {
    let bin = std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    Command::new(&bin).arg("-version").output().ok().filter(|o| o.status.success()).map(|_| bin)
}

fn make(dir: &Path, name: &str, src: &str) -> PathBuf {
    let out = dir.join(name);
    let st = Command::new(ffmpeg().unwrap())
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", src])
        .args(["-t", "6", "-c:v", "libx264", "-crf", "8", "-pix_fmt", "yuv420p"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success(), "ffmpeg failed for {name}");
    out
}

fn gpu() -> Option<GpuBackend> {
    let ctx = if std::env::var_os("OVE_GPU_SOFTWARE").is_some() {
        GpuContext::software()
    } else {
        GpuContext::new(GpuOptions { allow_software: true })
    };
    match ctx {
        Ok(c) => Some(GpuBackend::new(c)),
        Err(e) if std::env::var_os("OVE_REQUIRE_GPU").is_some() => panic!("no GPU: {e}"),
        Err(e) => {
            eprintln!("skipped: no GPU adapter ({e})");
            None
        }
    }
}

const BLENDS: [&str; 8] =
    ["multiply", "screen", "overlay", "soft-light", "difference", "color-dodge", "hue", "luminosity"];

/// Main: one 0.1 s clip per effect from 0 s, then blend and grade clips. Overlay: a transformed, cropped, rounded
/// clip with a blend mode over each blend clip. Text over everything; a wipe join and a shader item at the end.
fn project(dir: &Path) -> (Document, Vec<(f64, String)>) {
    let a = make(dir, "a.mp4", "testsrc2=s=640x360:r=30");
    let b = make(dir, "b.mp4", "smptehdbars=s=640x360:r=30");
    let media = |id: &str, p: &Path| Op::AddMedia {
        media: json!({"id": id, "data": {"type": "video", "fileName": p.file_name().unwrap().to_string_lossy(), "duration": 6, "hasAudio": false}}),
        entry: Some(json!({"mediaId": id, "nativePath": p.display().to_string(), "ownedByBundle": false})),
    };
    let mut ops = vec![
        Op::SetAspectRatio { aspect_ratio: "16:9".into() },
        Op::AddTrack { track: json!({"id": "text", "name": "Text", "order": 0, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "over", "name": "Overlay", "order": 1, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "main", "name": "Main", "order": 2, "type": "video"}) },
        media("ma", &a),
        media("mb", &b),
    ];
    let mut checks = Vec::new();
    let mut t = 0.0f64;
    let clip = |ops: &mut Vec<Op>, track: &str, id: String, start: f64, end: f64, extra: serde_json::Value| {
        let mut item = json!({"id": id, "trackId": track, "type": "video", "startTime": start, "endTime": end,
                              "mediaItemId": if track == "main" { "ma" } else { "mb" }});
        if let (Some(m), Some(e)) = (item.as_object_mut(), extra.as_object()) {
            m.extend(e.clone());
        }
        ops.push(Op::AddItem { item });
    };
    for spec in EFFECTS {
        clip(
            &mut ops,
            "main",
            format!("fx-{}", spec.id),
            t,
            t + 0.1,
            json!({"effects": [{"id": "e", "type": spec.id}]}),
        );
        checks.push((t + 0.05, format!("effect {}", spec.id)));
        t += 0.1;
    }
    for (n, mode) in BLENDS.iter().enumerate() {
        clip(&mut ops, "main", format!("base-{n}"), t, t + 0.1, json!({}));
        clip(
            &mut ops,
            "over",
            format!("blend-{n}"),
            t,
            t + 0.1,
            json!({"blendMode": mode, "opacity": 0.85, "borderRadius": 40, "crop": {"top": 5, "right": 8, "bottom": 3, "left": 6},
                   "transform": {"x": 120, "y": -60, "scale": 0.7, "rotation": 6}}),
        );
        checks.push((t + 0.05, format!("blend {mode} over a transformed clip")));
        t += 0.1;
    }
    clip(
        &mut ops,
        "main",
        "graded".into(),
        t,
        t + 0.1,
        json!({"colorGrading": {"exposure": 0.2, "contrast": 0.15, "saturation": 0.3, "temperature": -0.3},
               "colorGrade": {"wheels": {"gain": {"r": 0.1, "g": 0.0, "b": -0.05, "y": 0.0}}, "lut": {"preset": "cinematic", "intensity": 0.7}}}),
    );
    checks.push((t + 0.05, "grade and LUT".into()));
    t += 0.1;
    // A wipe join between two clips, sampled mid-transition.
    clip(&mut ops, "main", "out".into(), t, t + 0.5, json!({}));
    clip(
        &mut ops,
        "main",
        "in".into(),
        t + 0.5,
        t + 1.0,
        json!({"transitionIn": {"type": "wipe", "direction": "left", "duration": 0.4}}),
    );
    checks.push((t + 0.65, "wipe join".into()));
    clip(
        &mut ops,
        "over",
        "push".into(),
        t + 0.5,
        t + 1.0,
        json!({"transitionIn": {"type": "push-up", "duration": 0.4}}),
    );
    checks.push((t + 0.62, "push over a wipe".into()));
    t += 1.0;
    clip(&mut ops, "main", "tail".into(), t, t + 0.5, json!({}));
    ops.push(Op::AddItem {
        item: json!({"id": "title", "trackId": "text", "type": "text", "startTime": 0, "endTime": t + 0.5,
                     "textContent": "PARITY", "fontSize": 160, "fontWeight": 700, "color": "#ffd000", "strokeColor": "#000000", "strokeWidth": 6}),
    });
    ops.push(Op::AddTrack { track: json!({"id": "shader", "name": "Shader", "order": 0, "type": "video"}) });
    ops.push(Op::AddItem {
        item: json!({"id": "sh", "trackId": "shader", "type": "shader", "startTime": t, "endTime": t + 0.5,
                     "shaderComposition": {"template": "vhs", "amount": 60}}),
    });
    checks.push((t + 0.25, "shader item".into()));
    let doc = apply_batch(&Document::new("p", "Parity", 1), &ops).unwrap().doc;
    (doc, checks)
}

#[test]
fn a_project_renders_the_same_on_both_backends() {
    if ffmpeg().is_none() {
        eprintln!("skipped: no ffmpeg CLI to make fixtures (set OVE_FIXTURE_FFMPEG)");
        return;
    }
    let Some(gpu) = gpu() else { return };
    let adapter = gpu.adapter();
    ove_media::init();
    let d = tempfile::tempdir().unwrap();
    let (doc, checks) = project(d.path());
    let height = 360;
    let mut cpu = Renderer::new(None, height);
    let mut gpu = Renderer::with_backend(None, height, Box::new(gpu));
    assert_eq!(gpu.backend_name(), "gpu");
    let mut failed = Vec::new();
    let (mut worst, mut worst_mean) = (0u8, 0.0f64);
    for (at, what) in &checks {
        let us = (at * 1e6).round() as i64;
        let a = cpu.render(&doc, us);
        let b = gpu.render(&doc, us);
        let m = compare(&a, &b);
        worst = worst.max(m.max);
        worst_mean = worst_mean.max(m.mean);
        if !m.within(&TOLERANCE) {
            failed.push(format!("{what} at {at:.2} s: {m}"));
        }
    }
    eprintln!(
        "adapter: {adapter} · {} frames at {height}p · worst max {worst}/255 · worst mean {worst_mean:.3}/255",
        checks.len()
    );
    assert!(failed.is_empty(), "frames differ beyond the parity tolerance on {adapter}:\n{}", failed.join("\n"));
}
