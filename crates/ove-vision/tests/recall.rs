//! The recall harness (add-semantic-media-search task 3.1) on a small synthetic set: generated clips with known
//! content, indexed through the real sampler and model, and text queries labelled with the clip that answers them.
//! It reports recall@5, recall@10 and MRR for the shipped encoders and for the full-precision graphs.
//!
//! The real measurement needs a labelled set of the owner's own footage (at least 300 shots and 100 queries); this
//! set only proves the harness and catches gross regressions. Needs the model files (`OVE_SIGLIP_DIR`, as in
//! `tests/model.rs`, including `vision_model.onnx` and `text_model.onnx`), ONNX Runtime (`OVE_ONNXRUNTIME_LIB`) and
//! the ffmpeg CLI (`OVE_FIXTURE_FFMPEG`). `OVE_RECALL_SET` names a JSON file of a labelled set to use instead (see
//! `ove_vision::eval::Judged`; its media ids are file paths).
#![cfg(feature = "media")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ove_models::ModelError;
use ove_models::infer::{Backend, Session, Tensor};
use ove_vision::eval::{Judged, Metrics, metrics};
use ove_vision::index::{Index, index_path};
use ove_vision::indexer::{MediaKind, ensure_index};
use ove_vision::model::{Encoder, ModelFiles, Runner, SIGLIP2_TEXT, SIGLIP2_VISION, SiglipEncoder};
use ove_vision::rank::{Scoring, Source, rank};

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

/// Synthetic clips: a lavfi source (with noise where the picture would otherwise be flat) and the query it answers.
const CLIPS: [(&str, &str, &str); 12] = [
    ("bars", "smptehdbars=s=320x180:r=25", "vertical colour bars on a test card"),
    ("fractal", "mandelbrot=s=320x180:r=25", "a colourful fractal"),
    ("pattern", "testsrc2=s=320x180:r=25", "a test pattern with a moving counter"),
    ("dots", "life=s=320x180:r=25:mold=10:ratio=0.2:seed=3", "scattered dots on a black background"),
    ("red", "color=c=0xC02020:s=320x180:r=25,noise=alls=40:allf=t", "a red surface"),
    ("blue", "color=c=0x2040C0:s=320x180:r=25,noise=alls=40:allf=t", "a blue surface"),
    ("rings", "zoneplate=s=320x180:r=25:kx2=60:ky2=60", "concentric rings"),
    ("triangles", "sierpinski=s=320x180:r=25", "a fractal made of triangles"),
    ("gradient", "gradients=s=320x180:r=25:speed=0.01", "a smooth colour gradient"),
    ("stripes", "rgbtestsrc=s=320x180:r=25", "red, green and blue stripes"),
    ("yellow", "color=c=0xE0C020:s=320x180:r=25,noise=alls=40:allf=t", "a yellow surface"),
    ("noise", "cellauto=s=320x180:r=25:rule=30", "black and white noise"),
];

fn ffmpeg() -> String {
    std::env::var("OVE_FIXTURE_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

fn make_clips(dir: &Path) -> Vec<(String, PathBuf)> {
    CLIPS
        .iter()
        .map(|(name, src, _)| {
            let out = dir.join(format!("{name}.mp4"));
            let st = Command::new(ffmpeg())
                .args(["-loglevel", "error", "-y", "-f", "lavfi", "-t", "3", "-i", src])
                .args(["-pix_fmt", "yuv420p", "-c:v", "libx264", "-crf", "20"])
                .arg(&out)
                .status()
                .unwrap();
            assert!(st.success(), "ffmpeg failed for {name}");
            (name.to_string(), out)
        })
        .collect()
}

fn files(vision: &str, text: &str) -> ModelFiles {
    let d = PathBuf::from(std::env::var_os("OVE_SIGLIP_DIR").expect("set OVE_SIGLIP_DIR"));
    ModelFiles {
        vision: d.join(vision),
        text: d.join(text),
        tokenizer: d.join("tokenizer.model"),
        vision_sha256: format!("{vision}-{text}"),
        text_sha256: text.to_string(),
    }
}

/// Indexes every clip with `enc` into its own bundle and evaluates the judged queries.
fn evaluate(
    bundle: &Path,
    clips: &[(String, PathBuf)],
    judged: &[Judged],
    enc: &mut dyn Encoder,
) -> (Metrics, Vec<String>) {
    for (name, path) in clips {
        ensure_index(bundle, path, name, MediaKind::Video, enc, &mut |_| true).unwrap();
    }
    let indexes: Vec<(String, Index)> =
        clips.iter().map(|(n, _)| (n.clone(), Index::open(&index_path(bundle, n)).unwrap())).collect();
    let sources: Vec<Source> = indexes.iter().map(|(n, i)| Source { media_id: n, index: i }).collect();
    let mut ranked = Vec::new();
    let mut lines = Vec::new();
    // "More like this": each clip's own frame finds its own shot first.
    for (name, index) in &indexes {
        let Some(q) = ove_vision::rank::frame_query(index, 1.5) else { continue };
        let r = rank(&sources, &q, Scoring::Similar { min: 0.0 }, 3);
        assert_eq!(&r[0].media_id, name, "more like this from {name}");
        lines.push(format!(
            "like {name}: {}",
            r.iter().map(|m| format!("{} {:.2}", m.media_id, m.cosine)).collect::<Vec<_>>().join(", ")
        ));
    }
    for j in judged {
        let q = enc.text(&j.query).unwrap();
        let r = rank(&sources, &q, Scoring::Text { min: 0.0 }, 10);
        lines.push(format!(
            "{:?}: {}",
            j.query,
            r.iter().take(3).map(|m| format!("{} {:.2}", m.media_id, m.confidence)).collect::<Vec<_>>().join(", ")
        ));
        ranked.push(r.into_iter().map(|m| (m.media_id, m.shot)).collect());
    }
    (metrics(judged, &ranked), lines)
}

#[test]
#[ignore = "needs the model files (OVE_SIGLIP_DIR), ONNX Runtime (OVE_ONNXRUNTIME_LIB) and the ffmpeg CLI"]
fn recall_on_the_synthetic_set() {
    let lib = std::env::var("OVE_ONNXRUNTIME_LIB").expect("set OVE_ONNXRUNTIME_LIB");
    let runner = InProcess {
        backend: ove_models::onnx::OnnxBackend::load(Path::new(&lib)).unwrap(),
        sessions: RefCell::new(HashMap::new()),
    };
    let dir = tempfile::tempdir().unwrap();
    let (clips, judged): (Vec<(String, PathBuf)>, Vec<Judged>) = match std::env::var_os("OVE_RECALL_SET") {
        Some(p) => {
            let judged: Vec<Judged> = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
            let mut clips: Vec<(String, PathBuf)> = Vec::new();
            for j in &judged {
                for (m, _) in &j.relevant {
                    if !clips.iter().any(|(n, _)| n == m) {
                        clips.push((m.clone(), PathBuf::from(m)));
                    }
                }
            }
            (clips, judged)
        }
        None => {
            let clips = make_clips(dir.path());
            let judged = CLIPS
                .iter()
                .map(|(name, _, q)| Judged { query: q.to_string(), relevant: vec![(name.to_string(), 0)] })
                .collect();
            (clips, judged)
        }
    };
    let mut results = Vec::new();
    for (label, vision, text) in
        [("shipped", SIGLIP2_VISION, SIGLIP2_TEXT), ("full precision", "vision_model.onnx", "text_model.onnx")]
    {
        let mut enc = SiglipEncoder::new(&runner, files(vision, text)).unwrap();
        let bundle = dir.path().join(format!("{}.ove", label.replace(' ', "-")));
        let (m, lines) = evaluate(&bundle, &clips, &judged, &mut enc);
        eprintln!(
            "{label}: {} queries, recall@5 {:.3}, recall@10 {:.3}, MRR {:.3}",
            m.queries, m.recall_at_5, m.recall_at_10, m.mrr
        );
        for l in lines {
            eprintln!("  {l}");
        }
        results.push(m);
    }
    let (shipped, full) = (results[0], results[1]);
    // Gross-regression floor on this easy set, and the shipped model within 2 points of full precision.
    assert!(shipped.mrr >= 0.6, "{shipped:?}");
    assert!(full.recall_at_5 - shipped.recall_at_5 <= 0.02 + 1e-9, "{shipped:?} vs {full:?}");
}
