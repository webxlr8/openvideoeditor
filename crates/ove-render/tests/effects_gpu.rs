//! The GPU path of effects gives the CPU's picture: every WGSL port is run on the sample frame and compared with the
//! CPU at the same parameters, amount and time. Skipped (with a message) when no GPU opens.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use ove_render::effects::{self, EFFECTS, Params, gpu};
use serde_json::{Map, Value, json};

/// Mean absolute channel difference, and the share of channels off by more than 24 (of 255).
fn diff(a: &[u8], b: &[u8]) -> (f64, f64) {
    let (mut sum, mut far) = (0u64, 0usize);
    for (x, y) in a.iter().zip(b) {
        let d = x.abs_diff(*y);
        sum += u64::from(d);
        far += usize::from(d > 24);
    }
    (sum as f64 / a.len() as f64, far as f64 / a.len() as f64)
}

/// Parameters away from the defaults: numbers moved a third of the way through their range, colours shifted.
fn moved(id: &str) -> Map<String, Value> {
    let spec = effects::find(id).unwrap();
    let v = serde_json::to_value(spec).unwrap();
    let mut out = Map::new();
    for p in v["params"].as_array().unwrap() {
        let name = p["name"].as_str().unwrap().to_string();
        match (p["kind"]["kind"].as_str(), &p["default"]) {
            (Some("number" | "percent"), d) => {
                let (lo, hi) = (p["kind"]["min"].as_f64().unwrap(), p["kind"]["max"].as_f64().unwrap());
                let x = d.as_f64().unwrap() + (hi - lo) / 3.0;
                out.insert(name, json!(if x > hi { x - (hi - lo) * 2.0 / 3.0 } else { x }));
            }
            (Some("toggle"), d) => {
                out.insert(name, json!(d.as_f64().unwrap() < 0.5));
            }
            (Some("color"), d) => {
                let c: Vec<f64> = d.as_array().unwrap().iter().map(|x| (x.as_f64().unwrap() + 0.3) % 1.0).collect();
                out.insert(name, json!(c));
            }
            (Some("point"), _) => {
                out.insert(name, json!([0.4, 0.6]));
            }
            _ => {}
        }
    }
    out
}

#[test]
fn every_gpu_port_matches_the_cpu() {
    if !gpu::available() {
        eprintln!("no GPU for effects: skipped");
        return;
    }
    let base = effects::sample_image(320, 180);
    let mut ported = 0;
    let mut bad = Vec::new();
    // OVE_GPU_TEST_ONLY=id,id checks just those effects.
    let only = std::env::var("OVE_GPU_TEST_ONLY").ok();
    let wanted = |id: &str| only.as_deref().is_none_or(|o| o.split(',').any(|x| x.trim() == id));
    for spec in EFFECTS.iter().filter(|s| gpu::wgsl(s.id).is_some() && wanted(s.id)) {
        ported += 1;
        for (label, values, amount, t) in [("defaults", Map::new(), 1.0, 0.0), ("moved", moved(spec.id), 0.8, 1.3)] {
            let mut cpu = base.clone();
            // Below gpu::MIN_PIXELS, `apply` stays on the CPU.
            assert!(effects::apply(&mut cpu, spec.id, &values, amount, t));
            let mut on_gpu = base.clone();
            gpu::run(&mut on_gpu, spec, &Params::resolve(spec, &values), amount, t)
                .unwrap_or_else(|e| panic!("{}: {e}", spec.id));
            let (mean, far) = diff(&cpu.data, &on_gpu.data);
            eprintln!("{:<22} {label:<8} mean {mean:.3}  far {:.3} %", spec.id, far * 100.0);
            if mean > 1.0 || far > 0.005 {
                bad.push(format!("{} ({label}): mean {mean:.3}, {:.2} % far off", spec.id, far * 100.0));
            }
        }
    }
    assert!(ported > 0);
    assert!(bad.is_empty(), "GPU and CPU differ:\n{}", bad.join("\n"));
}

#[test]
fn every_effect_has_a_gpu_port() {
    let missing: Vec<&str> = EFFECTS.iter().filter(|s| gpu::wgsl(s.id).is_none()).map(|s| s.id).collect();
    assert!(missing.is_empty(), "no WGSL port: {missing:?}");
    for spec in EFFECTS {
        assert!(gpu::shader_source(spec).is_some_and(|s| s.contains("fn effect(")), "{}", spec.id);
    }
}
