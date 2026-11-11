//! Project bundles on disk: library, durable persistence (write-ahead revision log + snapshot), single-writer
//! locking, revision history and restore, media registry status.

pub mod bundle;
pub mod error;
pub mod fsio;
pub mod library;
pub mod lock;
pub mod media;
pub mod project;

pub use error::{Result, StoreError};
pub use library::{Library, Listing};
pub use project::{Committed, Mode, Project};
