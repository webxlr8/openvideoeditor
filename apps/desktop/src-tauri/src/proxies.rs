//! Proxies in the app (add-background-tasks-and-media-derivatives, task group 5): the project's "Use proxies"
//! setting, the proxy task (a P2 derivative task in the decode class), and which file playback, skim sheets and
//! analysis read. Export and agent frames never read proxies (`ove_render::Renderer` reads them only when told to,
//! and only timeline playback tells it).
//!
//! A proxy task first checks whether the file needs a proxy (`ove_media::proxy::check`: 4K, 10-bit long-GOP, or slower
//! than real time to decode) and keeps the answer in `cache/proxy-check/<contentId>.json`, so the decode check runs
//! once per file. A file that needs one gets `proxies/<contentId>_1080.mp4`, written through the export pipeline.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use ove_core::ops::Op;
use ove_media::proxy::Check;
use ove_tasks::{Class, Priority, Scheduler, TaskCtx, TaskError, TaskFilter, TaskId, TaskSpec};
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::error::AppResult;
use crate::projects::{ProjectView, edit, serial};
use crate::worker::relock;

/// Peak memory of a proxy encode: decoder state for a 4K source, a few 1080p RGBA frames and the encoder.
const PROXY_MEM: u64 = 512 * 1024 * 1024;

/// Content ids a proxy task was queued for in the open project this session.
#[derive(Default)]
pub struct Proxies {
    queued: Mutex<(PathBuf, BTreeSet<String>)>,
}

impl Proxies {
    /// Forgets what was queued, so the next media check queues missing proxies again (after a clear).
    pub fn forget(&self, bundle: &Path) {
        let mut q = relock(&self.queued);
        if q.0 == bundle {
            q.1.clear();
        }
    }

    /// Marks `cid` queued; false when it already was.
    fn mark(&self, bundle: &Path, cid: &str) -> bool {
        let mut q = relock(&self.queued);
        if q.0 != bundle {
            *q = (bundle.to_path_buf(), BTreeSet::new());
        }
        q.1.insert(cid.to_string())
    }
}

/// Where a file's proxy check is kept.
pub fn check_path(bundle: &Path, content_id: &str) -> PathBuf {
    bundle.join("cache").join("proxy-check").join(format!("{content_id}.json"))
}

/// The stored check of the current version, if any.
pub fn read_check(bundle: &Path, content_id: &str) -> Option<Check> {
    let c: Check = serde_json::from_slice(&std::fs::read(check_path(bundle, content_id)).ok()?).ok()?;
    (c.version == ove_media::proxy::VERSION).then_some(c)
}

/// What a proxy task ended with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The proxy exists.
    Ready,
    /// The file plays well without one.
    NotNeeded,
}

/// Queues the proxy of a video: checks whether it needs one (once per content), then writes
/// `proxies/<contentId>_1080.mp4`. Returns the task, or the one already queued for the same content.
pub fn queue_proxy(
    scheduler: &Scheduler,
    project: &str,
    bundle: &Path,
    content_id: &str,
    source: PathBuf,
    done: impl FnOnce(Outcome) + Send + 'static,
) -> TaskId {
    let name = source.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let out = ove_media::proxy::path(bundle, content_id);
    let (b, cid) = (bundle.to_path_buf(), content_id.to_string());
    let spec = TaskSpec::new("proxy", format!("Proxy: {name}"), Priority::Derivative, Class::Decode)
        .project(project)
        .memory(PROXY_MEM)
        .key(format!("proxy:{}", out.display()));
    scheduler.submit(spec, move |ctx| {
        ctx.checkpoint()?;
        if out.is_file() {
            done(Outcome::Ready);
            return Ok(());
        }
        let fail = |e: String| TaskError::Failed(format!("No proxy for “{name}”: {e}"));
        let check = match read_check(&b, &cid) {
            Some(c) => c,
            None => {
                let info = ove_media::probe(&source).map_err(|e| fail(e.to_string()))?;
                let c = ove_media::proxy::check(&source, &info).map_err(|e| fail(e.to_string()))?;
                let bytes = serde_json::to_vec(&c).map_err(|e| fail(e.to_string()))?;
                ove_tasks::write_output::<std::io::Error>(check_path(&b, &cid), |p| std::fs::write(p, &bytes))
                    .map_err(|e| fail(e.to_string()))?;
                c
            }
        };
        if !check.needed() {
            done(Outcome::NotNeeded);
            return Ok(());
        }
        ctx.checkpoint()?;
        let r = encode_beside(ctx, source, out)?;
        match r {
            Ok(r) => {
                tracing::info!(file = %name, encoder = ?r.encoder, seconds = r.seconds_taken, "proxy made");
                done(Outcome::Ready);
                Ok(())
            }
            Err(ove_render::ExportError::Cancelled) => Err(TaskError::Cancelled),
            Err(e) => Err(fail(e.to_string())),
        }
    })
}

/// How often the task looks at its encoder thread.
const POLL: Duration = Duration::from_millis(20);

/// Runs the encode on a helper thread, so a cancelled task ends within one poll even while the encoder is still
/// setting up (opening a hardware decoder or encoder takes hundreds of milliseconds and cannot be interrupted). The
/// helper stops at its next frame and the export pipeline removes its partial file; the final proxy name only ever
/// holds a complete file. While the task is paused, the helper waits before its next frame.
fn encode_beside(
    ctx: &TaskCtx,
    source: PathBuf,
    out: PathBuf,
) -> Result<Result<ove_render::proxy::ProxyResult, ove_render::ExportError>, TaskError> {
    let token = ctx.cancel_token().clone();
    let progress = Arc::new(AtomicU64::new(0f64.to_bits()));
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (tx, rx) = channel();
    let (t, p, g) = (token.clone(), progress.clone(), gate.clone());
    let spawned = std::thread::Builder::new().name("proxy-encode".into()).spawn(move || {
        ove_tasks::os::enter_background();
        let r = ove_render::proxy::make(&source, &out, |f| {
            p.store(f.to_bits(), Ordering::Relaxed);
            let (held, cv) = &*g;
            let mut h = relock(held);
            while *h && !t.is_cancelled() {
                h = cv.wait_timeout(h, POLL).map(|(h, _)| h).unwrap_or_else(|e| e.into_inner().0);
            }
            !t.is_cancelled()
        });
        let _ = tx.send(r);
    });
    if let Err(e) = spawned {
        return Err(TaskError::Failed(format!("the proxy encoder could not start: {e}")));
    }
    let hold = |on: bool| {
        *relock(&gate.0) = on;
        gate.1.notify_all();
    };
    loop {
        match rx.recv_timeout(POLL) {
            Ok(r) => return Ok(r),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(TaskError::Failed("the proxy encoder stopped unexpectedly".into()));
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        ctx.progress(f64::from_bits(progress.load(Ordering::Relaxed)));
        // The helper waits while the checkpoint blocks (a pause); a cancel returns at once.
        hold(true);
        let c = ctx.checkpoint();
        hold(false);
        c?;
    }
}

/// The file skim sheets and analysis read for a media file: its proxy when the project uses proxies and the proxy
/// exists, else the original.
pub fn derivative_source(bundle: &Path, doc: &ove_core::Document, content_id: &str, original: PathBuf) -> PathBuf {
    if ove_render::proxy::enabled(doc)
        && let Some(p) = ove_media::proxy::existing(bundle, content_id)
    {
        return p;
    }
    original
}

/// Queues proxies of the open project's online videos when "Use proxies" is on: once per content per session, and
/// only where no proxy exists. `online` holds (media id, content id, original path) of online videos.
pub fn queue_missing(app: &AppHandle, bundle: &Path, doc: &ove_core::Document, online: Vec<(String, PathBuf)>) {
    if !ove_render::proxy::enabled(doc) {
        return;
    }
    let state = app.state::<AppState>();
    let proxies = app.state::<Proxies>();
    let project = crate::tasks::project_key(bundle);
    for (cid, source) in online {
        if ove_media::proxy::existing(bundle, &cid).is_some() || !proxies.mark(bundle, &cid) {
            continue;
        }
        let a = app.clone();
        queue_proxy(&state.tasks, &project, bundle, &cid, source, move |outcome| {
            if outcome == Outcome::Ready {
                // The timeline is rebuilt with the new proxy (playback picks it up).
                crate::projects::emit_view_soon(&a);
            }
        });
    }
}

/// Turns the project's "Use proxies" setting on or off (one revision, through the document pipeline). Turning it
/// off cancels queued proxy tasks; finished proxies stay on disk (Settings → Storage clears them).
#[tauri::command]
#[specta::specta]
pub async fn set_use_proxies(app: AppHandle, on: bool) -> AppResult<ProjectView> {
    let v = serial(&app, "set_use_proxies", move |app, state| {
        let op = if on {
            let mut set = Map::new();
            set.insert(ove_render::proxy::SETTING.into(), Value::Bool(true));
            Op::UpdateProject { set, unset: vec![] }
        } else {
            Op::UpdateProject { set: Map::new(), unset: vec![ove_render::proxy::SETTING.into()] }
        };
        let v = edit(app, state, vec![op], if on { "use proxies" } else { "stop using proxies" })?;
        if !on {
            let filter = TaskFilter { project: Some(v.bundle.clone()), kind: Some("proxy".into()) };
            state.tasks.cancel_matching(&filter);
            app.state::<Proxies>().forget(Path::new(&v.bundle));
        }
        Ok(v)
    })
    .await?;
    crate::projects::refresh_media(&app);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small clip needs no proxy: the check is stored, no proxy is written, and a second task reuses the check.
    #[test]
    fn a_file_that_plays_well_gets_no_proxy_and_the_check_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let clip = crate::tasks::testing::video(dir.path(), "clip.mp4");
        let bundle = dir.path().join("P.ove");
        let s = crate::tasks::testing::scheduler();
        let (tx, rx) = std::sync::mpsc::channel();
        let t = tx.clone();
        let id = queue_proxy(&s, "p", &bundle, "c1", clip.clone(), move |o| t.send(o).unwrap());
        assert_eq!(s.wait_task(id, Duration::from_secs(60)).unwrap().state, ove_tasks::TaskState::Done);
        assert_eq!(rx.recv().unwrap(), Outcome::NotNeeded);
        let c = read_check(&bundle, "c1").expect("the check is stored");
        assert!(!c.needed() && c.decode_speed.is_some());
        assert!(ove_media::proxy::existing(&bundle, "c1").is_none());
        // A stored "needed" answer is trusted, so the second task encodes without checking again.
        let forced = Check {
            version: ove_media::proxy::VERSION,
            reason: Some(ove_media::proxy::Reason::SlowDecode),
            decode_speed: Some(0.5),
        };
        std::fs::write(check_path(&bundle, "c1"), serde_json::to_vec(&forced).unwrap()).unwrap();
        let id = queue_proxy(&s, "p", &bundle, "c1", clip, move |o| tx.send(o).unwrap());
        assert_eq!(s.wait_task(id, Duration::from_secs(60)).unwrap().state, ove_tasks::TaskState::Done);
        assert_eq!(rx.recv().unwrap(), Outcome::Ready);
        let proxy = ove_media::proxy::existing(&bundle, "c1").expect("a proxy");
        let info = ove_media::probe(&proxy).unwrap();
        assert_eq!(info.video.unwrap().codec, "h264");
        let left: Vec<_> =
            std::fs::read_dir(proxy.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "only the proxy: {left:?}");
    }

    /// Cancelling a running proxy task stops it and removes its partial file.
    #[test]
    fn a_cancelled_proxy_task_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let clip = crate::tasks::testing::long_video(dir.path(), "long.mp4", 20.0);
        let bundle = dir.path().join("P.ove");
        let forced = Check {
            version: ove_media::proxy::VERSION,
            reason: Some(ove_media::proxy::Reason::SlowDecode),
            decode_speed: None,
        };
        std::fs::create_dir_all(check_path(&bundle, "c2").parent().unwrap()).unwrap();
        std::fs::write(check_path(&bundle, "c2"), serde_json::to_vec(&forced).unwrap()).unwrap();
        let s = crate::tasks::testing::scheduler();
        let id = queue_proxy(&s, "p", &bundle, "c2", clip, |_| {});
        // Cancel once it is running.
        let started = std::time::Instant::now();
        while s.get(id).is_some_and(|t| t.state != ove_tasks::TaskState::Running) {
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        let asked = std::time::Instant::now();
        s.cancel(id);
        let end = s.wait_task(id, Duration::from_secs(10)).unwrap();
        let took = asked.elapsed();
        assert!(matches!(end.state, ove_tasks::TaskState::Cancelled | ove_tasks::TaskState::Done), "{end:?}");
        if end.state == ove_tasks::TaskState::Cancelled {
            assert!(ove_media::proxy::existing(&bundle, "c2").is_none());
            // The encoder thread removes its partial file when it notices (at its next frame, or after set-up).
            let dir = bundle.join("proxies");
            let left = || -> Vec<_> {
                std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.file_name()).collect()).unwrap_or_default()
            };
            let waited = std::time::Instant::now();
            while !left().is_empty() && waited.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(left().is_empty(), "{:?}", left());
            assert!(took < Duration::from_millis(500), "cancel took {took:?}");
            println!("proxy task cancel took {took:?}");
        }
    }

    #[test]
    fn derivative_work_reads_the_proxy_only_when_the_setting_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("P.ove");
        std::fs::create_dir_all(bundle.join("proxies")).unwrap();
        std::fs::write(ove_media::proxy::path(&bundle, "c3"), b"x").unwrap();
        let original = PathBuf::from("original.mp4");
        let mut doc = crate::tasks::testing::title_doc(1.0);
        assert_eq!(derivative_source(&bundle, &doc, "c3", original.clone()), original);
        doc.project.extra.insert("useProxies".into(), Value::Bool(true));
        assert_eq!(derivative_source(&bundle, &doc, "c3", original.clone()), ove_media::proxy::path(&bundle, "c3"));
        assert_eq!(derivative_source(&bundle, &doc, "missing", original.clone()), original, "no proxy yet");
    }
}
