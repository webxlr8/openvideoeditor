//! The pure-Rust backend (feature `tract`). Kept buildable until the runtime spike has its macOS row (add-model-runtime
//! task 1.2); the default backend is ONNX Runtime ([`crate::onnx`]). CPU only, one thread.

use std::path::Path;
use std::sync::Arc;

use tract_onnx::prelude::*;

use crate::error::{ModelError, Result};
use crate::infer::{Backend, Session, Tensor as OveTensor, TensorData};

/// tract, compiled into the binary.
#[derive(Debug, Clone, Copy, Default)]
pub struct TractBackend;

type Plan = Arc<TypedSimplePlan>;

impl Backend for TractBackend {
    fn name(&self) -> &'static str {
        "tract"
    }

    fn load(&self, model: &Path) -> Result<Box<dyn Session>> {
        let fail = |e: &dyn std::fmt::Display| {
            ModelError::JobFailed(format!("could not load the model {}: {e}", model.display()))
        };
        let graph = tract_onnx::onnx().model_for_path(model).map_err(|e| fail(&e))?;
        let names: Vec<String> =
            graph.output_outlets().map_err(|e| fail(&e))?.iter().map(|o| graph.node(o.node).name.clone()).collect();
        let plan = graph.into_optimized().map_err(|e| fail(&e))?.into_runnable().map_err(|e| fail(&e))?;
        Ok(Box::new(TractSession { plan, outputs: names }))
    }
}

struct TractSession {
    plan: Plan,
    outputs: Vec<String>,
}

fn job(what: &str, e: impl std::fmt::Display) -> ModelError {
    ModelError::JobFailed(format!("{what}: {e}"))
}

impl Session for TractSession {
    /// Inputs are taken in the model's input order (tract plans are positional).
    fn run(&mut self, inputs: &[OveTensor]) -> Result<Vec<OveTensor>> {
        let mut values: TVec<TValue> = TVec::new();
        for t in inputs {
            t.check()?;
            let v = match &t.data {
                TensorData::F32(d) => tract_onnx::prelude::Tensor::from_shape(&t.shape, d),
                TensorData::I64(d) => tract_onnx::prelude::Tensor::from_shape(&t.shape, d),
            }
            .map_err(|e| job(&format!("input {}", t.name), e))?;
            values.push(v.into());
        }
        let out = self.plan.run(values).map_err(|e| job("the model failed", e))?;
        let mut result = Vec::with_capacity(out.len());
        for (i, v) in out.iter().enumerate() {
            let name = self.outputs.get(i).cloned().unwrap_or_else(|| format!("output{i}"));
            let shape = v.shape().to_vec();
            let plain = v.try_as_plain_ram().map_err(|e| job(&name, e))?;
            let t = if let Ok(s) = plain.as_slice::<f32>() {
                OveTensor::f32(name, &shape, s.to_vec())
            } else if let Ok(s) = plain.as_slice::<i64>() {
                OveTensor::i64(name, &shape, s.to_vec())
            } else {
                return Err(ModelError::JobFailed(format!(
                    "output {name} has an unsupported type ({:?})",
                    v.datum_type()
                )));
            };
            result.push(t);
        }
        Ok(result)
    }
}
