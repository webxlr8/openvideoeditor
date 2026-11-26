//! The app's background task scheduler (crate `ove-tasks`): created once at start-up, paused while the timeline
//! plays and on battery, and used by import (probes and thumbnails), captions and export.

use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::Duration;

use ove_tasks::{Class, Options, PauseReason, Priority, Scheduler, TaskError, TaskId, TaskSpec};
use tauri::AppHandle;
use tauri_specta::Event;

use crate::PlaybackChanged;

/// How often the power source, low-power mode and thermal state are read.
const POWER_POLL: Duration = Duration::from_secs(15);

/// A probe the queue has not started this long after import starts outside the queue, so a paused or busy queue
/// never holds up an import.
pub const PROBE_WATCHDOG: Duration = Duration::from_secs(2);

/// An export the queue has not started this long after the person asked starts outside the queue.
pub const EXPORT_WATCHDOG: Duration = Duration::from_secs(5);

/// Peak memory of making one thumbnail (one decoded frame plus decoder state).
const THUMBNAIL_MEM: u64 = 96 * 1024 * 1024;

/// The scheduler for the app, with its power watcher running.
pub fn new_scheduler() -> Scheduler {
    let s = Scheduler::new(Options::default());
    s.watch_power(POWER_POLL);
    s
}

/// Pauses derivative and analysis work while the timeline plays.
pub fn follow_playback(app: &AppHandle, scheduler: &Scheduler) {
    let s = scheduler.clone();
    PlaybackChanged::listen(app, move |e| s.set_pause(PauseReason::Playback, e.payload.0.playing));
}

/// The project a task belongs to, as surfaces show it: the bundle path.
pub fn project_key(bundle: &Path) -> String {
    bundle.display().to_string()
}

/// Probes files as interactive tasks (four at a time, the io limit) and returns each file's canonical path and
/// result, in input order. Probes are quick; import waits for them because a clip's length decides where it goes.
pub fn probe_all(
    scheduler: &Scheduler,
    project: &str,
    paths: &[String],
) -> Vec<(PathBuf, Result<ove_media::MediaInfo, String>)> {
    let (tx, rx) = channel();
    let mut ids: Vec<TaskId> = Vec::with_capacity(paths.len());
    for (n, path) in paths.iter().enumerate() {
        let pb = PathBuf::from(path);
        let canonical = dunce::canonicalize(&pb).unwrap_or(pb);
        let name = file_name(&canonical);
        let tx = tx.clone();
        let spec = TaskSpec::new("probe", format!("Reading {name}"), Priority::Interactive, Class::Io)
            .project(project)
            .memory(16 * 1024 * 1024)
            .watchdog(PROBE_WATCHDOG);
        let id = scheduler.submit(spec, move |_ctx| {
            let result = crate::engine::precheck(&canonical)
                .map_err(|e| e.message)
                .and_then(|()| ove_media::probe(&canonical).map_err(|e| format!("“{name}” can't be read: {e}.")));
            let _ = tx.send((n, canonical, result));
            Ok(())
        });
        ids.push(id);
    }
    drop(tx);
    let mut out: Vec<Option<(PathBuf, Result<ove_media::MediaInfo, String>)>> =
        (0..paths.len()).map(|_| None).collect();
    for (n, canonical, result) in rx {
        out[n] = Some((canonical, result));
    }
    // A probe that ended without reporting (it panicked) is reported as unreadable.
    out.into_iter()
        .zip(paths)
        .map(|(r, path)| {
            r.unwrap_or_else(|| {
                let p = PathBuf::from(path);
                let name = file_name(&p);
                (p, Err(format!("“{name}” can't be read: the probe stopped unexpectedly.")))
            })
        })
        .collect()
}

/// Queues a thumbnail for a video file: a derivative task that writes `out` through a part file, then calls
/// `done` with whether the thumbnail exists. Returns the task, or the one already queued for the same output.
pub fn queue_thumbnail(
    scheduler: &Scheduler,
    project: &str,
    source: PathBuf,
    out: PathBuf,
    duration_us: i64,
    done: impl FnOnce(bool) + Send + 'static,
) -> TaskId {
    let name = file_name(&source);
    let spec = TaskSpec::new("thumbnail", format!("Thumbnail: {name}"), Priority::Derivative, Class::Decode)
        .project(project)
        .memory(THUMBNAIL_MEM)
        .key(format!("thumbnail:{}", out.display()));
    scheduler.submit(spec, move |ctx| {
        ctx.checkpoint()?;
        if out.is_file() {
            done(true);
            return Ok(());
        }
        let part = ove_tasks::PartFile::new(&out).map_err(|e| TaskError::Failed(e.to_string()))?;
        let made = crate::projects::make_thumbnail(&source, part.path(), duration_us).is_some();
        ctx.checkpoint()?;
        let ok = made && part.commit().is_ok();
        if !ok {
            tracing::warn!(path = %source.display(), "no thumbnail");
        }
        done(ok);
        if ok { Ok(()) } else { Err(TaskError::Failed(format!("No thumbnail could be made for “{name}”."))) }
    })
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Test media without fixture tools: a short title clip exported by our own renderer.
#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;

    use ove_core::model::Document;
    use ove_core::ops::{Op, apply_batch};
    use serde_json::json;

    /// A document with one text item `seconds` long.
    pub fn title_doc(seconds: f64) -> Document {
        let ops = vec![
            Op::AddTrack { track: json!({"id": "text", "name": "Text", "order": 0, "type": "video"}) },
            Op::AddItem {
                item: json!({"id": "t", "trackId": "text", "type": "text", "startTime": 0, "endTime": seconds, "textContent": "HI"}),
            },
        ];
        apply_batch(&Document::new("p", "Test", 1), &ops).unwrap().doc
    }

    pub fn small() -> ove_render::ExportSettings {
        ove_render::ExportSettings { height: 90, fps: 10, ..Default::default() }
    }

    /// Exports a 0.6 s 160×90 clip to `dir/name`.
    pub fn video(dir: &Path, name: &str) -> PathBuf {
        let out = dir.join(name);
        ove_render::export(&title_doc(0.6), None, &out, &small(), &AtomicBool::new(false), |_| {}).unwrap();
        out
    }

    /// A scheduler with test-sized timing.
    pub fn scheduler() -> ove_tasks::Scheduler {
        let mut limits = ove_tasks::Limits::detect();
        limits.tick = std::time::Duration::from_millis(20);
        ove_tasks::Scheduler::new(ove_tasks::Options { limits, background_threads: true })
    }
}
