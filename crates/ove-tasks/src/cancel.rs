//! Cooperative cancellation.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::model::Cancelled;

/// A shared cancel flag. Cheap to clone and to check; long loops check it per frame or per chunk.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        CancelToken::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// `Err(Cancelled)` once cancelled.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() { Err(Cancelled) } else { Ok(()) }
    }

    /// The flag itself, for code that takes `&AtomicBool` (the exporter, the transcriber).
    pub fn as_atomic(&self) -> &AtomicBool {
        &self.0
    }

    /// A token over an existing flag (a job's own cancel button keeps working).
    pub fn from_shared(flag: Arc<AtomicBool>) -> Self {
        CancelToken(flag)
    }

    /// The shared flag, for code that keeps an `Arc<AtomicBool>`.
    pub fn shared(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}
