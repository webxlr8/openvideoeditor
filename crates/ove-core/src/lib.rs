//! Project document core: model, field registry, operations, invariants, revisions, undo/redo, migrations.
//! Pure: no filesystem or platform dependencies (persistence lives in `ove-store`).

pub mod diff;
pub mod invariants;
pub mod jsonschema;
pub mod migrate;
pub mod model;
pub mod ops;
pub mod revision;
pub mod schema;
#[cfg(feature = "testgen")]
pub mod testgen;
pub mod time;

pub use model::{Document, Item, MediaItem, RegistryEntry, Track};
pub use ops::{Applied, BatchError, Op, apply_batch};
pub use revision::{Clock, Prepared, RevisionRecord, Session, Source, SystemClock};
