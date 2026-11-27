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
        text: d.join(SIGLIP2_TEXT),
        tokenizer: d.join("tokenizer.model"),
        vision_sha256: String::new(),
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
