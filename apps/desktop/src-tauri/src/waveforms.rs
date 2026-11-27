//! Import-time audio analysis and waveform peaks (add-audio-mixing-and-monitoring, design D2/D3).
//!
//! Every online media file with sound is analysed once in the background (a derivative task on the scheduler): one
//! decode writes its peak pyramid and loudness summary to `cache/waveforms/<contentId>.*`. The timeline asks for peaks
//! of a source range with [`peak_range`]; the answer comes from the cache and never decodes. When a file's content
//! changes, its content id changes, so it is analysed again. A file whose audio cannot be decoded is recorded as an
//! analysis error and its clips show no waveform; other media is not affected.

use std::path::{Path, PathBuf};

use ove_analyze::analysis::{self, Status};
use ove_tasks::{Class, Priority, Scheduler, TaskError, TaskId, TaskSpec};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};

/// Peak memory of one analysis pass (decoder state, the growing base level, filters).
const ANALYSIS_MEM: u64 = 64 * 1024 * 1024;

/// A media file's analysis finished (or failed): the UI asks for its peaks again.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct AudioAnalysisChanged {
    pub content_id: String,
    /// ready | error
    pub status: String,
}

/// Peaks of a source range, or why there are none yet.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PeakRangeView {
    /// ready | pending | error | none (the media has no sound)
    pub status: String,
    pub message: Option<String>,
    /// The media's content id: the key for caching peaks in the UI and matching [`AudioAnalysisChanged`].
    pub content_id: Option<String>,
    pub channels: u32,
    pub bins: u32,
    /// Pyramid level read (bin size 256 × 2^level sample frames).
    pub level: u32,
    /// `bins × channels × 2` values (min, max) from -127 to 127.
    pub data: Vec<i8>,
}

impl PeakRangeView {
    fn empty(status: &str, message: Option<String>, content_id: Option<String>) -> Self {
        PeakRangeView { status: status.into(), message, content_id, channels: 0, bins: 0, level: 0, data: vec![] }
    }
}

/// The task key of a content id's analysis.
pub fn task_key(bundle: &Path, content_id: &str) -> String {
    format!("analysis:{}", analysis::cache_paths(bundle, content_id).peaks.display())
}

/// Queues the analysis of one media file: a derivative task (cpu class) that decodes the audio once and writes the
/// cache through part files. A finished or failed analysis is not repeated. `done` gets whether peaks exist.
pub fn queue_analysis(
    scheduler: &Scheduler,
    project: &str,
    bundle: &Path,
    content_id: &str,
    source: PathBuf,
    done: impl FnOnce(bool) + Send + 'static,
) -> TaskId {
    let name = source.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (b, cid) = (bundle.to_path_buf(), content_id.to_string());
    let spec = TaskSpec::new("audio-analysis", format!("Audio analysis: {name}"), Priority::Derivative, Class::Cpu)
        .project(project)
        .memory(ANALYSIS_MEM)
        .key(task_key(bundle, content_id));
    scheduler.submit(spec, move |ctx| {
        match analysis::status(&b, &cid) {
            Status::Ready(_) => {
                done(true);
                return Ok(());
            }
            Status::Failed(_) => {
                done(false);
                return Ok(());
            }
            Status::Missing => {}
        }
        let result = analysis::analyse_file(&source, &mut |f| {
            ctx.progress(f);
            ctx.checkpoint().is_ok()
        });
        match result {
            Ok(Some(a)) => {
                ctx.checkpoint()?;
                analysis::write_cache(&b, &cid, &a)
                    .map_err(|e| TaskError::Failed(format!("No waveform for “{name}”: {e}")))?;
                done(true);
                Ok(())
            }
            Ok(None) => Err(TaskError::Cancelled),
            Err(e) => {
                let message = format!("The sound of “{name}” could not be analysed: {e}");
                let _ = analysis::write_error(&b, &cid, &message);
                done(false);
                Err(TaskError::Failed(message))
            }
        }
    })
}

/// True when a media entry has sound: audio files, and video files whose probe found an audio stream.
pub fn media_has_sound(data: &serde_json::Value) -> bool {
    match data.get("type").and_then(|v| v.as_str()) {
        Some("audio") => true,
        Some("video") => data.get("hasAudio").and_then(|v| v.as_bool()).unwrap_or(true),
        _ => false,
    }
}

/// The open project's bundle, a media item's content id and whether it has sound.
fn open_media(app: &AppHandle, media_id: &str) -> AppResult<(PathBuf, Option<String>, bool)> {
    let state = app.state::<crate::AppState>();
    let g = crate::projects::lock(&state.projects.open)
        .map_err(|_| AppError::new(ErrorCode::Internal, "The project is busy."))?;
    let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::NotFound, "No project is open."))?;
    let doc = p.doc();
    let media = doc
        .project
        .media
        .get(media_id)
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, format!("No media {media_id} in this project.")))?;
    let sound = media.fields.get("data").is_some_and(media_has_sound);
    let cid = doc.registry.get(media_id).and_then(|e| e.fields.get("contentId")?.as_str().map(str::to_string));
    Ok((p.bundle().to_path_buf(), cid, sound))
}

/// Peaks of a cached analysis for source seconds `start..end` in `bins` columns.
pub fn peaks_for(bundle: &Path, content_id: &str, start: f64, end: f64, bins: u32) -> PeakRangeView {
    let cid = Some(content_id.to_string());
    match analysis::status(bundle, content_id) {
        Status::Missing => PeakRangeView::empty("pending", None, cid),
        Status::Failed(m) => PeakRangeView::empty("error", Some(m), cid),
        Status::Ready(_) => match analysis::peak_range(bundle, content_id, start, end, bins as usize) {
            Ok(r) => PeakRangeView {
                status: "ready".into(),
                message: None,
                content_id: cid,
                channels: r.channels as u32,
                bins: r.bins as u32,
                level: r.level as u32,
                data: r.data,
            },
            Err(e) => PeakRangeView::empty("error", Some(format!("The waveform cache could not be read: {e}")), cid),
        },
    }
}

/// Peaks of a media file's source range (seconds) in `bins` columns, from the analysis cache. Never decodes: while
/// the analysis is not ready the status is `pending`, and its task is moved to interactive priority.
#[tauri::command]
#[specta::specta]
pub async fn peak_range(app: AppHandle, media_id: String, start: f64, end: f64, bins: u32) -> AppResult<PeakRangeView> {
    crate::worker::blocking("peak_range", move || {
        if !(start.is_finite() && end.is_finite()) || bins == 0 {
            return Err(AppError::new(ErrorCode::InvalidArgument, "A peak range needs finite times and bins."));
        }
        let (bundle, cid, sound) = open_media(&app, &media_id)?;
        if !sound {
            return Ok(PeakRangeView::empty("none", None, None));
        }
        let Some(cid) = cid else { return Ok(PeakRangeView::empty("pending", None, None)) };
        let view = peaks_for(&bundle, &cid, start, end, bins);
        if view.status == "pending"
            && let Some(id) = app.state::<crate::AppState>().tasks.find_key(&task_key(&bundle, &cid))
        {
            app.state::<crate::AppState>().tasks.promote(id, Priority::Interactive);
        }
        Ok(view)
    })
    .await
}

/// One volume keyframe as the timeline draws it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VolumeKey {
    /// Seconds from the item's start.
    pub time: f64,
    /// Percent, 0–200.
    pub value: f64,
}

/// What the timeline needs to draw a clip's waveform and gain line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ItemAudio {
    /// Static volume, percent (0–200).
    pub volume: f64,
    pub reverse: bool,
    /// Audio fade durations, seconds (0 when none).
    pub fade_in: f64,
    pub fade_out: f64,
    /// Volume keyframes in time order (empty when none).
    pub volume_keys: Vec<VolumeKey>,
    pub volume_easing: String,
}

/// The audio view of a video or audio item; `None` for other kinds.
pub fn item_audio(i: &ove_core::model::Item) -> Option<ItemAudio> {
    use serde_json::Value;
    if i.kind != "video" && i.kind != "audio" {
        return None;
    }
    let secs = |v: Option<&Value>| v.and_then(ove_core::time::json_to_micros).map_or(0.0, |us| us as f64 / 1e6);
    let track = i
        .fields
        .get("keyframeTracks")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|t| t.get("property").and_then(Value::as_str) == Some("volume")));
    let mut volume_keys: Vec<VolumeKey> = track
        .and_then(|t| t.get("keyframes")?.as_array())
        .map(|ks| {
            ks.iter()
                .filter_map(|k| Some(VolumeKey { time: secs(k.get("time")), value: k.get("value")?.as_f64()? }))
                .collect()
        })
        .unwrap_or_default();
    volume_keys.sort_by(|a, b| a.time.total_cmp(&b.time));
    Some(ItemAudio {
        volume: i.fields.get("volume").and_then(Value::as_f64).unwrap_or(100.0),
        reverse: i.fields.get("playbackDirection").and_then(Value::as_str) == Some("reverse"),
        fade_in: secs(i.fields.get("audioFadeIn").and_then(|f| f.get("duration"))),
        fade_out: secs(i.fields.get("audioFadeOut").and_then(|f| f.get("duration"))),
        volume_keys,
        volume_easing: track.and_then(|t| t.get("easing")?.as_str()).unwrap_or("linear").to_string(),
    })
}
/// Tells the UI a content id's analysis finished.
pub fn emit_changed(app: &AppHandle, content_id: &str, ok: bool) {
    let status = if ok { "ready" } else { "error" };
    let _ = AudioAnalysisChanged { content_id: content_id.to_string(), status: status.into() }.emit(app);
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};

    /// Writes a 16-bit stereo 48 kHz WAV of `secs` seconds: left `amp`·sin(`freq`), right half as loud.
    pub fn wav(dir: &Path, name: &str, freq: f64, amp: f64, secs: f64) -> PathBuf {
        let frames = (secs * 48_000.0) as u32;
        let mut b = Vec::with_capacity(44 + frames as usize * 4);
        let data_len = frames * 4;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&48_000u32.to_le_bytes());
        b.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            let v = amp * (std::f64::consts::TAU * freq * i as f64 / 48_000.0).sin();
            b.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
            b.extend_from_slice(&((v * 0.5 * 32767.0) as i16).to_le_bytes());
        }
        let out = dir.join(name);
        std::fs::write(&out, b).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
        out
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use super::*;

    fn run(s: &Scheduler, bundle: &Path, cid: &str, source: PathBuf) -> (bool, ove_tasks::TaskState) {
        let (tx, rx) = channel();
        let id = queue_analysis(s, "p", bundle, cid, source, move |ok| {
            let _ = tx.send(ok);
        });
        let info = s.wait_task(id, Duration::from_secs(30)).expect("the task finishes");
        (rx.recv_timeout(Duration::from_secs(1)).unwrap_or(false), info.state)
    }

    #[test]
    fn analysis_runs_once_per_content_and_again_after_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("p.ove");
        let s = crate::tasks::testing::scheduler();
        let file = testing::wav(dir.path(), "music.wav", 440.0, 0.5, 2.0);
        let cid1 = ove_store::media::content_id(&file).unwrap();
        assert_eq!(run(&s, &bundle, &cid1, file.clone()), (true, ove_tasks::TaskState::Done));
        let first = peaks_for(&bundle, &cid1, 0.0, 2.0, 100);
        assert_eq!(first.status, "ready");
        assert_eq!((first.channels, first.bins), (2, 100));
        let modified = std::fs::metadata(analysis::cache_paths(&bundle, &cid1).peaks).unwrap().modified().unwrap();
        // Queued again for the same content: reused, not re-analysed.
        assert!(run(&s, &bundle, &cid1, file.clone()).0);
        assert_eq!(
            std::fs::metadata(analysis::cache_paths(&bundle, &cid1).peaks).unwrap().modified().unwrap(),
            modified
        );
        // The file changes on disk: a new content id, so it is analysed again and the new peaks are quieter.
        let file = testing::wav(dir.path(), "music.wav", 440.0, 0.1, 2.0);
        let cid2 = ove_store::media::content_id(&file).unwrap();
        assert_ne!(cid1, cid2);
        assert_eq!(peaks_for(&bundle, &cid2, 0.0, 2.0, 100).status, "pending");
        assert!(run(&s, &bundle, &cid2, file).0);
        let second = peaks_for(&bundle, &cid2, 0.0, 2.0, 100);
        let max = |v: &PeakRangeView| v.data.iter().copied().max().unwrap();
        assert!(max(&first) > 60 && max(&second) < 15, "{} then {}", max(&first), max(&second));
    }

    #[test]
    fn unreadable_audio_reports_an_error_without_blocking_other_media() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("p.ove");
        let s = crate::tasks::testing::scheduler();
        let junk = dir.path().join("broken.wav");
        std::fs::write(&junk, b"RIFF\x10\0\0\0WAVEnot really audio at all").unwrap();
        let good = testing::wav(dir.path(), "good.wav", 220.0, 0.5, 1.0);
        let (ok, state) = run(&s, &bundle, "deadbeefdeadbeefdeadbeefdeadbeef", junk);
        assert!(!ok);
        assert_eq!(state, ove_tasks::TaskState::Failed);
        let v = peaks_for(&bundle, "deadbeefdeadbeefdeadbeefdeadbeef", 0.0, 1.0, 10);
        assert_eq!(v.status, "error");
        assert!(v.message.unwrap().contains("broken.wav"));
        assert!(v.data.is_empty());
        let cid = ove_store::media::content_id(&good).unwrap();
        assert!(run(&s, &bundle, &cid, good).0);
        assert_eq!(peaks_for(&bundle, &cid, 0.0, 1.0, 10).status, "ready");
    }

    #[test]
    fn sound_detection() {
        use serde_json::json;
        assert!(media_has_sound(&json!({"type": "audio"})));
        assert!(media_has_sound(&json!({"type": "video"})));
        assert!(!media_has_sound(&json!({"type": "video", "hasAudio": false})));
        assert!(!media_has_sound(&json!({"type": "image"})));
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;

    #[test]
    fn item_audio_reads_volume_fades_and_keyframes() {
        let v = serde_json::json!({"id": "a", "trackId": "t", "type": "audio", "startTime": 0, "endTime": 4,
            "volume": 80, "playbackDirection": "reverse", "audioFadeIn": {"duration": 0.5},
            "keyframeTracks": [{"property": "opacity", "keyframes": [{"time": 0, "value": 1}]},
                {"property": "volume", "easing": "ease-in", "keyframes": [{"time": 2, "value": 10}, {"time": 1, "value": 100}]}]});
        let a = item_audio(&ove_core::model::Item::from_json(&v, "a").unwrap()).unwrap();
        assert_eq!(a.volume, 80.0);
        assert!(a.reverse);
        assert_eq!((a.fade_in, a.fade_out), (0.5, 0.0));
        assert_eq!(a.volume_keys, vec![VolumeKey { time: 1.0, value: 100.0 }, VolumeKey { time: 2.0, value: 10.0 }]);
        assert_eq!(a.volume_easing, "ease-in");
        let t = serde_json::json!({"id": "t", "trackId": "t", "type": "text", "startTime": 0, "endTime": 4});
        assert!(item_audio(&ove_core::model::Item::from_json(&t, "t").unwrap()).is_none());
    }
}
