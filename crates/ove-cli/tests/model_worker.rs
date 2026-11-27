//! `ove model-worker` through `ove_models::worker::WorkerClient`: idle exit, a killed worker fails its job and the next
//! job succeeds, a failed job keeps the worker, and the round trip of a 256×256×3 tensor.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use ove_models::ModelError;
use ove_models::infer::Tensor;
use ove_models::worker::WorkerClient;

fn client(idle_secs: u64) -> WorkerClient {
    let idle = idle_secs.to_string();
    WorkerClient::new(env!("CARGO_BIN_EXE_ove"), &["model-worker", "--idle-secs", &idle])
        .env("OVE_ONNXRUNTIME_LIB", "")
        .env("OVE_MODELS_DIR", &empty_store())
}

/// A model store with nothing in it (so the worker never finds a runtime installed on this computer).
fn empty_store() -> String {
    std::env::temp_dir().join(format!("ove-empty-models-{}", std::process::id())).display().to_string()
}

#[test]
fn a_clean_profile_worker_reports_runtime_not_installed() {
    let w = client(30);
    let model = std::env::temp_dir().join("no-such-model.onnx");
    let e = w.run(&model.display().to_string(), &[Tensor::f32("x", &[1], vec![0.0])]).unwrap_err();
    assert_eq!(e.code(), "model.jobFailed");
    assert!(e.to_string().contains("runtime not installed"), "{e}");
    // Built-in models still run, and the worker survived.
    assert!(w.run("builtin:echo", &[Tensor::f32("x", &[1], vec![1.0])]).is_ok());
}

/// The worker finds the runtime the store installed (no `OVE_ONNXRUNTIME_LIB`): with the runtime loaded, a missing
/// model is a model error, not "runtime not installed". Needs `OVE_TEST_RUNTIME_STORE`, a store with `onnxruntime`.
#[test]
#[ignore = "needs a model store with the runtime installed (OVE_TEST_RUNTIME_STORE)"]
fn the_worker_loads_the_runtime_from_the_store() {
    let store = std::env::var("OVE_TEST_RUNTIME_STORE").expect("set OVE_TEST_RUNTIME_STORE");
    let w = WorkerClient::new(env!("CARGO_BIN_EXE_ove"), &["model-worker"])
        .env("OVE_ONNXRUNTIME_LIB", "")
        .env("OVE_MODELS_DIR", &store);
    let model = std::env::temp_dir().join("no-such-model.onnx");
    let e = w.run(&model.display().to_string(), &[Tensor::f32("x", &[1], vec![0.0])]).unwrap_err();
    assert!(e.to_string().contains("could not load the model"), "{e}");
}

fn kill(pid: u32) {
    let status = if cfg!(windows) {
        std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output().unwrap().status
    } else {
        std::process::Command::new("kill").args(["-9", &pid.to_string()]).output().unwrap().status
    };
    assert!(status.success(), "could not kill the worker {pid}");
}

fn wait_until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < limit {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    f()
}

#[test]
fn the_worker_exits_when_idle_and_the_next_job_starts_a_new_one() {
    let c = client(1);
    let input = vec![Tensor::f32("x", &[3], vec![1.0, 2.0, 3.0])];
    assert_eq!(c.run("builtin:echo", &input).unwrap(), input);
    let first = c.pid();
    assert!(c.is_running());
    // Idle for longer than --idle-secs: the process has exited by itself.
    assert!(wait_until(Duration::from_secs(5), || !c.is_running()), "the worker is still running after being idle");
    assert_eq!(c.run("builtin:echo", &input).unwrap(), input);
    assert_ne!(c.pid(), first, "a new worker served the next job");
}

#[test]
fn a_killed_worker_fails_its_job_and_the_next_job_succeeds() {
    let c = Arc::new(client(120));
    // Start the worker, then give it a long job.
    c.run("builtin:echo", &[]).unwrap();
    let pid = c.pid();
    let c2 = c.clone();
    let job = std::thread::spawn(move || c2.run("builtin:sleep", &[Tensor::f32("ms", &[1], vec![10_000.0])]));
    std::thread::sleep(Duration::from_millis(300));
    let t0 = Instant::now();
    kill(pid);
    let result = job.join().unwrap();
    let took = t0.elapsed();
    match result {
        Err(ModelError::Inference(msg)) => assert!(msg.contains("stopped during the job"), "{msg}"),
        other => panic!("expected a failed job, got {other:?}"),
    }
    assert!(took < Duration::from_secs(3), "the job failed {took:?} after the kill");
    // The editor side keeps working: the next job starts a new worker.
    let input = vec![Tensor::i64("n", &[2], vec![4, 5])];
    assert_eq!(c.run("builtin:echo", &input).unwrap(), input);
    assert_ne!(c.pid(), pid);
}

#[test]
fn a_failed_job_keeps_the_worker() {
    let c = client(120);
    c.run("builtin:echo", &[]).unwrap();
    let pid = c.pid();
    // No runtime library is configured: an ONNX model cannot load, and the worker says so.
    match c.run("does-not-exist.onnx", &[Tensor::f32("x", &[1], vec![0.0])]) {
        Err(ModelError::JobFailed(msg)) => assert!(msg.contains("runtime"), "{msg}"),
        other => panic!("expected JobFailed, got {other:?}"),
    }
    c.run("builtin:echo", &[]).unwrap();
    assert_eq!(c.pid(), pid, "the same worker served the next job");
    // Inputs whose shape and data disagree never reach the worker.
    assert!(c.run("builtin:echo", &[Tensor::f32("x", &[2, 2], vec![0.0; 3])]).is_err());
}

#[test]
fn image_tensor_round_trip_is_fast() {
    let c = client(120);
    let n = 256 * 256 * 3;
    let input = vec![Tensor::f32("image", &[1, 256, 256, 3], (0..n).map(|i| i as f32 / n as f32).collect())];
    for _ in 0..5 {
        assert_eq!(c.run("builtin:echo", &input).unwrap(), input);
    }
    let mut times: Vec<f64> = (0..50)
        .map(|_| {
            let t0 = Instant::now();
            let out = c.run("builtin:echo", &input).unwrap();
            let dt = t0.elapsed().as_secs_f64() * 1e3;
            assert_eq!(out[0].shape, vec![1, 256, 256, 3]);
            dt
        })
        .collect();
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    eprintln!(
        "256x256x3 f32 round trip: median {median:.3} ms, min {:.3} ms, max {:.3} ms",
        times[0],
        times[times.len() - 1]
    );
    assert!(median <= 2.0, "median round trip {median:.3} ms");
}

fn read_f32(path: &std::path::Path) -> Vec<f32> {
    std::fs::read(path).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

/// Runs a session over the spike fixtures and returns the worst difference from the reference outputs per model.
fn spike_parity(dir: &std::path::Path, run: &mut dyn FnMut(&str, &[Tensor]) -> Vec<Tensor>) -> (f32, f32) {
    let vad_model = dir.join("silero_vad_16k.onnx").display().to_string();
    let mut vad_worst = 0f32;
    for fx in ["speech", "speech_noise", "music"] {
        let rows = read_f32(&dir.join(format!("vad_{fx}.in.f32")));
        let reference = read_f32(&dir.join(format!("vad_{fx}.ref.f32")));
        let mut state = vec![0f32; 256];
        let mut probs = Vec::new();
        for row in rows.chunks_exact(576) {
            let out = run(
                &vad_model,
                &[Tensor::f32("input", &[1, 576], row.to_vec()), Tensor::f32("state", &[2, 1, 128], state.clone())],
            );
            let get = |n: &str| out.iter().find(|t| t.name == n).unwrap().as_f32().unwrap().to_vec();
            probs.push(get("prob")[0]);
            state = get("state_out");
        }
        vad_worst = vad_worst.max(max_diff(&probs, &reference));
    }
    let beat_model = dir.join("beat_this_small0.onnx").display().to_string();
    let mut beat_worst = 0f32;
    for fx in ["drums100", "drums128", "speech"] {
        let spect = read_f32(&dir.join(format!("beat_{fx}.in.f32")));
        let reference = read_f32(&dir.join(format!("beat_{fx}.ref.f32")));
        let out = run(&beat_model, &[Tensor::f32("spect", &[1, 1500, 128], spect)]);
        let mut got = out.iter().find(|t| t.name == "beat").unwrap().as_f32().unwrap().to_vec();
        got.extend_from_slice(out.iter().find(|t| t.name == "downbeat").unwrap().as_f32().unwrap());
        beat_worst = beat_worst.max(max_diff(&got, &reference));
    }
    (vad_worst, beat_worst)
}

/// The two spike models (tools/model-spike) through the adapter, in this process and in the worker, against the
/// PyTorch reference outputs. Needs `OVE_SPIKE_DIR` (the export folder) and `OVE_ONNXRUNTIME_LIB`.
#[test]
#[ignore = "needs the spike exports (OVE_SPIKE_DIR) and the ONNX Runtime library (OVE_ONNXRUNTIME_LIB)"]
fn spike_models_run_through_the_adapter_with_parity() {
    use ove_models::infer::Backend;
    let dir = std::path::PathBuf::from(std::env::var_os("OVE_SPIKE_DIR").expect("set OVE_SPIKE_DIR"));
    let lib = std::env::var("OVE_ONNXRUNTIME_LIB").expect("set OVE_ONNXRUNTIME_LIB");

    let backend = ove_models::onnx::OnnxBackend::load(std::path::Path::new(&lib)).unwrap();
    let mut sessions: std::collections::HashMap<String, Box<dyn ove_models::infer::Session>> = Default::default();
    let t0 = Instant::now();
    let (vad, beat) = spike_parity(&dir, &mut |model, inputs| {
        let s = sessions.entry(model.to_string()).or_insert_with(|| backend.load(std::path::Path::new(model)).unwrap());
        s.run(inputs).unwrap()
    });
    eprintln!("in process: silero {vad:.3e}, beat this {beat:.3e} ({:?})", t0.elapsed());
    assert!(vad < 1e-3 && beat < 1e-3);

    let worker = WorkerClient::new(env!("CARGO_BIN_EXE_ove"), &["model-worker"]).env("OVE_ONNXRUNTIME_LIB", &lib);
    let t0 = Instant::now();
    let (vad, beat) = spike_parity(&dir, &mut |model, inputs| worker.run(model, inputs).unwrap());
    eprintln!("in the worker: silero {vad:.3e}, beat this {beat:.3e} ({:?})", t0.elapsed());
    assert!(vad < 1e-3 && beat < 1e-3);
}
