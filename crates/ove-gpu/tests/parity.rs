//! CPU/GPU parity (design D3, task 2.2): every case of `ove_render::parity` on the CPU reference and on the GPU
//! backend, compared with the specification's tolerances (max 2/255, mean 0.25/255, CIEDE2000 p99 1.0).
//!
//! - `OVE_GPU_SOFTWARE=1` uses the software adapter (WARP on Windows, lavapipe on Linux) as CI does; otherwise the
//!   best hardware adapter, falling back to software.
//! - `OVE_PARITY_HEIGHT` sets the frame height (default 1080; the width is 16:9).
//! - Without any adapter the test is skipped, unless `OVE_REQUIRE_GPU=1`.
//!
//! Every run prints the adapter name, per-case metrics and the time each backend took.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::time::Instant;

use ove_gpu::compositor::GpuBackend;
use ove_gpu::{GpuContext, GpuOptions};
use ove_render::backend::{Backend, CpuBackend};
use ove_render::parity::{self, TOLERANCE};

fn open() -> Option<GpuBackend> {
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

fn height() -> u32 {
    std::env::var("OVE_PARITY_HEIGHT").ok().and_then(|v| v.parse().ok()).unwrap_or(1080)
}

#[test]
fn gpu_matches_cpu_within_tolerance() {
    let Some(mut gpu) = open() else { return };
    let h = height();
    let w = (h * 16 / 9) & !1;
    let adapter = gpu.adapter();
    eprintln!("adapter: {adapter} · frames {w}x{h}");
    let cases = parity::cases(w, h);
    let inputs = parity::Inputs::new(w, h);
    let mut cpu = CpuBackend::new();
    let (mut cpu_ms, mut gpu_ms) = (0.0, 0.0);
    let mut rows = Vec::new();
    let mut times = Vec::new();
    for c in &cases {
        let t = Instant::now();
        let a = parity::run(&mut cpu, c, &inputs);
        let c_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let b = parity::run(&mut gpu, c, &inputs);
        let g_ms = t.elapsed().as_secs_f64() * 1e3;
        (cpu_ms, gpu_ms) = (cpu_ms + c_ms, gpu_ms + g_ms);
        let metrics = parity::compare(&a, &b);
        times.push((c_ms, g_ms));
        rows.push(parity::Row { name: c.name.clone(), pass: metrics.within(&TOLERANCE), metrics });
    }
    // Per case: the metrics, then the CPU and GPU time of the whole case (upload, operation, readback).
    for (r, (c_ms, g_ms)) in rows.iter().zip(&times) {
        eprintln!(
            "{} {:<52} {} · cpu {c_ms:.1} ms · gpu {g_ms:.1} ms",
            if r.pass { "ok  " } else { "FAIL" },
            r.name,
            r.metrics
        );
    }
    let worst = rows.iter().map(|r| r.metrics.max).max().unwrap_or(0);
    let c = &gpu.counters;
    eprintln!(
        "{} cases · CPU {cpu_ms:.0} ms · GPU {gpu_ms:.0} ms (with uploads and readbacks) · worst max {worst}/255 · \
         GPU passes {} · CPU fallbacks {}",
        rows.len(),
        c.passes.load(std::sync::atomic::Ordering::Relaxed),
        c.cpu_fallbacks.load(std::sync::atomic::Ordering::Relaxed)
    );
    assert_eq!(gpu.held(), 0, "every image is released");
    let failed = parity::failures(&rows);
    assert!(failed.is_empty(), "GPU parity failures on {adapter}:\n{failed}");
}

/// Task 2.3: a deliberately wrong shader (the difference blend scaled by 0.9) fails parity, and the report names
/// the case and the measured difference.
#[test]
fn a_wrong_shader_fails_and_is_named() {
    let Some(mut gpu) = open() else { return };
    let wrong = "case 10u: { return abs(d - s) * 0.9; }";
    gpu.rebuild_draw_shader(|src| {
        assert!(src.contains("case 10u: { return abs(d - s); }"));
        src.replace("case 10u: { return abs(d - s); }", wrong)
    });
    let rows = parity::check(&mut CpuBackend::new(), &mut gpu, &parity::cases(320, 180), 320, 180, &TOLERANCE);
    let report = parity::failures(&rows);
    eprintln!("{report}");
    let names: Vec<&str> = report.lines().map(|l| l.split(':').next().unwrap()).collect();
    assert_eq!(names, ["blend difference", "blend difference, transformed"], "{report}");
    assert!(report.contains("max ") && report.contains("CIEDE2000 p99"), "{report}");
}

/// The GPU backend's native operations are really on the GPU: drawing, grading, LUTs and mixing run without a CPU
/// fallback.
#[test]
fn core_operations_run_natively() {
    let Some(mut gpu) = open() else { return };
    let inputs = parity::Inputs::new(320, 180);
    for c in parity::cases(320, 180).iter().filter(|c| {
        c.name.starts_with("draw")
            || c.name.starts_with("blend")
            || c.name.starts_with("lut")
            || c.name.starts_with("grade")
    }) {
        let before = gpu.counters.cpu_fallbacks.load(std::sync::atomic::Ordering::Relaxed);
        parity::run(&mut gpu, c, &inputs);
        let after = gpu.counters.cpu_fallbacks.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(before, after, "{} fell back to the CPU", c.name);
    }
    assert_eq!(gpu.name(), "gpu");
}
