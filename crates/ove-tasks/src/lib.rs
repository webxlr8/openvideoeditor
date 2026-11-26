//! Background task scheduler shared by the desktop app and the CLI.
//!
//! A task declares a priority (export, interactive, derivative, analysis), a resource class (decode, inference,
//! cpu, io) and its estimated peak memory. The scheduler starts tasks in priority order within per-class limits and a
//! memory budget, ages waiting tasks so none starve, runs them at the operating system's background priority, pauses
//! derivative and analysis work during playback, export, battery power, low-power mode and thermal pressure, and
//! reports progress at most four times per second. See `docs/cli/tasks.md` for feature authors.

pub mod cancel;
pub mod model;
pub mod os;
pub mod output;
pub mod queue;
pub mod scheduler;

pub use cancel::CancelToken;
pub use model::{
    Cancelled, Class, PauseReason, Priority, TaskError, TaskEvent, TaskFilter, TaskId, TaskInfo, TaskSpec, TaskState,
    WaitOutcome,
};
pub use output::{PartFile, part_path, remove_stale_parts, write_output};
pub use queue::Limits;
pub use scheduler::{Job, Options, Scheduler, TaskCtx};

/// The feature authors' guide (docs/cli/tasks.md); its Rust example runs as a doc test.
#[cfg(doctest)]
#[doc = include_str!("../../../docs/cli/tasks.md")]
pub struct AuthorsGuide;
