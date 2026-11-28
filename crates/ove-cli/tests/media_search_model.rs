//! The visual search model through `ove model-worker` (add-semantic-media-search tasks 1.2 and 2.3): the worker path
//! gives the same embeddings as running the model in this process. Needs the model files (`OVE_SIGLIP_DIR` with
//! `tokenizer.model` and the shipped encoders) and the runtime library (`OVE_ONNXRUNTIME_LIB`).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code: a failure here is a failed test")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ove_models::infer::{Backend, Session, Tensor};
use ove_models::worker::WorkerClient;
use ove_vision::index::dot;
use ove_vision::model::{Encoder, ModelFiles, Runner, SIGLIP2_TEXT, SIGLIP2_VISION, SiglipEncoder};
use ove_vision::preprocess::pixel_values;

struct InProcess {
    backend: ove_models::onnx::OnnxBackend,
    sessions: RefCell<HashMap<PathBuf, Box<dyn Session>>>,
}

impl Runner for InProcess {
    fn run(&self, model: &Path, inputs: &[Tensor]) -> Result<Vec<Tensor>, ove_models::ModelError> {
        let mut s = self.sessions.borrow_mut();
        if !s.contains_key(model) {
            s.insert(model.to_path_buf(), self.backend.load(model)?);
        }
        s.get_mut(model).unwrap().run(inputs)
    }
}

fn files() -> ModelFiles {
    let d = PathBuf::from(std::env::var_os("OVE_SIGLIP_DIR").expect("set OVE_SIGLIP_DIR"));
    ModelFiles {
        vision: d.join(SIGLIP2_VISION),
        // `OVE_SIGLIP_TEXT` names another text encoder file (to compare candidates).
        text: d.join(std::env::var("OVE_SIGLIP_TEXT").unwrap_or_else(|_| SIGLIP2_TEXT.into())),
        tokenizer: d.join("tokenizer.model"),
        vision_sha256: String::new(),
        text_sha256: String::new(),
    }
}

fn picture(seed: u32) -> Vec<f32> {
    let (w, h) = (320usize, 180usize);
    let rgba: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = ((i % w) as u32, (i / w) as u32);
            let v = ((x * (seed + 3) + y * (seed + 7)) % 256) as u8;
            [v, v.wrapping_mul(3), 255 - v, 255]
        })
        .collect();
    pixel_values(&rgba, w, h)
}

#[test]
#[ignore = "needs the model files (OVE_SIGLIP_DIR) and ONNX Runtime (OVE_ONNXRUNTIME_LIB)"]
fn the_worker_gives_the_in_process_embeddings() {
    let lib = std::env::var("OVE_ONNXRUNTIME_LIB").expect("set OVE_ONNXRUNTIME_LIB");
    let local = InProcess {
        backend: ove_models::onnx::OnnxBackend::load(Path::new(&lib)).unwrap(),
        sessions: RefCell::new(HashMap::new()),
    };
    let worker = WorkerClient::new(env!("CARGO_BIN_EXE_ove"), &["model-worker"]).env("OVE_ONNXRUNTIME_LIB", &lib);
    let mut a = SiglipEncoder::new(&local, files()).unwrap();
    let mut b = SiglipEncoder::new(&worker, files()).unwrap();
    let pixels: Vec<Vec<f32>> = (0..10).map(picture).collect();
    let ea = a.images(&pixels).unwrap();
    let t0 = Instant::now();
    let eb = b.images(&pixels).unwrap();
    let first = t0.elapsed();
    let t0 = Instant::now();
    let eb2 = b.images(&pixels).unwrap();
    let warm = t0.elapsed();
    for (x, y) in ea.iter().zip(&eb) {
        assert!(dot(x, y) > 0.9999, "{}", dot(x, y));
    }
    assert_eq!(eb, eb2);
    let ta = a.text("a boat in a harbour").unwrap();
    let tb = b.text("a boat in a harbour").unwrap();
    assert!(dot(&ta, &tb) > 0.9999);
    eprintln!(
        "worker: 10 images in {first:?} with model load, {warm:?} warm ({:.0} ms per image)",
        warm.as_secs_f64() * 100.0
    );
}

/// The golden pictures and texts written from the original framework (`crates/ove-vision/tests/reference/`).
fn reference() -> (PathBuf, serde_json::Value) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ove-vision/tests/reference");
    let g = serde_json::from_slice(&std::fs::read(dir.join("reference.json")).unwrap()).unwrap();
    (dir, g)
}

fn golden_picture(dir: &Path, name: &str) -> Vec<f32> {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(dir.join(format!("{name}.png"))).unwrap()));
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf,
        _ => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
    };
    pixel_values(&rgba, info.width as usize, info.height as usize)
}

fn floats(v: &serde_json::Value) -> Vec<f32> {
    v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect()
}

/// Peak working set of a process, from the operating system (Windows), in bytes.
fn peak_bytes(pid: u32) -> Option<u64> {
    if !cfg!(windows) {
        return None;
    }
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!("(Get-Process -Id {pid}).PeakWorkingSet64")])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// The shipped encoders in the model worker against the original framework (cosine ≥ 0.99 on every golden picture
/// and text), the worker's peak memory with both encoders loaded, and its exit once idle.
#[test]
#[ignore = "needs the model files (OVE_SIGLIP_DIR) and ONNX Runtime (OVE_ONNXRUNTIME_LIB)"]
fn the_worker_matches_the_original_framework_and_exits_when_idle() {
    let lib = std::env::var("OVE_ONNXRUNTIME_LIB").expect("set OVE_ONNXRUNTIME_LIB");
    let idle = 4u64;
    let worker = WorkerClient::new(env!("CARGO_BIN_EXE_ove"), &["model-worker", "--idle-secs", &idle.to_string()])
        .env("OVE_ONNXRUNTIME_LIB", &lib);
    let mut enc = SiglipEncoder::new(&worker, files()).unwrap();
    let (dir, g) = reference();
    let mut worst_image = 1f32;
    for p in g["pictures"].as_array().unwrap() {
        let e = enc.images(&[golden_picture(&dir, p["name"].as_str().unwrap())]).unwrap().pop().unwrap();
        worst_image = worst_image.min(dot(&e, &floats(&p["embedding"])));
    }
    let mut worst_text = 1f32;
    for t in g["texts"].as_array().unwrap() {
        let e = enc.text(t["text"].as_str().unwrap()).unwrap();
        worst_text = worst_text.min(dot(&e, &floats(&t["embedding"])));
    }
    let pid = worker.pid();
    let peak = peak_bytes(pid);
    eprintln!(
        "worker (pid {pid}) against the original framework: worst image {worst_image:.4}, text {worst_text:.4}; peak working set {}",
        peak.map(|b| format!("{:.0} MB", b as f64 / 1e6)).unwrap_or_else(|| "not measured".into())
    );
    assert!(worst_image >= 0.99 && worst_text >= 0.99, "image {worst_image}, text {worst_text}");
    // Idle: the worker exits by itself after `idle` seconds without a job.
    assert!(worker.is_running());
    let t0 = Instant::now();
    while worker.is_running() && t0.elapsed() < std::time::Duration::from_secs(idle + 10) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(!worker.is_running(), "the worker is still running {:?} after its last job", t0.elapsed());
    eprintln!("the worker exited {:.1} s after its last job (idle limit {idle} s)", t0.elapsed().as_secs_f64());
}
