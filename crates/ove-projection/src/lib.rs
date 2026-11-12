//! The timeline as text: a deterministic literal TSX projection of the project document, a parse-only checker,
//! a three-way merge that applies edited projections onto the live document, text-edit apply, verify lints and the
//! generated typings.
#![forbid(unsafe_code)]

pub mod apply;
pub mod format;
pub mod merge;
pub mod normal;
pub mod parse;
pub mod print;
pub mod read;
pub mod textedit;
pub mod typings;
pub mod verify;

pub use apply::{ApplyInput, BaseError, Plan, Refusal, plan};
pub use merge::Prefer;
pub use print::{PrintContext, print};
