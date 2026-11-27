//! The ONNX backend: ONNX Runtime loaded at run time from a library file (design D4: the library downloads on first
//! use like a model, so the installer does not grow). CPU only.

use std::path::Path;
use std::sync::OnceLock;

use ort::session::Session as OrtSession;
use ort::value::{DynValue, Tensor as OrtTensor, TensorElementType};

use crate::error::{ModelError, Result};
use crate::infer::{Backend, Session, Tensor, TensorData};

/// The process-wide ONNX Runtime environment. The library can be loaded once per process.
static ENV: OnceLock<std::result::Result<ort::environment::Environment, String>> = OnceLock::new();

/// ONNX Runtime from a library file.
#[derive(Debug, Clone, Copy)]
pub struct OnnxBackend {
    threads: usize,
}

impl OnnxBackend {
    /// Loads the ONNX Runtime library at `library` (once per process; later calls reuse the first library).
    pub fn load(library: &Path) -> Result<OnnxBackend> {
        let env = ENV.get_or_init(|| {
            ort::init_from(library)
                .map_err(|e| e.to_string())
                .and_then(|b| b.with_name("ove").build().map_err(|e| e.to_string()))
        });
        match env {
            Ok(_) => Ok(OnnxBackend { threads: default_threads() }),
            Err(e) => Err(ModelError::RuntimeMissing(format!(
                "could not load the inference runtime {}: {e}",
                library.display()
            ))),
        }
    }

    /// Threads for one model (default: up to 4).
    pub fn with_threads(mut self, threads: usize) -> OnnxBackend {
        self.threads = threads.max(1);
        self
    }
}

fn default_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get().min(4)).unwrap_or(1)
}

fn ort_err(what: &str, e: impl std::fmt::Display) -> ModelError {
    ModelError::JobFailed(format!("{what}: {e}"))
}

impl Backend for OnnxBackend {
    fn name(&self) -> &'static str {
        "onnxruntime"
    }

    fn load(&self, model: &Path) -> Result<Box<dyn Session>> {
        let Some(Ok(env)) = ENV.get() else {
            return Err(ModelError::RuntimeMissing("the inference runtime is not loaded".into()));
        };
        let fail = |e: &dyn std::fmt::Display| {
            ModelError::JobFailed(format!("could not load the model {}: {e}", model.display()))
        };
        let builder = OrtSession::builder(env).map_err(|e| fail(&e))?;
        let mut builder = builder.with_intra_threads(self.threads).map_err(|e| fail(&e))?;
        let session = builder.commit_from_file(model).map_err(|e| fail(&e))?;
        Ok(Box::new(OnnxSession { session }))
    }
}

struct OnnxSession {
    session: OrtSession,
}

impl Session for OnnxSession {
    fn run(&mut self, inputs: &[Tensor]) -> Result<Vec<Tensor>> {
        let mut values: Vec<(String, DynValue)> = Vec::with_capacity(inputs.len());
        for t in inputs {
            t.check()?;
            let shape: Vec<i64> = t.shape.iter().map(|&d| d as i64).collect();
            let v = match &t.data {
                TensorData::F32(d) => OrtTensor::from_array((shape, d.clone())).map(|x| x.into_dyn()),
                TensorData::I64(d) => OrtTensor::from_array((shape, d.clone())).map(|x| x.into_dyn()),
            }
            .map_err(|e| ort_err(&format!("input {}", t.name), e))?;
            values.push((t.name.clone(), v));
        }
        let outputs = self.session.run(values).map_err(|e| ort_err("the model failed", e))?;
        let mut out = Vec::with_capacity(outputs.len());
        for (name, value) in outputs.iter() {
            let ty = value.dtype().tensor_type();
            let tensor = match ty {
                Some(TensorElementType::Float32) => {
                    let (shape, data) = value.try_extract_tensor::<f32>().map_err(|e| ort_err(name, e))?;
                    Tensor::f32(name, &dims(shape), data.to_vec())
                }
                Some(TensorElementType::Int64) => {
                    let (shape, data) = value.try_extract_tensor::<i64>().map_err(|e| ort_err(name, e))?;
                    Tensor::i64(name, &dims(shape), data.to_vec())
                }
                other => {
                    return Err(ModelError::JobFailed(format!("output {name} has an unsupported type ({other:?})")));
                }
            };
            out.push(tensor);
        }
        Ok(out)
    }
}

fn dims(shape: &ort::value::Shape) -> Vec<usize> {
    shape.iter().map(|&d| d.max(0) as usize).collect()
}
