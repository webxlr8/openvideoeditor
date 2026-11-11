//! Generation jobs: what was asked, where it goes on the timeline, and how it went. Jobs live in the project bundle
//! (`cache/jobs/<id>.json`) so any process — the app, `ove`, a background worker — can follow or finish them.
//! Outputs are saved in the bundle (`media/generated/`), owned by it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::Kind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl Status {
    pub fn done(self) -> bool {
        matches!(self, Status::Succeeded | Status::Failed | Status::Cancelled)
    }
}

/// What to make.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Request {
    pub prompt: String,
    /// Model id from the catalog (or `fal:<endpoint>`); `None` = the kind's default.
    pub model: Option<String>,
    /// Seconds (video, music, sound effects; voice: a target the speech is not stretched to).
    pub duration: Option<f64>,
    /// `16:9`, `9:16`, `1:1`, … (image, video). Default: the project's aspect.
    pub aspect: Option<String>,
    /// First frame / reference image (path).
    pub image: Option<PathBuf>,
    /// Voice id or name (voice).
    pub voice: Option<String>,
    /// Voice settings.
    pub speed: Option<f64>,
    pub stability: Option<f64>,
    pub style: Option<f64>,
    pub negative: Option<String>,
    pub seed: Option<u64>,
    /// Instrumental only (music).
    pub instrumental: bool,
    /// Placement: timeline start (seconds) and track (name or id).
    pub start: Option<f64>,
    pub track: Option<String>,
    /// Clip name.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub kind: Kind,
    pub model: String,
    pub request: Request,
    pub status: Status,
    /// 0–1 when known.
    pub progress: Option<f64>,
    /// The placeholder item this job fills.
    pub item_id: Option<String>,
    /// Estimated cost in USD shown before starting.
    pub estimate_usd: Option<f64>,
    /// Provider request id (fal), for cancel and diagnostics.
    pub remote_id: Option<String>,
    /// Output file, relative to the bundle.
    pub output: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Process running it.
    pub worker_pid: Option<u32>,
    /// Provider-specific details (seconds generated, voice used, …).
    #[serde(default)]
    pub details: Value,
}

pub fn jobs_dir(bundle: &Path) -> PathBuf {
    bundle.join("cache").join("jobs")
}

pub fn output_dir(bundle: &Path) -> PathBuf {
    bundle.join("media").join("generated")
}

pub fn new_id(now_ms: i64) -> String {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    format!("g_{now_ms:x}_{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

pub fn save(bundle: &Path, job: &Job) -> std::io::Result<()> {
    let dir = jobs_dir(bundle);
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{}.json.tmp", job.id));
    std::fs::write(&tmp, serde_json::to_vec_pretty(job).unwrap_or_default())?;
    std::fs::rename(tmp, dir.join(format!("{}.json", job.id)))
}

pub fn load(bundle: &Path, id: &str) -> Option<Job> {
    if id.contains(['/', '\\']) {
        return None;
    }
    serde_json::from_slice(&std::fs::read(jobs_dir(bundle).join(format!("{id}.json"))).ok()?).ok()
}

/// All jobs of a project, newest first.
pub fn list(bundle: &Path) -> Vec<Job> {
    let mut v: Vec<Job> = std::fs::read_dir(jobs_dir(bundle))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|j: &Job| std::cmp::Reverse(j.created_at));
    v
}

/// A job whose id starts with `q`, in any project of `bundles`.
pub fn find<'a>(bundles: impl IntoIterator<Item = &'a Path>, q: &str) -> Option<(PathBuf, Job)> {
    bundles
        .into_iter()
        .find_map(|b| list(b).into_iter().find(|j| j.id == q || j.id.starts_with(q)).map(|j| (b.to_path_buf(), j)))
}

/// Marks a cancel request: the worker checks for this file between polls.
pub fn request_cancel(bundle: &Path, id: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(jobs_dir(bundle))?;
    std::fs::write(jobs_dir(bundle).join(format!("{id}.cancel")), b"")
}

pub fn cancel_requested(bundle: &Path, id: &str) -> bool {
    jobs_dir(bundle).join(format!("{id}.cancel")).exists()
}
