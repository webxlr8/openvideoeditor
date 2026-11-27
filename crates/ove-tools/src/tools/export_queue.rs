//! The export queue's tools (change extend-export-formats-and-queue, design D5–D9): `export_video` queues a job (and
//! waits for it with `wait`), `job_status`, `cancel_job` and `list_export_jobs` read and control the queue, and
//! [`run_worker`] is the worker behind `ove export --worker`. The queue lives in the app's data folder, so the app,
//! `ove` and MCP see the same jobs.

use crate::ctx::ToolCtx;
use crate::fail::Fail;
use crate::receipt::Receipt;
use crate::spec::Kind::{Bool, Str};
use crate::spec::{Arg, Args, arg, req};

pub const JOB_STATUS_ABOUT: &str = "An export job's state (queued, running, done, failed, cancelled), progress, encoder, output and error. With `wait`, returns once the job has finished.";
pub const JOB_STATUS_ARGS: &[Arg] =
    &[req("job_id", Str, "The job id export_video returned"), arg("wait", Bool, "Return when the job has finished")];
pub const CANCEL_JOB_ABOUT: &str = "Cancel a queued or running export job: a running one stops within 2 seconds and leaves no partial file. Refused for a finished job, whose file is kept.";
pub const CANCEL_JOB_ARGS: &[Arg] = &[req("job_id", Str, "The job id export_video returned")];
pub const LIST_EXPORT_JOBS_ABOUT: &str = "List the export queue, oldest first: every queued and running job, and those finished in the last day (all: every job). The app, `ove` and MCP share this queue.";
pub const LIST_EXPORT_JOBS_ARGS: &[Arg] = &[arg("all", Bool, "Include jobs finished more than a day ago")];

#[cfg(feature = "media")]
use std::path::PathBuf;

#[cfg(feature = "media")]
use ove_render::queue::{CancelError, ExportJob, JobStatus, Queue};
#[cfg(feature = "media")]
use serde_json::{Value, json};

#[cfg(feature = "media")]
use super::export::{export_fail, settings_from_args};

/// The export queue: `OVE_EXPORT_QUEUE_DIR`, else `<app data>/exports` (the app, `ove` and MCP share it).
#[cfg(feature = "media")]
pub fn queue() -> Result<Queue, Fail> {
    std::env::var_os(ove_render::queue::QUEUE_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| ove_models::app_data_dir().map(|d| d.join("exports")))
        .map(Queue::new)
        .ok_or_else(|| Fail::new("export.queue", "could not find the app data folder for the export queue"))
}

/// Who a call comes from, as the Exports panel shows it: an MCP client's name, else "assistant", "mcp", "cli" or
/// "person".
#[cfg(feature = "media")]
pub fn requested_by(ctx: &dyn ToolCtx) -> String {
    use ove_core::revision::ActorKind;
    let actor = ctx.actor();
    if let Some(c) = actor.as_ref().and_then(|a| a.client.clone()).filter(|c| !c.is_empty()) {
        return c;
    }
    match (actor.map(|a| a.kind), ctx.source()) {
        (Some(ActorKind::Assistant), _) => "assistant".into(),
        (Some(ActorKind::Mcp), _) | (_, ove_core::Source::Mcp) => "mcp".into(),
        (Some(ActorKind::Cli), _) | (_, ove_core::Source::Cli) => "cli".into(),
        _ => "person".into(),
    }
}

/// The revision pin key of a job.
#[cfg(feature = "media")]
fn pin_key(job: &ExportJob) -> String {
    format!("export-{}", job.id)
}

/// The `ove` program that runs the worker: `OVE_EXPORT_WORKER_PROGRAM`, this program when it is `ove`, else the
/// `ove` next to it (the app's bundle), else `ove` on the search path.
#[cfg(feature = "media")]
pub fn worker_program() -> PathBuf {
    if let Some(p) = std::env::var_os("OVE_EXPORT_WORKER_PROGRAM").filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    let name = if cfg!(windows) { "ove.exe" } else { "ove" };
    let me = std::env::current_exe().ok();
    if let Some(me) = me.as_ref().filter(|p| p.file_name().is_some_and(|n| n == name)) {
        return me.clone();
    }
    me.and_then(|p| p.parent().map(|d| d.join(name))).filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(name))
}

/// Puts back jobs whose worker died and starts a worker when jobs wait and none runs. Returns the new worker's
/// process id when one was started.
#[cfg(feature = "media")]
pub fn ensure_worker(q: &Queue) -> Result<Option<u32>, Fail> {
    q.recover();
    if q.live_worker().is_some() || q.next_queued().is_none() {
        return Ok(None);
    }
    let program = worker_program();
    ove_render::queue::spawn_detached(&program, &["export", "--worker"], &[(ove_render::queue::QUEUE_DIR_ENV, q.dir())])
        .map(Some)
        .map_err(|e| {
            Fail::new("export.worker", format!("could not start the export worker {}: {e}", program.display()))
        })
}

/// Checks an export request against the project now (so a bad range or an offline file is refused before a job
/// exists), pins the current revision and queues the job.
#[cfg(feature = "media")]
pub fn enqueue(ctx: &mut dyn ToolCtx, a: &Args) -> Result<ExportJob, Fail> {
    let out = PathBuf::from(a.req_str("out")?);
    let out = std::path::absolute(&out).unwrap_or(out);
    let settings = settings_from_args(a, &out)?;
    let bundle = ctx.bundle(a.project())?;
    let (doc, bundle, rev) = crate::ctx::snapshot(ctx, &bundle)?;
    ove_render::export::prepare(&doc, Some(&bundle), &settings).map_err(export_fail)?;
    let q = queue()?;
    let by = requested_by(ctx);
    let job = q
        .enqueue(&bundle, &doc.project.name, rev, settings, &out, &by)
        .map_err(|e| Fail::new("export.queue", format!("could not queue the export: {e}")))?;
    if let Err(e) = ove_store::pins::pin(&bundle, &pin_key(&job), rev) {
        tracing::warn!(job = %job.id, error = %e, "could not pin the export's revision");
    }
    ensure_worker(&q)?;
    Ok(job)
}

/// A job as tools return it.
#[cfg(feature = "media")]
pub fn job_json(job: &ExportJob) -> Value {
    let mut v = serde_json::to_value(job).unwrap_or_default();
    v["jobId"] = json!(job.id);
    v["kind"] = json!("export");
    v
}

/// One line about a job.
#[cfg(feature = "media")]
pub fn job_text(job: &ExportJob) -> String {
    let what = format!("{} → {}", job.project_name, job.out.display());
    match job.status {
        JobStatus::Queued => format!("{} queued: {what}", job.id),
        JobStatus::Running => format!("{} running {:.0}%: {what}", job.id, job.progress * 100.0),
        JobStatus::Done => match &job.result {
            Some(r) => format!("{} done: {}", job.id, result_line(r)),
            None => format!("{} done: {what}", job.id),
        },
        JobStatus::Failed => format!("{} failed: {}", job.id, job.error.as_deref().unwrap_or("")),
        JobStatus::Cancelled => format!("{} cancelled: {what}", job.id),
    }
}

/// The summary of a finished export from its result JSON.
#[cfg(feature = "media")]
fn result_line(r: &Value) -> String {
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let n = |k: &str| r.get(k).and_then(Value::as_f64).unwrap_or_default();
    match (r.get("videoFormat").and_then(Value::as_str), r.get("encoderLabel").and_then(Value::as_str)) {
        (Some(f), Some(e)) => format!(
            "exported {} ({}x{} {f}, {e}, {:.1} s) in {:.1} s",
            s("path"),
            n("width"),
            n("height"),
            n("durationSeconds"),
            n("secondsTaken")
        ),
        _ => format!(
            "exported {} (audio only, {}, {:.1} s) in {:.1} s",
            s("path"),
            s("audioEncoder"),
            n("durationSeconds"),
            n("secondsTaken")
        ),
    }
}

/// Waits for a job to finish, reporting its progress; at most `limit`.
#[cfg(feature = "media")]
fn wait_job(ctx: &mut dyn ToolCtx, q: &Queue, id: &str, limit: std::time::Duration) -> Result<ExportJob, Fail> {
    let mut last = -1i64;
    let job = q
        .wait(id, limit, |j| {
            let pct = (j.progress * 100.0) as i64;
            if j.status == JobStatus::Running && pct != last {
                last = pct;
                let detail = json!({"jobId": j.id, "frame": j.frame, "totalFrames": j.total_frames, "fraction": j.progress, "encoder": j.encoder});
                ctx.progress(pct, &detail);
            }
            // A worker that died while we wait: recover the job and start another.
            if j.status == JobStatus::Running && j.worker_pid.is_some_and(|p| !ove_render::queue::process_alive(p)) {
                let _ = ensure_worker(q);
            }
        })
        .ok_or_else(|| unknown_job(id))?;
    if last >= 0 {
        ctx.progress_done();
    }
    Ok(job)
}

#[cfg(feature = "media")]
fn unknown_job(id: &str) -> Fail {
    Fail::new("job.unknown", format!("there is no job {id:?}")).details(json!({"jobId": id}))
}

/// How long a waiting call waits at most before returning the job as it is.
#[cfg(feature = "media")]
const WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// `export_video`: queues the export; with `wait`, returns when it has finished (the export's result), else at once
/// with the job id.
#[cfg(feature = "media")]
pub fn export_video(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let job = enqueue(ctx, a)?;
    if !a.bool("wait") {
        let text = format!(
            "{} queued ({}); follow it with job_status {{\"job_id\": \"{}\"}}",
            job.id,
            job.out.display(),
            job.id
        );
        return Ok(Receipt::new(job_json(&job), text));
    }
    let q = queue()?;
    let done = wait_job(ctx, &q, &job.id, WAIT_LIMIT)?;
    finished_receipt(done)
}

/// The receipt of a job that has finished: the export result, or the job's failure.
#[cfg(feature = "media")]
fn finished_receipt(job: ExportJob) -> Result<Receipt, Fail> {
    match job.status {
        JobStatus::Done => {
            let mut v = job.result.clone().unwrap_or_default();
            v["jobId"] = json!(job.id);
            let text = job.result.as_ref().map(result_line).unwrap_or_else(|| job_text(&job));
            Ok(Receipt::new(v, text))
        }
        JobStatus::Failed | JobStatus::Cancelled => Err(Fail::new(
            job.error_code.clone().unwrap_or_else(|| {
                if job.status == JobStatus::Cancelled { "export.cancelled".into() } else { "export.failed".into() }
            }),
            job.error.clone().unwrap_or_else(|| "the export did not finish".into()),
        )
        .details(json!({"jobId": job.id}))),
        _ => Ok(Receipt::new(job_json(&job), job_text(&job))),
    }
}

/// `job_status`: an export job's state and progress; with `wait`, returns once it has finished.
#[cfg(feature = "media")]
pub fn job_status(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let id = a.req_str("job_id")?;
    let q = queue()?;
    // Restart recovery: jobs of a worker that died go back in the queue, and a worker starts for them.
    ensure_worker(&q)?;
    let job = q.find(id).ok_or_else(|| unknown_job(id))?;
    let job = if a.bool("wait") && !job.status.finished() {
        ensure_worker(&q)?;
        wait_job(ctx, &q, &job.id, WAIT_LIMIT)?
    } else {
        job
    };
    Ok(Receipt::new(job_json(&job), job_text(&job)))
}

/// `cancel_job`: cancels a queued or running export job (a running one stops within 2 seconds and leaves no file).
#[cfg(feature = "media")]
pub fn cancel_job(_ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let id = a.req_str("job_id")?;
    let q = queue()?;
    let job = q.cancel(id).map_err(|e| match e {
        CancelError::Unknown(id) => unknown_job(&id),
        CancelError::Finished(ref j) => Fail::new("job.finished", e.to_string()).details(json!({"jobId": j.id})),
        CancelError::Io(_) => Fail::new("export.queue", e.to_string()),
    })?;
    // A running job stops at its next frame: wait for the worker to say so.
    let job = if job.status == JobStatus::Running {
        q.wait(&job.id, std::time::Duration::from_secs(3), |_| {}).unwrap_or(job)
    } else {
        job
    };
    Ok(Receipt::new(job_json(&job), job_text(&job)))
}

/// `list_export_jobs`: the export jobs in the queue, oldest first (finished ones from the last day, or all with
/// `all`).
#[cfg(feature = "media")]
pub fn list_export_jobs(_ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let q = queue()?;
    ensure_worker(&q)?;
    let jobs: Vec<ExportJob> =
        q.list().into_iter().filter(|j| a.bool("all") || !j.status.finished() || recent(j)).collect();
    let text = if jobs.is_empty() {
        "no export jobs".to_string()
    } else {
        jobs.iter().map(job_text).collect::<Vec<_>>().join("\n")
    };
    Ok(Receipt::new(json!({"jobs": jobs.iter().map(job_json).collect::<Vec<_>>()}), text))
}

/// Finished within the last day.
#[cfg(feature = "media")]
fn recent(j: &ExportJob) -> bool {
    j.finished_at.is_some_and(|t| ove_render::queue::now_ms() - t < 24 * 3600 * 1000)
}

/// Runs queued jobs until none is left (`ove export --worker`): each job renders its project as it was at the
/// revision it was queued at.
#[cfg(feature = "media")]
pub fn run_worker(q: &Queue) -> std::io::Result<usize> {
    ove_render::queue::run_worker(
        q,
        |job| {
            if !job.project.is_dir() {
                return Err(format!(
                    "project not found: {} (moved or deleted?); open it and export again",
                    job.project.display()
                ));
            }
            let clock: std::sync::Arc<dyn ove_core::Clock> = std::sync::Arc::new(ove_core::SystemClock);
            let p = ove_store::Project::open(&job.project, ove_store::Mode::ReadOnly, clock)
                .map_err(|e| format!("could not open {}: {e}", job.project.display()))?;
            let doc = p.doc_at(job.rev).map_err(|_| {
                format!(
                    "the project as it was when this export was queued (revision {}) is no longer available; \
                     export again",
                    job.rev
                )
            })?;
            Ok((doc, Some(job.project.clone())))
        },
        |job| ove_store::pins::unpin(&job.project, &pin_key(job)),
    )
}

#[cfg(not(feature = "media"))]
fn unavailable() -> Fail {
    Fail::new("export.unavailable", "this build of ove has no export; build it with `--features media`")
}

#[cfg(not(feature = "media"))]
pub fn export_video(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(unavailable())
}

#[cfg(not(feature = "media"))]
pub fn job_status(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(unavailable())
}

#[cfg(not(feature = "media"))]
pub fn cancel_job(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(unavailable())
}

#[cfg(not(feature = "media"))]
pub fn list_export_jobs(_: &mut dyn ToolCtx, _: &Args) -> Result<Receipt, Fail> {
    Err(unavailable())
}
