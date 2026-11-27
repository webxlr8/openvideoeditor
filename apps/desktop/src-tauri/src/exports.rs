//! The export dialog and the Exports panel over the shared export queue (change extend-export-formats-and-queue,
//! group 5). The dialog's choices are an [`ExportForm`], a plain mirror of the export settings model; it is checked
//! as the person changes it ([`check_export`]) and queued with [`queue_export`]. The queue is the one `ove` and MCP
//! use (job files in the app's data folder, run by the detached `ove export --worker`): this module only reads and
//! writes those job files through `ove-tools`, so there is one job system. "Save range as media" and "Render in place"
//! run the same registry tool agents call, recorded as the person's edit.

use std::path::{Path, PathBuf};

use ove_core::revision::Source;
use ove_render::export::settings::{AudioPlan, Plan};
use ove_render::export::{AudioCodec, Container, ExportSettings, Fps, Inventory, VideoCodec, encoders, frame_size};
use ove_render::queue::{CancelError, ExportJob, JobStatus, Queue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::worker;

/// Video settings of an [`ExportForm`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VideoForm {
    /// `h264`, `hevc`, `prores`, `dnxhr`, `vp9` or `av1`.
    pub codec: String,
    /// One of the codec's profiles ([`ExportOptions`]); `None` = the codec's default.
    pub profile: Option<String>,
    /// 8, 10 or 12; `None` = the codec's (or profile's) default.
    pub bit_depth: Option<u8>,
    /// `very low`, `low`, `medium`, `high` or `very high`.
    pub quality: String,
    /// Keep the timeline's transparency (ProRes 4444, VP9 in WebM).
    pub alpha: bool,
}

/// Audio settings of an [`ExportForm`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioForm {
    /// `aac`, `pcm`, `flac` or `opus`.
    pub codec: String,
    /// PCM and FLAC: 16 or 24.
    pub bit_depth: Option<u8>,
    /// AAC and Opus, bits per second; `None` follows the video quality.
    pub bitrate: Option<u32>,
}

/// What the export dialog sends: the export settings model in a shape the UI can hold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportForm {
    /// `mp4`, `mov`, `mkv`, `webm`, `m4a`, `wav` or `flac`.
    pub container: String,
    /// `None`: audio only.
    pub video: Option<VideoForm>,
    /// `None`: no sound.
    pub audio: Option<AudioForm>,
    /// `match` (the timeline's size) or a height in pixels.
    pub size: String,
    /// `match` (the timeline's rate) or frames per second.
    pub fps: String,
    /// Start and end in seconds; `None` = the whole timeline.
    pub range: Option<(f64, f64)>,
    /// Integrated loudness target in LUFS; `None` = off.
    pub loudness: Option<f64>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::InvalidArgument, message)
}

fn number_or_match(s: &str, what: &str) -> AppResult<Value> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("match") {
        return Ok(json!("match"));
    }
    t.parse::<u32>().map(|n| json!(n)).map_err(|_| invalid(format!("{what} is “match” or a number, not “{s}”")))
}

impl ExportForm {
    /// The export settings this form stands for.
    pub fn to_settings(&self) -> AppResult<ExportSettings> {
        let video = self.video.as_ref().map(|v| {
            json!({"codec": v.codec, "profile": v.profile, "bitDepth": v.bit_depth, "quality": v.quality, "alpha": v.alpha})
        });
        let audio =
            self.audio.as_ref().map(|a| json!({"codec": a.codec, "bitDepth": a.bit_depth, "bitrate": a.bitrate}));
        let v = json!({
            "container": self.container,
            "video": video,
            "audio": audio,
            "size": number_or_match(&self.size, "size")?,
            "fps": number_or_match(&self.fps, "frame rate")?,
            "range": self.range.map(|(a, b)| [a, b]),
            "loudness": self.loudness,
        });
        serde_json::from_value::<ExportSettings>(v)
            .map_err(|e| invalid(format!("These export settings are not valid: {e}.")))
    }

    /// The form for export settings (a preset, or a queued job's settings).
    pub fn from_settings(s: &ExportSettings) -> ExportForm {
        let s = s.clone().with_defaults();
        let word = |v: Value| match v {
            Value::String(w) => w,
            other => other.to_string(),
        };
        ExportForm {
            container: s.container.name().to_string(),
            video: s.video.as_ref().map(|v| VideoForm {
                codec: v.codec.name().to_string(),
                profile: v.profile.clone(),
                bit_depth: v.bit_depth,
                quality: word(serde_json::to_value(v.quality).unwrap_or_default()),
                alpha: v.alpha,
            }),
            audio: s.audio.as_ref().map(|a| AudioForm {
                codec: a.codec.name().to_string(),
                bit_depth: a.bit_depth,
                bitrate: a.bitrate,
            }),
            size: word(serde_json::to_value(s.size).unwrap_or_default()),
            fps: word(serde_json::to_value(s.fps).unwrap_or_default()),
            range: s.range.map(|[a, b]| (a, b)),
            loudness: s.loudness,
        }
    }
}

/// A named preset as the dialog lists it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportPresetView {
    pub name: String,
    pub about: String,
    pub form: ExportForm,
    /// "MOV · ProRes 422 HQ · PCM 24-bit".
    pub summary: String,
    /// Why this computer cannot write it ("no HEVC encoder on this computer …"); `None` when it can.
    pub unavailable: Option<String>,
}

/// One choice in a list (a profile, a codec).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportChoice {
    pub id: String,
    pub label: String,
    /// False when this computer has no encoder (or muxer) for it.
    pub available: bool,
}

/// A container and what it may hold.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContainerChoice {
    pub id: String,
    pub label: String,
    pub audio_only: bool,
    pub available: bool,
    pub video_codecs: Vec<String>,
    pub audio_codecs: Vec<String>,
}

/// A video codec with its profiles.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VideoCodecChoice {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub profiles: Vec<ExportChoice>,
}

/// Everything the export dialog offers.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub presets: Vec<ExportPresetView>,
    pub containers: Vec<ContainerChoice>,
    pub video_codecs: Vec<VideoCodecChoice>,
    pub audio_codecs: Vec<ExportChoice>,
    /// Loudness targets in LUFS.
    pub loudness_targets: Vec<f64>,
}

/// The dialog's check of a form against the open timeline and this computer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportCheck {
    pub ok: bool,
    /// Why it cannot be exported, naming the conflicting choices.
    pub message: Option<String>,
    /// The fields the message is about (`container`, `video.codec`, `alpha`, `range`, …).
    pub fields: Vec<String>,
    /// The file extension for the container.
    pub extension: String,
    /// "MOV · ProRes 422 HQ · PCM 24-bit · 1920×1080 · 30 fps".
    pub summary: String,
    /// Output frame size ("1920×1080"); `None` for audio only.
    pub frame_size: Option<String>,
}

/// An export job as the Exports panel shows it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportJobView {
    pub id: String,
    pub project_name: String,
    /// True when the job's project is the one open in the editor ("Export again" needs it).
    pub this_project: bool,
    pub out: String,
    pub file_name: String,
    /// `queued`, `running`, `done`, `failed` or `cancelled`.
    pub status: String,
    /// 0–1.
    pub progress: f64,
    pub frame: u32,
    pub total_frames: u32,
    /// The encoder as people read it, once known.
    pub encoder: Option<String>,
    /// "MOV · ProRes 422 HQ · PCM 24-bit".
    pub summary: String,
    pub range: Option<(f64, f64)>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    /// "person", "assistant", "cli" or an MCP client's name.
    pub requested_by: String,
    /// Milliseconds since 1970.
    pub created_at: f64,
    pub finished_at: Option<f64>,
    pub seconds_taken: Option<f64>,
    /// "Loudness −14.0 LUFS (was −22.3)" when the export normalised loudness.
    pub loudness: Option<String>,
    pub form: ExportForm,
}

/// What "Save range as media" or "Render in place" made.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedRange {
    pub media_id: String,
    /// In the project bundle (`media/rendered/…`).
    pub file: String,
    /// The clip of the new media when the range was rendered in place.
    pub item_id: Option<String>,
    pub replaced: bool,
    /// "ProRes 422 HQ", "DNxHR HQ", …
    pub format: Option<String>,
}

fn profile_label(codec: VideoCodec, id: &str) -> String {
    match (codec, id) {
        (VideoCodec::Prores, "proxy") => "422 Proxy".into(),
        (VideoCodec::Prores, "lt") => "422 LT".into(),
        (VideoCodec::Prores, "standard") => "422".into(),
        (VideoCodec::Prores, "hq") => "422 HQ".into(),
        (VideoCodec::Prores, "4444") => "4444".into(),
        (VideoCodec::Dnxhr, "hqx") => "HQX (10-bit)".into(),
        (VideoCodec::Hevc, "main") => "Main (8-bit)".into(),
        (VideoCodec::Hevc, "main10") => "Main 10 (10-bit)".into(),
        (VideoCodec::H264, "high") => "High".into(),
        _ => id.to_uppercase(),
    }
}

fn audio_label(a: &AudioPlan) -> String {
    match a.codec {
        AudioCodec::Pcm | AudioCodec::Flac => format!("{} {}-bit", a.codec.label(), a.bit_depth),
        _ => format!("{} {} kb/s", a.codec.label(), a.bitrate / 1000),
    }
}

/// "MOV · ProRes 422 HQ · PCM 24-bit".
fn plan_summary(plan: &Plan) -> String {
    let mut parts = vec![plan.container.label().to_string()];
    if let Some(v) = &plan.video {
        parts.push(if v.alpha { format!("{} with alpha", v.label()) } else { v.label() });
    }
    match &plan.audio {
        Some(a) => parts.push(audio_label(a)),
        None if plan.video.is_some() => parts.push("no sound".into()),
        None => {}
    }
    if let Some(l) = plan.loudness {
        parts.push(format!("{l} LUFS"));
    }
    parts.join(" · ")
}

/// The summary of settings that may not validate (a job's, a preset's).
fn summary(s: &ExportSettings) -> String {
    match s.validate(None) {
        Ok(plan) => plan_summary(&plan),
        Err(_) => s.container.label().to_string(),
    }
}

/// Why this computer cannot write these settings, if it cannot.
fn unavailable(s: &ExportSettings) -> Option<String> {
    let plan = s.validate(None).ok()?;
    encoders::choose(&plan, Inventory::current(), None).err().map(|e| sentence(&e.message))
}

/// First letter up, full stop at the end.
fn sentence(s: &str) -> String {
    let s = s.trim().trim_end_matches('.');
    let mut c = s.chars();
    let first: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
    format!("{first}.")
}

fn codec_available(codec: VideoCodec, inv: &Inventory) -> bool {
    encoders::video_candidates(codec, false).iter().any(|e| inv.has_encoder(e))
}

/// The dialog's choices and presets, with what this computer can write.
pub fn options() -> ExportOptions {
    ove_media::init();
    let inv = Inventory::current();
    let presets = ove_render::export::presets()
        .iter()
        .map(|p| ExportPresetView {
            name: p.name.clone(),
            about: p.about.clone(),
            form: ExportForm::from_settings(&p.settings),
            summary: summary(&p.settings),
            unavailable: unavailable(&p.settings),
        })
        .collect();
    let containers = Container::ALL
        .iter()
        .map(|&c| ContainerChoice {
            id: c.name().into(),
            label: c.label().into(),
            audio_only: c.audio_only(),
            available: encoders::file_writer(c, inv).is_ok(),
            video_codecs: c.video_codecs().iter().map(|v| v.name().to_string()).collect(),
            audio_codecs: c.audio_codecs().iter().map(|a| a.name().to_string()).collect(),
        })
        .collect();
    let video_codecs = VideoCodec::ALL
        .iter()
        .map(|&v| VideoCodecChoice {
            id: v.name().into(),
            label: v.label().into(),
            available: codec_available(v, inv),
            profiles: v
                .profiles()
                .iter()
                .map(|p| ExportChoice { id: p.to_string(), label: profile_label(v, p), available: true })
                .collect(),
        })
        .collect();
    let audio_codecs = [AudioCodec::Aac, AudioCodec::Pcm, AudioCodec::Flac, AudioCodec::Opus]
        .iter()
        .map(|&a| {
            let probe = ExportSettings {
                container: if a == AudioCodec::Flac { Container::Flac } else { Container::Mkv },
                video: None,
                audio: Some(ove_render::export::AudioSettings { codec: a, ..Default::default() }),
                ..ExportSettings::default()
            };
            let available = match probe.validate(None) {
                Ok(plan) => plan.audio.as_ref().is_some_and(|ap| encoders::audio_writer(ap, inv).is_ok()),
                Err(_) => false,
            };
            ExportChoice { id: a.name().into(), label: a.label().into(), available }
        })
        .collect();
    ExportOptions {
        presets,
        containers,
        video_codecs,
        audio_codecs,
        loudness_targets: vec![-14.0, -16.0, -23.0, -24.0],
    }
}

/// Checks a form against a timeline of `duration` (µs) and aspect ratio, and against this computer's encoders.
pub fn check(form: &ExportForm, duration: Option<i64>, aspect: &str, timeline_fps: f64) -> ExportCheck {
    let fail = |message: String, fields: Vec<String>, extension: String| ExportCheck {
        ok: false,
        message: Some(message),
        fields,
        extension,
        summary: String::new(),
        frame_size: None,
    };
    let settings = match form.to_settings() {
        Ok(s) => s,
        Err(e) => return fail(e.message, vec![], form.container.clone()),
    };
    let extension = settings.container.extension().to_string();
    let plan = match settings.validate(duration) {
        Ok(p) => p,
        Err(e) => return fail(sentence(&e.message), e.fields, extension),
    };
    if let Err(e) = encoders::choose(&plan, Inventory::current(), None) {
        let mut fields = e.fields;
        if fields.is_empty() {
            fields.push(if plan.video.is_some() { "video.codec".into() } else { "audio.codec".into() });
        }
        return fail(sentence(&e.message), fields, extension);
    }
    let mut text = plan_summary(&plan);
    let size = plan.video.as_ref().map(|_| {
        let (w, h) = frame_size(aspect, plan.size);
        format!("{w}×{h}")
    });
    if let Some(s) = &size {
        let fps = match plan.fps {
            Fps::Rate(n) => n as f64,
            Fps::MatchTimeline => timeline_fps.round().clamp(1.0, 120.0),
        };
        text = format!("{text} · {s} · {fps} fps");
    }
    if let Some((a, b)) = plan.range {
        text = format!("{text} · {:.2}–{:.2} s", a as f64 / 1e6, b as f64 / 1e6);
    }
    ExportCheck { ok: true, message: None, fields: vec![], extension, summary: text, frame_size: size }
}

fn same_bundle(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// A queue job as the panel shows it. `open` is the bundle open in the editor.
pub fn job_view(job: &ExportJob, open: Option<&Path>) -> ExportJobView {
    let r = job.result.as_ref();
    let rs = |k: &str| r.and_then(|r| r.get(k)).and_then(Value::as_str).map(str::to_string);
    let loudness = r.and_then(|r| r.get("loudness")).filter(|l| !l.is_null()).map(|l| {
        let target = l["targetLufs"].as_f64().unwrap_or_default();
        match l["beforeLufs"].as_f64() {
            Some(b) => format!("Loudness {target:.1} LUFS (was {b:.1})"),
            None => "Silent mix: no loudness change".to_string(),
        }
    });
    ExportJobView {
        id: job.id.clone(),
        project_name: job.project_name.clone(),
        this_project: open.is_some_and(|o| same_bundle(o, &job.project)),
        out: job.out.display().to_string(),
        file_name: job.out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        status: job.status.name().to_string(),
        progress: job.progress,
        frame: job.frame.min(u32::MAX as u64) as u32,
        total_frames: job.total_frames.min(u32::MAX as u64) as u32,
        encoder: rs("encoderLabel").or_else(|| job.encoder.as_deref().map(ove_render::export::encoder_label)),
        summary: summary(&job.settings),
        range: job.settings.range.map(|[a, b]| (a, b)),
        error: job.error.as_deref().map(sentence),
        warnings: job.warnings.clone(),
        requested_by: job.requested_by.clone(),
        created_at: job.created_at as f64,
        finished_at: job.finished_at.map(|t| t as f64),
        seconds_taken: r.and_then(|r| r.get("secondsTaken")).and_then(Value::as_f64),
        loudness,
        form: ExportForm::from_settings(&job.settings),
    }
}

/// The jobs the panel lists: every unfinished one, and those finished in the last day.
pub fn jobs_in(q: &Queue, open: Option<&Path>) -> Vec<ExportJobView> {
    let day = 24 * 3600 * 1000;
    let now = ove_render::queue::now_ms();
    q.list()
        .iter()
        .filter(|j| !j.status.finished() || j.finished_at.is_some_and(|t| now - t < day))
        .map(|j| job_view(j, open))
        .collect()
}

/// Cancels a job (a running one stops within 2 seconds and leaves no file).
pub fn cancel_in(q: &Queue, id: &str) -> AppResult<ExportJob> {
    q.cancel(id).map_err(|e| match e {
        CancelError::Unknown(_) => AppError::new(ErrorCode::NotFound, "That export is no longer in the queue."),
        CancelError::Finished(j) => invalid(format!("This export has already {}; its file is kept.", j.status.name())),
        CancelError::Io(e) => AppError::new(ErrorCode::Io, format!("Could not cancel the export: {e}.")),
    })
}

fn fail_to_error(f: ove_tools::Fail) -> AppError {
    let code = match f.code.as_str() {
        "export.refused" if f.message.contains("offline") => ErrorCode::MediaOffline,
        "app.noProject" | "export.invalid" | "export.refused" | "export.preset.unknown" | "rev.changed" => {
            ErrorCode::InvalidArgument
        }
        "job.unknown" => ErrorCode::NotFound,
        "file.unwritable" | "export.queue" => ErrorCode::Io,
        _ => ErrorCode::Internal,
    };
    AppError::new(code, sentence(&f.message))
}

fn queue() -> AppResult<Queue> {
    ove_tools::tools::export_queue::queue().map_err(fail_to_error)
}

/// The project open in the editor, for tools run on the person's behalf: edits are the person's (source `gui`) and
/// the editor shows them at once.
struct PersonHost {
    app: AppHandle,
}

impl ove_tools::AppHost for PersonHost {
    fn with_open(
        &mut self,
        edit: bool,
        f: &mut dyn FnMut(&mut ove_store::Project) -> Result<(), ove_tools::Fail>,
    ) -> Result<(), ove_tools::Fail> {
        let state = self.app.state::<AppState>();
        let mut g =
            crate::projects::lock(&state.projects.open).map_err(|e| ove_tools::Fail::new("app.busy", e.message))?;
        let p = g.as_mut().ok_or_else(|| ove_tools::Fail::new("app.noProject", "open a project first"))?;
        let before = p.rev();
        let out = f(p);
        if edit && p.rev() != before {
            crate::projects::emit(&self.app, Some(p));
        }
        out
    }
}

fn person_ctx(host: &mut PersonHost) -> ove_tools::AppCtx<'_> {
    ove_tools::AppCtx { host, source: Source::Gui, surface: ove_tools::Surface::InApp, actor: None }
}

fn open_bundle(app: &AppHandle) -> Option<PathBuf> {
    let state = app.state::<AppState>();
    crate::projects::lock(&state.projects.open).ok()?.as_ref().map(|p| p.bundle().to_path_buf())
}

/// Queues an export of the open project with `settings` to `out`, through the same path as `export_video`.
fn enqueue(app: &AppHandle, settings: &ExportSettings, out: &str) -> AppResult<ExportJob> {
    if out.trim().is_empty() {
        return Err(invalid("Choose where to save the file."));
    }
    let mut args = Map::new();
    args.insert("out".into(), json!(out));
    args.insert("settings".into(), serde_json::to_value(settings).unwrap_or_default());
    let mut host = PersonHost { app: app.clone() };
    let mut ctx = person_ctx(&mut host);
    ove_tools::tools::export_queue::enqueue(&mut ctx, &ove_tools::Args(&args)).map_err(fail_to_error)
}

/// Puts back jobs whose worker died and starts a worker when jobs wait (called when the app starts).
pub fn recover_on_start() {
    std::thread::spawn(|| {
        if let Ok(q) = queue()
            && let Err(e) = ove_tools::tools::export_queue::ensure_worker(&q)
        {
            tracing::warn!(error = %e.message, "could not start the export worker");
        }
    });
}

/// The presets, containers and codecs the export dialog offers.
#[tauri::command]
#[specta::specta]
pub async fn export_options() -> AppResult<ExportOptions> {
    worker::blocking("export_options", || Ok(options())).await
}

/// Checks the dialog's settings against the open timeline (range inside it) and this computer's encoders.
#[tauri::command]
#[specta::specta]
pub async fn check_export(app: AppHandle, form: ExportForm) -> AppResult<ExportCheck> {
    worker::blocking("check_export", move || {
        let state = app.state::<AppState>();
        let (duration, aspect, fps) = {
            let g = crate::projects::lock(&state.projects.open)?;
            match g.as_ref() {
                Some(p) => {
                    let d = p.doc();
                    (Some(d.duration()), d.project.aspect_ratio.clone(), d.fps())
                }
                None => (None, "16:9".to_string(), 30.0),
            }
        };
        ove_media::init();
        Ok(check(&form, duration, &aspect, fps))
    })
    .await
}

/// Queues an export of the open project. Refused at once (nothing queued) for impossible settings, offline media or
/// pending placeholders. The job renders the project as it is now, even if editing goes on.
#[tauri::command]
#[specta::specta]
pub async fn queue_export(app: AppHandle, form: ExportForm, path: String) -> AppResult<ExportJobView> {
    worker::blocking("queue_export", move || {
        let settings = form.to_settings()?;
        let job = enqueue(&app, &settings, &path)?;
        tracing::info!(job = %job.id, out = %job.out.display(), "export queued");
        Ok(job_view(&job, open_bundle(&app).as_deref()))
    })
    .await
}

/// The export queue: every queued and running job, and the jobs finished in the last day, oldest first. Also puts
/// back jobs whose worker died and starts a worker when jobs wait.
#[tauri::command]
#[specta::specta]
pub async fn export_jobs(app: AppHandle) -> AppResult<Vec<ExportJobView>> {
    worker::blocking("export_jobs", move || {
        let q = queue()?;
        if let Err(e) = ove_tools::tools::export_queue::ensure_worker(&q) {
            tracing::warn!(error = %e.message, "could not start the export worker");
        }
        Ok(jobs_in(&q, open_bundle(&app).as_deref()))
    })
    .await
}

/// Cancels a queued or running export.
#[tauri::command]
#[specta::specta]
pub async fn cancel_export_job(app: AppHandle, id: String) -> AppResult<ExportJobView> {
    worker::blocking("cancel_export_job", move || {
        let q = queue()?;
        let job = cancel_in(&q, &id)?;
        // A running job stops at its next frame: give the worker a moment to say so.
        let job = if job.status == JobStatus::Running {
            q.wait(&job.id, std::time::Duration::from_secs(3), |_| {}).unwrap_or(job)
        } else {
            job
        };
        Ok(job_view(&job, open_bundle(&app).as_deref()))
    })
    .await
}

/// Queues a finished job again with the same settings and file, rendering the open project as it is now.
#[tauri::command]
#[specta::specta]
pub async fn export_again(app: AppHandle, id: String) -> AppResult<ExportJobView> {
    worker::blocking("export_again", move || {
        let q = queue()?;
        let old = q.find(&id).ok_or_else(|| AppError::new(ErrorCode::NotFound, "That export is no longer listed."))?;
        if !open_bundle(&app).is_some_and(|o| same_bundle(&o, &old.project)) {
            return Err(invalid(format!("Open “{}” to export it again.", old.project_name)));
        }
        let job = enqueue(&app, &old.settings, &old.out.display().to_string())?;
        Ok(job_view(&job, open_bundle(&app).as_deref()))
    })
    .await
}

/// Removes finished jobs from the list (their files stay). Returns how many.
#[tauri::command]
#[specta::specta]
pub async fn clear_finished_exports() -> AppResult<u32> {
    worker::blocking("clear_finished_exports", || Ok(queue()?.clear_finished() as u32)).await
}

/// Renders the range `start`–`end` (seconds) into a new media file owned by the project. With `replace`, the same
/// revision replaces what the range covers on the video tracks with one clip of it (one undo restores the items).
#[tauri::command]
#[specta::specta]
pub async fn save_range_as_media(app: AppHandle, start: f64, end: f64, replace: bool) -> AppResult<SavedRange> {
    worker::blocking("save_range_as_media", move || {
        let mut args = Map::new();
        args.insert("start".into(), json!(start));
        args.insert("end".into(), json!(end));
        args.insert("replace".into(), json!(replace));
        let mut host = PersonHost { app: app.clone() };
        let mut ctx = person_ctx(&mut host);
        let r = ove_tools::tools::export_range::save_range_as_media(&mut ctx, &ove_tools::Args(&args))
            .map_err(fail_to_error)?;
        let v = &r.json;
        Ok(SavedRange {
            media_id: v["mediaId"].as_str().unwrap_or_default().to_string(),
            file: v["file"].as_str().unwrap_or_default().to_string(),
            item_id: v["itemId"].as_str().map(str::to_string),
            replaced: v["replaced"].as_bool().unwrap_or(replace),
            format: v["videoFormat"].as_str().map(str::to_string),
        })
    })
    .await
}

/// The container a file name implies (for the save dialog's filter).
pub fn container_label(extension: &str) -> &'static str {
    Container::parse(extension).map(|c| c.label()).unwrap_or("Video")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(
        container: &str,
        video: Option<(&str, Option<&str>, bool)>,
        audio: Option<(&str, Option<u8>)>,
    ) -> ExportForm {
        ExportForm {
            container: container.into(),
            video: video.map(|(codec, profile, alpha)| VideoForm {
                codec: codec.into(),
                profile: profile.map(str::to_string),
                bit_depth: None,
                quality: "high".into(),
                alpha,
            }),
            audio: audio.map(|(codec, bit_depth)| AudioForm { codec: codec.into(), bit_depth, bitrate: None }),
            size: "match".into(),
            fps: "match".into(),
            range: None,
            loudness: None,
        }
    }

    #[test]
    fn every_preset_round_trips_through_the_form() {
        ove_media::init();
        let o = options();
        assert_eq!(o.presets.len(), ove_render::export::presets().len());
        for (p, view) in ove_render::export::presets().iter().zip(&o.presets) {
            let back = view.form.to_settings().unwrap();
            assert_eq!(back, p.settings.clone().with_defaults(), "{}", p.name);
            assert!(!view.summary.is_empty());
        }
        let hq = o.presets.iter().find(|p| p.name == "ProRes 422 HQ master").unwrap();
        assert_eq!(hq.summary, "MOV · ProRes 422 HQ · PCM 24-bit");
        assert!(hq.unavailable.is_none(), "prores_ks is in every build");
        let wav = o.presets.iter().find(|p| p.name == "Audio WAV 24-bit").unwrap();
        assert_eq!((wav.form.video.is_none(), wav.summary.as_str()), (true, "WAV · PCM 24-bit"));
        // Codecs this computer lacks are named, not hidden.
        for p in &o.presets {
            if let Some(why) = &p.unavailable {
                assert!(why.ends_with('.') && why.chars().next().unwrap().is_uppercase(), "{why}");
            }
        }
    }

    #[test]
    fn a_refused_combination_names_the_conflicting_fields() {
        ove_media::init();
        let c = check(&form("mp4", Some(("prores", None, false)), Some(("aac", None))), Some(10_000_000), "16:9", 30.0);
        assert!(!c.ok);
        let m = c.message.unwrap();
        assert!(m.contains("ProRes") && m.contains("mp4"), "{m}");
        assert!(c.fields.iter().any(|f| f.contains("container")), "{:?}", c.fields);

        let c = check(&form("mp4", Some(("h264", None, true)), Some(("aac", None))), Some(10_000_000), "16:9", 30.0);
        assert!(!c.ok && c.fields.iter().any(|f| f.contains("alpha")), "{:?}", c.fields);

        let mut f = form("mov", Some(("prores", Some("hq"), false)), Some(("pcm", Some(24))));
        f.range = Some((2.0, 12.0));
        let c = check(&f, Some(10_000_000), "16:9", 30.0);
        assert!(!c.ok && c.fields == ["range"], "{c:?}");
        assert!(c.message.unwrap().contains("outside the timeline"));
    }

    #[test]
    fn a_valid_form_is_summarised_with_size_and_rate() {
        ove_media::init();
        let mut f = form("mov", Some(("prores", Some("hq"), false)), Some(("pcm", Some(24))));
        f.range = Some((2.0, 4.0));
        let c = check(&f, Some(10_000_000), "9:16", 25.0);
        assert!(c.ok, "{c:?}");
        assert_eq!(c.extension, "mov");
        assert_eq!(c.frame_size.as_deref(), Some("1080×1920"));
        assert_eq!(c.summary, "MOV · ProRes 422 HQ · PCM 24-bit · 1080×1920 · 25 fps · 2.00–4.00 s");

        let mut wav = form("wav", None, Some(("pcm", Some(24))));
        wav.loudness = Some(-14.0);
        let c = check(&wav, Some(10_000_000), "16:9", 30.0);
        assert!(c.ok && c.frame_size.is_none(), "{c:?}");
        assert_eq!(c.summary, "WAV · PCM 24-bit · -14 LUFS");

        let bad = ExportForm { size: "big".into(), ..wav };
        let c = check(&bad, None, "16:9", 30.0);
        assert!(!c.ok && c.message.unwrap().contains("size"));
    }

    #[test]
    fn the_panel_lists_jobs_from_the_queue_files_and_cancels_them() {
        let dir = tempfile::tempdir().unwrap();
        let q = Queue::new(dir.path().join("exports"));
        // No worker starts in this test: the lock is held as if one ran.
        let _lock = q.lock_worker().unwrap().unwrap();
        let bundle = dir.path().join("A.ove");
        let settings = ove_render::export::preset("Audio WAV 24-bit").unwrap().settings.clone();
        let a = q.enqueue(&bundle, "A", 3, settings.clone(), &dir.path().join("a.wav"), "person").unwrap();
        let b = q.enqueue(&bundle, "A", 3, settings, &dir.path().join("b.wav"), "claude-code").unwrap();
        let jobs = jobs_in(&q, Some(&bundle));
        assert_eq!(jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), [a.id.as_str(), b.id.as_str()]);
        assert_eq!(jobs[0].status, "queued");
        assert_eq!(jobs[0].file_name, "a.wav");
        assert_eq!(jobs[1].requested_by, "claude-code");
        assert_eq!(jobs[0].summary, "WAV · PCM 24-bit");
        assert!(jobs[0].form.video.is_none());

        let cancelled = cancel_in(&q, &b.id).unwrap();
        assert_eq!(cancelled.status, JobStatus::Cancelled);
        let jobs = jobs_in(&q, None);
        assert_eq!(jobs[1].status, "cancelled");
        assert!(!jobs[1].this_project);
        assert_eq!(cancel_in(&q, &b.id).unwrap_err().code, ErrorCode::InvalidArgument);
        assert_eq!(cancel_in(&q, "x_0_00000000").unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(q.clear_finished(), 1);
        assert_eq!(jobs_in(&q, None).len(), 1);
    }

    #[test]
    fn a_finished_job_shows_its_encoder_and_loudness() {
        let dir = tempfile::tempdir().unwrap();
        let q = Queue::new(dir.path());
        let mut job =
            q.enqueue(dir.path(), "A", 1, ExportSettings::h264(720, 30), &dir.path().join("a.mp4"), "person").unwrap();
        job.status = JobStatus::Done;
        job.finished_at = Some(ove_render::queue::now_ms());
        job.result = Some(json!({"encoderLabel": "H.264 (Media Foundation)", "secondsTaken": 2.5,
            "loudness": {"targetLufs": -14.0, "beforeLufs": -22.31}}));
        let v = job_view(&job, None);
        assert_eq!(v.encoder.as_deref(), Some("H.264 (Media Foundation)"));
        assert_eq!(v.seconds_taken, Some(2.5));
        assert_eq!(v.loudness.as_deref(), Some("Loudness -14.0 LUFS (was -22.3)"));
        assert_eq!(v.form.size, "720");
        assert_eq!(v.form.fps, "30");
    }
}
