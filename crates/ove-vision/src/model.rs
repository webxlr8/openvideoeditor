//! The image–text embedding model (design D1): SigLIP 2 base, patch 16, 256 px, 768-dimensional outputs, as two ONNX
//! encoders run through the shared inference runtime, out of process in the model worker.
//!
//! [`Encoder`] is what indexing and search use; [`SiglipEncoder`] is the real one. Its files come from the model
//! store (manifest [`ove_models::manifest::siglip2_manifest`]); when they or the runtime are missing, [`installed`]
//! reports "model not installed" with the download size and nothing is downloaded.

use std::path::{Path, PathBuf};

use ove_models::infer::Tensor;
use ove_models::{ModelError, NotInstalled, Store};

use crate::error::{Result, VisionError};
use crate::index::{DIM, Provenance, normalise};
use crate::preprocess::{IMAGE_SIZE, IMAGE_VALUES};
use crate::tokenizer::{TEXT_LENGTH, Tokenizer};

pub use ove_models::manifest::{SIGLIP2_ID, SIGLIP2_TEXT, SIGLIP2_TOKENIZER, SIGLIP2_VISION};

/// The model's name as the install offer shows it.
pub const MODEL_NAME: &str = "SigLIP 2 base (patch 16, 256 px)";

/// The model's learned similarity scale: `exp(logit_scale)` with `logit_scale = 4.7265129` from the released weights.
pub const LOGIT_SCALE: f32 = 112.901_18;
/// The model's learned similarity bias.
pub const LOGIT_BIAS: f32 = -16.771_803;

/// The preprocessing recorded in index files.
pub const PREPROCESSING: &str = "rgb-256x256-triangle-mean0.5-std0.5";

/// Images per encoder call.
pub const IMAGE_BATCH: usize = 8;

/// Calibrated match probability for a cosine similarity (sigmoid of the model's scaled, biased logit).
pub fn confidence(cosine: f32) -> f32 {
    1.0 / (1.0 + (-(LOGIT_SCALE * cosine + LOGIT_BIAS)).exp())
}

/// Something that turns images and text into unit-length embeddings of width [`DIM`].
pub trait Encoder {
    /// Embeds preprocessed images (each [`IMAGE_VALUES`] floats, see [`crate::preprocess::pixel_values`]).
    fn images(&mut self, pixels: &[Vec<f32>]) -> Result<Vec<Vec<f32>>>;
    /// Embeds a text query.
    fn text(&mut self, text: &str) -> Result<Vec<f32>>;
    /// What index files built with this encoder record.
    fn provenance(&self) -> Provenance;
}

/// Runs a model file on tensors: the model worker, or an in-process backend in tests.
pub trait Runner {
    fn run(&self, model: &Path, inputs: &[Tensor]) -> std::result::Result<Vec<Tensor>, ModelError>;
}

impl<R: Runner + ?Sized> Runner for &R {
    fn run(&self, model: &Path, inputs: &[Tensor]) -> std::result::Result<Vec<Tensor>, ModelError> {
        (**self).run(model, inputs)
    }
}

impl<R: Runner + ?Sized> Runner for std::sync::Arc<R> {
    fn run(&self, model: &Path, inputs: &[Tensor]) -> std::result::Result<Vec<Tensor>, ModelError> {
        (**self).run(model, inputs)
    }
}

impl Runner for ove_models::worker::WorkerClient {
    fn run(&self, model: &Path, inputs: &[Tensor]) -> std::result::Result<Vec<Tensor>, ModelError> {
        ove_models::worker::WorkerClient::run(self, &model.to_string_lossy(), inputs)
    }
}

/// The installed model's files.
#[derive(Debug, Clone)]
pub struct ModelFiles {
    pub vision: PathBuf,
    pub text: PathBuf,
    pub tokenizer: PathBuf,
    /// SHA-256 of the image encoder (from the manifest).
    pub vision_sha256: String,
}

/// The model's files when it is installed, else the "model not installed" answer (size, host, how to install).
/// Nothing is downloaded here.
pub fn installed(store: &Store) -> std::result::Result<ModelFiles, Box<NotInstalled>> {
    let manifest = match store.manifest(SIGLIP2_ID) {
        Ok(m) => m.clone(),
        Err(_) => ove_models::manifest::siglip2_manifest(),
    };
    let missing = || Box::new(NotInstalled::for_manifest(&manifest));
    if !store.is_installed(SIGLIP2_ID) {
        return Err(missing());
    }
    let file = |name: &str| store.file_path(SIGLIP2_ID, name).ok_or_else(missing);
    Ok(ModelFiles {
        vision: file(SIGLIP2_VISION)?,
        text: file(SIGLIP2_TEXT)?,
        tokenizer: file(SIGLIP2_TOKENIZER)?,
        vision_sha256: manifest.file(SIGLIP2_VISION).map(|f| f.sha256.clone()).unwrap_or_default(),
    })
}

/// The installed model run by `runner` (normally the shared model worker), or "model not installed". Nothing is
/// downloaded.
pub fn installed_encoder<R: Runner + Send + 'static>(store: &Store, runner: R) -> Result<Box<dyn Encoder + Send>> {
    let files = installed(store)
        .map_err(|n| VisionError::ModelNotInstalled { message: n.message.clone(), bytes: n.download_bytes })?;
    Ok(Box::new(SiglipEncoder::new(runner, files)?))
}

/// The provenance of indexes built by the model with these files.
pub fn provenance_of(files: &ModelFiles) -> Provenance {
    Provenance {
        model: SIGLIP2_ID.into(),
        model_hash: files.vision_sha256.clone(),
        sampler: crate::SAMPLER_VERSION,
        preprocessing: PREPROCESSING.into(),
    }
}

/// The SigLIP 2 encoders run through a [`Runner`].
pub struct SiglipEncoder<R: Runner> {
    runner: R,
    files: ModelFiles,
    tokenizer: Tokenizer,
}

impl<R: Runner> std::fmt::Debug for SiglipEncoder<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SiglipEncoder").field("files", &self.files).finish()
    }
}

fn model_err(e: ModelError) -> VisionError {
    match e {
        ModelError::RuntimeMissing(m) => VisionError::ModelNotInstalled { message: m, bytes: 0 },
        other => VisionError::Model(other.to_string()),
    }
}

/// The pooled embedding output of an encoder: `pooler_output` when the graph names it, else the first `[n, 768]`
/// output.
fn pooled(outputs: Vec<Tensor>, n: usize) -> Result<Vec<Vec<f32>>> {
    let pick = outputs
        .iter()
        .position(|t| t.name == "pooler_output")
        .or_else(|| outputs.iter().position(|t| t.shape == [n, DIM]))
        .ok_or_else(|| VisionError::Model("the encoder has no pooled output".into()))?;
    let t = &outputs[pick];
    let data = t.as_f32().ok_or_else(|| VisionError::Model("the pooled output is not f32".into()))?;
    if t.shape != [n, DIM] {
        return Err(VisionError::Model(format!("the pooled output has shape {:?}", t.shape)));
    }
    Ok(data
        .chunks_exact(DIM)
        .map(|c| {
            let mut v = c.to_vec();
            normalise(&mut v);
            v
        })
        .collect())
}

impl<R: Runner> SiglipEncoder<R> {
    pub fn new(runner: R, files: ModelFiles) -> Result<SiglipEncoder<R>> {
        let tokenizer = Tokenizer::from_file(&files.tokenizer)?;
        Ok(SiglipEncoder { runner, files, tokenizer })
    }

    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }
}

impl<R: Runner> Encoder for SiglipEncoder<R> {
    fn images(&mut self, pixels: &[Vec<f32>]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(pixels.len());
        for batch in pixels.chunks(IMAGE_BATCH) {
            let mut data = Vec::with_capacity(batch.len() * IMAGE_VALUES);
            for p in batch {
                if p.len() != IMAGE_VALUES {
                    return Err(VisionError::Model(format!("an image has {} values", p.len())));
                }
                data.extend_from_slice(p);
            }
            let input = Tensor::f32("pixel_values", &[batch.len(), 3, IMAGE_SIZE, IMAGE_SIZE], data);
            let outputs = self.runner.run(&self.files.vision, &[input]).map_err(model_err)?;
            out.extend(pooled(outputs, batch.len())?);
        }
        Ok(out)
    }

    fn text(&mut self, text: &str) -> Result<Vec<f32>> {
        let ids = self.tokenizer.encode_for_model(text);
        let input = Tensor::i64("input_ids", &[1, TEXT_LENGTH], ids);
        let outputs = self.runner.run(&self.files.text, &[input]).map_err(model_err)?;
        pooled(outputs, 1)?.pop().ok_or_else(|| VisionError::Model("no text embedding".into()))
    }

    fn provenance(&self) -> Provenance {
        provenance_of(&self.files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_is_calibrated() {
        // The logit is zero at cosine = -bias / scale (about 0.149): an even chance.
        let c0 = -LOGIT_BIAS / LOGIT_SCALE;
        assert!((confidence(c0) - 0.5).abs() < 1e-4);
        assert!(confidence(0.0) < 1e-6);
        assert!(confidence(0.3) > 0.99);
        assert!(confidence(0.13) > 0.1 && confidence(0.12) < 0.1);
    }

    #[test]
    fn a_missing_model_reports_its_size_and_installs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path());
        let n = installed(&store).unwrap_err();
        assert_eq!(n.id, SIGLIP2_ID);
        assert!(n.download_bytes > 300_000_000, "{}", n.download_bytes);
        assert!(n.message.contains("needs your consent"), "{}", n.message);
        // The consent text names the size people read, the model and its licence.
        assert!(n.message.contains("0.63 GB download"), "{}", n.message);
        assert!(n.message.contains("SigLIP 2 base") && n.message.contains("licence Apache-2.0"), "{}", n.message);
        assert_eq!(n.licence, "Apache-2.0");
        assert_eq!(std::fs::read_dir(dir.path()).map(|d| d.count()).unwrap_or(0), 0, "nothing was downloaded");
    }
}
