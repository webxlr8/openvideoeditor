//! Our pipeline against the original framework (add-semantic-media-search tasks 1.2 and 1.3). The golden file
//! `tests/reference/reference.json` and its pictures are written by `tools/vision-export/siglip2_export.py` from the
//! released checkpoint loaded by the reference framework: sampled preprocessed tensors, token ids and embeddings.
//!
//! - Preprocessing runs in every test run (the pictures and samples are in the repository).
//! - Token ids need the tokenizer model, and embeddings the shipped encoders and ONNX Runtime: `OVE_SIGLIP_DIR` and
//!   `OVE_ONNXRUNTIME_LIB` as in `tests/model.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ove_models::ModelError;
use ove_models::infer::{Backend, Session, Tensor};
use ove_vision::index::dot;
use ove_vision::model::{Encoder, ModelFiles, Runner, SIGLIP2_TEXT, SIGLIP2_VISION, SiglipEncoder};
use ove_vision::preprocess::pixel_values;
use ove_vision::tokenizer::Tokenizer;
use serde_json::Value;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference")
}

fn golden() -> Value {
    serde_json::from_slice(&std::fs::read(dir().join("reference.json")).unwrap()).unwrap()
}

/// RGBA pixels of a fixture picture.
fn picture(name: &str) -> (Vec<u8>, usize, usize) {
    let dec =
        png::Decoder::new(std::io::BufReader::new(std::fs::File::open(dir().join(format!("{name}.png"))).unwrap()));
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        other => panic!("{name}: {other:?}"),
    };
    (rgba, info.width as usize, info.height as usize)
}

fn floats(v: &Value) -> Vec<f32> {
    v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect()
}

/// Task 1.3: our resize and normalisation give the reference framework's preprocessed tensors.
#[test]
fn preprocessing_matches_the_reference_framework() {
    let g = golden();
    let stride = g["stride"].as_u64().unwrap() as usize;
    let mut worst = 0f32;
    let mut worst_mean = 0f32;
    for p in g["pictures"].as_array().unwrap() {
        let name = p["name"].as_str().unwrap();
        let (rgba, w, h) = picture(name);
        let ours = pixel_values(&rgba, w, h);
        let want = floats(&p["pixelSamples"]);
        let got: Vec<f32> = ours.iter().step_by(stride).copied().collect();
        assert_eq!(got.len(), want.len(), "{name}");
        let max = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        let plane = ours.len() / 3;
        for (c, m) in floats(&p["pixelMean"]).iter().enumerate() {
            let mean = ours[c * plane..(c + 1) * plane].iter().sum::<f32>() / plane as f32;
            worst_mean = worst_mean.max((mean - m).abs());
        }
        eprintln!("{name}: max sampled difference {max:.4}");
        worst = worst.max(max);
    }
    eprintln!(
        "worst sampled difference {worst:.4} (one 8-bit level is {:.4}); worst channel mean {worst_mean:.5}",
        2.0 / 255.0
    );
    // Both sides round the resized picture to 8 bits; the filters may land one level apart.
    assert!(worst <= 2.0 * 2.0 / 255.0 + 1e-4, "{worst}");
    assert!(worst_mean <= 0.002, "{worst_mean}");
}

struct InProcess {
    backend: ove_models::onnx::OnnxBackend,
    sessions: RefCell<HashMap<PathBuf, Box<dyn Session>>>,
}

impl Runner for InProcess {
    fn run(&self, model: &Path, inputs: &[Tensor]) -> Result<Vec<Tensor>, ModelError> {
        let mut s = self.sessions.borrow_mut();
        if !s.contains_key(model) {
            s.insert(model.to_path_buf(), self.backend.load(model)?);
        }
        s.get_mut(model).unwrap().run(inputs)
    }
}

/// Tasks 1.2 and 1.3: token ids from the reference framework's tokenizer, and the shipped encoders run on our
/// preprocessing and our token ids against the reference framework's embeddings (cosine ≥ 0.99).
#[test]
#[ignore = "needs the model files (OVE_SIGLIP_DIR) and ONNX Runtime (OVE_ONNXRUNTIME_LIB)"]
fn our_pipeline_matches_the_reference_embeddings() {
    let g = golden();
    let models = PathBuf::from(std::env::var_os("OVE_SIGLIP_DIR").expect("set OVE_SIGLIP_DIR"));
    let tok = Tokenizer::from_file(&models.join("tokenizer.model")).unwrap();
    for t in g["texts"].as_array().unwrap() {
        let want: Vec<i64> = t["ids"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        assert_eq!(tok.encode_for_model(t["text"].as_str().unwrap()), want, "{}", t["text"]);
    }
    let lib = std::env::var("OVE_ONNXRUNTIME_LIB").expect("set OVE_ONNXRUNTIME_LIB");
    let runner = InProcess {
        backend: ove_models::onnx::OnnxBackend::load(Path::new(&lib)).unwrap(),
        sessions: RefCell::new(HashMap::new()),
    };
    let vision = std::env::var("OVE_SIGLIP_VISION").unwrap_or_else(|_| SIGLIP2_VISION.into());
    let text = std::env::var("OVE_SIGLIP_TEXT").unwrap_or_else(|_| SIGLIP2_TEXT.into());
    let files = ModelFiles {
        vision: models.join(&vision),
        text: models.join(&text),
        tokenizer: models.join("tokenizer.model"),
        vision_sha256: String::new(),
    };
    let mut enc = SiglipEncoder::new(&runner, files).unwrap();
    let mut worst_image = 1f32;
    for p in g["pictures"].as_array().unwrap() {
        let (rgba, w, h) = picture(p["name"].as_str().unwrap());
        let e = enc.images(&[pixel_values(&rgba, w, h)]).unwrap().pop().unwrap();
        let c = dot(&e, &floats(&p["embedding"]));
        eprintln!("picture {}: cosine {c:.4}", p["name"]);
        worst_image = worst_image.min(c);
    }
    let mut worst_text = 1f32;
    for t in g["texts"].as_array().unwrap() {
        let e = enc.text(t["text"].as_str().unwrap()).unwrap();
        let c = dot(&e, &floats(&t["embedding"]));
        eprintln!("text {:?}: cosine {c:.4}", t["text"].as_str().unwrap().chars().take(30).collect::<String>());
        worst_text = worst_text.min(c);
    }
    eprintln!("{vision} + {text} against the reference framework: worst image {worst_image:.4}, text {worst_text:.4}");
    assert!(worst_image >= 0.99 && worst_text >= 0.99, "image {worst_image}, text {worst_text}");
}
