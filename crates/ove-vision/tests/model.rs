//! The real model (add-semantic-media-search tasks 1.2 and 1.3): golden token ids from the reference tokenizer,
//! and the shipped encoders against the full-precision export.
//!
//! These need the model files, which are not in the repository (0.63 GB, downloaded only with consent):
//! `OVE_SIGLIP_DIR` names a folder with `tokenizer.model`, the shipped encoders (`vision_model_fp16.onnx`,
//! `text_model_q4f16.onnx`) and, for the parity test, the full-precision `vision_model.onnx` and `text_model.onnx` of
//! the same export (`OVE_SIGLIP_VISION` / `OVE_SIGLIP_TEXT` name other candidate files).
//! `OVE_ONNXRUNTIME_LIB` names the ONNX Runtime library.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code: a failure here is a failed test")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ove_models::ModelError;
use ove_models::infer::{Backend, Session, Tensor};
use ove_vision::index::dot;
use ove_vision::model::{Encoder, ModelFiles, Runner, SiglipEncoder, confidence};
use ove_vision::preprocess::pixel_values;
use ove_vision::tokenizer::{TEXT_LENGTH, Tokenizer};
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(std::env::var_os("OVE_SIGLIP_DIR").expect("set OVE_SIGLIP_DIR"))
}

/// Models run in this process through ONNX Runtime.
struct InProcess {
    backend: ove_models::onnx::OnnxBackend,
    sessions: RefCell<HashMap<PathBuf, Box<dyn Session>>>,
}

impl InProcess {
    fn new() -> InProcess {
        let lib = std::env::var("OVE_ONNXRUNTIME_LIB").expect("set OVE_ONNXRUNTIME_LIB");
        InProcess {
            backend: ove_models::onnx::OnnxBackend::load(Path::new(&lib)).unwrap(),
            sessions: RefCell::new(HashMap::new()),
        }
    }
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

fn files(vision: &str, text: &str) -> ModelFiles {
    let d = dir();
    ModelFiles {
        vision: d.join(vision),
        text: d.join(text),
        tokenizer: d.join("tokenizer.model"),
        vision_sha256: String::new(),
    }
}

/// Synthetic test pictures: colour fields, stripes, a checkerboard, gradients and a disc.
fn pictures() -> Vec<(String, Vec<u8>, usize, usize)> {
    let (w, h) = (320usize, 180usize);
    let make = |f: &dyn Fn(usize, usize) -> [u8; 3]| -> Vec<u8> {
        (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| (0..1).map(move |_| (x, y))))
            .flat_map(|(x, y)| {
                let c = f(x, y);
                [c[0], c[1], c[2], 255]
            })
            .collect()
    };
    vec![
        ("red".into(), make(&|_, _| [220, 20, 20]), w, h),
        ("blue".into(), make(&|_, _| [20, 40, 220]), w, h),
        ("green".into(), make(&|_, _| [30, 180, 40]), w, h),
        ("stripes".into(), make(&|x, _| if (x / 20) % 2 == 0 { [255, 255, 255] } else { [0, 0, 0] }), w, h),
        (
            "checkerboard".into(),
            make(&|x, y| if ((x / 30) + (y / 30)) % 2 == 0 { [250, 250, 250] } else { [10, 10, 10] }),
            w,
            h,
        ),
        ("gradient".into(), make(&|x, _| [(x * 255 / w) as u8, (x * 255 / w) as u8, (x * 255 / w) as u8]), w, h),
        (
            "disc".into(),
            make(&|x, y| {
                let (dx, dy) = (x as f64 - 160.0, y as f64 - 90.0);
                if dx * dx + dy * dy < 60.0 * 60.0 { [250, 210, 0] } else { [10, 10, 60] }
            }),
            w,
            h,
        ),
    ]
}

#[test]
#[ignore = "needs the tokenizer model (OVE_SIGLIP_DIR)"]
fn token_ids_match_the_reference_tokenizer() {
    let tok = Tokenizer::from_file(&dir().join("tokenizer.model")).unwrap();
    assert_eq!(tok.vocab_size(), 256_000);
    let golden: Vec<Value> =
        serde_json::from_str(include_str!("golden/tokens.json")).expect("golden/tokens.json is valid JSON");
    for g in &golden {
        let text = g["text"].as_str().unwrap();
        let want: Vec<i64> = g["ids"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        let got = tok.encode_for_model(text);
        assert_eq!(got.len(), TEXT_LENGTH);
        if want.contains(&1) {
            assert_eq!(got, want, "{text:?}");
        } else {
            // The reference cut this over-long text without its end marker; the model's own pipeline keeps it
            // (design.md Results): the first 63 ids agree and the last is the end marker.
            assert_eq!(got[..TEXT_LENGTH - 1], want[..TEXT_LENGTH - 1], "{text:?}");
            assert_eq!(got[TEXT_LENGTH - 1], 1);
        }
    }
    let t0 = std::time::Instant::now();
    for _ in 0..1000 {
        std::hint::black_box(tok.encode_for_model("boat in a harbour at dusk with people walking"));
    }
    eprintln!("tokenise: {:.1} µs per query", t0.elapsed().as_secs_f64() * 1e3);
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    dot(a, b) / (dot(a, a).sqrt() * dot(b, b).sqrt())
}

#[test]
#[ignore = "needs the model files (OVE_SIGLIP_DIR) and ONNX Runtime (OVE_ONNXRUNTIME_LIB)"]
fn shipped_encoders_agree_with_full_precision() {
    let runner = InProcess::new();
    let vision = std::env::var("OVE_SIGLIP_VISION").unwrap_or_else(|_| ove_vision::model::SIGLIP2_VISION.into());
    let text = std::env::var("OVE_SIGLIP_TEXT").unwrap_or_else(|_| ove_vision::model::SIGLIP2_TEXT.into());
    eprintln!("candidates: {vision}, {text}");
    let mut shipped = SiglipEncoder::new(&runner, files(&vision, &text)).unwrap();
    let mut full = SiglipEncoder::new(&runner, files("vision_model.onnx", "text_model.onnx")).unwrap();
    let pics = pictures();
    let pixels: Vec<Vec<f32>> = pics.iter().map(|(_, rgba, w, h)| pixel_values(rgba, *w, *h)).collect();
    let t0 = std::time::Instant::now();
    let a = shipped.images(&pixels).unwrap();
    let shipped_ms = t0.elapsed().as_secs_f64() * 1000.0 / pixels.len() as f64;
    let b = full.images(&pixels).unwrap();
    let mut worst = 1f32;
    for ((name, ..), (x, y)) in pics.iter().zip(a.iter().zip(&b)) {
        let c = cosine(x, y);
        eprintln!("image {name}: cosine shipped vs full {c:.4}");
        worst = worst.min(c);
    }
    let texts = ["a red image", "a photo of a cat", "boat in a harbour at dusk", "a black and white checkerboard"];
    let mut worst_text = 1f32;
    for t in texts {
        let c = cosine(&shipped.text(t).unwrap(), &full.text(t).unwrap());
        eprintln!("text {t:?}: cosine shipped vs full {c:.4}");
        worst_text = worst_text.min(c);
    }
    eprintln!("shipped image encoder: {shipped_ms:.0} ms per image (batch of {}, CPU)", pixels.len());
    eprintln!("worst cosine: images {worst:.4}, texts {worst_text:.4}");
    assert!(worst >= 0.99 && worst_text >= 0.99, "images {worst}, texts {worst_text}");
}

#[test]
#[ignore = "needs the model files (OVE_SIGLIP_DIR) and ONNX Runtime (OVE_ONNXRUNTIME_LIB)"]
fn text_queries_find_the_matching_picture() {
    let runner = InProcess::new();
    let mut enc =
        SiglipEncoder::new(&runner, files(ove_vision::model::SIGLIP2_VISION, ove_vision::model::SIGLIP2_TEXT)).unwrap();
    let pics = pictures();
    let pixels: Vec<Vec<f32>> = pics.iter().map(|(_, rgba, w, h)| pixel_values(rgba, *w, *h)).collect();
    let emb = enc.images(&pixels).unwrap();
    for (query, want) in [
        ("a solid red background", "red"),
        ("a solid blue background", "blue"),
        ("a green background", "green"),
        ("a yellow circle on a dark background", "disc"),
        ("a black and white checkerboard pattern", "checkerboard"),
    ] {
        let q = enc.text(query).unwrap();
        let mut ranked: Vec<(f32, &str)> = emb.iter().zip(&pics).map(|(e, (n, ..))| (dot(e, &q), n.as_str())).collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        eprintln!(
            "{query:?}: {}",
            ranked.iter().map(|(s, n)| format!("{n} {s:.3} ({:.2})", confidence(*s))).collect::<Vec<_>>().join(", ")
        );
        assert_eq!(ranked[0].1, want, "{query}");
    }
}
