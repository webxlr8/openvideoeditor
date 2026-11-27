//! The inference runtime library (ONNX Runtime): where it is, and the "runtime not installed" answer when it is not.
//!
//! The library is installed into the model store like a model (id [`RUNTIME_ID`], from the official wheel, see
//! [`crate::manifest::runtime_manifest`]), with its licence and third-party notices next to it. It is native code
//! from a third-party host, so it downloads only after the person agrees (see [`crate::store::needs_consent`]).
//! `OVE_ONNXRUNTIME_LIB` names a library file directly instead (development, tests, managed machines).

use std::path::PathBuf;

use crate::error::{ModelError, Result};
pub use crate::manifest::{RUNTIME_ID, RUNTIME_VERSION, RuntimePlatform};
use crate::store::{NotInstalled, Store};

/// The environment variable that names the runtime library file directly.
pub const LIBRARY_ENV: &str = "OVE_ONNXRUNTIME_LIB";

/// The library file named by `OVE_ONNXRUNTIME_LIB`, when set.
pub fn library_from_env() -> Option<PathBuf> {
    std::env::var_os(LIBRARY_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The installed runtime library: `OVE_ONNXRUNTIME_LIB` when set, else the library in `store`. When neither exists,
/// a `model.runtimeMissing` error that says "runtime not installed" with the download size, host and how to install.
pub fn library_path(store: &Store) -> Result<PathBuf> {
    if let Some(p) = library_from_env() {
        return Ok(p);
    }
    installed_library(store).ok_or_else(|| ModelError::RuntimeMissing(missing(store).message))
}

/// The library in `store`, when the runtime is installed there.
pub fn installed_library(store: &Store) -> Option<PathBuf> {
    store.installed_paths(RUNTIME_ID)?.into_iter().next()
}

/// The "runtime not installed" result for this platform.
pub fn missing(store: &Store) -> NotInstalled {
    match store.manifest(RUNTIME_ID) {
        Ok(m) => NotInstalled::for_manifest(m),
        Err(_) => NotInstalled {
            id: RUNTIME_ID.into(),
            purpose: "Inference runtime".into(),
            bytes: 0,
            download_bytes: 0,
            host: String::new(),
            install: String::new(),
            licence: String::new(),
            message: "runtime not installed: no inference runtime is available for this platform".into(),
        },
    }
}

/// True for the "runtime not installed" error.
pub fn is_missing(e: &ModelError) -> bool {
    matches!(e, ModelError::RuntimeMissing(_))
}
