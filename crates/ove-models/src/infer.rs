//! The inference adapter: one way for every feature to run a model, whatever runs it.
//!
//! A [`Backend`] loads a model file into a [`Session`]; a session takes named tensors and returns named tensors.
//! Models flagged in-process (10 MB or less) run in the calling process; the others run in the model worker
//! (`ove model-worker`, see [`crate::worker`]), so the editor keeps its memory budget and survives a model crash.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{ModelError, Result};
use crate::manifest::Manifest;

/// Element type of a tensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DType {
    F32,
    I64,
}

/// The elements of a tensor, row-major.
#[derive(Debug, Clone, PartialEq)]
pub enum TensorData {
    F32(Vec<f32>),
    I64(Vec<i64>),
}

impl TensorData {
    pub fn dtype(&self) -> DType {
        match self {
            TensorData::F32(_) => DType::F32,
            TensorData::I64(_) => DType::I64,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            TensorData::F32(v) => v.len(),
            TensorData::I64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Little-endian bytes, as the worker protocol carries them.
    pub fn to_le_bytes(&self) -> Vec<u8> {
        match self {
            TensorData::F32(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            TensorData::I64(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        }
    }

    pub fn from_le_bytes(dtype: DType, b: &[u8]) -> Result<TensorData> {
        let bad = || ModelError::Inference(format!("tensor bytes ({}) do not fit the element type", b.len()));
        Ok(match dtype {
            DType::F32 => {
                if !b.len().is_multiple_of(4) {
                    return Err(bad());
                }
                TensorData::F32(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
            }
            DType::I64 => {
                if !b.len().is_multiple_of(8) {
                    return Err(bad());
                }
                TensorData::I64(
                    b.chunks_exact(8)
                        .map(|c| i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
                        .collect(),
                )
            }
        })
    }
}

/// The elements as their in-memory bytes. Both shipped platforms are little-endian, which is the wire format, so
/// the worker protocol moves tensors without converting each element.
#[cfg(target_endian = "little")]
pub(crate) fn data_bytes(d: &TensorData) -> &[u8] {
    match d {
        // SAFETY: f32 and i64 have no padding and every byte of them is initialised, so viewing the slice's memory
        // as bytes (same length in bytes, alignment 1) is sound.
        TensorData::F32(v) => unsafe {
            std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v.as_slice()))
        },
        // SAFETY: as above.
        TensorData::I64(v) => unsafe {
            std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v.as_slice()))
        },
    }
}

/// Reads `bytes` bytes of `dtype` elements straight into a new buffer (little-endian targets: no per-element copy).
#[cfg(target_endian = "little")]
pub(crate) fn read_data(r: &mut impl std::io::Read, dtype: DType, bytes: usize) -> Result<TensorData> {
    let size = match dtype {
        DType::F32 => 4,
        DType::I64 => 8,
    };
    if !bytes.is_multiple_of(size) {
        return Err(ModelError::Inference(format!("tensor bytes ({bytes}) do not fit the element type")));
    }
    let n = bytes / size;
    let io = |e: std::io::Error| ModelError::Inference(format!("model worker protocol: {e}"));
    Ok(match dtype {
        DType::F32 => {
            let mut v = vec![0f32; n];
            // SAFETY: every bit pattern is a valid f32; the byte view covers exactly the vector's initialised memory.
            let b = unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr().cast::<u8>(), bytes) };
            r.read_exact(b).map_err(io)?;
            TensorData::F32(v)
        }
        DType::I64 => {
            let mut v = vec![0i64; n];
            // SAFETY: every bit pattern is a valid i64; the byte view covers exactly the vector's initialised memory.
            let b = unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr().cast::<u8>(), bytes) };
            r.read_exact(b).map_err(io)?;
            TensorData::I64(v)
        }
    })
}

/// A named tensor.
#[derive(Debug, Clone, PartialEq)]
pub struct Tensor {
    pub name: String,
    pub shape: Vec<usize>,
    pub data: TensorData,
}

impl Tensor {
    pub fn f32(name: impl Into<String>, shape: &[usize], data: Vec<f32>) -> Tensor {
        Tensor { name: name.into(), shape: shape.to_vec(), data: TensorData::F32(data) }
    }

    pub fn i64(name: impl Into<String>, shape: &[usize], data: Vec<i64>) -> Tensor {
        Tensor { name: name.into(), shape: shape.to_vec(), data: TensorData::I64(data) }
    }

    /// Checks that the shape and the element count agree.
    pub fn check(&self) -> Result<()> {
        let n: usize = self.shape.iter().product();
        if n != self.data.len() {
            return Err(ModelError::Inference(format!(
                "tensor {} has shape {:?} ({n} elements) but {} elements",
                self.name,
                self.shape,
                self.data.len()
            )));
        }
        Ok(())
    }

    pub fn as_f32(&self) -> Option<&[f32]> {
        match &self.data {
            TensorData::F32(v) => Some(v),
            TensorData::I64(_) => None,
        }
    }
}

/// A loaded model.
pub trait Session: Send {
    fn run(&mut self, inputs: &[Tensor]) -> Result<Vec<Tensor>>;
}

/// Something that can load model files.
pub trait Backend: Send + Sync {
    fn name(&self) -> &'static str;
    fn load(&self, model: &Path) -> Result<Box<dyn Session>>;
}

/// Built-in diagnostic models, used by the tests and to check that the worker answers: `builtin:echo` returns its
/// inputs; `builtin:sleep` sleeps for the milliseconds in its first input's first element, then returns its inputs.
pub fn builtin(model: &str) -> Option<Box<dyn Session>> {
    match model {
        "builtin:echo" => Some(Box::new(Echo { sleep: false })),
        "builtin:sleep" => Some(Box::new(Echo { sleep: true })),
        _ => None,
    }
}

struct Echo {
    sleep: bool,
}

impl Session for Echo {
    fn run(&mut self, inputs: &[Tensor]) -> Result<Vec<Tensor>> {
        if self.sleep {
            let ms = match inputs.first().map(|t| &t.data) {
                Some(TensorData::F32(v)) => v.first().map(|x| *x as u64),
                Some(TensorData::I64(v)) => v.first().map(|x| *x as u64),
                None => None,
            };
            std::thread::sleep(Duration::from_millis(ms.unwrap_or(0)));
        }
        Ok(inputs.to_vec())
    }
}

/// Where a model should run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// In this process (small models).
    InProcess,
    /// In the model worker.
    Worker,
}

pub fn placement(m: &Manifest) -> Placement {
    if m.in_process && m.total_bytes() <= crate::manifest::IN_PROCESS_MAX_BYTES {
        Placement::InProcess
    } else {
        Placement::Worker
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tensor_bytes_round_trip() {
        let t = TensorData::F32(vec![1.5, -2.0, f32::MAX]);
        assert_eq!(TensorData::from_le_bytes(DType::F32, &t.to_le_bytes()).unwrap(), t);
        let t = TensorData::I64(vec![1, -2, i64::MIN]);
        assert_eq!(TensorData::from_le_bytes(DType::I64, &t.to_le_bytes()).unwrap(), t);
        assert!(TensorData::from_le_bytes(DType::F32, &[0, 1, 2]).is_err());
    }

    #[test]
    fn shape_must_match_the_data() {
        assert!(Tensor::f32("x", &[2, 3], vec![0.0; 6]).check().is_ok());
        assert!(Tensor::f32("x", &[2, 3], vec![0.0; 5]).check().is_err());
    }

    #[test]
    fn echo_returns_its_inputs() {
        let mut s = builtin("builtin:echo").unwrap();
        let t = vec![Tensor::f32("a", &[2], vec![1.0, 2.0])];
        assert_eq!(s.run(&t).unwrap(), t);
        assert!(builtin("other").is_none());
    }
}
