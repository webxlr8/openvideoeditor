//! Per-frame cost of each effect and transition mask at 1920×1080 (design D2: port order by measured cost). For each
//! one: the CPU time of the operation alone, the GPU backend's time for it (the native shader, or the per-layer CPU
//! fallback: readback, CPU code, upload), and whether it is native. Prints a table sorted by GPU-backend cost; run
//! with `cargo test --release -p ove-gpu --test costs -- --ignored --nocapture`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::sync::Arc;
use std::time::Instant;

use ove_gpu::compositor::{GpuBackend, NATIVE_EFFECTS};
use ove_gpu::{GpuContext, GpuOptions};
use ove_render::backend::Backend;
use ove_render::effects::{self, EFFECTS};
use ove_render::transitions::{Role, TransState, split};
use serde_json::Map;

const RUNS: usize = 5;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

#[test]
#[ignore = "measurement: run on purpose with --ignored --nocapture"]
fn effect_and_mask_costs_at_1080p() {
    let ctx = if std::env::var_os("OVE_GPU_SOFTWARE").is_some() {
        GpuContext::software()
    } else {
        GpuContext::new(GpuOptions { allow_software: true })
    };
    let Ok(ctx) = ctx else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let mut gpu = GpuBackend::new(ctx);
    let adapter = gpu.adapter();
    let frame = Arc::new(ove_render::parity::mosaic(1920, 1080));
    let mut rows: Vec<(String, f64, f64, bool)> = Vec::new();
    let empty = Map::new();
    for spec in EFFECTS {
        let cpu = median(
            (0..RUNS)
                .map(|_| {
                    let mut img = (*frame).clone();
                    let t = Instant::now();
                    effects::apply(&mut img, spec.id, &empty, 1.0, 1.0);
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect(),
        );
        let tex = gpu.upload(&frame);
        gpu.effect(tex, spec.id, &empty, 1.0, 1.0); // builds the pipeline
        gpu.finish();
        let g = median(
            (0..RUNS)
                .map(|_| {
                    let t = Instant::now();
                    gpu.effect(tex, spec.id, &empty, 1.0, 1.0);
                    gpu.finish();
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect(),
        );
        gpu.release(tex);
        rows.push((format!("effect {}", spec.id), cpu, g, NATIVE_EFFECTS.contains(&spec.id)));
    }
    for kind in ["wipe-left", "radial-wipe", "ink-bleed", "mask-glitch"] {
        let (_, dir) = split(kind, None);
        let st = TransState { kind: kind.into(), dir, role: Role::In, q: 0.4 };
        let cpu = median(
            (0..RUNS)
                .map(|_| {
                    let mut img = (*frame).clone();
                    let t = Instant::now();
                    st.apply_mask(&mut img);
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect(),
        );
        let tex = gpu.upload(&frame);
        gpu.transition_mask(tex, &st);
        gpu.finish();
        let g = median(
            (0..RUNS)
                .map(|_| {
                    let t = Instant::now();
                    gpu.transition_mask(tex, &st);
                    gpu.finish();
                    t.elapsed().as_secs_f64() * 1e3
                })
                .collect(),
        );
        gpu.release(tex);
        rows.push((format!("mask {kind}"), cpu, g, true));
    }
    rows.sort_by(|a, b| b.2.total_cmp(&a.2));
    eprintln!("adapter: {adapter} · 1920x1080 · median of {RUNS}");
    for (name, cpu, g, native) in &rows {
        eprintln!(
            "{name:<32} cpu {cpu:>7.1} ms · gpu backend {g:>7.1} ms · {}",
            if *native { "native" } else { "CPU fallback" }
        );
    }
    let fallback = rows.iter().filter(|r| !r.3).count();
    eprintln!("{} native, {fallback} on the CPU fallback", rows.len() - fallback);
}
