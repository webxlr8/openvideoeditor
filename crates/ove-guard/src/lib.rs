//! Small guards shared by the crates that talk to the network or turn names into paths, kept here so they depend on
//! one low-level crate instead of each other:
//!
//! - [`redact`]: a process-wide registry of secret values (API keys register themselves when loaded) and a writer
//!   that scrubs them, and anything that looks like an API-key or `Authorization` header, from log output.
//! - [`endpoint`]: when an endpoint override from the environment (`OVE_ANTHROPIC_BASE_URL`, `OVE_FAL_QUEUE_URL`, …)
//!   is honoured, so release builds cannot be pointed at another host to collect keys.
//! - [`id`]: the one check for ids that become file names (chat logs, job files, cancel markers).

pub mod endpoint;
pub mod id;
pub mod redact;

pub use id::is_safe_id;
pub use redact::{REDACTED, RedactingWriter, redact, register_secret};
