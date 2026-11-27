//! Export: render every frame, mix the audio, encode and write the file the [`ExportSettings`] ask for.
//!
//! The settings model and presets live in [`settings`], the choice of encoder and muxer per codec in [`encoders`]
//! (design D1, D2 of change extend-export-formats-and-queue). Video codecs: H.264, HEVC and AV1 (hardware or
//! operating-system encoders only, except the H.264 fallbacks of change 6), ProRes, DNxHR and VP9. Audio: AAC, PCM
//! (packed here, no encoder needed), FLAC and Opus. Containers: MP4, MOV, MKV, WebM, M4A, FLAC and WAV (written
//! here). An export can cover a range of the timeline, keep the composite's alpha (ProRes 4444, VP9 in WebM) and carry
//! HDR colour tags when its frame source produces HDR frames.
//!
//! The file is written to a sibling temporary file (`name.partial.ext`), flushed to disk and renamed over the
//! destination only when complete, so a failed or cancelled export never leaves a broken file or destroys an earlier
//! export of the same name. Export is refused while placeholders are pending or media files are offline, and for
//! settings this computer cannot write, before anything renders.

pub mod encoders;
pub mod pcm;
pub mod settings;

use std::collections::BTreeSet;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use ffmpeg_the_third::ffi;
use ove_core::model::Document;
use ove_core::time::Micros;
use serde::Serialize;

pub use encoders::{Choice, Inventory, encoder_label, is_hardware_encoder};
pub use settings::{
    AudioCodec, AudioSettings, Container, ExportSettings, Fps, HdrTransfer, Plan, Preset, Quality, SettingsError, Size,
    VideoCodec, VideoSettings, preset, presets,
};

use crate::audio::{CHANNELS, Mixer, RATE, frames_for};
use crate::eval::{canvas_size, reference_size};
use crate::render::Renderer;
use encoders::{AudioWriter, FileWriter};
use settings::{AudioPlan, Profile, VideoPlan};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub frame: u64,
    pub total_frames: u64,
    pub fraction: f64,
    /// The video encoder in use (the audio encoder for audio-only exports).
    pub encoder: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub container: Container,
    /// 0 × 0 for audio-only exports.
    pub width: u32,
    pub height: u32,
    /// Video frames written (0 for audio-only).
    pub frames: u64,
    pub fps: u32,
    pub duration_seconds: f64,
    /// The exported part of the timeline in seconds, when not the whole timeline.
    pub range: Option<[f64; 2]>,
    /// "ProRes 422 HQ", "HEVC 10-bit", …; `None` for audio-only exports.
    pub video_format: Option<String>,
    /// The FFmpeg encoder that wrote the video.
    pub video_encoder: Option<String>,
    /// The encoder as people read it: "ProRes (FFmpeg encoder)", "HEVC (NVIDIA NVENC)", …
    pub encoder_label: Option<String>,
    pub audio_encoder: Option<String>,
    /// Candidates tried before the one used, each with why it did not open.
    pub encoder_fallbacks: Vec<String>,
    /// Set when the video encoder is a fallback for H.264 (see [`encoder_warning`]).
    pub encoder_warning: Option<String>,
    pub seconds_taken: f64,
    /// Loudness normalisation, when the settings set a target: the mix before and after, and the gain applied.
    pub loudness: Option<LoudnessReport>,
    /// The compositing backend that made the frames and why ("gpu: GPU self-test passed (…)"); `None` for
    /// audio-only exports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
}

/// What loudness normalisation did (design D8): the mix measured before (pass 1) and as written (pass 2).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoudnessReport {
    pub target_lufs: f64,
    /// Integrated loudness and true peak of the mix without normalisation; `None` loudness = silent.
    pub before_lufs: Option<f64>,
    pub before_true_peak_dbtp: f64,
    pub after_lufs: Option<f64>,
    pub after_true_peak_dbtp: f64,
    /// The static gain applied before the limiter (0 for a silent mix).
    pub gain_db: f64,
    /// The mix had no audible sound, so no gain was applied.
    pub silent: bool,
}

/// Measures the mix of `[start, end)` as it would export without normalisation (pass 1: audio only, no video).
pub fn measure_mix(
    doc: &Document,
    bundle: Option<&Path>,
    start: Micros,
    end: Micros,
    cancel: &AtomicBool,
) -> Result<crate::loudness::LoudnessSummary, ExportError> {
    measure_mix_with_gain(doc, bundle, start, end, 0.0, cancel)
}

/// Measures the mix as it would export with a static gain (dB) before the true-peak limiter.
pub fn measure_mix_with_gain(
    doc: &Document,
    bundle: Option<&Path>,
    start: Micros,
    end: Micros,
    gain_db: f64,
    cancel: &AtomicBool,
) -> Result<crate::loudness::LoudnessSummary, ExportError> {
    let mut mixer = Mixer::new(bundle);
    mixer.set_gain_db(gain_db);
    let mut meter = crate::loudness::StereoLoudness::default();
    let total = frames_for(end - start);
    let mut done = 0usize;
    while done < total {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let n = (total - done).min(RATE as usize);
        meter.push(&mixer.mix(doc, start + (done as i128 * 1_000_000 / RATE as i128) as Micros, n));
        done += n;
    }
    Ok(meter.summary())
}
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}")]
    Refused(String),
    /// The settings cannot be written; the message names the conflicting fields.
    #[error("{0}")]
    Invalid(SettingsError),
    #[error("export was cancelled")]
    Cancelled,
    #[error("encoding failed: {0}")]
    Ffmpeg(String),
}

fn err(what: &str, code: i32) -> ExportError {
    let mut buf = [0 as std::ffi::c_char; 256];
    // SAFETY: `buf` is writable for `buf.len()` bytes and `av_strerror` writes at most that many, NUL included.
    unsafe { ffi::av_strerror(code, buf.as_mut_ptr(), buf.len()) };
    // SAFETY: `buf` is zero-initialised and `av_strerror` leaves a NUL inside it, so the string is terminated
    // within the buffer, which outlives the borrow.
    let msg = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
    ExportError::Ffmpeg(format!("{what}: {msg}"))
}

/// H.264 encoder names to try, best first.
pub fn encoder_candidates() -> Vec<&'static str> {
    encoders::h264_candidates()
}

/// True for H.264 encoders.
pub fn is_h264_encoder(name: &str) -> bool {
    name.starts_with("h264") || matches!(name, "libopenh264" | "libx264")
}

/// A plain-language warning when `encoder` is a fallback (not hardware, not H.264): files from it are larger and
/// some players, phones and sites do not accept them.
pub fn encoder_warning(encoder: &str) -> Option<String> {
    if is_hardware_encoder(encoder) || is_h264_encoder(encoder) {
        return None;
    }
    let what = if encoder == "mpeg4" { "MPEG-4 Part 2".to_string() } else { format!("the {encoder} encoder") };
    Some(format!(
        "No H.264 encoder is available on this computer, so the video was saved as {what}. It plays in VLC and most \
         editors, but some players, phones and websites may not accept it, and the file is larger."
    ))
}

/// The temporary file an export to `out` is written to: `name.partial.ext` next to it.
pub fn partial_path(out: &Path) -> PathBuf {
    let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "export".into());
    let name = match out.extension() {
        Some(ext) => format!("{stem}.partial.{}", ext.to_string_lossy()),
        None => format!("{stem}.partial"),
    };
    out.with_file_name(name)
}

/// Names of the media files the timeline uses that are not on disk (sorted, each once). Missing files would export
/// as black frames and silence.
pub fn offline_media(doc: &Document, bundle: Option<&Path>) -> Vec<String> {
    offline_media_in(doc, bundle, None)
}

/// Like [`offline_media`], for the items that overlap `range` (start, end in microseconds) only.
pub fn offline_media_in(doc: &Document, bundle: Option<&Path>, range: Option<(Micros, Micros)>) -> Vec<String> {
    let mut names = BTreeSet::new();
    for item in doc.project.items.values().filter(|i| !i.is_pending()) {
        if let Some((a, b)) = range
            && (item.end <= a || item.start >= b)
        {
            continue;
        }
        let Some(id) = item.media_id() else { continue };
        let path = crate::sources::media_path(doc, bundle, id);
        if path.as_deref().is_some_and(|p| p.is_file()) {
            continue;
        }
        let name = doc
            .project
            .media
            .get(id)
            .and_then(|m| m.file_name())
            .map(str::to_string)
            .or_else(|| path.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| id.to_string());
        names.insert(name);
    }
    names.into_iter().collect()
}

/// Whether the timeline compositor can deliver HDR frames. It cannot yet: the HDR working space belongs to change
/// add-color-management-and-subject-mattes, which switches this on.
pub fn hdr_frames_available(_doc: &Document) -> bool {
    false
}

/// The refusal for an HDR export of an SDR-only frame source.
pub const HDR_UNAVAILABLE: &str = "HDR timelines are not available yet: this timeline produces SDR frames only; \
     export without HDR";

/// An export that passed every check: the validated settings, what writes them, and the resolved frame size, rate
/// and time span.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub plan: Plan,
    pub choice: Choice,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub start: Micros,
    pub end: Micros,
}

impl Prepared {
    /// Video frames the export writes (0 for audio-only).
    pub fn total_frames(&self) -> u64 {
        if self.plan.video.is_none() {
            return 0;
        }
        (((self.end - self.start) as i128 * self.fps as i128 + 999_999) / 1_000_000) as u64
    }
}

fn frame_rate(doc: &Document, fps: Fps) -> u32 {
    match fps {
        Fps::Rate(n) => n,
        Fps::MatchTimeline => (doc.fps().round() as u32).clamp(1, 120),
    }
}

/// The output frame size for a document's aspect ratio.
pub fn frame_size(aspect: &str, size: Size) -> (u32, u32) {
    match size {
        Size::Height(h) => canvas_size(aspect, h),
        Size::MatchTimeline => canvas_size(aspect, reference_size(aspect).1),
    }
}

/// Checks the settings against what this build and computer can write (no document needed).
fn check_settings(settings: &ExportSettings, duration: Micros) -> Result<(Plan, Choice), ExportError> {
    let plan = settings.validate(Some(duration)).map_err(ExportError::Invalid)?;
    let choice =
        encoders::choose(&plan, Inventory::current(), settings.encoder.as_deref()).map_err(ExportError::Invalid)?;
    Ok((plan, choice))
}

/// Checks that the timeline can be exported with these settings and resolves them, without rendering anything.
pub fn prepare(doc: &Document, bundle: Option<&Path>, settings: &ExportSettings) -> Result<Prepared, ExportError> {
    let pending = doc.project.items.values().filter(|i| i.is_pending()).count();
    if pending > 0 {
        return Err(ExportError::Refused(format!(
            "{pending} pending placeholder{} still need media; fill or remove them before exporting",
            if pending == 1 { "" } else { "s" }
        )));
    }
    let duration = doc.duration();
    if duration <= 0 {
        return Err(ExportError::Refused("the timeline is empty".into()));
    }
    ove_media::init();
    let (plan, choice) = check_settings(settings, duration)?;
    if plan.hdr.is_some() && !hdr_frames_available(doc) {
        return Err(ExportError::Refused(HDR_UNAVAILABLE.into()));
    }
    let (start, end) = plan.range.unwrap_or((0, duration));
    let offline = offline_media_in(doc, bundle, Some((start, end)));
    if !offline.is_empty() {
        let names = offline.iter().map(|n| format!("“{n}”")).collect::<Vec<_>>().join(", ");
        let what = if offline.len() == 1 { "this media file is" } else { "these media files are" };
        return Err(ExportError::Refused(format!(
            "{what} offline: {names}; reconnect the drive or relink the media before exporting"
        )));
    }
    let (width, height) = if plan.video.is_some() { frame_size(&doc.project.aspect_ratio, plan.size) } else { (0, 0) };
    Ok(Prepared { fps: frame_rate(doc, plan.fps), plan, choice, width, height, start, end })
}

/// Checks that the timeline can be exported with these settings, without rendering anything.
pub fn preflight(doc: &Document, bundle: Option<&Path>, settings: &ExportSettings) -> Result<(), ExportError> {
    prepare(doc, bundle, settings).map(|_| ())
}

/// A rendered picture handed to the encoder.
pub enum Picture {
    /// 8 bits per channel, straight alpha (the compositor's output).
    Rgba8(crate::Rgba),
    /// 16 bits per channel RGBA, little-endian bytes, for HDR frame sources (BT.2020 primaries, the transfer of the
    /// export's HDR setting already applied).
    Rgba16 { w: u32, h: u32, data: Vec<u8> },
}

/// A frame source other than a timeline: tests and future HDR sources.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration: Micros,
    /// The source delivers HDR frames ([`Picture::Rgba16`]).
    pub hdr: bool,
}

/// Exports frames and sound from callbacks instead of a timeline (size and rate from `spec`; the settings' size and
/// frame rate are ignored).
pub fn export_frames(
    spec: SourceSpec,
    settings: &ExportSettings,
    out: &Path,
    mut frame: impl FnMut(Micros) -> Picture,
    mut sound: impl FnMut(Micros, usize) -> Vec<f32>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<ExportResult, ExportError> {
    ove_media::init();
    let (plan, choice) = check_settings(settings, spec.duration)?;
    if plan.hdr.is_some() && !spec.hdr {
        return Err(ExportError::Refused(HDR_UNAVAILABLE.into()));
    }
    let (start, end) = plan.range.unwrap_or((0, spec.duration));
    let (width, height) = if plan.video.is_some() { (spec.width & !1, spec.height & !1) } else { (0, 0) };
    let p = Prepared { plan, choice, width, height, fps: spec.fps, start, end };
    run(&p, out, Some(&mut frame), &mut sound, cancel, &mut progress)
}

/// Exports the timeline (or the settings' range of it) to `out`. `progress` is called after every frame; set
/// `cancel` to stop. The file at `out` is replaced only by a complete export: on failure or cancel it is left as it
/// was and the temporary file is removed.
pub fn export(
    doc: &Document,
    bundle: Option<&Path>,
    out: &Path,
    settings: &ExportSettings,
    cancel: &AtomicBool,
    progress: impl FnMut(Progress),
) -> Result<ExportResult, ExportError> {
    export_with_backend(doc, bundle, out, settings, None, cancel, progress)
}

/// [`export`] composited by `backend` (design D1; the CPU reference when `None`). The backend stays the same for the
/// whole job, unless its device is lost, when the rest of the frames are made on the CPU (design D9).
pub fn export_with_backend(
    doc: &Document,
    bundle: Option<&Path>,
    out: &Path,
    settings: &ExportSettings,
    backend: Option<Box<dyn crate::backend::Backend>>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<ExportResult, ExportError> {
    let p = prepare(doc, bundle, settings)?;
    let mut mixer = Mixer::new(bundle);
    // Loudness target: pass 1 measures the mix, pass 2 writes it with one static gain before the limiter.
    let before = match p.plan.loudness {
        Some(_) => Some(measure_mix(doc, bundle, p.start, p.end, cancel)?),
        None => None,
    };
    let gain_db = match (p.plan.loudness, before.and_then(|b| b.integrated_lufs)) {
        (Some(target), Some(measured)) => normalising_gain(doc, bundle, &p, target, measured, cancel)?,
        _ => 0.0,
    };
    mixer.set_gain_db(gain_db);
    let mut after = crate::loudness::StereoLoudness::default();
    let measure_after = before.is_some();
    let mut sound = |t: Micros, n: usize| {
        let s = mixer.mix(doc, t, n);
        if measure_after {
            after.push(&s);
        }
        s
    };
    let mut used = None;
    let mut result = if p.plan.video.is_none() {
        run(&p, out, None, &mut sound, cancel, &mut progress)?
    } else {
        // The backend passed in, else the program's provider for the settings' choice, else the CPU reference.
        let backend = match backend {
            Some(b) => Some(b),
            None => match crate::backend::provide(settings.backend.as_deref())
                .map_err(|e| ExportError::Invalid(SettingsError::new(&["backend"], e)))?
            {
                Some((b, why)) => {
                    used = Some(why);
                    Some(b)
                }
                None => None,
            },
        };
        let mut renderer = match backend {
            Some(b) => Renderer::with_backend(bundle, p.height, b),
            None => Renderer::new(bundle, p.height),
        };
        used = Some(used.unwrap_or_else(|| format!("{}: the backend the caller chose", renderer.backend_name())));
        renderer.set_transparent(p.plan.video.as_ref().is_some_and(|v| v.alpha));
        let mut frame = |t: Micros| Picture::Rgba8(renderer.render(doc, t));
        let r = run(&p, out, Some(&mut frame), &mut sound, cancel, &mut progress)?;
        if let Some(why) = renderer.fallback() {
            used = Some(format!("cpu: {why}"));
        }
        r
    };
    result.backend = used;
    if let (Some(target), Some(b)) = (p.plan.loudness, before) {
        let a = after.summary();
        result.loudness = Some(LoudnessReport {
            target_lufs: target,
            before_lufs: b.integrated_lufs,
            before_true_peak_dbtp: b.true_peak_dbtp,
            after_lufs: a.integrated_lufs,
            after_true_peak_dbtp: a.true_peak_dbtp,
            gain_db: (gain_db * 100.0).round() / 100.0,
            silent: b.integrated_lufs.is_none(),
        });
    }
    Ok(result)
}

/// The largest gain loudness normalisation applies.
const MAX_GAIN_DB: f64 = 40.0;

/// The static gain that brings the limited mix to `target`. The first guess is `target - measured`; when the true-peak
/// limiter then takes loudness away (a peaky mix), the mix is measured again with the gain and limiter until it is
/// within 0.1 LU (at most five audio-only passes), so the export lands on the target.
fn normalising_gain(
    doc: &Document,
    bundle: Option<&Path>,
    p: &Prepared,
    target: f64,
    measured: f64,
    cancel: &AtomicBool,
) -> Result<f64, ExportError> {
    let mut gain = (target - measured).clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
    for _ in 0..5 {
        let Some(l) = measure_mix_with_gain(doc, bundle, p.start, p.end, gain, cancel)?.integrated_lufs else { break };
        if (target - l).abs() <= 0.1 {
            break;
        }
        gain = (gain + target - l).clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
    }
    Ok(gain)
}

/// Removes the temporary export file when dropped (failure, cancel or panic) unless it was kept.
struct PartialFile {
    path: PathBuf,
    keep: bool,
}

impl Drop for PartialFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Flushes the finished temporary file to disk and renames it over the destination.
fn finish_file(tmp: &Path, out: &Path) -> std::io::Result<()> {
    // Opened for writing (nothing is written): Windows' FlushFileBuffers refuses a read-only handle.
    std::fs::OpenOptions::new().write(true).open(tmp)?.sync_all()?;
    std::fs::rename(tmp, out)?;
    // The rename itself is durable once the directory is flushed (not possible on Windows; rename is enough there).
    #[cfg(unix)]
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

type FrameFn<'a> = &'a mut dyn FnMut(Micros) -> Picture;
type SoundFn<'a> = &'a mut dyn FnMut(Micros, usize) -> Vec<f32>;

/// Writes the prepared export: frames and sound in time order into the chosen file writer.
fn run(
    p: &Prepared,
    out: &Path,
    mut frame: Option<FrameFn<'_>>,
    sound: SoundFn<'_>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<ExportResult, ExportError> {
    let started = std::time::Instant::now();
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut partial = PartialFile { path: partial_path(out), keep: false };
    let mut sink = Sink::open(&partial.path, p)?;
    let encoder = sink.encoder_name();
    let total = p.total_frames();
    let total_samples = frames_for(p.end - p.start) as i64;
    let result = (|| -> Result<(), ExportError> {
        let a_n = sink.audio_frame_samples() as i64;
        let mut audio_pts: i64 = 0;
        // Writes sound up to sample `until` (whole encoder frames, and the short last one at the very end).
        let mut audio_until = |sink: &mut Sink, until: i64, last: bool| -> Result<(), ExportError> {
            if a_n == 0 {
                return Ok(());
            }
            while audio_pts < until && (last || audio_pts + a_n <= until) {
                let n = a_n.min(until - audio_pts) as usize;
                let samples = sound(p.start + audio_pts * 1_000_000 / RATE as i64, n);
                sink.audio(&samples, n, audio_pts)?;
                audio_pts += n as i64;
            }
            Ok(())
        };
        match frame.as_mut() {
            Some(frame) => {
                for i in 0..total {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(ExportError::Cancelled);
                    }
                    let t: Micros = p.start + (i as i128 * 1_000_000 / p.fps as i128) as Micros;
                    sink.video(&frame(t), i as i64)?;
                    let video_end = ((i + 1) as i128 * RATE as i128 / p.fps as i128) as i64;
                    audio_until(&mut sink, video_end.min(total_samples), false)?;
                    progress(Progress {
                        frame: i + 1,
                        total_frames: total,
                        fraction: (i + 1) as f64 / total as f64,
                        encoder: encoder.clone(),
                    });
                }
                audio_until(&mut sink, total_samples, true)?;
            }
            None => {
                // Audio only: progress per tenth of a second of sound.
                let step = RATE as i64 / 10;
                let mut done = 0i64;
                while done < total_samples {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(ExportError::Cancelled);
                    }
                    done = (done + step).min(total_samples);
                    audio_until(&mut sink, done, done == total_samples)?;
                    progress(Progress {
                        frame: done as u64,
                        total_frames: total_samples as u64,
                        fraction: done as f64 / total_samples.max(1) as f64,
                        encoder: encoder.clone(),
                    });
                }
            }
        }
        sink.finish()
    })();
    let (video_encoder, audio_encoder, fallbacks) = sink.names();
    // Closes the file before it is flushed, renamed or removed.
    drop(sink);
    result?;
    finish_file(&partial.path, out)
        .map_err(|e| ExportError::Ffmpeg(format!("could not save the finished file as {}: {e}", out.display())))?;
    partial.keep = true;
    let duration_seconds =
        if p.plan.video.is_some() { total as f64 / p.fps as f64 } else { total_samples as f64 / RATE as f64 };
    Ok(ExportResult {
        path: out.display().to_string(),
        container: p.plan.container,
        width: p.width,
        height: p.height,
        frames: total,
        fps: p.fps,
        duration_seconds,
        range: p.plan.range.map(|(a, b)| [a as f64 / 1e6, b as f64 / 1e6]),
        video_format: p.plan.video.as_ref().map(VideoPlan::label),
        encoder_label: video_encoder.as_deref().map(encoder_label),
        encoder_warning: video_encoder
            .as_deref()
            .filter(|_| p.plan.video.as_ref().is_some_and(|v| v.codec == VideoCodec::H264))
            .and_then(encoder_warning),
        video_encoder,
        audio_encoder,
        encoder_fallbacks: fallbacks,
        seconds_taken: started.elapsed().as_secs_f64(),
        loudness: None,
        backend: None,
    })
}

/// Where the export goes: an FFmpeg muxer, or a WAV file written here.
enum Sink {
    Ff(Box<Muxer>),
    Wav(Option<pcm::WavWriter>, u8),
}

impl Sink {
    fn open(path: &Path, p: &Prepared) -> Result<Sink, ExportError> {
        match p.choice.file {
            FileWriter::Wav => {
                let depth = p.plan.audio.as_ref().map(|a| a.bit_depth).unwrap_or(24);
                let w = pcm::WavWriter::create(path, RATE, CHANNELS as u16, depth)
                    .map_err(|e| ExportError::Ffmpeg(format!("open output file: {e}")))?;
                Ok(Sink::Wav(Some(w), depth))
            }
            FileWriter::Muxer(name) => Ok(Sink::Ff(Box::new(Muxer::open(path, name, p)?))),
        }
    }

    fn encoder_name(&self) -> String {
        match self {
            Sink::Ff(m) => m.video.as_ref().map(|v| v.encoder.clone()).or_else(|| m.audio_name()).unwrap_or_default(),
            Sink::Wav(_, d) => AudioWriter::Pcm.name(*d).to_string(),
        }
    }

    /// (video encoder, audio encoder, fallbacks)
    fn names(&self) -> (Option<String>, Option<String>, Vec<String>) {
        match self {
            Sink::Ff(m) => (m.video.as_ref().map(|v| v.encoder.clone()), m.audio_name(), m.fallbacks.clone()),
            Sink::Wav(_, d) => (None, Some(AudioWriter::Pcm.name(*d).to_string()), Vec::new()),
        }
    }

    /// Samples per channel in one audio write (0: no audio).
    fn audio_frame_samples(&self) -> usize {
        match self {
            Sink::Ff(m) => m.audio.as_ref().map(|a| a.frame_samples).unwrap_or(0),
            Sink::Wav(..) => 4800,
        }
    }

    fn video(&mut self, pic: &Picture, pts: i64) -> Result<(), ExportError> {
        match self {
            Sink::Ff(m) => m.video_frame(pic, pts),
            Sink::Wav(..) => Ok(()),
        }
    }

    /// `n` stereo sample frames (interleaved) at sample position `pts`.
    fn audio(&mut self, samples: &[f32], n: usize, pts: i64) -> Result<(), ExportError> {
        match self {
            Sink::Ff(m) => m.audio_frame(samples, n, pts),
            Sink::Wav(w, _) => match w {
                Some(w) => w
                    .write(&samples[..(n * CHANNELS).min(samples.len())])
                    .map_err(|e| ExportError::Ffmpeg(format!("write: {e}"))),
                None => Ok(()),
            },
        }
    }

    fn finish(&mut self) -> Result<(), ExportError> {
        match self {
            Sink::Ff(m) => m.finish(),
            Sink::Wav(w, _) => match w.take() {
                Some(w) => w.finish().map_err(|e| ExportError::Ffmpeg(format!("write: {e}"))),
                None => Ok(()),
            },
        }
    }
}

/// The video encoder, its stream, its frame buffer and the RGB → YUV converter.
struct VideoOut {
    ctx: *mut ffi::AVCodecContext,
    st: *mut ffi::AVStream,
    frame: *mut ffi::AVFrame,
    sws: *mut ffi::SwsContext,
    /// The source format `sws` was made for.
    sws_src: ffi::AVPixelFormat,
    width: u32,
    height: u32,
    encoder: String,
    /// BT.709 or BT.2020 matrix for the converter.
    matrix: i32,
}

enum AudioKind {
    /// Through an FFmpeg encoder; `frame` holds `frame_samples` samples per channel in `ctx`'s sample format.
    Encoded { ctx: *mut ffi::AVCodecContext, frame: *mut ffi::AVFrame, name: String },
    /// PCM packed here into packets.
    Pcm { bit_depth: u8 },
}

struct AudioOut {
    st: *mut ffi::AVStream,
    kind: AudioKind,
    frame_samples: usize,
    buf: Vec<u8>,
}

/// The FFmpeg output: muxer, encoders, frame buffers and converter.
///
/// Invariant: a `Muxer` is only handed out by [`Muxer::open`] after every pointer it holds was allocated
/// (non-null), the encoders opened, the frames given buffers and the header written; `video` and `audio` are `Some`
/// exactly for the streams the export has. The pointers are owned exclusively by the muxer and freed once, in `drop`;
/// that is what makes its methods safe to call.
struct Muxer {
    oc: *mut ffi::AVFormatContext,
    video: Option<VideoOut>,
    audio: Option<AudioOut>,
    pkt: *mut ffi::AVPacket,
    fallbacks: Vec<String>,
}

impl Drop for Muxer {
    fn drop(&mut self) {
        // SAFETY: every pointer is null or owned by this muxer (a muxer that failed to open is dropped half-built,
        // hence the null checks; the `*_free` functions accept null). Each is freed exactly once, here. The I/O
        // context is closed before the format context that refers to it is freed.
        unsafe {
            ffi::av_packet_free(&mut self.pkt);
            if let Some(v) = self.video.as_mut() {
                ffi::av_frame_free(&mut v.frame);
                if !v.sws.is_null() {
                    ffi::sws_freeContext(v.sws);
                }
                ffi::avcodec_free_context(&mut v.ctx);
            }
            if let Some(AudioOut { kind: AudioKind::Encoded { ctx, frame, .. }, .. }) = self.audio.as_mut() {
                ffi::av_frame_free(frame);
                ffi::avcodec_free_context(ctx);
            }
            if !self.oc.is_null() {
                if !(*self.oc).pb.is_null() {
                    ffi::avio_closep(&mut (*self.oc).pb);
                }
                ffi::avformat_free_context(self.oc);
            }
        }
    }
}

const SWS_BILINEAR: i32 = 2;
const SWS_CS_ITU709: i32 = 1;
const SWS_CS_ITU601: i32 = 5;
const SWS_CS_BT2020: i32 = 9;

/// Mastering display colour volume (FFmpeg's `AVMasteringDisplayMetadata`, same layout).
#[repr(C)]
struct MasteringDisplay {
    display_primaries: [[ffi::AVRational; 2]; 3],
    white_point: [ffi::AVRational; 2],
    min_luminance: ffi::AVRational,
    max_luminance: ffi::AVRational,
    has_primaries: libc::c_int,
    has_luminance: libc::c_int,
}

/// Content light level (FFmpeg's `AVContentLightMetadata`, same layout).
#[repr(C)]
struct ContentLight {
    max_cll: libc::c_uint,
    max_fall: libc::c_uint,
}

/// The mastering metadata written with HDR exports until the colour-management change supplies the timeline's own:
/// a P3-D65 display of 1000 nits peak and 0.0001 nits black, MaxCLL 1000, MaxFALL 400.
fn hdr_metadata() -> (MasteringDisplay, ContentLight) {
    let r = |num: i32| ffi::AVRational { num, den: 50_000 };
    let md = MasteringDisplay {
        display_primaries: [[r(34_000), r(16_000)], [r(13_250), r(34_500)], [r(7_500), r(3_000)]],
        white_point: [r(15_635), r(16_450)],
        min_luminance: ffi::AVRational { num: 1, den: 10_000 },
        max_luminance: ffi::AVRational { num: 1000, den: 1 },
        has_primaries: 1,
        has_luminance: 1,
    };
    (md, ContentLight { max_cll: 1000, max_fall: 400 })
}

/// The supported values of an encoder setting (pixel or sample formats); `None` when the encoder takes any.
///
/// # Safety
/// `codec` must be a valid encoder returned by FFmpeg.
unsafe fn supported<T: Copy>(codec: *const ffi::AVCodec, what: ffi::AVCodecConfig) -> Option<Vec<T>> {
    let mut out: *const libc::c_void = ptr::null();
    let mut n: libc::c_int = 0;
    // SAFETY: `codec` is valid (caller); FFmpeg points `out` at a static list of `n` values of the config's type,
    // which `T` is at each call site.
    unsafe {
        let r = ffi::avcodec_get_supported_config(ptr::null(), codec, what, 0, &mut out, &mut n);
        if r < 0 || out.is_null() {
            return None;
        }
        Some(std::slice::from_raw_parts(out as *const T, n.max(0) as usize).to_vec())
    }
}

fn pix_fmt(name: &str) -> ffi::AVPixelFormat {
    let c = CString::new(name).unwrap_or_default();
    // SAFETY: `c` is a valid NUL-terminated string for the call.
    unsafe { ffi::av_get_pix_fmt(c.as_ptr()) }
}

/// The pixel format to feed `name`: the first of the plan's preferences the encoder supports.
fn choose_pix_fmt(codec: *const ffi::AVCodec, name: &str, v: &VideoPlan) -> Option<ffi::AVPixelFormat> {
    let mut prefs: Vec<&str> = encoders::pixel_formats(v).to_vec();
    // Hardware encoders take semi-planar formats natively (and Media Foundation and Quick Sync only those).
    if is_hardware_encoder(name) && !v.alpha {
        prefs.sort_by_key(|p| !(p.starts_with("nv") || p.starts_with("p0") || p.starts_with("p2")));
    }
    // SAFETY: `codec` is a valid encoder (found by name just before).
    let list: Option<Vec<ffi::AVPixelFormat>> = unsafe { supported(codec, ffi::AVCodecConfig::PIX_FORMAT) };
    prefs.into_iter().map(pix_fmt).find(|f| list.as_ref().is_none_or(|l| l.contains(f)))
}

/// FFmpeg's text for an error code.
fn ffmpeg_reason(code: i32) -> String {
    match err("", code) {
        ExportError::Ffmpeg(s) => s.trim_start_matches(": ").to_string(),
        e => e.to_string(),
    }
}

/// Encodes one blank picture with an opened encoder context and flushes it; the context is used up afterwards.
///
/// # Safety
/// `ctx` is an opened video encoder context.
unsafe fn trial_encode(ctx: *mut ffi::AVCodecContext) -> Result<(), String> {
    // SAFETY: `ctx` is open (caller); the frame and packet are allocated, checked and freed here, and the frame is
    // given buffers of the encoder's size and format before it is sent.
    unsafe {
        let mut f = ffi::av_frame_alloc();
        let mut pkt = ffi::av_packet_alloc();
        let result = (|| {
            if f.is_null() || pkt.is_null() {
                return Err("out of memory".to_string());
            }
            (*f).format = (*ctx).pix_fmt.0;
            (*f).width = (*ctx).width;
            (*f).height = (*ctx).height;
            let r = ffi::av_frame_get_buffer(f, 0);
            if r < 0 {
                return Err(ffmpeg_reason(r));
            }
            for i in 0..4 {
                let (d, l) = ((*f).data[i], (*f).linesize[i]);
                if !d.is_null() && l > 0 {
                    // Luma rows in full; other planes have at least half as many rows (4:2:0 has exactly half).
                    let bytes = (l * (*f).height) as usize / if i == 0 { 1 } else { 2 };
                    ptr::write_bytes(d, if i == 0 { 16 } else { 128 }, bytes);
                }
            }
            (*f).pts = 0;
            let r = ffi::avcodec_send_frame(ctx, f);
            if r < 0 {
                return Err(ffmpeg_reason(r));
            }
            ffi::avcodec_send_frame(ctx, ptr::null());
            loop {
                let r = ffi::avcodec_receive_packet(ctx, pkt);
                if r == ffi::AVERROR_EOF {
                    return Ok(());
                }
                if r < 0 && r != ffi::AVERROR(libc::EAGAIN) {
                    return Err(ffmpeg_reason(r));
                }
                ffi::av_packet_unref(pkt);
                if r == ffi::AVERROR(libc::EAGAIN) {
                    return Ok(());
                }
            }
        })();
        ffi::av_frame_free(&mut f);
        ffi::av_packet_free(&mut pkt);
        result
    }
}

/// Sets an encoder option (ignored by encoders that do not have it).
fn set_opt(opts: &mut *mut ffi::AVDictionary, k: &str, v: &str) {
    let (Ok(k), Ok(v)) = (CString::new(k), CString::new(v)) else { return };
    // SAFETY: `opts` is a dictionary pointer (null or allocated by FFmpeg) and the strings are copied by FFmpeg.
    unsafe { ffi::av_dict_set(opts, k.as_ptr(), v.as_ptr(), 0) };
}

/// The video bit rate for codecs whose size is not set by the profile.
fn video_bitrate(v: &VideoPlan, w: u32, h: u32, fps: u32) -> u64 {
    if let Some(b) = v.bitrate {
        return b;
    }
    let factor = if v.codec == VideoCodec::H264 { 1.0 } else { 0.6 };
    let bits = w as f64 * h as f64 * fps as f64 * v.quality.bits_per_pixel() * factor;
    (bits as u64).clamp(2_000_000, 150_000_000)
}

impl Muxer {
    /// Opens the output, the encoders and writes the header.
    fn open(path: &Path, muxer: &str, p: &Prepared) -> Result<Muxer, ExportError> {
        let cpath =
            CString::new(path.to_string_lossy().as_bytes()).map_err(|_| ExportError::Refused("bad path".into()))?;
        let cmux = CString::new(muxer).map_err(|_| ExportError::Refused("bad muxer".into()))?;
        let out_of_memory = || ExportError::Ffmpeg("out of memory".into());
        // SAFETY: plain FFmpeg C API use. Every allocation is checked for null before it is dereferenced and is
        // stored in `m` as soon as it exists, so an early return drops `m`, which frees whatever was allocated so
        // far. The C strings outlive the calls they are passed to.
        unsafe {
            let mut m =
                Muxer { oc: ptr::null_mut(), video: None, audio: None, pkt: ffi::av_packet_alloc(), fallbacks: vec![] };
            if m.pkt.is_null() {
                return Err(out_of_memory());
            }
            let r = ffi::avformat_alloc_output_context2(&mut m.oc, ptr::null(), cmux.as_ptr(), cpath.as_ptr());
            if r < 0 || m.oc.is_null() {
                return Err(err("output", r));
            }
            // Constant types differ between FFmpeg versions' bindings, hence the casts.
            #[allow(clippy::unnecessary_cast)]
            let global = ((*(*m.oc).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32) != 0;
            if let Some(v) = &p.plan.video {
                m.open_video(v, p, global)?;
            }
            if let (Some(a), Some(w)) = (&p.plan.audio, p.choice.audio) {
                m.open_audio(a, w, global)?;
            }
            #[allow(clippy::unnecessary_cast)]
            let r = ffi::avio_open(&mut (*m.oc).pb, cpath.as_ptr(), ffi::AVIO_FLAG_WRITE as i32);
            if r < 0 {
                return Err(err("open output file", r));
            }
            let r = ffi::avformat_write_header(m.oc, ptr::null_mut());
            if r < 0 {
                return Err(err("write header", r));
            }
            Ok(m)
        }
    }

    /// Opens the first video encoder candidate that works, its stream and its frame buffer.
    ///
    /// # Safety
    /// `self.oc` is a valid output context whose header is not written yet.
    unsafe fn open_video(&mut self, v: &VideoPlan, p: &Prepared, global: bool) -> Result<(), ExportError> {
        let (w, h, fps) = (p.width, p.height, p.fps);
        let hdr = p.plan.hdr;
        let bitrate = video_bitrate(v, w, h, fps);
        // SAFETY: as for `open`: every allocation is checked and owned by `self` (or freed here) before the next
        // call can fail; the fields read by `avcodec_open2` and `av_frame_get_buffer` are set first.
        unsafe {
            let mut opened: Option<(*mut ffi::AVCodecContext, String)> = None;
            for name in &p.choice.video {
                let Ok(cname) = CString::new(name.as_str()) else { continue };
                let codec = ffi::avcodec_find_encoder_by_name(cname.as_ptr());
                if codec.is_null() {
                    self.fallbacks.push(format!("{name}: not in this build"));
                    continue;
                }
                let Some(fmt) = choose_pix_fmt(codec, name, v) else {
                    self.fallbacks.push(format!(
                        "{name}: cannot take {}-bit {} pictures",
                        v.bit_depth,
                        v.chroma.label()
                    ));
                    continue;
                };
                // A configured, opened encoder context, or why it could not be opened.
                let configure = || -> Result<*mut ffi::AVCodecContext, String> {
                    let mut c = ffi::avcodec_alloc_context3(codec);
                    if c.is_null() {
                        return Err("out of memory".into());
                    }
                    (*c).width = w as i32;
                    (*c).height = h as i32;
                    (*c).time_base = ffi::AVRational { num: 1, den: fps as i32 };
                    (*c).framerate = ffi::AVRational { num: fps as i32, den: 1 };
                    (*c).sample_aspect_ratio = ffi::AVRational { num: 1, den: 1 };
                    (*c).pix_fmt = fmt;
                    match hdr {
                        Some(t) => {
                            (*c).colorspace = ffi::AVColorSpace::BT2020_NCL;
                            (*c).color_primaries = ffi::AVColorPrimaries::BT2020;
                            (*c).color_trc = match t {
                                HdrTransfer::Pq => ffi::AVColorTransferCharacteristic::SMPTE2084,
                                HdrTransfer::Hlg => ffi::AVColorTransferCharacteristic::ARIB_STD_B67,
                            };
                        }
                        None => {
                            (*c).colorspace = ffi::AVColorSpace::BT709;
                            (*c).color_primaries = ffi::AVColorPrimaries::BT709;
                            (*c).color_trc = ffi::AVColorTransferCharacteristic::BT709;
                        }
                    }
                    (*c).color_range = ffi::AVColorRange::MPEG;
                    if global {
                        (*c).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
                    }
                    let mut opts: *mut ffi::AVDictionary = ptr::null_mut();
                    match (v.codec, v.profile) {
                        (_, Profile::Prores(pr)) => {
                            (*c).profile = pr.number();
                            set_opt(&mut opts, "profile", pr.option());
                            set_opt(&mut opts, "vendor", "apl0");
                        }
                        (_, Profile::Dnxhr(d)) => set_opt(&mut opts, "profile", d.option()),
                        (VideoCodec::Vp9, _) => {
                            (*c).bit_rate = 0;
                            (*c).gop_size = (fps * 5) as i32;
                            set_opt(&mut opts, "crf", &v.quality.vp9_crf().to_string());
                            set_opt(&mut opts, "deadline", "good");
                            set_opt(&mut opts, "cpu-used", "4");
                            set_opt(&mut opts, "row-mt", "1");
                            if v.alpha {
                                set_opt(&mut opts, "auto-alt-ref", "0");
                            }
                        }
                        _ => {
                            (*c).gop_size = (fps * 2) as i32;
                            (*c).max_b_frames = 0;
                            (*c).bit_rate = bitrate as i64;
                            if v.codec == VideoCodec::Hevc && v.bit_depth == 10 {
                                (*c).profile = ffi::AV_PROFILE_HEVC_MAIN_10;
                                set_opt(&mut opts, "profile", "main10");
                            }
                            if name == "libx264" {
                                set_opt(&mut opts, "preset", "veryfast");
                            }
                        }
                    }
                    let r = ffi::avcodec_open2(c, codec, &mut opts);
                    ffi::av_dict_free(&mut opts);
                    if r < 0 {
                        ffi::avcodec_free_context(&mut c);
                        return Err(ffmpeg_reason(r));
                    }
                    Ok(c)
                };
                let mut c = match configure() {
                    Ok(c) => c,
                    Err(reason) => {
                        tracing::info!(encoder = %name, %reason, "encoder unavailable, trying the next");
                        self.fallbacks.push(format!("{name}: could not be opened ({reason})"));
                        continue;
                    }
                };
                // A hardware encoder can open and still refuse every picture (no device behind the API, a driver
                // without this format): encode a test picture first, on a context of its own.
                if is_hardware_encoder(name) {
                    let trial = trial_encode(c);
                    ffi::avcodec_free_context(&mut c);
                    if let Err(reason) = trial {
                        tracing::info!(encoder = %name, %reason, "encoder failed a test picture, trying the next");
                        self.fallbacks.push(format!("{name}: could not encode a test picture ({reason})"));
                        continue;
                    }
                    c = match configure() {
                        Ok(c) => c,
                        Err(reason) => {
                            self.fallbacks.push(format!("{name}: could not be opened ({reason})"));
                            continue;
                        }
                    };
                }
                opened = Some((c, name.clone()));
                break;
            }
            let Some((ctx, encoder)) = opened else {
                let tried = self.fallbacks.join("; ");
                return Err(ExportError::Refused(match v.codec {
                    VideoCodec::Hevc => format!(
                        "no HEVC encoder on this computer could be opened ({tried}); export ProRes or H.264 instead"
                    ),
                    VideoCodec::Av1 => {
                        format!(
                            "no AV1 hardware encoder on this computer could be opened ({tried}); export VP9 or H.264 instead"
                        )
                    }
                    VideoCodec::H264 => format!("no H.264 encoder could be opened (tried {tried})"),
                    c => format!("no {} encoder could be opened ({tried})", c.label()),
                }));
            };
            self.video = Some(VideoOut {
                ctx,
                st: ptr::null_mut(),
                frame: ptr::null_mut(),
                sws: ptr::null_mut(),
                sws_src: ffi::AVPixelFormat::NONE,
                width: w,
                height: h,
                encoder,
                matrix: if hdr.is_some() { SWS_CS_BT2020 } else { SWS_CS_ITU709 },
            });
            let vo = self.video.as_mut().ok_or_else(|| ExportError::Ffmpeg("video".into()))?;
            vo.st = ffi::avformat_new_stream(self.oc, ptr::null());
            if vo.st.is_null() {
                return Err(ExportError::Ffmpeg("out of memory".into()));
            }
            let par = (*vo.st).codecpar;
            ffi::avcodec_parameters_from_context(par, ctx);
            (*vo.st).time_base = (*ctx).time_base;
            if hdr.is_some() {
                let (md, cl) = hdr_metadata();
                let sd = ffi::av_packet_side_data_new(
                    &mut (*par).coded_side_data,
                    &mut (*par).nb_coded_side_data,
                    ffi::AVPacketSideDataType::MASTERING_DISPLAY_METADATA,
                    std::mem::size_of::<MasteringDisplay>(),
                    0,
                );
                if sd.is_null() {
                    return Err(ExportError::Ffmpeg("out of memory".into()));
                }
                ptr::copy_nonoverlapping(
                    (&md as *const MasteringDisplay).cast::<u8>(),
                    (*sd).data,
                    std::mem::size_of::<MasteringDisplay>(),
                );
                let sd = ffi::av_packet_side_data_new(
                    &mut (*par).coded_side_data,
                    &mut (*par).nb_coded_side_data,
                    ffi::AVPacketSideDataType::CONTENT_LIGHT_LEVEL,
                    std::mem::size_of::<ContentLight>(),
                    0,
                );
                if sd.is_null() {
                    return Err(ExportError::Ffmpeg("out of memory".into()));
                }
                ptr::copy_nonoverlapping(
                    (&cl as *const ContentLight).cast::<u8>(),
                    (*sd).data,
                    std::mem::size_of::<ContentLight>(),
                );
            }
            vo.frame = ffi::av_frame_alloc();
            if vo.frame.is_null() {
                return Err(ExportError::Ffmpeg("out of memory".into()));
            }
            let f = vo.frame;
            (*f).format = (*ctx).pix_fmt.0;
            (*f).width = w as i32;
            (*f).height = h as i32;
            (*f).colorspace = (*ctx).colorspace;
            (*f).color_primaries = (*ctx).color_primaries;
            (*f).color_trc = (*ctx).color_trc;
            (*f).color_range = (*ctx).color_range;
            let r = ffi::av_frame_get_buffer(f, 0);
            if r < 0 {
                return Err(err("frame buffer", r));
            }
            Ok(())
        }
    }

    /// Opens the audio encoder (or the PCM stream), its stream and its frame buffer.
    ///
    /// # Safety
    /// `self.oc` is a valid output context whose header is not written yet.
    unsafe fn open_audio(&mut self, a: &AudioPlan, writer: AudioWriter, global: bool) -> Result<(), ExportError> {
        // SAFETY: as for `open`.
        unsafe {
            let st = ffi::avformat_new_stream(self.oc, ptr::null());
            if st.is_null() {
                return Err(ExportError::Ffmpeg("out of memory".into()));
            }
            let name = match writer {
                AudioWriter::Pcm => {
                    let par = (*st).codecpar;
                    let bytes = i32::from(a.bit_depth / 8);
                    (*par).codec_type = ffi::AVMediaType::AUDIO;
                    (*par).codec_id =
                        if a.bit_depth == 16 { ffi::AVCodecID::PCM_S16LE } else { ffi::AVCodecID::PCM_S24LE };
                    (*par).sample_rate = RATE as i32;
                    ffi::av_channel_layout_default(&mut (*par).ch_layout, CHANNELS as i32);
                    (*par).bits_per_coded_sample = i32::from(a.bit_depth);
                    (*par).bits_per_raw_sample = i32::from(a.bit_depth);
                    (*par).block_align = bytes * CHANNELS as i32;
                    (*par).bit_rate = RATE as i64 * CHANNELS as i64 * i64::from(a.bit_depth);
                    (*st).time_base = ffi::AVRational { num: 1, den: RATE as i32 };
                    self.audio = Some(AudioOut {
                        st,
                        kind: AudioKind::Pcm { bit_depth: a.bit_depth },
                        frame_samples: 1024,
                        buf: Vec::new(),
                    });
                    return Ok(());
                }
                AudioWriter::Encoder(n) => n,
            };
            let cname = CString::new(name).map_err(|_| ExportError::Refused("bad encoder".into()))?;
            let codec = ffi::avcodec_find_encoder_by_name(cname.as_ptr());
            if codec.is_null() {
                return Err(ExportError::Refused(format!("this build has no {name} encoder")));
            }
            let ctx = ffi::avcodec_alloc_context3(codec);
            if ctx.is_null() {
                return Err(ExportError::Ffmpeg("out of memory".into()));
            }
            // Owned by `self` from here, so an early return frees it.
            self.audio = Some(AudioOut {
                st,
                kind: AudioKind::Encoded { ctx, frame: ptr::null_mut(), name: name.to_string() },
                frame_samples: 0,
                buf: Vec::new(),
            });
            use ffi::AVSampleFormat as S;
            let prefs: &[S] = match (a.codec, a.bit_depth) {
                (AudioCodec::Flac, 16) => &[S::S16, S::S16P],
                (AudioCodec::Flac, _) => &[S::S32, S::S32P],
                (AudioCodec::Opus, _) => &[S::FLT, S::FLTP, S::S16],
                _ => &[S::FLTP, S::FLT, S::S16],
            };
            let list: Option<Vec<S>> = supported(codec, ffi::AVCodecConfig::SAMPLE_FORMAT);
            let fmt = prefs
                .iter()
                .copied()
                .find(|f| list.as_ref().is_none_or(|l| l.contains(f)))
                .ok_or_else(|| ExportError::Ffmpeg(format!("{name} takes none of the mixer's sample formats")))?;
            (*ctx).sample_fmt = fmt;
            (*ctx).sample_rate = RATE as i32;
            ffi::av_channel_layout_default(&mut (*ctx).ch_layout, CHANNELS as i32);
            if a.bitrate > 0 {
                (*ctx).bit_rate = i64::from(a.bitrate);
            }
            if a.codec == AudioCodec::Flac {
                (*ctx).bits_per_raw_sample = i32::from(a.bit_depth);
            }
            (*ctx).time_base = ffi::AVRational { num: 1, den: RATE as i32 };
            if global {
                (*ctx).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
            let r = ffi::avcodec_open2(ctx, codec, ptr::null_mut());
            if r < 0 {
                return Err(err(&format!("{} encoder", a.codec.label()), r));
            }
            ffi::avcodec_parameters_from_context((*st).codecpar, ctx);
            (*st).time_base = (*ctx).time_base;
            let fs = (*ctx).frame_size;
            let frame_samples = if fs > 0 { fs as usize } else { 1024 };
            let af = ffi::av_frame_alloc();
            if af.is_null() {
                return Err(ExportError::Ffmpeg("out of memory".into()));
            }
            if let Some(AudioOut { kind: AudioKind::Encoded { frame, .. }, frame_samples: n, .. }) = self.audio.as_mut()
            {
                *frame = af;
                *n = frame_samples;
            }
            (*af).format = fmt.0;
            (*af).sample_rate = RATE as i32;
            ffi::av_channel_layout_default(&mut (*af).ch_layout, CHANNELS as i32);
            (*af).nb_samples = frame_samples as i32;
            let r = ffi::av_frame_get_buffer(af, 0);
            if r < 0 {
                return Err(err("audio frame buffer", r));
            }
            Ok(())
        }
    }

    fn audio_name(&self) -> Option<String> {
        self.audio.as_ref().map(|a| match &a.kind {
            AudioKind::Encoded { name, .. } => name.clone(),
            AudioKind::Pcm { bit_depth } => AudioWriter::Pcm.name(*bit_depth).to_string(),
        })
    }

    /// Writes every packet `ctx` has ready to stream `st`.
    ///
    /// # Safety
    /// `ctx` is an open encoder of this muxer and `st` its stream; the header has been written.
    unsafe fn drain(&mut self, ctx: *mut ffi::AVCodecContext, st: *mut ffi::AVStream) -> Result<(), ExportError> {
        // SAFETY: by the caller's guarantee and the type invariant; `avcodec_receive_packet` fills `pkt` and
        // `av_interleaved_write_frame` takes its contents, leaving it blank for the next round.
        unsafe {
            loop {
                let r = ffi::avcodec_receive_packet(ctx, self.pkt);
                if r == ffi::AVERROR(libc::EAGAIN) || r == ffi::AVERROR_EOF {
                    return Ok(());
                }
                if r < 0 {
                    return Err(err("encode", r));
                }
                ffi::av_packet_rescale_ts(self.pkt, (*ctx).time_base, (*st).time_base);
                (*self.pkt).stream_index = (*st).index;
                let r = ffi::av_interleaved_write_frame(self.oc, self.pkt);
                if r < 0 {
                    return Err(err("write", r));
                }
            }
        }
    }

    /// Encodes one picture of the export's size.
    fn video_frame(&mut self, pic: &Picture, pts: i64) -> Result<(), ExportError> {
        let Some(v) = self.video.as_mut() else { return Ok(()) };
        let (w, h) = (v.width, v.height);
        let (src_fmt, data, stride): (ffi::AVPixelFormat, &[u8], usize) = match pic {
            Picture::Rgba8(img) if img.w == w && img.h == h => (ffi::AVPixelFormat::RGBA, &img.data, w as usize * 4),
            Picture::Rgba16 { w: pw, h: ph, data } if *pw == w && *ph == h => {
                (ffi::AVPixelFormat::RGBA64LE, data, w as usize * 8)
            }
            _ => return Err(ExportError::Ffmpeg(format!("a rendered frame is not {w}x{h}"))),
        };
        if data.len() < stride * h as usize {
            return Err(ExportError::Ffmpeg(format!("a rendered frame is not {w}x{h}")));
        }
        // SAFETY: by the type invariant `frame` has buffers for `w`×`h` in the encoder's format; the converter is
        // (re)made for exactly the `w`×`h` `src_fmt` → encoder-format conversion, and `data` holds at least `h` rows
        // of `stride` bytes (checked above), which is all `sws_scale` reads. The encoder takes its own reference to
        // the frame's data.
        unsafe {
            if v.sws.is_null() || v.sws_src != src_fmt {
                if !v.sws.is_null() {
                    ffi::sws_freeContext(v.sws);
                }
                v.sws = ffi::sws_getContext(
                    w as i32,
                    h as i32,
                    src_fmt,
                    w as i32,
                    h as i32,
                    (*v.ctx).pix_fmt,
                    SWS_BILINEAR,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                if v.sws.is_null() {
                    return Err(ExportError::Ffmpeg("no colour converter".into()));
                }
                v.sws_src = src_fmt;
                ffi::sws_setColorspaceDetails(
                    v.sws,
                    ffi::sws_getCoefficients(SWS_CS_ITU601),
                    1,
                    ffi::sws_getCoefficients(v.matrix),
                    0,
                    0,
                    1 << 16,
                    1 << 16,
                );
            }
            let r = ffi::av_frame_make_writable(v.frame);
            if r < 0 {
                return Err(err("frame", r));
            }
            let src = [data.as_ptr(), ptr::null(), ptr::null(), ptr::null()];
            let strides = [stride as i32, 0, 0, 0];
            ffi::sws_scale(
                v.sws,
                src.as_ptr(),
                strides.as_ptr(),
                0,
                h as i32,
                (*v.frame).data.as_ptr(),
                (*v.frame).linesize.as_ptr(),
            );
            (*v.frame).pts = pts;
            let r = ffi::avcodec_send_frame(v.ctx, v.frame);
            if r < 0 {
                return Err(err("send video frame", r));
            }
            let (ctx, st) = (v.ctx, v.st);
            self.drain(ctx, st)
        }
    }

    /// `n` stereo sample frames (interleaved floats; missing samples are silence) at sample position `pts`.
    fn audio_frame(&mut self, interleaved: &[f32], n: usize, pts: i64) -> Result<(), ExportError> {
        let Some(a) = self.audio.as_mut() else { return Ok(()) };
        let n = n.min(a.frame_samples);
        let get = |i: usize, c: usize| interleaved.get(i * CHANNELS + c).copied().unwrap_or(0.0);
        match a.kind {
            AudioKind::Pcm { bit_depth } => {
                a.buf.clear();
                let mut tmp = Vec::with_capacity(n * CHANNELS);
                for i in 0..n {
                    for c in 0..CHANNELS {
                        tmp.push(get(i, c));
                    }
                }
                pcm::pack(&tmp, bit_depth, &mut a.buf);
                // SAFETY: `pkt` and the stream are valid (type invariant); the packet gets a buffer of exactly the
                // copied length, and `av_interleaved_write_frame` takes it over.
                unsafe {
                    let r = ffi::av_new_packet(self.pkt, a.buf.len() as i32);
                    if r < 0 {
                        return Err(err("audio packet", r));
                    }
                    ptr::copy_nonoverlapping(a.buf.as_ptr(), (*self.pkt).data, a.buf.len());
                    (*self.pkt).pts = pts;
                    (*self.pkt).dts = pts;
                    (*self.pkt).duration = n as i64;
                    (*self.pkt).stream_index = (*a.st).index;
                    let r = ffi::av_interleaved_write_frame(self.oc, self.pkt);
                    if r < 0 {
                        return Err(err("write", r));
                    }
                }
                Ok(())
            }
            AudioKind::Encoded { ctx, frame, .. } => {
                // SAFETY: by the type invariant `frame` has buffers for `frame_samples` samples per channel in the
                // encoder's sample format (planar: `CHANNELS` planes); `n` ≤ `frame_samples`, and each branch writes
                // exactly `n` samples per channel in that layout.
                unsafe {
                    // Back to the full size first: a frame made writable is reallocated at its current size.
                    (*frame).nb_samples = a.frame_samples as i32;
                    let r = ffi::av_frame_make_writable(frame);
                    if r < 0 {
                        return Err(err("audio frame", r));
                    }
                    (*frame).nb_samples = n as i32;
                    use ffi::AVSampleFormat as S;
                    let fmt = S((*frame).format);
                    let d = (*frame).data;
                    for i in 0..n {
                        for c in 0..CHANNELS {
                            let x = get(i, c);
                            let k = i * CHANNELS + c;
                            match fmt {
                                S::FLTP => *(d[c] as *mut f32).add(i) = x,
                                S::FLT => *(d[0] as *mut f32).add(k) = x,
                                S::S16 => *(d[0] as *mut i16).add(k) = pcm::to_s16(x),
                                S::S16P => *(d[c] as *mut i16).add(i) = pcm::to_s16(x),
                                S::S32 => *(d[0] as *mut i32).add(k) = pcm::to_s24(x) << 8,
                                S::S32P => *(d[c] as *mut i32).add(i) = pcm::to_s24(x) << 8,
                                _ => return Err(ExportError::Ffmpeg("unexpected audio sample format".into())),
                            }
                        }
                    }
                    (*frame).pts = pts;
                    let r = ffi::avcodec_send_frame(ctx, frame);
                    if r < 0 {
                        return Err(err("send audio frame", r));
                    }
                    let st = a.st;
                    self.drain(ctx, st)
                }
            }
        }
    }

    /// Flushes the encoders and writes the trailer.
    fn finish(&mut self) -> Result<(), ExportError> {
        // SAFETY: by the type invariant the encoders are open and the header has been written; a null frame is the
        // documented way to flush an encoder.
        unsafe {
            if let Some(v) = &self.video {
                let (ctx, st) = (v.ctx, v.st);
                ffi::avcodec_send_frame(ctx, ptr::null());
                self.drain(ctx, st)?;
            }
            if let Some(AudioOut { kind: AudioKind::Encoded { ctx, .. }, st, .. }) = &self.audio {
                let (ctx, st) = (*ctx, *st);
                ffi::avcodec_send_frame(ctx, ptr::null());
                self.drain(ctx, st)?;
            }
            let r = ffi::av_write_trailer(self.oc);
            if r < 0 {
                return Err(err("trailer", r));
            }
            Ok(())
        }
    }
}

/// What `ove doctor` reports about one encoder.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderCheck {
    pub codec: VideoCodec,
    pub encoder: String,
    pub label: String,
    /// Wrote a test file.
    pub works: bool,
    /// "not in this build", or why it failed.
    pub detail: Option<String>,
}

/// The run-time encoder inventory: every video encoder candidate of this platform, whether this build has it, and
/// whether it writes a one-frame test file on this computer.
pub fn encoder_report() -> Vec<EncoderCheck> {
    ove_media::init();
    let inv = Inventory::current();
    let dir = std::env::temp_dir().join(format!("ove-encoder-check-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let mut out = Vec::new();
    for codec in VideoCodec::ALL {
        let container = match codec {
            VideoCodec::Prores | VideoCodec::Dnxhr => Container::Mov,
            VideoCodec::Vp9 => Container::Webm,
            _ => Container::Mp4,
        };
        for name in encoders::video_candidates(codec, false) {
            let mut check = EncoderCheck {
                codec,
                encoder: name.to_string(),
                label: encoder_label(name),
                works: false,
                detail: None,
            };
            if !inv.has_encoder(name) {
                check.detail = Some("not in this build".into());
                out.push(check);
                continue;
            }
            let settings = ExportSettings {
                container,
                video: Some(VideoSettings { codec, ..VideoSettings::default() }),
                audio: None,
                encoder: Some(name.to_string()),
                ..ExportSettings::default()
            };
            let spec = SourceSpec { width: 320, height: 180, fps: 30, duration: 34_000, hdr: false };
            let file = dir.join(format!("{name}.{}", container.extension()));
            let r = export_frames(
                spec,
                &settings,
                &file,
                |_| Picture::Rgba8(crate::Rgba::filled(320, 180, [40, 90, 160, 255])),
                |_, n| vec![0.0; n * CHANNELS],
                &AtomicBool::new(false),
                |_| {},
            );
            match r {
                Ok(_) => check.works = true,
                Err(e) => check.detail = Some(e.to_string()),
            }
            let _ = std::fs::remove_file(&file);
            out.push(check);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    out
}
