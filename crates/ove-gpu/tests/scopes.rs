//! Scope binning on the GPU (task 6.1): the compute pass gives exactly the CPU counts for every scope, and its cost
//! per 540p frame is measured (design: ≤ 2 ms on the GPU). Skipped without a GPU adapter (software with
//! `OVE_GPU_SOFTWARE=1`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::sync::Arc;
use std::time::Instant;

use ove_gpu::compositor::GpuBackend;
use ove_gpu::{GpuContext, GpuOptions};
use ove_render::backend::Backend;
use ove_render::scopes::{self, Scope};

#[test]
fn gpu_scopes_equal_cpu_scopes() {
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
    let frame = Arc::new(ove_render::parity::mosaic(960, 540));
    let t = gpu.upload(&frame);
    for scope in Scope::ALL {
        let cpu = scopes::bin(&frame, scope);
        let g = gpu.scope(t, scope).expect("the GPU ran the scope pass");
        assert_eq!(g, cpu, "{scope:?} counts differ on {adapter}");
        let n = 10;
        let start = Instant::now();
        for _ in 0..n {
            std::hint::black_box(gpu.scope(t, scope));
        }
        let ms = start.elapsed().as_secs_f64() * 1e3 / n as f64;
        eprintln!("{scope:?} on {adapter}: {ms:.2} ms per 540p frame (compute pass and 256 KB readback)");
    }
    gpu.release(t);
}
