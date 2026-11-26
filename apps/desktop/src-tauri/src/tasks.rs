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

/// Peak memory of building one skim sheet (decoder state, small RGBA frames, the sheet).
const SKIM_MEM: u64 = 128 * 1024 * 1024;

/// Where a media file's skim sheet lives: `cache/skim/<contentId>.jpg` and its layout `cache/skim/<contentId>.json`.
pub fn skim_paths(bundle: &Path, content_id: &str) -> (PathBuf, PathBuf) {
    let dir = bundle.join("cache").join("skim");
    (dir.join(format!("{content_id}.jpg")), dir.join(format!("{content_id}.json")))
}

/// The layout of a finished skim sheet of the current version, if there is one.
pub fn read_skim(bundle: &Path, content_id: &str) -> Option<ove_media::skim::SkimSheet> {
    let (jpg, json) = skim_paths(bundle, content_id);
    let sheet: ove_media::skim::SkimSheet = serde_json::from_slice(&std::fs::read(json).ok()?).ok()?;
    (sheet.version == ove_media::skim::VERSION && jpg.is_file()).then_some(sheet)
}

/// Queues a skim sheet for a video: a derivative task (decode class) that writes the image and then its layout
/// through part files, so a sheet with a layout is always complete. A finished sheet is reused. `done` gets whether
/// the sheet exists. Returns the task, or the one already queued for the same content.
pub fn queue_skim(
    scheduler: &Scheduler,
    project: &str,
    bundle: &Path,
    content_id: &str,
    source: PathBuf,
    duration_us: i64,
    done: impl FnOnce(bool) + Send + 'static,
) -> TaskId {
    let name = file_name(&source);
    let (jpg, json) = skim_paths(bundle, content_id);
    let (b, cid) = (bundle.to_path_buf(), content_id.to_string());
    let spec = TaskSpec::new("skim", format!("Skim sheet: {name}"), Priority::Derivative, Class::Decode)
        .project(project)
        .memory(SKIM_MEM)
        .key(format!("skim:{}", jpg.display()));
    scheduler.submit(spec, move |ctx| {
        if read_skim(&b, &cid).is_some() {
            done(true);
            return Ok(());
        }
        let fail = |e: String| TaskError::Failed(format!("No skim sheet for “{name}”: {e}"));
        let n = ove_media::skim::tile_count(duration_us, None);
        let rendered = ove_media::skim::render(&source, duration_us, n, |f| {
            ctx.progress(f);
            ctx.checkpoint().is_ok()
        })
        .map_err(|e| fail(e.to_string()))?;
        let Some(rendered) = rendered else { return Err(TaskError::Cancelled) };
        let bytes = ove_media::skim::encode_jpeg(&rendered).map_err(|e| fail(e.to_string()))?;
        ctx.checkpoint()?;
        let layout = serde_json::to_vec(&rendered.sheet).map_err(|e| fail(e.to_string()))?;
        ove_tasks::write_output::<std::io::Error>(&jpg, |p| std::fs::write(p, &bytes))
            .map_err(|e| fail(e.to_string()))?;
        ove_tasks::write_output::<std::io::Error>(&json, |p| std::fs::write(p, &layout))
            .map_err(|e| fail(e.to_string()))?;
        done(true);
        Ok(())
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

    /// Exports a `seconds` long 160×90 clip at 10 fps (a keyframe every 2 s) to `dir/name`.
    pub fn long_video(dir: &Path, name: &str, seconds: f64) -> PathBuf {
        let out = dir.join(name);
        ove_render::export(&title_doc(seconds), None, &out, &small(), &AtomicBool::new(false), |_| {}).unwrap();
        out
    }

    /// A scheduler with test-sized timing.
    pub fn scheduler() -> ove_tasks::Scheduler {
        let mut limits = ove_tasks::Limits::detect();
        limits.tick = std::time::Duration::from_millis(20);
        ove_tasks::Scheduler::new(ove_tasks::Options { limits, background_threads: true })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ove_media::VideoDecoder;

    use super::*;

    /// Golden tile times: a 60 s clip at 10 fps gets 60 tiles, tile i at exactly i seconds (the first frame at or
    /// after the start of its part). Decode work drops below a full decode when the clip has more than one keyframe.
    #[test]
    fn skim_sheet_tiles_and_decode_work() {
        let dir = tempfile::tempdir().unwrap();
        let clip = testing::long_video(dir.path(), "long.mp4", 60.0);
        let info = ove_media::probe(&clip).unwrap();
        let duration = info.duration_us.unwrap();
        let n = ove_media::skim::tile_count(duration, None);
        assert_eq!(n, 120.min((duration as f64 / 500_000.0).round() as usize).max(60));
        // The golden case uses 60 tiles over 60 s.
        let t = std::time::Instant::now();
        let r = ove_media::skim::render(&clip, 60_000_000, 60, |_| true).unwrap().unwrap();
        let took = t.elapsed();
        let want: Vec<i64> = (0..60).map(|i| i * 1_000_000).collect();
        for (i, (&got, &w)) in r.sheet.times_us.iter().zip(&want).enumerate() {
            assert!((got - w).abs() <= 1_000, "tile {i}: {got} µs, want {w} µs");
        }
        assert_eq!((r.sheet.columns, r.sheet.rows, r.sheet.tile_width), (10, 6, 160));
        assert_eq!((r.width, r.height), (1600, 6 * r.sheet.tile_height));

        // A full decode, for comparison.
        let mut full = ove_media::FfmpegVideoDecoder::open(&clip, Default::default()).unwrap();
        let mut total = 0u64;
        while full.next_frame().unwrap().is_some() {
            total += 1;
        }
        let mut probe = ove_media::FfmpegVideoDecoder::open(&clip, Default::default()).unwrap();
        let keyframes =
            (0..60).filter_map(|s| probe.keyframe_before(s * 1_000_000)).collect::<std::collections::BTreeSet<_>>();
        assert!(r.frames_decoded <= total, "{} of {total}", r.frames_decoded);
        if keyframes.len() > 1 {
            assert!(
                (r.frames_decoded as f64) < 0.7 * total as f64,
                "decoded {} of {total} frames with {} keyframes",
                r.frames_decoded,
                keyframes.len()
            );
        }
        let jpeg = ove_media::skim::encode_jpeg(&r).unwrap();
        println!(
            "skim: 60 tiles in {took:?}, decoded {} of {total} frames ({} keyframes), sheet {} KB",
            r.frames_decoded,
            keyframes.len(),
            jpeg.len() / 1024
        );
    }

    /// The task writes the image and its layout, reuses a finished sheet, and leaves nothing when cancelled.
    #[test]
    fn skim_task_writes_reuses_and_cancels_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let clip = testing::long_video(dir.path(), "clip.mp4", 20.0);
        let bundle = dir.path().join("P.ove");
        let s = testing::scheduler();
        let id = queue_skim(&s, "p", &bundle, "c1", clip.clone(), 20_000_000, |ok| assert!(ok));
        assert_eq!(s.wait_task(id, Duration::from_secs(60)).unwrap().state, ove_tasks::TaskState::Done);
        let sheet = read_skim(&bundle, "c1").expect("a finished sheet");
        assert_eq!(sheet.times_us.len(), 60);
        let (jpg, json) = skim_paths(&bundle, "c1");
        let modified = std::fs::metadata(&jpg).unwrap().modified().unwrap();
        // Queued again: reused, not rebuilt.
        let again = queue_skim(&s, "p", &bundle, "c1", clip.clone(), 20_000_000, |ok| assert!(ok));
        s.wait_task(again, Duration::from_secs(60)).unwrap();
        assert_eq!(std::fs::metadata(&jpg).unwrap().modified().unwrap(), modified);
        // Cancelled while paused: no sheet, no part files.
        s.set_pause(ove_tasks::PauseReason::Playback, true);
        let other = queue_skim(&s, "p", &bundle, "c2", clip, 20_000_000, |_| {});
        s.cancel(other);
        assert_eq!(s.wait_task(other, Duration::from_secs(5)).unwrap().state, ove_tasks::TaskState::Cancelled);
        assert!(read_skim(&bundle, "c2").is_none());
        let left: Vec<_> = std::fs::read_dir(jpg.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 2, "only c1's image and layout: {left:?}");
        assert!(json.is_file());
    }
}
