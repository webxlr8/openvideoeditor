//! `ove model-worker`: the out-of-process model runner the app starts on demand (protocol in
//! `ove_models::worker`). It loads ONNX Runtime from `OVE_ONNXRUNTIME_LIB` when set, serves jobs on stdin/stdout one
//! at a time, and exits after `--idle-secs` (default 120) without work.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use ove_models::infer::Backend;
use ove_models::onnx::OnnxBackend;

pub fn main(idle_secs: u64) -> ExitCode {
    // A runtime that fails to load is reported per job ("runtime not installed"); built-in models still work.
    let backend = std::env::var_os("OVE_ONNXRUNTIME_LIB")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map(|p| OnnxBackend::load(&p));
    let backend: Option<&dyn Backend> = match &backend {
        Some(Ok(b)) => Some(b),
        Some(Err(e)) => {
            eprintln!("model worker: {e}");
            None
        }
        None => None,
    };
    match ove_models::worker::serve(std::io::stdin(), std::io::stdout().lock(), Duration::from_secs(idle_secs), backend)
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("model worker: {e}");
            ExitCode::from(1)
        }
    }
}
