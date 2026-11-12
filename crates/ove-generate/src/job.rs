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

/// Refuses ids that are not plain names ([`ove_guard::is_safe_id`]): job ids become file names (job file, cancel
/// marker, output), and they come from job files in the bundle and from the command line.
pub fn check_id(id: &str) -> std::io::Result<()> {
    if ove_guard::is_safe_id(id) {
        Ok(())
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("invalid job id {id:?}")))
    }
}

pub fn save(bundle: &Path, job: &Job) -> std::io::Result<()> {
    check_id(&job.id)?;
    let dir = jobs_dir(bundle);
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{}.json.tmp", job.id));
    std::fs::write(&tmp, serde_json::to_vec_pretty(job).unwrap_or_default())?;
    std::fs::rename(tmp, dir.join(format!("{}.json", job.id)))
}

/// A job by exact id. A file whose recorded id differs from its name is ignored.
pub fn load(bundle: &Path, id: &str) -> Option<Job> {
    check_id(id).ok()?;
    let job: Job = serde_json::from_slice(&std::fs::read(jobs_dir(bundle).join(format!("{id}.json"))).ok()?).ok()?;
    (job.id == id).then_some(job)
}

/// All jobs of a project, newest first. Job files with an unsafe id, or one that differs from the file name, are
/// skipped.
pub fn list(bundle: &Path) -> Vec<Job> {
    let mut v: Vec<Job> = std::fs::read_dir(jobs_dir(bundle))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let path = e.path();
                    let stem = path.file_stem()?.to_str()?.to_string();
                    if path.extension()? != "json" || check_id(&stem).is_err() {
                        return None;
                    }
                    let job: Job = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
                    (job.id == stem).then_some(job)
                })
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
    check_id(id)?;
    std::fs::create_dir_all(jobs_dir(bundle))?;
    std::fs::write(jobs_dir(bundle).join(format!("{id}.cancel")), b"")
}

pub fn cancel_requested(bundle: &Path, id: &str) -> bool {
    check_id(id).is_ok() && jobs_dir(bundle).join(format!("{id}.cancel")).exists()
}

/// An unfinished job not updated for this long is given up whatever its worker (providers report progress at least
/// every few seconds while polling, and a single request times out after 5 minutes).
pub const STALE_AFTER_MS: i64 = 30 * 60_000;
/// A running job whose worker process is gone is given up once it has not been updated for this long (so a worker
/// that just started has saved its first progress).
pub const DEAD_WORKER_GRACE_MS: i64 = 60_000;

/// Whether process `pid` exists on this computer. Errs on the side of "alive" when it cannot tell.
pub fn process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(unix)]
    {
        let Ok(p) = libc::pid_t::try_from(pid) else { return false };
        if p <= 0 {
            return false;
        }
        // SAFETY: signal 0 sends nothing; it only checks that the process exists and may be signalled.
        let r = unsafe { libc::kill(p, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")),
            _ => true,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        true
    }
}

/// Marks unfinished jobs that will never finish as failed: a running job whose worker process is gone (after
/// [`DEAD_WORKER_GRACE_MS`] without an update), and any queued or running job not updated for [`STALE_AFTER_MS`].
/// Returns them, so the caller removes their pending placeholders ([`crate::place::failed_ops`]).
pub fn reap_stale_jobs(bundle: &Path, now_ms: i64) -> Vec<Job> {
    let mut reaped = Vec::new();
    for mut job in list(bundle).into_iter().filter(|j| !j.status.done()) {
        let idle = now_ms - job.updated_at;
        let dead = job.status == Status::Running
            && idle > DEAD_WORKER_GRACE_MS
            && job.worker_pid.is_some_and(|p| !process_alive(p));
        let stale = idle > STALE_AFTER_MS;
        if !(dead || stale) {
            continue;
        }
        job.status = Status::Failed;
        job.error = Some(if dead {
            "the generation stopped before finishing (the program running it closed or crashed)".into()
        } else {
            format!("no progress for {} minutes; given up", STALE_AFTER_MS / 60_000)
        });
        job.updated_at = now_ms;
        if save(bundle, &job).is_ok() {
            tracing::warn!(job = %job.id, "generation job given up: {}", job.error.as_deref().unwrap_or(""));
            reaped.push(job);
        }
    }
    reaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str) -> Job {
        Job {
            id: id.into(),
            kind: Kind::Image,
            model: "m".into(),
            request: Request::default(),
            status: Status::Queued,
            progress: None,
            item_id: None,
            estimate_usd: None,
            remote_id: None,
            output: None,
            error: None,
            created_at: 0,
            updated_at: 0,
            worker_pid: None,
            details: Value::Null,
        }
    }

    #[test]
    fn ids_cannot_escape_the_jobs_folder() {
        let d = tempfile::tempdir().unwrap();
        let bundle = d.path().join("p.ove");
        for id in ["../../x", "..", "a/b", "a\\b", "", "x.y"] {
            assert!(request_cancel(&bundle, id).is_err(), "{id:?}");
            assert!(!cancel_requested(&bundle, id));
            assert!(load(&bundle, id).is_none());
            assert!(save(&bundle, &job(id)).is_err());
        }
        assert!(!d.path().join("x.cancel").exists() && !bundle.join("x.cancel").exists());

        save(&bundle, &job("g_1")).unwrap();
        request_cancel(&bundle, "g_1").unwrap();
        assert!(cancel_requested(&bundle, "g_1"));
        assert_eq!(load(&bundle, "g_1").unwrap().id, "g_1");

        // A job file from elsewhere whose recorded id is a path is ignored by load and list.
        let evil = serde_json::to_vec(&job("../../evil")).unwrap();
        std::fs::write(jobs_dir(&bundle).join("g_2.json"), &evil).unwrap();
        assert!(load(&bundle, "g_2").is_none());
        assert_eq!(list(&bundle).iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["g_1"]);
    }

    /// The id of a process that has exited.
    fn dead_pid() -> u32 {
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    #[test]
    fn stale_and_orphaned_jobs_are_failed() {
        let d = tempfile::tempdir().unwrap();
        let bundle = d.path();
        let now = 10 * STALE_AFTER_MS;
        let put = |id: &str, status: Status, pid: Option<u32>, idle_ms: i64| {
            let mut j = job(id);
            j.status = status;
            j.worker_pid = pid;
            j.updated_at = now - idle_ms;
            save(bundle, &j).unwrap();
        };
        let gone = dead_pid();
        assert!(!process_alive(gone));
        assert!(process_alive(std::process::id()));
        put("g_dead", Status::Running, Some(gone), 2 * 60_000);
        put("g_dead_but_just_started", Status::Running, Some(gone), 5_000);
        put("g_alive", Status::Running, Some(std::process::id()), 2 * 60_000);
        put("g_alive_but_silent", Status::Running, Some(std::process::id()), STALE_AFTER_MS + 1);
        put("g_queued_forever", Status::Queued, None, STALE_AFTER_MS + 1);
        put("g_done", Status::Succeeded, Some(gone), STALE_AFTER_MS + 1);

        let mut reaped: Vec<String> = reap_stale_jobs(bundle, now).into_iter().map(|j| j.id).collect();
        reaped.sort();
        assert_eq!(reaped, ["g_alive_but_silent", "g_dead", "g_queued_forever"]);
        let dead = load(bundle, "g_dead").unwrap();
        assert_eq!(dead.status, Status::Failed);
        assert!(dead.error.unwrap().contains("stopped before finishing"));
        assert_eq!(load(bundle, "g_alive").unwrap().status, Status::Running);
        assert_eq!(load(bundle, "g_dead_but_just_started").unwrap().status, Status::Running);
        assert_eq!(load(bundle, "g_done").unwrap().status, Status::Succeeded);
        assert!(reap_stale_jobs(bundle, now).is_empty(), "reaping is idempotent");
    }
}
