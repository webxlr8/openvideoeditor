//! Indexing as background work (task 2.3): one P3 (analysis) task per media file, in the inference class, on the
//! shared `ove-tasks` scheduler. The task builds the encoder when it starts (normally the SigLIP encoders run by the
//! model worker), so nothing loads while it waits; a second submit for the same content returns the queued task.

use std::path::PathBuf;

use ove_tasks::{Class, Priority, Scheduler, TaskError, TaskId, TaskSpec};

use crate::indexer::{Built, MediaKind, ensure_index};
use crate::model::Encoder;

/// The task kind, as the Background Tasks popover and `ove tasks` show it.
pub const KIND: &str = "media-index";

/// Peak memory of the indexing task in this process (decoded sample frames and the encoder's inputs). The model
/// itself runs in the worker process, outside the editor's budget.
pub const TASK_MEMORY: u64 = 192 * 1024 * 1024;

/// The task's inputs key: one task per content.
pub fn key(content_id: &str) -> String {
    format!("{KIND}:{content_id}")
}

/// What to index.
#[derive(Debug, Clone)]
pub struct Job {
    pub project: String,
    pub bundle: PathBuf,
    pub source: PathBuf,
    pub content_id: String,
    pub kind: MediaKind,
    /// The file's name, for the task label.
    pub name: String,
}

/// Queues indexing of one media file. `encoder` is called when the task starts. `done` gets the outcome (also on
/// failure; not when cancelled).
pub fn submit(
    scheduler: &Scheduler,
    job: Job,
    encoder: impl FnOnce() -> crate::Result<Box<dyn Encoder + Send>> + Send + 'static,
    done: impl FnOnce(Result<Built, String>) + Send + 'static,
) -> TaskId {
    let spec = TaskSpec::new(KIND, format!("Indexing {}", job.name), Priority::Analysis, Class::Inference)
        .project(job.project.clone())
        .memory(TASK_MEMORY)
        .key(key(&job.content_id));
    scheduler.submit(spec, move |ctx| {
        ctx.checkpoint()?;
        let mut enc = match encoder() {
            Ok(e) => e,
            Err(e) => {
                let m = e.to_string();
                done(Err(m.clone()));
                return Err(TaskError::Failed(m));
            }
        };
        let mut cancelled = false;
        let r = ensure_index(&job.bundle, &job.source, &job.content_id, job.kind, enc.as_mut(), &mut |f| {
            ctx.progress(f);
            let ok = ctx.checkpoint().is_ok();
            cancelled |= !ok;
            ok
        });
        match r {
            Ok(b) => {
                done(Ok(b));
                Ok(())
            }
            Err(_) if cancelled => Err(TaskError::Cancelled),
            Err(e) => {
                let m = format!("“{}” could not be indexed: {e}", job.name);
                done(Err(m.clone()));
                Err(TaskError::Failed(m))
            }
        }
    })
}
