//! Backend selection and fallback (design D9, task 3.3): the self-test, a forced self-test failure, forced backends,
//! and a device loss in the middle of rendering.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};
use ove_gpu::GpuContext;
use ove_gpu::compositor::{Choice, GpuBackend, SelectOptions, select, select_with, self_test};
use ove_render::Renderer;
use ove_render::parity::{TOLERANCE, compare};
use serde_json::json;

/// A project without media: a shape-free text title over a colour grade adjustment and a background shader.
fn doc() -> Document {
    let ops = vec![
        Op::SetAspectRatio { aspect_ratio: "16:9".into() },
        Op::AddTrack { track: json!({"id": "text", "name": "Text", "order": 0, "type": "video"}) },
        Op::AddTrack { track: json!({"id": "bg", "name": "Background", "order": 1, "type": "video"}) },
        Op::AddItem {
            item: json!({"id": "bg", "trackId": "bg", "type": "shader", "startTime": 0, "endTime": 4,
                         "shaderComposition": {"template": "mesh-gradient", "amount": 100}}),
        },
        Op::AddItem {
            item: json!({"id": "t", "trackId": "text", "type": "text", "startTime": 0, "endTime": 4, "textContent": "FALLBACK",
                         "fontSize": 180, "color": "#ffffff", "blendMode": "overlay", "transform": {"rotation": 5, "scale": 1.2}}),
        },
    ];
    apply_batch(&Document::new("p", "Select", 1), &ops).unwrap().doc
}

fn hardware() -> Option<GpuBackend> {
    match GpuContext::new(ove_gpu::GpuOptions { allow_software: true }) {
        Ok(c) => Some(GpuBackend::new(c)),
        Err(e) => {
            eprintln!("skipped: no GPU adapter ({e})");
            None
        }
    }
}

#[test]
fn self_test_passes_and_auto_picks_a_backend() {
    let Some(mut gpu) = hardware() else { return };
    let adapter = gpu.adapter();
    assert_eq!(self_test(&mut gpu), Ok(()), "self-test on {adapter}");
    let s = select(Choice::Auto).unwrap();
    eprintln!("auto: {} — {}", s.name, s.reason);
    if adapter.device_type == "Cpu" {
        assert_eq!(s.name, "cpu", "a software adapter is not used automatically");
    } else {
        assert_eq!(s.name, "gpu");
        assert!(s.reason.contains("self-test passed"), "{}", s.reason);
    }
}

#[test]
fn a_failed_self_test_falls_back_to_the_cpu_with_the_reason() {
    if hardware().is_none() {
        return;
    }
    let s = select_with(Choice::Auto, SelectOptions { fail_self_test: true }).unwrap();
    assert_eq!(s.name, "cpu");
    assert!(
        s.reason.contains("GPU self-test failed") || s.reason.contains("software GPU"),
        "the overlay states why: {}",
        s.reason
    );
    assert_eq!(s.backend.name(), "cpu");
}

#[test]
fn forced_backends() {
    let s = select(Choice::Cpu).unwrap();
    assert_eq!((s.name, s.backend.name()), ("cpu", "cpu"));
    if hardware().is_some() {
        let g = select(Choice::Gpu).unwrap();
        assert_eq!((g.name, g.backend.name()), ("gpu", "gpu"));
        assert!(g.adapter.is_some());
    }
    assert_eq!(Choice::parse("GPU"), Some(Choice::Gpu));
    assert_eq!(Choice::parse("histogram"), None);
}

#[test]
fn device_loss_while_rendering_continues_on_the_cpu() {
    let Some(gpu) = hardware() else { return };
    let loss = gpu.loss_handle();
    let doc = doc();
    let mut r = Renderer::with_backend(None, 270, Box::new(gpu));
    let mut reference = Renderer::new(None, 270);
    let before = r.render(&doc, 500_000);
    assert_eq!(r.backend_name(), "gpu");
    let m = compare(&before, &reference.render(&doc, 500_000));
    assert!(m.within(&TOLERANCE), "GPU frame before the loss: {m}");
    loss.trigger();
    let after = r.render(&doc, 1_500_000);
    assert_eq!(r.backend_name(), "cpu", "the renderer switched backends");
    assert!(r.fallback().unwrap().contains("device lost"), "{:?}", r.fallback());
    assert_eq!(after.data, reference.render(&doc, 1_500_000).data, "the frame after the loss is the CPU frame");
    // Later frames keep coming.
    assert_eq!(r.render(&doc, 2_500_000).w, after.w);
}
