//! `ove generate …`: images, video, voice, music and sound effects with the user's own provider keys (change 14).
//!
//! Nothing is spent without `--yes` (or `--dry-run` to see the plan and estimate). A pending placeholder goes on the
//! timeline first (one revision); the job runs in this process (default) or a background worker (`--no-wait`), and
//! when it finishes the output is saved in the bundle and the placeholder filled (another revision). Both edits go
//! through the running app when it has the project open.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand};
use ove_generate::keys::{KeyStore, OsKeyStore};
use ove_generate::{Job, Kind, Provider, Request, Status};
use serde_json::{Value, json};

use crate::common::{Ctx, Fail, Out, edit, now_ms, via};

#[derive(Args, Clone)]
pub struct GenArgs {
    pub project: String,
    pub prompt: String,
    /// Model id (`ove generate models`); any fal endpoint as fal:<owner>/<model>.
    #[arg(long)]
    pub model: Option<String>,
    /// Seconds of output (video, music, sound effects; image clip length).
    #[arg(long)]
    pub duration: Option<f64>,
    /// 16:9, 9:16, 1:1, … (default: the project's).
    #[arg(long)]
    pub aspect: Option<String>,
    /// First frame / reference image.
    #[arg(long)]
    pub image: Option<PathBuf>,
    /// Voice name or id (voice).
    #[arg(long)]
    pub voice: Option<String>,
    #[arg(long)]
    pub speed: Option<f64>,
    #[arg(long)]
    pub stability: Option<f64>,
    #[arg(long)]
    pub style: Option<f64>,
    /// What to avoid (models that support it).
    #[arg(long)]
    pub negative: Option<String>,
    #[arg(long)]
    pub seed: Option<u64>,
    /// No vocals (music).
    #[arg(long)]
    pub instrumental: bool,
    /// Timeline start in seconds (default: after the last clip on the track).
    #[arg(long)]
    pub start: Option<f64>,
    /// Track name or id (default: Overlay for images and video, Audio for sound).
    #[arg(long)]
    pub track: Option<String>,
    /// Clip name.
    #[arg(long)]
    pub name: Option<String>,
    /// Confirm spending (required to start).
    #[arg(short, long)]
    pub yes: bool,
    /// Show the plan and estimate; spend nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Return once started; a background worker finishes the job (`ove generate status <job>`).
    #[arg(long)]
    pub no_wait: bool,
}

#[derive(Subcommand)]
pub enum GenerateCmd {
    /// A still image placed as a clip.
    Image(GenArgs),
    /// A video clip (text to video, or from a first frame with --image).
    Video(GenArgs),
    /// Speech from text (ElevenLabs).
    Voice(GenArgs),
    /// Music from a description.
    Music(GenArgs),
    /// A sound effect from a description.
    Sfx(GenArgs),
    /// A job's status; --wait until it finishes, --cancel to stop it.
    Status {
        job: String,
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        cancel: bool,
    },
    /// Jobs of a project, newest first.
    List { project: String },
    /// Models with provider, kind, durations, aspects and list price.
    Models {
        #[arg(long)]
        kind: Option<String>,
    },
    /// Voices in the ElevenLabs account.
    Voices {
        #[arg(long)]
        search: Option<String>,
    },
    /// Provider keys: status, set (reads the key from stdin), clear.
    Key {
        #[command(subcommand)]
        cmd: KeyCmd,
    },
    /// Runs a job (the background worker).
    #[command(hide = true, name = "_run")]
    Run { bundle: PathBuf, job: String },
}

#[derive(Subcommand)]
pub enum KeyCmd {
    Status,
    /// Store a key in the system credential store (`echo $KEY | ove generate key set fal`).
    Set {
        provider: String,
    },
    Clear {
        provider: String,
    },
}

fn provider(s: &str) -> Result<Provider, Fail> {
    Provider::parse(s)
        .ok_or_else(|| Fail::new("provider.unknown", format!("unknown provider {s} (fal, openai, elevenlabs)")))
}

fn request(a: &GenArgs) -> Request {
    Request {
        prompt: a.prompt.clone(),
        model: a.model.clone(),
        duration: a.duration,
        aspect: a.aspect.clone(),
        image: a.image.as_ref().map(|p| dunce::canonicalize(p).unwrap_or_else(|_| p.clone())),
        voice: a.voice.clone(),
        speed: a.speed,
        stability: a.stability,
        style: a.style,
        negative: a.negative.clone(),
        seed: a.seed,
        instrumental: a.instrumental,
        start: a.start,
        track: a.track.clone(),
        name: a.name.clone(),
    }
}

fn job_json(j: &Job) -> Value {
    serde_json::to_value(j).unwrap_or(Value::Null)
}

fn job_text(j: &Job) -> String {
    let mut s = format!(
        "{} {} {} ({})",
        j.id,
        j.kind.as_str(),
        match j.status {
            Status::Queued => "queued",
            Status::Running => "running",
            Status::Succeeded => "done",
            Status::Failed => "failed",
            Status::Cancelled => "cancelled",
        },
        j.model
    );
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

fn money(v: Option<f64>) -> String {
    v.map(|x| format!("≈ ${x:.3}")).unwrap_or_else(|| "cost unknown".into())
}

fn start(ctx: &Ctx, kind: Kind, a: &GenArgs) -> Out {
    let req = request(a);
    if let Some(img) = &req.image
        && !img.is_file()
    {
        return Err(Fail::new("file.notFound", format!("{} is not a file", img.display())));
    }
    let item_id = format!("i_gen_{:x}", now_ms());
    let p = ctx.read(&a.project)?;
    let plan = ove_generate::plan(p.doc(), kind, &req, &item_id).map_err(|e| Fail::new("generate.invalid", e))?;
    let plan_v = serde_json::to_value(&plan).unwrap_or(Value::Null);
    let summary = format!(
        "{} with {} ({}, {:.1} s) on {} at {:.2} s — {} (billed by {})",
        kind.as_str(),
        plan.model.label,
        plan.aspect,
        plan.seconds,
        plan.track_id,
        plan.start,
        money(plan.estimate_usd),
        plan.model.provider.label()
    );
    if a.dry_run {
        return Ok((json!({"plan": plan_v, "dryRun": true}), format!("would generate {summary}")));
    }
    if !a.yes {
        return Err(Fail::new(
            "confirm.required",
            format!(
                "this spends money on your {} account: {summary}. Run again with --yes",
                plan.model.provider.label()
            ),
        )
        .details(json!({"plan": plan_v})));
    }
    let store = OsKeyStore;
    if store.get(plan.model.provider).is_none() {
        return Err(Fail::new(
            "key.missing",
            format!(
                "no {} API key: `ove generate key set {}` (reads it from stdin), set {}, or add it in the app's Settings",
                plan.model.provider.label(),
                plan.model.provider.id(),
                ove_generate::keys::env_var(plan.model.provider)
            ),
        ));
    }
    // Placeholder (re-planned on the document the edit is applied to).
    let (job, bundle, how) = edit(ctx, &a.project, |w| {
        let plan = ove_generate::plan(w.doc(), kind, &req, &item_id).map_err(|e| Fail::new("generate.invalid", e))?;
        let job = ove_generate::new_job(&plan, &req, &ove_generate::job::new_id(now_ms()), now_ms());
        w.apply(plan.ops.clone(), ctx.source, format!("generate {}: placeholder", kind.as_str()))?;
        ove_generate::job::save(w.bundle(), &job).map_err(|e| Fail::new("file.unwritable", e.to_string()))?;
        Ok((job, w.bundle().to_path_buf(), via(w)))
    })?;
    if a.no_wait {
        let exe = std::env::current_exe().map_err(|e| Fail::new("worker.failed", e.to_string()))?;
        std::process::Command::new(exe)
            .arg("--library")
            .arg(&ctx.root)
            .args(["generate", "_run"])
            .arg(&bundle)
            .arg(&job.id)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| Fail::new("worker.failed", format!("could not start the background worker: {e}")))?;
        let mut v = job_json(&job);
        v["started"] = json!(true);
        return Ok((
            v,
            format!("started {} — {summary}{how}; `ove generate status {} --wait` to follow", job.id, job.id),
        ));
    }
    let done = run_job(ctx, &bundle, &job.id)?;
    let text = job_text(&done);
    if done.status != Status::Succeeded {
        return Err(Fail::new("generate.failed", text).details(job_json(&done)));
    }
    Ok((job_json(&done), format!("{text}{how}")))
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

/// Runs a saved job and applies its result (or marks the placeholder when it failed).
pub fn run_job(ctx: &Ctx, bundle: &Path, id: &str) -> Result<Job, Fail> {
    let mut job =
        ove_generate::job::load(bundle, id).ok_or_else(|| Fail::new("job.notFound", format!("no job {id}")))?;
    let res = ove_generate::execute(bundle, &mut job, &OsKeyStore, &|| false);
    let project = bundle.display().to_string();
    match res {
        Ok(()) => {
            let rel = job.output.clone().unwrap_or_default();
            let file = bundle.join(&rel);
            let probed = probe(&file);
            if job.kind != Kind::Image || probed.width.is_some() {
                crate::media::thumbnail(bundle, &format!("m_{}", job.id.trim_start_matches("g_")), &file);
            }
            edit(ctx, &project, |w| {
                let ops = ove_generate::finish_ops(w.doc(), &job, &rel, &probed, now_ms());
                w.apply(
                    ops,
                    ctx.source,
                    format!(
                        "generate {}: {}",
                        job.kind.as_str(),
                        job.request.prompt.chars().take(60).collect::<String>()
                    ),
                )
            })?;
        }
        Err(_) => {
            let _ = edit(ctx, &project, |w| {
                let ops = ove_generate::place::failed_ops(w.doc(), &job);
                if ops.is_empty() {
                    return Ok(0);
                }
                w.apply(
                    ops,
                    ctx.source,
                    format!(
                        "generate {}: {}",
                        job.kind.as_str(),
                        if job.status == Status::Cancelled { "cancelled" } else { "failed" }
                    ),
                )
            });
        }
    }
    Ok(job)
}

/// Fails the project's jobs whose worker is gone (or that stopped making progress) and removes their pending
/// placeholders, so they neither show as running forever nor block export.
fn reap(ctx: &Ctx, bundle: &Path) {
    let reaped = ove_generate::job::reap_stale_jobs(bundle, now_ms());
    if reaped.is_empty() {
        return;
    }
    let _ = edit(ctx, &bundle.display().to_string(), |w| {
        let ops: Vec<_> = reaped.iter().flat_map(|j| ove_generate::place::failed_ops(w.doc(), j)).collect();
        if ops.is_empty() {
            return Ok(0);
        }
        w.apply(ops, ctx.source, format!("generate: {} stopped job(s)", reaped.len()))
    });
}

fn bundles(ctx: &Ctx) -> Vec<PathBuf> {
    ctx.lib.list().map(|v| v.iter().map(|l| l.bundle().to_path_buf()).collect()).unwrap_or_default()
}

pub fn run(ctx: &Ctx, cmd: &GenerateCmd) -> Out {
    match cmd {
        GenerateCmd::Image(a) => start(ctx, Kind::Image, a),
        GenerateCmd::Video(a) => start(ctx, Kind::Video, a),
        GenerateCmd::Voice(a) => start(ctx, Kind::Voice, a),
        GenerateCmd::Music(a) => start(ctx, Kind::Music, a),
        GenerateCmd::Sfx(a) => start(ctx, Kind::Sfx, a),
        GenerateCmd::Run { bundle, job } => {
            let j = run_job(ctx, bundle, job)?;
            Ok((job_json(&j), job_text(&j)))
        }
        GenerateCmd::Status { job, wait, cancel } => {
            let all = bundles(ctx);
            let (bundle, mut j) = ove_generate::job::find(all.iter().map(PathBuf::as_path), job)
                .ok_or_else(|| Fail::new("job.notFound", format!("no job {job} in the library")))?;
            if !j.status.done() {
                reap(ctx, &bundle);
                j = ove_generate::job::load(&bundle, &j.id).unwrap_or(j);
            }
            if *cancel && !j.status.done() {
                ove_generate::job::request_cancel(&bundle, &j.id)
                    .map_err(|e| Fail::new("file.unwritable", e.to_string()))?;
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(1800);
            while (*wait || *cancel) && !j.status.done() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(500));
                // A worker that died leaves the job running forever: give it up.
                if j.status == Status::Running && now_ms() - j.updated_at > ove_generate::job::DEAD_WORKER_GRACE_MS {
                    reap(ctx, &bundle);
                }
                j = ove_generate::job::load(&bundle, &j.id).unwrap_or(j);
                if *cancel && !*wait {
                    break;
                }
            }
            Ok((job_json(&j), job_text(&j)))
        }
        GenerateCmd::List { project } => {
            let b = ctx.find(project)?;
            reap(ctx, &b);
            let jobs = ove_generate::job::list(&b);
            let text = if jobs.is_empty() {
                "no generation jobs".into()
            } else {
                jobs.iter().map(job_text).collect::<Vec<_>>().join("\n")
            };
            Ok((json!({"jobs": jobs}), text))
        }
        GenerateCmd::Models { kind } => {
            let k = kind
                .as_deref()
                .map(|k| {
                    Kind::parse(k)
                        .ok_or_else(|| Fail::new("argument.invalid", "kind is image, video, voice, music or sfx"))
                })
                .transpose()?;
            let store = OsKeyStore;
            let ms: Vec<_> =
                ove_generate::catalog::models().into_iter().filter(|m| k.is_none_or(|k| m.kind == k)).collect();
            let text = ms
                .iter()
                .map(|m| {
                    format!(
                        "{}\t{}\t{}{}\t{}{}",
                        m.id,
                        m.kind.as_str(),
                        m.label,
                        if m.default { " (default)" } else { "" },
                        match m.price {
                            ove_generate::catalog::Price::Image { usd } => format!("${usd}/image"),
                            ove_generate::catalog::Price::Second { usd } => format!("${usd}/s"),
                            ove_generate::catalog::Price::KiloChar { usd } => format!("${usd}/1k chars"),
                            ove_generate::catalog::Price::Unknown => "?".into(),
                        },
                        if store.get(m.provider).is_none() {
                            format!("\t(no {} key)", m.provider.label())
                        } else {
                            String::new()
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok((
                json!({"models": ms, "keys": ove_generate::keys::status(&store)}),
                format!("{text}\nPrices are list prices to estimate cost; your provider bills you."),
            ))
        }
        GenerateCmd::Voices { search } => {
            let (key, _) = OsKeyStore
                .get(Provider::ElevenLabs)
                .ok_or_else(|| Fail::new("key.missing", "no ElevenLabs API key (`ove generate key set elevenlabs`)"))?;
            let v = ove_generate::providers::elevenlabs::voices(&key, search.as_deref())
                .map_err(|e| Fail::new(&e.code, e.message))?;
            let text = v
                .iter()
                .map(|x| format!("{}\t{}\t{}", x.id, x.name, x.category.as_deref().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\n");
            Ok((json!({"voices": v}), text))
        }
        GenerateCmd::Key { cmd } => {
            let store = OsKeyStore;
            match cmd {
                KeyCmd::Status => {
                    let s = ove_generate::keys::status(&store);
                    let text = s
                        .iter()
                        .map(|k| {
                            format!(
                                "{}\t{}",
                                k["label"].as_str().unwrap_or(""),
                                if k["hasKey"] == true {
                                    format!("set ({})", k["source"].as_str().unwrap_or(""))
                                } else {
                                    format!("not set — {}", k["keyUrl"].as_str().unwrap_or(""))
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    Ok((json!({"providers": s}), text))
                }
                KeyCmd::Set { provider: p } => {
                    let p = provider(p)?;
                    let mut key = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut key)
                        .map_err(|e| Fail::new("stdin", e.to_string()))?;
                    store.set(p, &key).map_err(|e| Fail::new("key.invalid", e))?;
                    Ok((
                        json!({"provider": p.id(), "saved": true}),
                        format!("saved the {} key in the system credential store", p.label()),
                    ))
                }
                KeyCmd::Clear { provider: p } => {
                    let p = provider(p)?;
                    store.clear(p).map_err(|e| Fail::new("key.clear", e))?;
                    Ok((json!({"provider": p.id(), "cleared": true}), format!("removed the {} key", p.label())))
                }
            }
        }
    }
}
