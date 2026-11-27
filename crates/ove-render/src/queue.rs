//! The export queue (change extend-export-formats-and-queue, design D5–D7): one queue for every project, kept as job
//! files in the app's data folder (`exports/<job-id>.json`) so the app, `ove` and MCP see the same jobs and a restart
//! loses nothing. A detached worker process (`ove export --worker`) runs the jobs one at a time in queue order; it
//! renders the document as it was at the revision the job was queued at (the caller of [`run_worker`] supplies the
//! document for a job).
//!
//! Who writes what, so two processes never overwrite each other's state: the worker writes the job it runs (status,
//! progress, result); a cancel of a running job only leaves a marker file (`<id>.cancel`) that the worker acts on; a
//! cancel of a queued job writes the marker and the `cancelled` status, and the worker re-reads both before it starts
//! a job. Job files are replaced atomically (written to a temporary file, then renamed).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ove_core::model::Document;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::export::{ExportError, ExportSettings, Progress};

/// Environment variable overriding the queue folder (tests, portable installs).
pub const QUEUE_DIR_ENV: &str = "OVE_EXPORT_QUEUE_DIR";
/// A running job whose worker process is gone is put back in the queue once its file has not changed for this long.
pub const DEAD_WORKER_GRACE_MS: i64 = 30_000;
/// Environment variable overriding [`DEAD_WORKER_GRACE_MS`] (tests).
pub const DEAD_WORKER_GRACE_ENV: &str = "OVE_EXPORT_DEAD_GRACE_MS";
/// Starts a job may make before it is marked failed.
pub const MAX_ATTEMPTS: u32 = 3;
/// The job file is rewritten at most this often while a job runs (4 times a second).
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// How often a running job looks for a cancel request.
pub const CANCEL_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn finished(self) -> bool {
        matches!(self, JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled)
    }

    pub fn name(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }
}

/// One export in the queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportJob {
    pub id: String,
    /// The project bundle.
    pub project: PathBuf,
    pub project_name: String,
    /// The revision the job renders (the current one when it was queued).
    pub rev: i64,
    pub settings: ExportSettings,
    pub out: PathBuf,
    pub status: JobStatus,
    /// 0–1.
    pub progress: f64,
    pub frame: u64,
    pub total_frames: u64,
    /// The encoder in use, once known.
    pub encoder: Option<String>,
    /// The export result (`ExportResult` as JSON) when done.
    pub result: Option<Value>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    /// The refusal or failure code when failed (`export.refused`, `export.failed`, …).
    pub error_code: Option<String>,
    /// Who queued it: "person", "assistant", "cli", or an MCP client's name.
    pub requested_by: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
    pub worker_pid: Option<u32>,
    /// Starts so far (a crashed worker's job restarts from the beginning).
    pub attempt: u32,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A new job id: `x_<ms hex>_<random hex>` (`x` for export).
pub fn new_id(now: i64) -> String {
    let rand = {
        // Process id and a counter make ids made in the same millisecond differ.
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&(std::process::id(), n, now, Instant::now()), &mut h);
        std::hash::Hasher::finish(&h) as u32
    };
    format!("x_{now:x}_{rand:08x}")
}

/// Export job ids are plain names (they become file names).
pub fn is_job_id(id: &str) -> bool {
    id.starts_with("x_") && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn bad_id(id: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{id:?} is not an export job id"))
}

/// The queue in one folder.
#[derive(Debug, Clone)]
pub struct Queue {
    dir: PathBuf,
}

impl Queue {
    pub fn new(dir: impl Into<PathBuf>) -> Queue {
        Queue { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn cancel_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.cancel"))
    }

    /// Writes a job file atomically.
    pub fn save(&self, job: &ExportJob) -> std::io::Result<()> {
        if !is_job_id(&job.id) {
            return Err(bad_id(&job.id));
        }
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!("{}.json.{}.tmp", job.id, std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec_pretty(job).unwrap_or_default())?;
        // Windows refuses to replace a file another process is reading at that instant: retry briefly.
        let mut tries = 0;
        loop {
            match std::fs::rename(&tmp, self.path(&job.id)) {
                Ok(()) => return Ok(()),
                Err(e) if tries < 20 => {
                    tries += 1;
                    tracing::debug!(job = %job.id, error = %e, "job file busy; retrying");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e);
                }
            }
        }
    }

    /// A job by exact id.
    pub fn load(&self, id: &str) -> Option<ExportJob> {
        if !is_job_id(id) {
            return None;
        }
        // A reader can meet the file mid-rename on Windows: try a few times.
        for _ in 0..5 {
            if let Ok(bytes) = std::fs::read(self.path(id)) {
                return serde_json::from_slice::<ExportJob>(&bytes).ok().filter(|j| j.id == id);
            }
            if !self.path(id).exists() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

    /// Every job, oldest first (queue order).
    pub fn list(&self) -> Vec<ExportJob> {
        let mut v: Vec<ExportJob> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let id = name.strip_suffix(".json")?;
                        self.load(id)
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        v
    }

    /// A job by id or unique id prefix.
    pub fn find(&self, q: &str) -> Option<ExportJob> {
        self.load(q).or_else(|| {
            let mut m: Vec<ExportJob> = self.list().into_iter().filter(|j| j.id.starts_with(q)).collect();
            if m.len() == 1 { m.pop() } else { None }
        })
    }

    /// Adds a job to the end of the queue.
    pub fn enqueue(
        &self,
        project: &Path,
        project_name: &str,
        rev: i64,
        settings: ExportSettings,
        out: &Path,
        requested_by: &str,
    ) -> std::io::Result<ExportJob> {
        let now = now_ms();
        let job = ExportJob {
            id: new_id(now),
            project: project.to_path_buf(),
            project_name: project_name.to_string(),
            rev,
            settings,
            out: out.to_path_buf(),
            status: JobStatus::Queued,
            progress: 0.0,
            frame: 0,
            total_frames: 0,
            encoder: None,
            result: None,
            warnings: Vec::new(),
            error: None,
            error_code: None,
            requested_by: requested_by.to_string(),
            created_at: now,
            started_at: None,
            updated_at: now,
            finished_at: None,
            worker_pid: None,
            attempt: 0,
        };
        self.save(&job)?;
        Ok(job)
    }

    pub fn cancel_requested(&self, id: &str) -> bool {
        is_job_id(id) && self.cancel_path(id).exists()
    }

    /// Cancels a queued or running job. A queued job is cancelled at once; a running one stops within a couple of
    /// seconds (its worker sees the request). Refused for a finished job, whose output stays.
    pub fn cancel(&self, id: &str) -> Result<ExportJob, CancelError> {
        let mut job = self.find(id).ok_or_else(|| CancelError::Unknown(id.to_string()))?;
        if job.status.finished() {
            return Err(CancelError::Finished(Box::new(job)));
        }
        std::fs::write(self.cancel_path(&job.id), b"").map_err(CancelError::Io)?;
        if job.status == JobStatus::Queued {
            job.status = JobStatus::Cancelled;
            job.error = Some("cancelled before it started".into());
            job.updated_at = now_ms();
            job.finished_at = Some(job.updated_at);
            self.save(&job).map_err(CancelError::Io)?;
        }
        Ok(job)
    }

    /// Waits until a job has finished (or `timeout` passed) and returns it as last read.
    pub fn wait(&self, id: &str, timeout: Duration, mut on_change: impl FnMut(&ExportJob)) -> Option<ExportJob> {
        let start = Instant::now();
        let mut last_update = -1;
        loop {
            let job = self.load(id)?;
            if job.updated_at != last_update {
                last_update = job.updated_at;
                on_change(&job);
            }
            if job.status.finished() || start.elapsed() >= timeout {
                return Some(job);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The worker lock file: present while a worker runs, holding its process id.
    fn worker_lock(&self) -> PathBuf {
        self.dir.join("worker.lock")
    }

    /// The process id of the running worker, if one is alive.
    pub fn live_worker(&self) -> Option<u32> {
        let pid: u32 = std::fs::read_to_string(self.worker_lock()).ok()?.trim().parse().ok()?;
        (pid == std::process::id() || process_alive(pid)).then_some(pid)
    }

    /// Takes the worker lock; `None` when another live worker holds it.
    pub fn lock_worker(&self) -> std::io::Result<Option<WorkerLock>> {
        std::fs::create_dir_all(&self.dir)?;
        for _ in 0..2 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(self.worker_lock()) {
                Ok(mut f) => {
                    use std::io::Write;
                    write!(f, "{}", std::process::id())?;
                    return Ok(Some(WorkerLock { path: self.worker_lock() }));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if self.live_worker().is_some() {
                        return Ok(None);
                    }
                    // A dead worker's lock.
                    let _ = std::fs::remove_file(self.worker_lock());
                }
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    /// Puts back the jobs whose worker died (running, worker gone, file unchanged for the grace period) with their
    /// partial output removed, or fails them after [`MAX_ATTEMPTS`] starts. Returns the jobs changed.
    pub fn recover(&self) -> Vec<ExportJob> {
        let grace = std::env::var(DEAD_WORKER_GRACE_ENV)
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(DEAD_WORKER_GRACE_MS);
        let now = now_ms();
        let mut changed = Vec::new();
        for mut job in self.list().into_iter().filter(|j| j.status == JobStatus::Running) {
            let alive = job.worker_pid.is_some_and(|p| p == std::process::id() || process_alive(p));
            if alive || now - job.updated_at < grace {
                continue;
            }
            let _ = std::fs::remove_file(crate::export::partial_path(&job.out));
            job.worker_pid = None;
            job.updated_at = now;
            if job.attempt >= MAX_ATTEMPTS {
                job.status = JobStatus::Failed;
                job.error = Some(format!(
                    "the export stopped {} times before finishing (the program running it closed or crashed); \
                     export again, and if it keeps stopping, report it with the log",
                    job.attempt
                ));
                job.error_code = Some("export.crashed".into());
                job.finished_at = Some(now);
            } else {
                job.status = JobStatus::Queued;
                job.progress = 0.0;
                job.frame = 0;
                job.warnings.push(format!("restarted from the beginning after attempt {} stopped", job.attempt));
            }
            if self.save(&job).is_ok() {
                tracing::warn!(job = %job.id, status = job.status.name(), "export worker gone; job recovered");
                changed.push(job);
            }
        }
        changed
    }

    /// The oldest queued job that has no cancel request.
    pub fn next_queued(&self) -> Option<ExportJob> {
        self.list().into_iter().find(|j| j.status == JobStatus::Queued && !self.cancel_requested(&j.id))
    }

    /// Removes finished jobs' files (and their cancel markers); returns how many.
    pub fn clear_finished(&self) -> usize {
        let mut n = 0;
        for j in self.list().into_iter().filter(|j| j.status.finished()) {
            if std::fs::remove_file(self.path(&j.id)).is_ok() {
                n += 1;
            }
            let _ = std::fs::remove_file(self.cancel_path(&j.id));
        }
        n
    }
}

/// Removes the worker lock when the worker ends.
pub struct WorkerLock {
    path: PathBuf,
}

impl Drop for WorkerLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CancelError {
    #[error("there is no export job {0}")]
    Unknown(String),
    #[error("export job {} has already {}; its output is kept", .0.id, .0.status.name())]
    Finished(Box<ExportJob>),
    #[error("could not cancel: {0}")]
    Io(std::io::Error),
}

/// What a job renders: the document at its pinned revision, or why it cannot be had (then the job fails with that
/// message).
pub type Loaded = Result<(Document, Option<PathBuf>), String>;

/// Runs jobs until none is queued. `load` gives a job's document; `release` is called once a job has finished (to
/// drop its revision pin). Returns the number of jobs run.
pub fn run_worker(
    queue: &Queue,
    mut load: impl FnMut(&ExportJob) -> Loaded,
    mut release: impl FnMut(&ExportJob),
) -> std::io::Result<usize> {
    let Some(_lock) = queue.lock_worker()? else {
        tracing::info!("another export worker is running");
        return Ok(0);
    };
    let mut ran = 0;
    loop {
        queue.recover();
        let Some(job) = queue.next_queued() else { break };
        run_job(queue, job, &mut load, &mut release)?;
        ran += 1;
    }
    // Cancelled queued jobs release their pins too.
    for j in queue.list().into_iter().filter(|j| j.status == JobStatus::Cancelled && j.started_at.is_none()) {
        release(&j);
    }
    Ok(ran)
}

fn run_job(
    queue: &Queue,
    mut job: ExportJob,
    load: &mut impl FnMut(&ExportJob) -> Loaded,
    release: &mut impl FnMut(&ExportJob),
) -> std::io::Result<()> {
    let now = now_ms();
    job.status = JobStatus::Running;
    job.attempt += 1;
    job.worker_pid = Some(std::process::id());
    job.started_at = Some(now);
    job.updated_at = now;
    job.progress = 0.0;
    job.error = None;
    queue.save(&job)?;
    tracing::info!(job = %job.id, attempt = job.attempt, out = %job.out.display(), "export started");

    // Cancel requests arrive as a marker file; a watcher turns them into the flag the export polls every frame.
    let cancel = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (q, id, cancel, finished) = (queue.clone(), job.id.clone(), cancel.clone(), finished.clone());
        std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                if q.cancel_requested(&id) {
                    cancel.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::sleep(CANCEL_POLL);
            }
        })
    };
    let outcome = match load(&job) {
        Err(message) => Err(("export.revision", message)),
        Ok((doc, bundle)) => {
            let mut last_write = Instant::now() - PROGRESS_INTERVAL;
            let mut live = job.clone();
            let r = crate::export::export(&doc, bundle.as_deref(), &job.out, &job.settings, &cancel, |p: Progress| {
                live.progress = p.fraction;
                live.frame = p.frame;
                live.total_frames = p.total_frames;
                live.encoder = Some(p.encoder);
                if last_write.elapsed() >= PROGRESS_INTERVAL {
                    last_write = Instant::now();
                    live.updated_at = now_ms();
                    let _ = queue.save(&live);
                }
            });
            job = live;
            r.map_err(|e| {
                let code = match &e {
                    ExportError::Refused(_) => "export.refused",
                    ExportError::Invalid(_) => "export.invalid",
                    ExportError::Cancelled => "export.cancelled",
                    ExportError::Ffmpeg(_) => "export.failed",
                };
                (code, e.to_string())
            })
        }
    };
    finished.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    let now = now_ms();
    job.updated_at = now;
    job.finished_at = Some(now);
    job.worker_pid = None;
    match outcome {
        Ok(r) => {
            job.status = JobStatus::Done;
            job.progress = 1.0;
            if let Some(w) = &r.encoder_warning {
                job.warnings.push(w.clone());
            }
            job.encoder = r.video_encoder.clone().or_else(|| r.audio_encoder.clone());
            job.result = serde_json::to_value(&r).ok();
            tracing::info!(job = %job.id, seconds = r.seconds_taken, "export finished");
        }
        Err((code, message)) => {
            job.status = if code == "export.cancelled" { JobStatus::Cancelled } else { JobStatus::Failed };
            job.error = Some(message);
            job.error_code = Some(code.to_string());
            tracing::warn!(job = %job.id, code, "export did not finish");
        }
    }
    queue.save(&job)?;
    release(&job);
    Ok(())
}

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

/// Marks this process's standard handles as not inheritable, so a detached child does not hold a caller's pipe open
/// (`ove export --detach` run with its output captured would otherwise not finish until the worker does). Children
/// that inherit standard streams on purpose still get them: the standard library duplicates them for the child.
#[cfg(windows)]
fn no_inherit_std_handles() {
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation};
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: `GetStdHandle` returns this process's handle (or null / invalid, skipped); changing its inherit flag
        // affects only later child processes.
        unsafe {
            let h = GetStdHandle(which);
            if !h.is_null() && h != INVALID_HANDLE_VALUE {
                SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

/// Starts `program args…` as a detached worker: it keeps running when the caller (the app window, a terminal) goes
/// away, with no console window and no inherited input or output.
pub fn spawn_detached(program: &Path, args: &[&str], envs: &[(&str, &Path)]) -> std::io::Result<u32> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    no_inherit_std_handles();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: a terminal's hang-up or Ctrl+C does not reach it.
        cmd.process_group(0);
    }
    let child = cmd.spawn()?;
    Ok(child.id())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn job_queue() -> (tempfile::TempDir, Queue) {
        let d = tempfile::tempdir().unwrap();
        let q = Queue::new(d.path().join("exports"));
        (d, q)
    }

    fn add(q: &Queue, name: &str) -> ExportJob {
        q.enqueue(Path::new("/p.ove"), "P", 1, ExportSettings::default(), Path::new(name), "cli").unwrap()
    }

    #[test]
    fn jobs_list_in_queue_order_and_find_by_prefix() {
        let (_d, q) = job_queue();
        let a = add(&q, "a.mp4");
        let b = add(&q, "b.mp4");
        let ids: Vec<String> = q.list().into_iter().map(|j| j.id).collect();
        assert_eq!(ids, [a.id.clone(), b.id.clone()]);
        assert_eq!(q.next_queued().unwrap().id, a.id);
        assert_eq!(q.find(&b.id).unwrap().out, Path::new("b.mp4"));
        assert!(q.load("../evil").is_none() && !is_job_id("g_123") && is_job_id(&a.id));
    }

    #[test]
    fn cancel_a_queued_job_and_refuse_a_finished_one() {
        let (_d, q) = job_queue();
        let a = add(&q, "a.mp4");
        let c = q.cancel(&a.id).unwrap();
        assert_eq!(c.status, JobStatus::Cancelled);
        assert!(q.next_queued().is_none(), "a cancelled job never starts");
        match q.cancel(&a.id) {
            Err(CancelError::Finished(j)) => assert_eq!(j.status, JobStatus::Cancelled),
            other => panic!("{other:?}"),
        }
        match q.cancel("x_nope") {
            Err(e @ CancelError::Unknown(_)) => assert!(e.to_string().contains("x_nope")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_dead_workers_job_is_requeued_then_failed_after_three_attempts() {
        let (_d, q) = job_queue();
        let mut j = add(&q, "a.mp4");
        let gone = dead_pid();
        for attempt in 1..=MAX_ATTEMPTS {
            j.status = JobStatus::Running;
            j.worker_pid = Some(gone);
            j.attempt = attempt;
            j.updated_at = now_ms() - DEAD_WORKER_GRACE_MS - 1;
            q.save(&j).unwrap();
            let changed = q.recover();
            assert_eq!(changed.len(), 1);
            j = q.load(&j.id).unwrap();
            if attempt < MAX_ATTEMPTS {
                assert_eq!(j.status, JobStatus::Queued, "attempt {attempt}");
            } else {
                assert_eq!(j.status, JobStatus::Failed);
                assert!(j.error.as_deref().unwrap().contains("3 times"));
            }
        }
        // A running job with a live worker, or one updated within the grace period, is left alone.
        let mut k = add(&q, "b.mp4");
        k.status = JobStatus::Running;
        k.worker_pid = Some(std::process::id());
        k.updated_at = 0;
        q.save(&k).unwrap();
        let mut other = add(&q, "c.mp4");
        other.status = JobStatus::Running;
        other.worker_pid = Some(gone);
        q.save(&other).unwrap();
        assert!(q.recover().is_empty());
    }

    #[test]
    fn one_worker_at_a_time() {
        let (_d, q) = job_queue();
        let lock = q.lock_worker().unwrap().unwrap();
        assert!(q.lock_worker().unwrap().is_none());
        assert_eq!(q.live_worker(), Some(std::process::id()));
        drop(lock);
        assert!(q.lock_worker().unwrap().is_some());
        // A lock left by a dead process is taken over.
        std::fs::write(q.dir().join("worker.lock"), dead_pid().to_string()).unwrap();
        assert!(q.lock_worker().unwrap().is_some());
    }
}
