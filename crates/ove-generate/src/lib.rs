//! AI generation with the user's own provider keys (roadmap change 14): images and video (fal.ai, OpenAI), voice,
//! music and sound effects (ElevenLabs). One job interface: plan (model, length, cost estimate, placeholder) →
//! run (provider, output saved in the bundle) → finish (media registered, placeholder filled). Nothing is spent
//! without the caller confirming the plan.

pub mod catalog;
pub mod job;
pub mod keys;
pub mod place;
pub mod providers;

use std::path::Path;

pub use catalog::{Kind, Model, Provider};
pub use job::{Job, Request, Status};
pub use place::{Plan, Probed, finish_ops, plan};
pub use providers::{GenError, Output};

/// A job record for a confirmed plan.
pub fn new_job(plan: &Plan, req: &Request, id: &str, now_ms: i64) -> Job {
    Job {
        id: id.into(),
        kind: plan.kind,
        model: plan.model.id.clone(),
        request: Request { aspect: Some(plan.aspect.clone()), ..req.clone() },
        status: Status::Queued,
        progress: None,
        item_id: Some(plan.item_id.clone()),
        estimate_usd: plan.estimate_usd,
        remote_id: None,
        output: None,
        error: None,
        created_at: now_ms,
        updated_at: now_ms,
        worker_pid: None,
        details: serde_json::Value::Null,
    }
}

fn now() -> i64 {
    ove_core::Clock::now_ms(&ove_core::SystemClock)
}

/// Runs a job to the end: calls the provider, saves the output under `media/generated/`, and keeps the job file
/// current (status, progress, output, error). `cancelled` is checked between polls (and the job's cancel marker).
pub fn execute(
    bundle: &Path,
    job: &mut Job,
    store: &dyn keys::KeyStore,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), GenError> {
    // The id names the output file.
    job::check_id(&job.id).map_err(|e| GenError::new("job.invalid", e.to_string()))?;
    let model = catalog::find(Some(&job.model), job.kind).map_err(|e| GenError::new("model.unknown", e))?;
    let res = (|| -> Result<(), GenError> {
        let (key, _) = store.get(model.provider).ok_or_else(|| {
            GenError::new(
                "key.missing",
                format!(
                    "no {} API key: add it in Settings → Generation, run `ove generate key set {}`, or set {}",
                    model.provider.label(),
                    model.provider.id(),
                    keys::env_var(model.provider)
                ),
            )
        })?;
        job.status = Status::Running;
        job.worker_pid = Some(std::process::id());
        job.updated_at = now();
        let _ = job::save(bundle, job);
        let id = job.id.clone();
        let is_cancelled = || cancelled() || job::cancel_requested(bundle, &id);
        let aspect = job.request.aspect.clone().unwrap_or_else(|| "16:9".into());
        let mut snapshot = job.clone();
        let mut progress = |p: Option<f64>, remote: Option<&str>| {
            if p.is_some() {
                snapshot.progress = p;
            }
            if let Some(r) = remote {
                snapshot.remote_id = Some(r.to_string());
            }
            snapshot.updated_at = now();
            let _ = job::save(bundle, &snapshot);
        };
        let dir = job::output_dir(bundle);
        std::fs::create_dir_all(&dir).map_err(|e| GenError::new("file.unwritable", e.to_string()))?;
        let out = providers::run(
            &model,
            &job.request,
            &aspect,
            &mut providers::Run { key: &key, cancelled: &is_cancelled, progress: &mut progress, out_dir: &dir },
        )?;
        job.remote_id = snapshot.remote_id.clone();
        let file = format!("{}.{}", job.id, out.ext);
        out.data.save(&dir.join(&file)).map_err(|e| GenError::new("file.unwritable", e.to_string()))?;
        job.output = Some(format!("media/generated/{file}"));
        job.details = out.details;
        Ok(())
    })();
    job.updated_at = now();
    match &res {
        Ok(()) => {
            job.status = Status::Succeeded;
            job.progress = Some(1.0);
        }
        Err(e) if e.code == "cancelled" => {
            job.status = Status::Cancelled;
            job.error = Some("cancelled".into());
        }
        Err(e) => {
            job.status = Status::Failed;
            job.error = Some(e.message.clone());
        }
    }
    let _ = job::save(bundle, job);
    res
}
