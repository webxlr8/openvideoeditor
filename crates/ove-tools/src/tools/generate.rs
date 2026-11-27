//! Generation as agent tools (change unify-agent-tool-layer, D7): `list_models`, `generate_image`,
//! `generate_video`, `generate_voice`, `generate_music`, `generate_sfx`, `generation_status` and
//! `cancel_generation`, over the `ove-generate` jobs the app and `ove generate` already use.
//!
//! Spending takes two calls and a person:
//! 1. A call without `confirm` returns the plan, the estimate and a quote id. It spends nothing and touches neither
//!    the project nor the provider.
//! 2. A call with `confirm: <quote id>` and the same arguments asks a person to approve the price: the running app's
//!    price card, the in-app question card, or the MCP client's confirmation prompt ([`ToolCtx::ask_spend`]). Only
//!    an approval spends; the agent's `confirm` alone never does (`confirm.required`).
//!
//! Approved, the request becomes a pending placeholder (one revision, the agent's) and a job that the existing
//! generation worker runs; the worker fills the placeholder as the same agent's change, so the agent's undo reverts
//! both. A quote expires after 10 minutes, is valid only for the request it priced, and a session cannot confirm
//! more than its spend cap (Settings → Generation).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ove_generate::catalog::Price;
use ove_generate::{Job, Kind, Request, Status};
use serde_json::{Value, json};

use crate::ctx::{ToolCtx, edit, read};
use crate::fail::{Fail, now_ms};
use crate::receipt::Receipt;
use crate::session::{Session, canonical, fnv1a};
use crate::spec::Args;

fn invalid(e: String) -> Fail {
    Fail::new("generate.invalid", e)
}

fn session_of(ctx: &dyn ToolCtx) -> Result<Session, Fail> {
    ctx.session().ok_or_else(|| Fail::new("tool.unavailable", "generation is not available in this context"))
}

/// The job as JSON (the job file's fields).
pub fn job_json(j: &Job) -> Value {
    serde_json::to_value(j).unwrap_or(Value::Null)
}

fn status_word(s: Status) -> &'static str {
    match s {
        Status::Queued => "queued",
        Status::Running => "running",
        Status::Succeeded => "done",
        Status::Failed => "failed",
        Status::Cancelled => "cancelled",
    }
}

/// One line about a job: id, kind, status, model, progress, output or error.
pub fn job_text(j: &Job) -> String {
    let mut s = format!("{} {} {} ({})", j.id, j.kind.as_str(), status_word(j.status), j.model);
    if let Some(p) = j.progress.filter(|_| !j.status.done()) {
        s.push_str(&format!(" {:.0}%", p * 100.0));
    }
    if let Some(o) = &j.output {
        s.push_str(&format!(" → {o}"));
    }
    if let Some(e) = &j.error {
        s.push_str(&format!(": {e}"));
    }
    s
}

pub fn price_text(p: &Price) -> String {
    match p {
        Price::Image { usd } => format!("${usd}/image"),
        Price::Second { usd } => format!("${usd}/s"),
        Price::KiloChar { usd } => format!("${usd}/1k chars"),
        Price::Unknown => "?".into(),
    }
}

/// `list_models`: the generation models (embedded catalog), their list prices and whether the person has a key.
pub fn list_models(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let kind = a
        .str("kind")
        .map(|k| {
            Kind::parse(k).ok_or_else(|| Fail::new("argument.invalid", "kind is image, video, voice, music or sfx"))
        })
        .transpose()?;
    let session = session_of(ctx)?;
    let keys = session.keys();
    let models: Vec<Value> = ove_generate::catalog::models()
        .into_iter()
        .filter(|m| kind.is_none_or(|k| m.kind == k))
        .map(|m| {
            let mut v = serde_json::to_value(&m).unwrap_or(Value::Null);
            v["priceText"] = price_text(&m.price).into();
            v["hasKey"] = keys.get(m.provider).is_some().into();
            v
        })
        .collect();
    let text = models
        .iter()
        .map(|m| {
            format!(
                "{}\t{}\t{}{}\t{}{}",
                m["id"].as_str().unwrap_or_default(),
                m["kind"].as_str().unwrap_or_default(),
                m["label"].as_str().unwrap_or_default(),
                if m["default"] == true { " (default)" } else { "" },
                m["priceText"].as_str().unwrap_or_default(),
                if m["hasKey"] == true { "" } else { "\t(no key)" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Receipt::new(
        json!({
            "models": models,
            "keys": ove_generate::keys::status(keys),
            "capUsd": session.cap_usd(),
            "spentUsd": session.spent_usd(),
        }),
        format!("{text}\nPrices are list prices to estimate cost; the provider bills the person."),
    ))
}

/// The generation request the arguments describe.
fn request(a: &Args, kind: Kind) -> Result<Request, Fail> {
    let prompt = if kind == Kind::Voice { a.req_str("text")? } else { a.req_str("prompt")? };
    let image = match a.str("image") {
        Some(p) => {
            let c = dunce::canonicalize(p).map_err(|e| Fail::new("file.notFound", format!("{p}: {e}")))?;
            if !c.is_file() {
                return Err(Fail::new("file.notFound", format!("{p} is not a file")));
            }
            Some(c)
        }
        None => None,
    };
    let seed = match a.i64("seed") {
        Some(s) if s >= 0 => Some(s as u64),
        Some(_) => return Err(Fail::new("argument.invalid", "seed must be ≥ 0")),
        None => None,
    };
    Ok(Request {
        prompt: prompt.to_string(),
        model: a.str("model").map(str::to_string),
        duration: a.f64("durationSeconds"),
        aspect: a.str("aspect").map(str::to_string),
        image,
        voice: a.str("voice").map(str::to_string),
        speed: a.f64("speed"),
        stability: a.f64("stability"),
        style: a.f64("style"),
        negative: a.str("negativePrompt").map(str::to_string),
        seed,
        instrumental: a.bool("instrumental"),
        start: a.f64("start"),
        track: a.str("track").map(str::to_string),
        name: a.str("name").map(str::to_string),
    })
}

/// The hash a quote is bound to: the tool, the project and every argument but `confirm`.
pub fn request_hash(tool: &str, bundle: &Path, a: &Args) -> String {
    let mut args = a.0.clone();
    args.remove("confirm");
    args.retain(|_, v| !v.is_null());
    fnv1a(&canonical(&json!({"tool": tool, "bundle": bundle, "args": args})))
}

/// Dollars, with a third decimal under ten cents (an image is $0.025).
fn money(v: f64) -> String {
    if v < 0.1 { format!("${v:.3}") } else { format!("${v:.2}") }
}

/// What a person reads before approving a spend (`card` as [`ToolCtx::ask_spend`] receives it): the question, then
/// the details on separate lines.
pub fn card_text(card: &Value) -> String {
    let plan = &card["plan"];
    let usd = |k: &str| card[k].as_f64().map(money).unwrap_or_else(|| "?".into());
    let who = card["actor"]["client"].as_str().or_else(|| card["actor"]["kind"].as_str()).unwrap_or("An agent");
    format!(
        "{who} asks to spend about {} on your {} account.\n{} with {} ({}, {:.1} s) in project {}:\n“{}”\nThis session has spent {} of its {} cap.",
        usd("estimateUsd"),
        plan["provider"].as_str().unwrap_or("provider"),
        capitalized(card["kind"].as_str().unwrap_or("media")),
        plan["modelLabel"].as_str().unwrap_or("?"),
        plan["aspect"].as_str().unwrap_or("?"),
        plan["seconds"].as_f64().unwrap_or(0.0),
        card["project"]["name"].as_str().unwrap_or("?"),
        card["prompt"].as_str().unwrap_or_default(),
        usd("spentUsd"),
        usd("capUsd"),
    )
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// `generate_image`, `generate_video`, `generate_voice`, `generate_music`, `generate_sfx`.
fn generate(ctx: &mut dyn ToolCtx, a: &Args, kind: Kind, tool: &str) -> Result<Receipt, Fail> {
    let session = session_of(ctx)?;
    let req = request(a, kind)?;
    let bundle = ctx.bundle(a.project())?;
    let (plan, project_name) = read(ctx, &bundle, |p| {
        let plan = ove_generate::plan(p.doc(), kind, &req, "i_gen_quote").map_err(invalid)?;
        Ok((plan, p.doc().project.name.clone()))
    })?;
    let estimate = plan.estimate_usd.filter(|v| v.is_finite() && *v >= 0.0).ok_or_else(|| {
        Fail::new(
            "spend.unknown",
            format!(
                "the cost of {} is not known, so it cannot be checked against the session's spend cap; use a model from list_models",
                plan.model.label
            ),
        )
    })?;
    let hash = request_hash(tool, &bundle, a);
    let plan_v = json!({
        "kind": kind.as_str(),
        "model": plan.model.id,
        "modelLabel": plan.model.label,
        "provider": plan.model.provider.label(),
        "aspect": plan.aspect,
        "seconds": plan.seconds,
        "track": plan.track_id,
        "start": plan.start,
    });
    let Some(quote_id) = a.str("confirm") else {
        // Step 1: a quote. Nothing is spent and nothing changes.
        session.check_cap(estimate)?;
        let card = json!({
            "tool": tool,
            "kind": kind.as_str(),
            "project": {"name": project_name, "bundle": bundle},
            "prompt": req.prompt,
            "plan": plan_v,
            "estimateUsd": estimate,
            "requestHash": hash,
            "actor": ctx.actor(),
        });
        let q = session.quote(hash, estimate, card);
        return Ok(Receipt::new(
            json!({
                "quoteId": q.id,
                "estimateUsd": estimate,
                "expiresAt": q.expires_ms(),
                "plan": plan_v,
                "spentUsd": session.spent_usd(),
                "capUsd": session.cap_usd(),
                "spent": false,
                "next": format!(
                    "Nothing was spent. To go ahead, call {tool} again with the same arguments and confirm: \"{}\". A person then approves the price; the quote lasts 10 minutes.",
                    q.id
                ),
            }),
            format!(
                "quote {}: {} with {} ({}, {:.1} s) ≈ {}, billed to the {} account; nothing spent",
                q.id,
                kind.as_str(),
                plan.model.label,
                plan.aspect,
                plan.seconds,
                money(estimate),
                plan.model.provider.label()
            ),
        ));
    };
    // Step 2: the quote must be this request's, still valid, and within the cap; then a person decides.
    let q = session.redeem(quote_id, &hash)?;
    session.check_cap(q.estimate_usd)?;
    if session.keys().get(plan.model.provider).is_none() {
        return Err(Fail::new(
            "key.missing",
            format!(
                "no {} API key: the person adds it in Settings → Generation (or `ove generate key set {}`); nothing was spent",
                plan.model.provider.label(),
                plan.model.provider.id()
            ),
        ));
    }
    let mut card = q.card.clone();
    card["quoteId"] = q.id.clone().into();
    card["expiresAt"] = q.expires_ms().into();
    card["spentUsd"] = session.spent_usd().into();
    card["capUsd"] = session.cap_usd().into();
    if !ctx.ask_spend(&card)? {
        return Err(Fail::new("confirm.declined", "The person declined this spend. Nothing was spent."));
    }
    let spent = session.commit(&q.id)?;
    match place_and_start(ctx, &bundle, kind, &req, &session) {
        Ok((job, rev, receipt)) => {
            let line = receipt["changedItems"]
                .as_array()
                .and_then(|l| l.iter().find(|c| c["id"].as_str() == job.item_id.as_deref()))
                .and_then(|c| c["text"].as_str())
                .unwrap_or_default()
                .to_string();
            let mut v = json!({
                "jobId": job.id,
                "status": status_word(job.status),
                "itemId": job.item_id,
                "line": line,
                "rev": rev,
                "estimateUsd": q.estimate_usd,
                "spentUsd": spent,
                "capUsd": session.cap_usd(),
            });
            if let (Some(o), Some(r)) = (v.as_object_mut(), receipt.as_object()) {
                for (k, x) in r {
                    o.insert(k.clone(), x.clone());
                }
            }
            let text = format!(
                "rev {rev}: approved; job {} started, placeholder {} — follow it with generation_status",
                job.id,
                job.item_id.as_deref().unwrap_or("?")
            );
            Ok(Receipt::new(v, text))
        }
        Err(f) => {
            session.refund(&q.id);
            Err(f)
        }
    }
}

/// Places the placeholder and saves the job (one revision, the caller's), then starts the worker. When the worker
/// cannot start, the job fails and its placeholder is removed again.
fn place_and_start(
    ctx: &mut dyn ToolCtx,
    bundle: &Path,
    kind: Kind,
    req: &Request,
    session: &Session,
) -> Result<(Job, i64, Value), Fail> {
    let now = session.now();
    let job_id = ove_generate::job::new_id(now);
    let item_id = format!("i_gen_{}", job_id.trim_start_matches("g_"));
    let actor = ctx.actor();
    let (job, rev, receipt) = edit(ctx, bundle, |w| {
        let before = w.doc().clone();
        let plan = ove_generate::plan(w.doc(), kind, req, &item_id).map_err(invalid)?;
        let mut job = ove_generate::new_job(&plan, req, &job_id, now);
        job.actor = actor.clone();
        let rev = w.apply(plan.ops.clone(), format!("generate {}: placeholder", kind.as_str()))?;
        ove_generate::job::save(w.bundle(), &job).map_err(|e| Fail::new("file.unwritable", e.to_string()))?;
        Ok((job, rev, crate::edits::receipt(&before, w.doc())))
    })?;
    if let Err(f) = ctx.start_generation(bundle, &job.id) {
        let mut failed = job.clone();
        failed.status = Status::Failed;
        failed.error = Some(format!("the generation worker could not start: {}", f.message));
        failed.updated_at = now_ms();
        let _ = ove_generate::job::save(bundle, &failed);
        let _ = edit(ctx, bundle, |w| {
            let ops = ove_generate::place::failed_ops(w.doc(), &failed);
            if ops.is_empty() {
                return Ok(0);
            }
            w.apply(ops, format!("generate {}: failed", kind.as_str()))
        });
        return Err(f);
    }
    Ok((job, rev, receipt))
}

pub fn generate_image(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    generate(ctx, a, Kind::Image, "generate_image")
}

pub fn generate_video(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    generate(ctx, a, Kind::Video, "generate_video")
}

pub fn generate_voice(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    generate(ctx, a, Kind::Voice, "generate_voice")
}

pub fn generate_music(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    generate(ctx, a, Kind::Music, "generate_music")
}

pub fn generate_sfx(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    generate(ctx, a, Kind::Sfx, "generate_sfx")
}

/// The projects a job can be in: the one the call names (or the session's), else every project of the library.
fn job_bundles(ctx: &mut dyn ToolCtx, project: Option<&str>) -> Result<Vec<PathBuf>, Fail> {
    match ctx.bundle(project) {
        Ok(b) => Ok(vec![b]),
        Err(f) if project.is_some() => Err(f),
        Err(_) => Ok(ctx.library()?.list()?.iter().map(|l| l.bundle().to_path_buf()).collect()),
    }
}

fn find_job(ctx: &mut dyn ToolCtx, a: &Args) -> Result<(PathBuf, Job), Fail> {
    let id = a.req_str("id")?;
    ove_generate::job::check_id(id).map_err(|_| Fail::new("argument.invalid", format!("{id:?} is not a job id")))?;
    let bundles = job_bundles(ctx, a.project())?;
    ove_generate::job::find(bundles.iter().map(PathBuf::as_path), id)
        .ok_or_else(|| Fail::new("job.notFound", format!("no generation job {id}")))
}

/// Settles the project's jobs whose worker is gone (or that stopped making progress): a provider request still
/// queued gets a new worker that resumes it (no second submission); the rest fail and their pending placeholders
/// are removed.
pub fn reap(ctx: &mut dyn ToolCtx, bundle: &Path) {
    let reaped = ove_generate::job::reap_stale_jobs(bundle, now_ms());
    for j in &reaped.resumable {
        // If the worker cannot start, the job stays queued (placeholder kept) until it is given up as stale.
        let _ = ctx.start_generation(bundle, &j.id);
    }
    if reaped.failed.is_empty() {
        return;
    }
    let _ = edit(ctx, bundle, |w| {
        let ops: Vec<_> = reaped.failed.iter().flat_map(|j| ove_generate::place::failed_ops(w.doc(), j)).collect();
        if ops.is_empty() {
            return Ok(0);
        }
        w.apply(ops, format!("generate: {} stopped job(s)", reaped.failed.len()))
    });
}

/// Longest `generation_status` waits in one call.
const WAIT: Duration = Duration::from_secs(120);

/// `generation_status`: a job's state; `wait` until it finishes (at most two minutes per call), `cancel` to stop it.
pub fn generation_status(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let (bundle, mut job) = find_job(ctx, a)?;
    if a.bool("cancel") && !job.status.done() {
        ove_generate::job::request_cancel(&bundle, &job.id).map_err(|e| Fail::new("file.unwritable", e.to_string()))?;
    }
    if !job.status.done() {
        reap(ctx, &bundle);
        job = ove_generate::job::load(&bundle, &job.id).unwrap_or(job);
    }
    let deadline = Instant::now() + WAIT;
    while a.bool("wait") && !job.status.done() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        if job.status == Status::Running && now_ms() - job.updated_at > ove_generate::job::DEAD_WORKER_GRACE_MS {
            reap(ctx, &bundle);
        }
        job = ove_generate::job::load(&bundle, &job.id).unwrap_or(job);
    }
    let text = job_text(&job);
    Ok(Receipt::new(job_json(&job), text))
}

/// `cancel_generation`: asks the job's worker to stop; a finished job is left as it is.
pub fn cancel_generation(ctx: &mut dyn ToolCtx, a: &Args) -> Result<Receipt, Fail> {
    let (bundle, job) = find_job(ctx, a)?;
    if !job.status.done() {
        ove_generate::job::request_cancel(&bundle, &job.id).map_err(|e| Fail::new("file.unwritable", e.to_string()))?;
    }
    let mut v = job_json(&job);
    v["cancelRequested"] = (!job.status.done()).into();
    let text = if job.status.done() {
        format!("{} already finished: {}", job.id, job_text(&job))
    } else {
        format!("asked {} to stop; generation_status shows when it has", job.id)
    };
    Ok(Receipt::new(v, text))
}

#[cfg(feature = "media")]
fn probe(path: &Path) -> ove_generate::Probed {
    match ove_media::probe(path) {
        Ok(i) => ove_generate::Probed {
            duration_us: i.duration_us,
            width: i.video.as_ref().map(|v| v.width),
            height: i.video.as_ref().map(|v| v.height),
            has_audio: i.audio.is_some(),
        },
        Err(_) => Default::default(),
    }
}

#[cfg(not(feature = "media"))]
fn probe(_: &Path) -> ove_generate::Probed {
    Default::default()
}

/// Runs a saved job (the generation worker) and applies its result: the output registered and the placeholder
/// filled, or the placeholder removed when it failed. The edit is recorded as the job's actor (the agent that asked
/// for it), so that agent's undo covers it.
pub fn run_job(
    ctx: &mut dyn ToolCtx,
    bundle: &Path,
    id: &str,
    keys: &dyn ove_generate::keys::KeyStore,
) -> Result<Job, Fail> {
    let mut job =
        ove_generate::job::load(bundle, id).ok_or_else(|| Fail::new("job.notFound", format!("no job {id}")))?;
    let res = ove_generate::execute(bundle, &mut job, keys, &|| false);
    let actor = job.actor.clone();
    match res {
        Ok(()) => {
            let rel = job.output.clone().unwrap_or_default();
            let file = bundle.join(&rel);
            let probed = probe(&file);
            if job.kind != Kind::Image || probed.width.is_some() {
                crate::tools::media::thumbnail(bundle, &format!("m_{}", job.id.trim_start_matches("g_")), &file);
            }
            edit(ctx, bundle, |w| {
                let ops = ove_generate::finish_ops(w.doc(), &job, &rel, &probed, now_ms());
                let summary = format!(
                    "generate {}: {}",
                    job.kind.as_str(),
                    job.request.prompt.chars().take(60).collect::<String>()
                );
                w.apply_by(ops, summary, actor.clone())
            })?;
        }
        Err(_) => {
            let _ = edit(ctx, bundle, |w| {
                let ops = ove_generate::place::failed_ops(w.doc(), &job);
                if ops.is_empty() {
                    return Ok(0);
                }
                let what = if job.status == Status::Cancelled { "cancelled" } else { "failed" };
                w.apply_by(ops, format!("generate {}: {what}", job.kind.as_str()), actor.clone())
            });
        }
    }
    Ok(job)
}
