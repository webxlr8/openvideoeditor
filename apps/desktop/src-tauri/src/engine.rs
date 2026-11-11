//! The playback engine thread: real decode (`ove-media`, hardware first with software fallback) driven by the
//! transport, clock and scheduler of `ove-playback`, with audio output when a device exists. Decoded frames go to the
//! viewer thread; state changes go to the UI as `playbackChanged` events.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ove_media::{AudioDecoder, AudioOutputSpec, DecodeWorker, DecoderOptions, SampleFormat, Samples};
use ove_playback::{AudioOutput, Command, Player, PlayerConfig, SystemTime, TimeSource, WorkerSource};
use tauri::AppHandle;
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::media::{self, MediaInfo};
use crate::transport::{PlaybackState, TransportCommand};
use crate::viewer::ViewerMsg;
use crate::{AppErrorRaised, PlaybackChanged};

/// Audio kept decoded ahead of the play head.
const AUDIO_AHEAD_US: i64 = 600_000;

pub enum EngineMsg {
    Open(PathBuf, Sender<AppResult<MediaInfo>>),
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
}

#[derive(Clone)]
pub struct EngineHandle {
    pub tx: Sender<EngineMsg>,
    pub shared: Arc<Shared>,
}

struct Audio {
    out: AudioOutput,
    decoder: AudioDecoder,
    /// Media time the decoder has filled the ring up to.
    filled_to: i64,
    eof: bool,
}

struct Session {
    player: Player<WorkerSource>,
    info: MediaInfo,
    fps: f64,
    audio: Option<Audio>,
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
        let source = WorkerSource::new(DecodeWorker::spawn(Box::new(decoder), ove_media::DEFAULT_QUEUE_FRAMES));
        let last_frame = (duration_us - interval).max(0);

        let audio = match (&probe.audio, AudioOutput::open_default(time.clone())) {
            (Some(_), Ok(out)) => {
                let (rate, channels) = out.ring().format();
                let spec = AudioOutputSpec { sample_rate: rate, channels, format: SampleFormat::F32 };
                match AudioDecoder::open(path, spec) {
                    Ok(decoder) => Some(Audio { out, decoder, filled_to: 0, eof: false }),
                    Err(e) => {
                        tracing::warn!(error = %e, "audio decode unavailable; video plays on the system clock");
                        None
                    }
                }
            }
            (_, Err(e)) => {
                *audio_ok = false;
                tracing::warn!(error = %e, "no audio output device; video plays on the system clock");
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
        Ok(Session { player, info, fps, audio })
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
            match a.decoder.next_chunk() {
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
            }
        }
    }

    fn restart_audio(&mut self, t_us: i64) {
        if let Some(a) = self.audio.as_mut() {
            a.out.ring().clear();
            a.eof = a.decoder.seek(t_us).is_err();
            a.filled_to = t_us;
        }
    }
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
    let (tx, rx) = channel::<EngineMsg>();
    let shared = Arc::new(Shared::default());
    let sh = shared.clone();
    crate::worker::spawn_contained("playback", move || run(app, viewer, rx, sh))?;
    Ok(EngineHandle { tx, shared })
}

fn run(app: AppHandle, viewer: Sender<ViewerMsg>, rx: Receiver<EngineMsg>, shared: Arc<Shared>) {
    let time: Arc<dyn TimeSource> = Arc::new(SystemTime::new());
    let mut session: Option<Session> = None;
    let (mut volume, mut muted) = (1.0f64, false);
    let mut audio_available = false;
    let mut last_sent: Option<PlaybackState> = None;
    // Frames still to step (positive forward, negative back); any other transport command cancels them.
    let mut pending_steps: i64 = 0;
    loop {
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
                    *shared.media.lock().unwrap() = Some(info.clone());
                    info
                });
                if let Err(e) = &result {
                    tracing::warn!(path = %path.display(), "open failed: {}", e.message);
                }
                let _ = reply.send(result);
            }
            Ok(EngineMsg::Close) => {
                session = None;
                pending_steps = 0;
                *shared.media.lock().unwrap() = None;
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
            if let Some(f) = out.present.and_then(to_gpu_frame) {
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
            let path = s.player.source_mut().decode_path().to_string();
            if let Ok(mut g) = shared.diag.lock() {
                *g = EngineDiag {
                    decode_path: path,
                    dropped_frames: d.dropped_frames,
                    presented_frames: d.presented_frames,
                    av_offset_ms: (d.av_offset.count > 0).then(|| d.av_offset.mean_us as f64 / 1000.0),
                    seek_p95_ms: (d.seek_latency.count > 0).then(|| d.seek_latency.p95_us as f64 / 1000.0),
                };
            }
        }
        let snap = snapshot(session.as_ref(), volume, muted, audio_available);
        if last_sent.as_ref() != Some(&snap) {
            *shared.state.lock().unwrap() = snap.clone();
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
