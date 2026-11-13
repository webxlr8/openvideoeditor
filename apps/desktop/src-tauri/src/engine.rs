//! The playback engine thread: real decode (`ove-media`, hardware first with software fallback) driven by the
//! transport, clock and scheduler of `ove-playback`, with audio output when a device exists. Decoded frames go to the
//! viewer thread; state changes go to the UI as `playbackChanged` events.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SendError, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ove_core::model::Document;
use ove_media::{AudioDecoder, AudioOutputSpec, DecodeWorker, DecoderOptions, SampleFormat, Samples};
use ove_playback::{
    AudioOutput, Command, FrameSource, Player, PlayerConfig, PresentableFrame, SourcePoll, SystemTime, TimeSource,
    WorkerSource,
};
use tauri::AppHandle;
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::media::{self, MediaInfo};
use crate::timeline::{TIMELINE_FPS, TimelineFrame, TimelineSource};
use crate::transport::{PlaybackState, TransportCommand};
use crate::viewer::ViewerMsg;
use crate::worker::relock;
use crate::{AppErrorRaised, PlaybackChanged};

/// Audio kept decoded ahead of the play head.
const AUDIO_AHEAD_US: i64 = 600_000;

pub enum EngineMsg {
    Open(PathBuf, Sender<AppResult<MediaInfo>>),
    /// Play the composited timeline of a project (document, bundle path), replying with its info.
    OpenTimeline(Arc<Document>, PathBuf, Sender<AppResult<MediaInfo>>),
    /// The open project changed: re-render the timeline (keeps position and play state).
    UpdateDoc(Arc<Document>, PathBuf),
    Close,
    Transport(TransportCommand),
    Volume(f64, bool),
}

/// Diagnostics the overlay shows.
#[derive(Debug, Clone, Default)]
pub struct EngineDiag {
    pub decode_path: String,
    pub dropped_frames: u64,
    pub presented_frames: u64,
    pub av_offset_ms: Option<f64>,
    pub seek_p95_ms: Option<f64>,
}

/// What other threads read.
#[derive(Default)]
pub struct Shared {
    pub state: Mutex<PlaybackState>,
    pub diag: Mutex<EngineDiag>,
    pub media: Mutex<Option<MediaInfo>>,
    /// When the engine loop last went round (ms on [`now_ms`]'s clock).
    beat: AtomicU64,
}

/// An engine that has not gone round its loop for this long while a request waits is considered stuck.
const STALL_MS: u64 = 5_000;
/// More restarts than this within [`RESTART_WINDOW`] means something is persistently wrong: stop restarting.
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(60);

fn now_ms() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}

impl Shared {
    fn beat(&self) {
        self.beat.store(now_ms(), Ordering::Relaxed);
    }

    fn stalled(&self) -> bool {
        now_ms().saturating_sub(self.beat.load(Ordering::Relaxed)) > STALL_MS
    }
}

/// The playback engine as the rest of the app sees it. The engine runs on its own thread with panics contained; a
/// panic ends that thread, and the next message restarts it (fresh state, same volume). A stuck engine is replaced
/// when a request to it times out.
pub struct EngineHandle {
    tx: Mutex<Sender<EngineMsg>>,
    pub shared: Arc<Shared>,
    app: AppHandle,
    viewer: Sender<ViewerMsg>,
    restarts: Mutex<Vec<Instant>>,
}

fn not_running() -> AppError {
    AppError::new(
        ErrorCode::WorkerPanic,
        "The playback engine keeps failing, so it was not restarted. Your project is saved; restart the app to play \
         again.",
    )
}

impl EngineHandle {
    /// Sends a message to the engine, restarting it first if it stopped.
    pub fn send(&self, msg: EngineMsg) -> AppResult<()> {
        let mut tx = relock(&self.tx);
        match tx.send(msg) {
            Ok(()) => Ok(()),
            Err(SendError(msg)) => {
                tracing::warn!("the playback engine had stopped; restarting it");
                *tx = self.restart()?;
                tx.send(msg).map_err(|_| not_running())
            }
        }
    }

    /// Sends a request built around a reply channel and waits up to `timeout` for the answer (`what` names the
    /// request in errors: "opening the file").
    pub fn request<T>(
        &self,
        make: impl FnOnce(Sender<AppResult<T>>) -> EngineMsg,
        timeout: Duration,
        what: &str,
    ) -> AppResult<T> {
        let (reply, rx) = channel();
        self.send(make(reply))?;
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Disconnected) => Err(AppError::new(
                ErrorCode::WorkerPanic,
                format!("Playback stopped by an internal error while {what}. It restarts on its own; try again."),
            )),
            Err(RecvTimeoutError::Timeout) if self.shared.stalled() => {
                tracing::error!(what, "the playback engine stopped responding; replacing it");
                let mut tx = relock(&self.tx);
                // Dropping the old sender ends the stuck engine's loop if it ever returns.
                *tx = self.restart()?;
                Err(AppError::new(
                    ErrorCode::Internal,
                    format!("Playback stopped responding while {what} and was restarted. Try again."),
                ))
            }
            Err(RecvTimeoutError::Timeout) => {
                Err(AppError::new(ErrorCode::Internal, format!("Playback took too long {what}. Try again.")))
            }
        }
    }

    fn restart(&self) -> AppResult<Sender<EngineMsg>> {
        {
            let mut r = relock(&self.restarts);
            r.retain(|t| t.elapsed() < RESTART_WINDOW);
            if r.len() >= MAX_RESTARTS {
                tracing::error!(restarts = r.len(), "the playback engine keeps failing; not restarting it");
                return Err(not_running());
            }
            r.push(Instant::now());
        }
        start(self.app.clone(), self.viewer.clone(), self.shared.clone()).map_err(|e| {
            tracing::error!(error = %e, "could not restart the playback engine");
            not_running()
        })
    }
}

struct Audio {
    out: AudioOutput,
    feed: Feed,
    /// Media time the ring is filled up to.
    filled_to: i64,
    eof: bool,
}

enum Feed {
    /// A file's own audio track.
    Decoder(AudioDecoder),
    /// The timeline's mixdown.
    Mix(ove_render::audio::Mixer, Arc<Document>),
}

/// A file (decode worker) or the composited timeline.
enum AnySource {
    Media(WorkerSource),
    Timeline(TimelineSource),
}

enum AnyFrame {
    Media(ove_media::VideoFrame),
    Timeline(TimelineFrame),
}

impl PresentableFrame for AnyFrame {
    fn pts_us(&self) -> i64 {
        match self {
            AnyFrame::Media(f) => f.pts_us,
            AnyFrame::Timeline(f) => f.pts_us,
        }
    }
    fn duration_us(&self) -> i64 {
        match self {
            AnyFrame::Media(f) => f.duration_us,
            AnyFrame::Timeline(f) => f.duration_us,
        }
    }
}

fn map_poll<F>(p: SourcePoll<F>, wrap: impl FnOnce(F) -> AnyFrame) -> SourcePoll<AnyFrame> {
    match p {
        SourcePoll::Frame(f) => SourcePoll::Frame(wrap(f)),
        SourcePoll::Pending => SourcePoll::Pending,
        SourcePoll::Eof => SourcePoll::Eof,
        SourcePoll::Error(e) => SourcePoll::Error(e),
    }
}

impl FrameSource for AnySource {
    type Frame = AnyFrame;
    fn seek(&mut self, t_us: i64) {
        match self {
            AnySource::Media(s) => s.seek(t_us),
            AnySource::Timeline(s) => s.seek(t_us),
        }
    }
    fn poll(&mut self) -> SourcePoll<AnyFrame> {
        match self {
            AnySource::Media(s) => map_poll(s.poll(), AnyFrame::Media),
            AnySource::Timeline(s) => map_poll(s.poll(), AnyFrame::Timeline),
        }
    }
}

struct Session {
    player: Player<AnySource>,
    info: MediaInfo,
    fps: f64,
    audio: Option<Audio>,
    /// Set in timeline mode: the bundle being played.
    timeline: Option<PathBuf>,
}

fn to_gpu_frame(f: ove_media::VideoFrame) -> Option<ove_gpu::VideoFrame> {
    use ove_gpu::{PixelFormat as G, YuvMatrix};
    let format = match f.format {
        ove_media::PixelFormat::Nv12 => G::Nv12,
        ove_media::PixelFormat::P010 => G::P010,
        ove_media::PixelFormat::Rgba8 => G::Rgba8,
        ove_media::PixelFormat::Native(_) => return None,
    };
    let matrix = match f.color.matrix {
        ove_media::ColorMatrix::Bt601 => YuvMatrix::Bt601,
        ove_media::ColorMatrix::Bt709 => YuvMatrix::Bt709,
        ove_media::ColorMatrix::Bt2020 => YuvMatrix::Bt2020,
    };
    let strides = f.planes.iter().map(|p| p.stride as u32).collect();
    let planes = f.planes.into_iter().map(|p| Arc::<[u8]>::from(p.data)).collect();
    Some(ove_gpu::VideoFrame {
        width: f.width,
        height: f.height,
        format,
        planes,
        strides,
        pts: f.pts_us,
        matrix,
        full_range: f.color.full_range,
    })
}

fn friendly(path: &std::path::Path, e: &ove_media::MediaError) -> AppError {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    AppError::new(ErrorCode::Corrupt, format!("“{name}” can't be played: {e}."))
}

impl Session {
    fn open(path: &std::path::Path, time: &Arc<dyn TimeSource>, audio_ok: &mut bool) -> AppResult<Session> {
        let probe = ove_media::probe(path).map_err(|e| friendly(path, &e))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let duration_us = probe.duration_us.unwrap_or(0).max(0);
        let video = probe.video.clone().ok_or_else(|| {
            AppError::new(
                ErrorCode::Unsupported,
                format!("“{name}” has no video stream; audio-only preview arrives with the timeline."),
            )
        })?;
        let fps = video.frame_rate.as_f64().filter(|f| *f > 0.0).unwrap_or(30.0);
        let interval = (1_000_000.0 / fps).round() as i64;
        let decoder = ove_media::open_video(path, DecoderOptions::default()).map_err(|e| friendly(path, &e))?;
        let worker =
            DecodeWorker::spawn(Box::new(decoder), ove_media::DEFAULT_QUEUE_FRAMES).map_err(|e| friendly(path, &e))?;
        let source = AnySource::Media(WorkerSource::new(worker));
        let last_frame = (duration_us - interval).max(0);

        let audio = match (&probe.audio, AudioOutput::open_default(time.clone())) {
            (Some(_), Ok(out)) => {
                let (rate, channels) = out.ring().format();
                let spec = AudioOutputSpec { sample_rate: rate, channels, format: SampleFormat::F32 };
                match AudioDecoder::open(path, spec) {
                    Ok(decoder) => Some(Audio { out, feed: Feed::Decoder(decoder), filled_to: 0, eof: false }),
                    Err(e) => {
                        tracing::warn!(error = %e, "audio decode unavailable; video plays on the system clock");
                        None
                    }
                }
            }
            (_, Err(e)) => {
                *audio_ok = false;
                no_audio_device(&e);
                None
            }
            (None, Ok(_)) => None,
        };
        if audio.is_some() {
            *audio_ok = true;
        }
        let ring = audio.as_ref().map(|a| a.out.ring().clone());
        let player = Player::new(
            source,
            PlayerConfig { frame_interval_us: interval, last_frame_us: last_frame },
            ring,
            time.now_us(),
        );
        let total_frames = ((duration_us as f64 / 1e6) * fps).round().max(1.0) as u32;
        let info = MediaInfo {
            path: path.display().to_string(),
            name,
            duration_seconds: duration_us as f64 / 1e6,
            width: video.width,
            height: video.height,
            frame_rate: fps,
            total_frames,
            video_codec: match &video.profile {
                Some(p) => format!("{} ({p})", video.codec),
                None => video.codec.clone(),
            },
            audio_layout: probe.audio.as_ref().map(|a| format!("{} {} Hz {}", a.codec, a.sample_rate, a.layout)),
            placeholder: false,
        };
        Ok(Session { player, info, fps, audio, timeline: None })
    }

    /// The composited timeline of a project, starting at `at_us`. Reuses an audio output when one is given.
    fn open_timeline(
        doc: Arc<Document>,
        bundle: PathBuf,
        at_us: i64,
        time: &Arc<dyn TimeSource>,
        reuse: Option<AudioOutput>,
        audio_ok: &mut bool,
    ) -> Session {
        let fps = TIMELINE_FPS;
        let interval = (1_000_000.0 / fps).round() as i64;
        let duration_us = doc.duration().max(interval);
        let out = match reuse {
            Some(o) => Some(o),
            None => match AudioOutput::open_default(time.clone()) {
                Ok(o) => Some(o),
                Err(e) => {
                    no_audio_device(&e);
                    None
                }
            },
        };
        *audio_ok = out.is_some();
        let audio = out.map(|out| Audio {
            out,
            feed: Feed::Mix(ove_render::audio::Mixer::new(Some(&bundle)), doc.clone()),
            filled_to: 0,
            eof: false,
        });
        let ring = audio.as_ref().map(|a| a.out.ring().clone());
        let (rw, rh) = ove_render::eval::reference_size(&doc.project.aspect_ratio);
        let info = MediaInfo {
            path: bundle.display().to_string(),
            name: doc.project.name.clone(),
            duration_seconds: duration_us as f64 / 1e6,
            width: rw,
            height: rh,
            frame_rate: fps,
            total_frames: ((duration_us as f64 / 1e6) * fps).ceil().max(1.0) as u32,
            video_codec: "timeline".into(),
            audio_layout: Some("stereo mix".into()),
            placeholder: false,
        };
        let source = AnySource::Timeline(TimelineSource::new(doc, bundle.clone()));
        let mut player = Player::new(
            source,
            PlayerConfig { frame_interval_us: interval, last_frame_us: (duration_us - interval).max(0) },
            ring,
            time.now_us(),
        );
        if at_us > 0 {
            player.command(Command::Seek(at_us.min(duration_us - 1)), time.now_us());
        }
        Session { player, info, fps, audio, timeline: Some(bundle) }
    }

    /// Keeps the ring filled ahead of the play head.
    fn feed_audio(&mut self) {
        let Some(a) = self.audio.as_mut() else { return };
        if a.eof {
            return;
        }
        let ring = a.out.ring().clone();
        let mut budget = 16;
        while ring.queued_us() < AUDIO_AHEAD_US && budget > 0 {
            budget -= 1;
            match &mut a.feed {
                Feed::Decoder(dec) => match dec.next_chunk() {
                    Ok(Some(chunk)) => {
                        let samples: Vec<f32> = match chunk.samples {
                            Samples::F32(v) => v,
                            Samples::S16(v) => v.into_iter().map(|s| s as f32 / 32768.0).collect(),
                        };
                        ring.push(chunk.pts_us, &samples);
                        a.filled_to = chunk.pts_us;
                    }
                    Ok(None) => {
                        a.eof = true;
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "audio decode error; continuing without audio");
                        a.eof = true;
                        break;
                    }
                },
                Feed::Mix(mixer, doc) => {
                    const BLOCK: usize = 1024;
                    if a.filled_to >= doc.duration() {
                        a.eof = true;
                        break;
                    }
                    let mixed = mixer.mix(doc, a.filled_to, BLOCK);
                    let (rate, channels) = ring.format();
                    ring.push(a.filled_to, &to_device(&mixed, rate, channels));
                    a.filled_to += (BLOCK as i64 * 1_000_000) / ove_render::audio::RATE as i64;
                }
            }
        }
    }

    fn restart_audio(&mut self, t_us: i64) {
        if let Some(a) = self.audio.as_mut() {
            a.out.ring().clear();
            a.eof = match &mut a.feed {
                Feed::Decoder(d) => d.seek(t_us).is_err(),
                Feed::Mix(..) => false,
            };
            a.filled_to = t_us;
        }
    }
}

/// Converts 48 kHz stereo to the device format (nearest-sample rate change, channel up/down-mix).
fn to_device(stereo: &[f32], rate: u32, channels: u16) -> Vec<f32> {
    let src_rate = ove_render::audio::RATE;
    if rate == src_rate && channels == 2 {
        return stereo.to_vec();
    }
    let frames_in = stereo.len() / 2;
    let frames_out = (frames_in as u64 * rate as u64 / src_rate as u64) as usize;
    let ch = channels.max(1) as usize;
    let mut out = Vec::with_capacity(frames_out * ch);
    for i in 0..frames_out {
        let j = ((i as u64 * src_rate as u64 / rate as u64) as usize).min(frames_in.saturating_sub(1));
        let (l, r) = (stereo[j * 2], stereo[j * 2 + 1]);
        for c in 0..ch {
            out.push(match (ch, c) {
                (1, _) => (l + r) * 0.5,
                (_, 0) => l,
                (_, 1) => r,
                _ => 0.0,
            });
        }
    }
    out
}

fn map_command(cmd: TransportCommand, s: &Session) -> Vec<Command> {
    match cmd {
        TransportCommand::Play => vec![Command::Play],
        TransportCommand::Pause => vec![Command::Pause],
        TransportCommand::TogglePlay => vec![Command::TogglePlay],
        // Steps are queued by the engine loop (one at a time, each after the previous frame arrived).
        TransportCommand::Step { .. } => vec![],
        TransportCommand::GoToStart => vec![Command::GoToStart],
        TransportCommand::GoToEnd => vec![Command::GoToEnd],
        TransportCommand::ShuttleForward => vec![Command::ShuttleForward],
        TransportCommand::ShuttleReverse => vec![Command::ShuttleReverse],
        TransportCommand::SeekFrame { frame } => {
            // The middle of the frame's interval, so the frame whose interval contains the time is shown.
            let t = ((f64::from(frame) + 0.5) / s.fps * 1e6) as i64;
            vec![Command::Seek(t)]
        }
        TransportCommand::SetLoop { enabled } => vec![Command::SetLoop(enabled)],
    }
}

/// Index of the frame whose presentation interval contains `pts_us` (tolerant of microsecond rounding and float
/// error: 2.88 s × 25 fps is frame 72, not 71).
pub fn frame_index(pts_us: i64, fps: f64) -> u32 {
    // Timestamps are rounded to whole microseconds, so a frame start can sit up to 0.5 µs early.
    ((pts_us as f64 + 1.0) * fps / 1e6).floor().max(0.0) as u32
}

/// Logs a missing audio output once as a warning (every timeline change reopens the output), then at debug level.
fn no_audio_device(e: &dyn std::fmt::Display) {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::debug!(error = %e, "no audio output device; playback runs on the system clock");
    } else {
        tracing::warn!(error = %e, "no audio output device; playback runs on the system clock");
    }
}

fn snapshot(s: Option<&Session>, volume: f64, muted: bool, audio_available: bool) -> PlaybackState {
    match s {
        None => PlaybackState {
            has_media: false,
            frame: 0,
            total_frames: 0,
            frame_rate: 30.0,
            playing: false,
            speed: 0.0,
            looping: false,
            volume,
            muted,
            audio_available,
        },
        Some(s) => {
            let t = s.player.transport();
            let frame = frame_index(t.position, s.fps);
            PlaybackState {
                has_media: true,
                frame: frame.min(s.info.total_frames.saturating_sub(1)),
                total_frames: s.info.total_frames,
                frame_rate: s.fps,
                playing: t.is_playing(),
                speed: if t.is_playing() { t.speed.factor() } else { 0.0 },
                looping: t.looping,
                volume,
                muted,
                audio_available,
            }
        }
    }
}

pub fn spawn(app: AppHandle, viewer: Sender<ViewerMsg>) -> std::io::Result<EngineHandle> {
    let shared = Arc::new(Shared::default());
    let tx = start(app.clone(), viewer.clone(), shared.clone())?;
    Ok(EngineHandle { tx: Mutex::new(tx), shared, app, viewer, restarts: Mutex::new(Vec::new()) })
}

/// Starts an engine thread. When it panics, the UI is told playback stopped; the thread then ends, which closes its
/// channel, so the next message restarts it.
fn start(app: AppHandle, viewer: Sender<ViewerMsg>, shared: Arc<Shared>) -> std::io::Result<Sender<EngineMsg>> {
    let (tx, rx) = channel::<EngineMsg>();
    shared.beat();
    std::thread::Builder::new().name("playback".into()).spawn(move || {
        let a = app.clone();
        let sh = shared.clone();
        if crate::worker::contain("playback", move || run(a, viewer, rx, sh)).is_err() {
            *relock(&shared.media) = None;
            let snap = {
                let mut s = relock(&shared.state);
                s.has_media = false;
                s.playing = false;
                s.speed = 0.0;
                s.clone()
            };
            let _ = PlaybackChanged(snap).emit(&app);
            let _ = AppErrorRaised(AppError::new(
                ErrorCode::WorkerPanic,
                "Playback stopped because of an internal error. Open the clip or timeline again to continue.",
            ))
            .emit(&app);
        }
    })?;
    Ok(tx)
}

fn run(app: AppHandle, viewer: Sender<ViewerMsg>, rx: Receiver<EngineMsg>, shared: Arc<Shared>) {
    let time: Arc<dyn TimeSource> = Arc::new(SystemTime::new());
    let mut session: Option<Session> = None;
    // A restarted engine keeps the volume the user set.
    let (mut volume, mut muted) = {
        let s = relock(&shared.state);
        (s.volume, s.muted)
    };
    let mut audio_available = false;
    let mut last_sent: Option<PlaybackState> = None;
    // Frames still to step (positive forward, negative back); any other transport command cancels them.
    let mut pending_steps: i64 = 0;
    loop {
        shared.beat();
        let busy = pending_steps != 0
            || session.as_ref().is_some_and(|s| s.player.transport().is_playing() || !s.player.is_settled());
        let timeout = if busy { Duration::from_millis(4) } else { Duration::from_millis(20) };
        match rx.recv_timeout(timeout) {
            Ok(EngineMsg::Open(path, reply)) => {
                let opened = Session::open(&path, &time, &mut audio_available);
                let result = opened.map(|s| {
                    let info = s.info.clone();
                    if let Some(a) = &s.audio {
                        a.out.ring().set_volume(volume as f32);
                        a.out.ring().set_muted(muted);
                    }
                    // A failed open keeps the previous file playing; a successful one replaces it.
                    session = Some(s);
                    *relock(&shared.media) = Some(info.clone());
                    info
                });
                if let Err(e) = &result {
                    tracing::warn!(path = %path.display(), "open failed: {}", e.message);
                }
                let _ = reply.send(result);
            }
            Ok(EngineMsg::OpenTimeline(doc, bundle, reply)) => {
                pending_steps = 0;
                let reuse = session.take().and_then(|s| s.audio.map(|a| a.out));
                let s = Session::open_timeline(doc, bundle, 0, &time, reuse, &mut audio_available);
                if let Some(a) = &s.audio {
                    a.out.ring().set_volume(volume as f32);
                    a.out.ring().set_muted(muted);
                }
                let info = s.info.clone();
                *relock(&shared.media) = Some(info.clone());
                session = Some(s);
                let _ = reply.send(Ok(info));
            }
            Ok(EngineMsg::UpdateDoc(doc, bundle)) => {
                // Only a session playing this project's timeline follows edits.
                if let Some(old) = session.take_if(|s| s.timeline.as_ref() == Some(&bundle)) {
                    let at = old.player.transport().position;
                    let playing = old.player.transport().is_playing();
                    let looping = old.player.transport().looping;
                    let reuse = old.audio.map(|a| a.out);
                    let mut s = Session::open_timeline(doc, bundle, at, &time, reuse, &mut audio_available);
                    let now = time.now_us();
                    if looping {
                        s.player.command(Command::SetLoop(true), now);
                    }
                    if playing {
                        s.player.command(Command::Play, now);
                    }
                    *relock(&shared.media) = Some(s.info.clone());
                    session = Some(s);
                }
            }
            Ok(EngineMsg::Close) => {
                session = None;
                pending_steps = 0;
                *relock(&shared.media) = None;
            }
            Ok(EngineMsg::Transport(cmd)) => {
                if let Some(s) = session.as_mut() {
                    let now = time.now_us();
                    match cmd {
                        TransportCommand::Step { frames } => {
                            if s.player.transport().is_playing() {
                                s.player.command(Command::Pause, now);
                            }
                            pending_steps += i64::from(frames);
                        }
                        TransportCommand::SetLoop { .. } => {}
                        _ => pending_steps = 0,
                    }
                    for c in map_command(cmd, s) {
                        s.player.command(c, now);
                    }
                }
            }
            Ok(EngineMsg::Volume(v, m)) => {
                (volume, muted) = (v.clamp(0.0, 1.0), m);
                if let Some(s) = session.as_mut() {
                    s.player.set_volume(volume as f32);
                    s.player.set_muted(muted);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if let Some(s) = session.as_mut() {
            if pending_steps != 0 && s.player.is_settled() {
                let (cmd, delta) =
                    if pending_steps > 0 { (Command::StepForward, -1) } else { (Command::StepBackward, 1) };
                s.player.command(cmd, time.now_us());
                pending_steps += delta;
            }
            let out = s.player.tick(time.now_us());
            if let Some(t) = out.audio_restart {
                s.restart_audio(t);
            }
            s.feed_audio();
            let shown = match out.present {
                Some(AnyFrame::Media(f)) => to_gpu_frame(f),
                Some(AnyFrame::Timeline(f)) => Some(crate::timeline::to_viewer(&f)),
                None => None,
            };
            if let Some(f) = shown {
                let _ = viewer.send(ViewerMsg::Frame(f));
            }
            if let Some(err) = out.error {
                let name = s.info.name.clone();
                let offline = !std::path::Path::new(&s.info.path).exists();
                let message = if offline {
                    format!(
                        "“{name}” is offline: the file was moved, deleted or its drive was disconnected. Playback stopped."
                    )
                } else {
                    format!("Playback of “{name}” stopped: {err}.")
                };
                let _ = AppErrorRaised(AppError::new(
                    if offline { ErrorCode::NotFound } else { ErrorCode::Corrupt },
                    message,
                ))
                .emit(&app);
                s.player.command(Command::Pause, time.now_us());
            }
            let d = s.player.diagnostics();
            let path = match s.player.source_mut() {
                AnySource::Media(w) => w.decode_path().to_string(),
                AnySource::Timeline(t) => format!(
                    "timeline compositor (CPU, {} ms/frame)",
                    t.render_ms.load(std::sync::atomic::Ordering::Relaxed)
                ),
            };
            *relock(&shared.diag) = EngineDiag {
                decode_path: path,
                dropped_frames: d.dropped_frames,
                presented_frames: d.presented_frames,
                av_offset_ms: (d.av_offset.count > 0).then(|| d.av_offset.mean_us as f64 / 1000.0),
                seek_p95_ms: (d.seek_latency.count > 0).then(|| d.seek_latency.p95_us as f64 / 1000.0),
            };
        }
        let snap = snapshot(session.as_ref(), volume, muted, audio_available);
        if last_sent.as_ref() != Some(&snap) {
            *relock(&shared.state) = snap.clone();
            let _ = PlaybackChanged(snap.clone()).emit(&app);
            last_sent = Some(snap);
        }
    }
}

/// Validates the path (plain-language errors for missing, folder, unsupported, empty) before decoding.
pub fn precheck(path: &std::path::Path) -> AppResult<()> {
    media::open(path).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::frame_index;

    #[test]
    fn frame_index_is_exact_at_frame_boundaries() {
        for (fps, n) in [(25.0, 72), (30.0, 100), (24.0, 1), (60.0, 3599)] {
            for k in 0..n {
                let pts = (k as f64 / fps * 1e6).round() as i64;
                assert_eq!(frame_index(pts, fps), k, "fps {fps} frame {k}");
            }
        }
        // NTSC rates: 29.97 fps pts rounded to microseconds.
        let fps = 30000.0 / 1001.0;
        for k in 0..2000u32 {
            let pts = (k as f64 * 1001.0 / 30000.0 * 1e6).round() as i64;
            assert_eq!(frame_index(pts, fps), k);
        }
    }
}
