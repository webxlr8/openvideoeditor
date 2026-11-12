//! The playback engine: transport + clock + scheduler over a frame source, driven by `tick(wall)` once per display
//! refresh (or faster). Presents a frame when its pts ≤ clock; drops it when it is late by more than one frame
//! interval or superseded by a later due frame.

use crate::audio::AudioRing;
use crate::clock::{ClockSource, Micros, PlaybackClock};
use crate::diagnostics::{Diagnostics, DiagnosticsSnapshot};
use crate::transport::{Command, Effect, Speed, Transport};

/// A decoded frame as the scheduler sees it.
pub trait PresentableFrame {
    fn pts_us(&self) -> Micros;
    /// 0 when unknown (the nominal interval is used).
    fn duration_us(&self) -> Micros;
}

/// Result of asking a source for its next frame without blocking.
#[derive(Debug)]
pub enum SourcePoll<F> {
    Frame(F),
    /// Not decoded yet.
    Pending,
    Eof,
    Error(String),
}

/// Decoded frames in presentation order with exact seeking (see `ove_media::VideoDecoder::seek`).
pub trait FrameSource {
    type Frame: PresentableFrame;
    fn seek(&mut self, t_us: Micros);
    fn poll(&mut self) -> SourcePoll<Self::Frame>;
}

#[derive(Debug, Clone, Copy)]
pub struct PlayerConfig {
    /// Nominal frame interval (1 / average frame rate), used when a frame has no duration.
    pub frame_interval_us: Micros,
    /// Presentation time of the last frame.
    pub last_frame_us: Micros,
}

/// What the host must do after a tick.
#[derive(Debug)]
pub struct TickOutput<F> {
    /// New frame to display.
    pub present: Option<F>,
    /// Audio must restart from this media time: seek the audio decoder there and refill the ring.
    pub audio_restart: Option<Micros>,
    /// Playback stopped because of this error (decoder failure, media offline).
    pub error: Option<String>,
}

impl<F> Default for TickOutput<F> {
    fn default() -> Self {
        TickOutput { present: None, audio_restart: None, error: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Waiting {
    None,
    /// First frame at a new position; `measure` records seek latency.
    Seek {
        target: Micros,
        since: Micros,
        measure: bool,
    },
    Step,
    /// Reverse playback asked for the frame containing `target`.
    Reverse,
}

pub struct Player<S: FrameSource> {
    source: S,
    transport: Transport,
    clock: PlaybackClock,
    interval: Micros,
    next: Option<S::Frame>,
    current: Option<(Micros, Micros)>, // (pts, end)
    waiting: Waiting,
    eof: bool,
    diag: Diagnostics,
    audio: Option<AudioRing>,
    audio_restart: Option<Micros>,
}

impl<S: FrameSource> Player<S> {
    /// Create a paused player; the first frame is requested immediately (shown on the first tick it is ready).
    pub fn new(source: S, config: PlayerConfig, audio: Option<AudioRing>, wall: Micros) -> Self {
        let mut p = Player {
            source,
            transport: Transport::new(config.last_frame_us),
            clock: PlaybackClock::default(),
            interval: config.frame_interval_us.max(1),
            next: None,
            current: None,
            waiting: Waiting::None,
            eof: false,
            diag: Diagnostics::default(),
            audio,
            audio_restart: None,
        };
        p.seek_source(0, wall, false);
        p
    }

    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// True when no seek or step is waiting for its frame. Hosts queue further steps until then, because a step
    /// issued while another is pending is merged into it.
    pub fn is_settled(&self) -> bool {
        self.waiting == Waiting::None
    }

    pub fn clock_source(&self) -> ClockSource {
        self.clock.source()
    }

    pub fn diagnostics(&self) -> DiagnosticsSnapshot {
        self.diag.snapshot(self.clock.source())
    }

    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    /// Attach or detach the audio output (device appeared / lost). Without audio the system clock is used.
    pub fn set_audio(&mut self, audio: Option<AudioRing>, wall: Micros) {
        self.audio = audio;
        self.update_audio(wall, true);
    }

    pub fn set_volume(&mut self, v: f32) {
        if let Some(a) = &self.audio {
            a.set_volume(v);
        }
    }

    pub fn set_muted(&mut self, m: bool) {
        if let Some(a) = &self.audio {
            a.set_muted(m);
        }
    }

    pub fn command(&mut self, cmd: Command, wall: Micros) {
        for e in self.transport.handle(cmd) {
            self.apply(e, wall);
        }
    }

    fn seek_source(&mut self, t: Micros, wall: Micros, measure: bool) {
        self.source.seek(t);
        self.next = None;
        self.eof = false;
        self.waiting = Waiting::Seek { target: t, since: wall, measure };
        self.clock.pause(wall);
        self.clock.set_position(t, wall);
        if let Some(a) = &self.audio {
            a.clear();
            a.set_paused(true);
        }
    }

    fn apply(&mut self, e: Effect, wall: Micros) {
        match e {
            Effect::Seek(t) => self.seek_source(t, wall, true),
            Effect::StepForward => {
                if !matches!(self.waiting, Waiting::Seek { .. }) {
                    self.waiting = Waiting::Step;
                }
            }
            Effect::StepBackward => {
                if let Some((pts, _)) = self.current
                    && pts > 0
                {
                    self.seek_source(pts - 1, wall, false);
                }
            }
            Effect::Run => {
                if !matches!(self.waiting, Waiting::Seek { .. }) {
                    {
                        let from = self.clock_position(wall);
                        self.start_clock(from, wall);
                    }
                }
            }
            Effect::Halt => {
                self.clock.pause(wall);
                if let Some((pts, _)) = self.current {
                    self.clock.set_position(pts, wall);
                }
                if matches!(self.waiting, Waiting::Reverse) {
                    self.waiting = Waiting::None;
                }
                self.update_audio(wall, false);
            }
        }
    }

    fn clock_position(&mut self, wall: Micros) -> Micros {
        match self.current {
            Some((pts, _)) if !self.clock.is_running() => pts,
            _ => self.clock.media_time(wall),
        }
    }

    fn start_clock(&mut self, from: Micros, wall: Micros) {
        if self.transport.speed.is_reverse() {
            // Reverse fetches frames by position; the sequential lookahead is not valid.
            self.next = None;
        }
        self.clock.start(from, wall, self.transport.speed.factor());
        self.update_audio(wall, true);
    }

    fn update_audio(&mut self, wall: Micros, restart: bool) {
        let audible = self.transport.audio_audible() && self.clock.is_running();
        self.clock.set_audio_enabled(audible && self.audio.is_some());
        if let Some(a) = &self.audio {
            if audible {
                if restart {
                    let t = self.clock.media_time(wall);
                    a.clear();
                    self.audio_restart = Some(t);
                }
                a.set_paused(false);
            } else {
                a.set_paused(true);
            }
        }
    }

    fn present(&mut self, f: &S::Frame) {
        let interval = if f.duration_us() > 0 { f.duration_us() } else { self.interval };
        self.current = Some((f.pts_us(), f.pts_us() + interval));
        self.transport.position = f.pts_us();
        self.diag.presented += 1;
    }

    fn superseded(&mut self) {
        if self.transport.speed == Speed::Fwd1 {
            self.diag.dropped += 1;
        } else {
            self.diag.skipped += 1;
        }
    }

    fn fail(&mut self, e: String, wall: Micros, out: &mut TickOutput<S::Frame>) {
        out.error = Some(e);
        self.waiting = Waiting::None;
        for fx in self.transport.handle(Command::Pause) {
            self.apply(fx, wall);
        }
    }

    pub fn tick(&mut self, wall: Micros) -> TickOutput<S::Frame> {
        let mut out = TickOutput::default();
        if let Some(a) = &self.audio
            && let Some(p) = a.position()
        {
            self.clock.on_audio_position(p);
        }

        match self.waiting {
            Waiting::Seek { target, since, measure } => match self.source.poll() {
                SourcePoll::Frame(f) => {
                    self.waiting = Waiting::None;
                    if measure {
                        self.diag.seek_latency.push(wall - since);
                    }
                    self.present(&f);
                    if self.transport.is_playing() {
                        self.start_clock(target, wall);
                    } else {
                        self.clock.set_position(f.pts_us(), wall);
                    }
                    out.present = Some(f);
                }
                SourcePoll::Pending => {}
                SourcePoll::Eof => {
                    self.waiting = Waiting::None;
                    self.eof = true;
                }
                SourcePoll::Error(e) => self.fail(e, wall, &mut out),
            },
            Waiting::Step => {
                let polled = match self.next.take() {
                    Some(f) => SourcePoll::Frame(f),
                    None => self.source.poll(),
                };
                match polled {
                    SourcePoll::Frame(f) => {
                        self.waiting = Waiting::None;
                        self.present(&f);
                        self.clock.set_position(f.pts_us(), wall);
                        out.present = Some(f);
                    }
                    SourcePoll::Pending => {}
                    SourcePoll::Eof => {
                        self.waiting = Waiting::None;
                        self.eof = true;
                    }
                    SourcePoll::Error(e) => self.fail(e, wall, &mut out),
                }
            }
            Waiting::Reverse => match self.source.poll() {
                SourcePoll::Frame(f) => {
                    self.waiting = Waiting::None;
                    self.present(&f);
                    out.present = Some(f);
                }
                SourcePoll::Pending => {}
                SourcePoll::Eof => self.waiting = Waiting::None,
                SourcePoll::Error(e) => self.fail(e, wall, &mut out),
            },
            Waiting::None => {
                if self.transport.is_playing() && self.clock.is_running() {
                    if self.transport.speed.is_reverse() {
                        self.tick_reverse(wall, &mut out);
                    } else {
                        self.tick_forward(wall, &mut out);
                    }
                }
            }
        }
        out.audio_restart = self.audio_restart.take();
        out
    }

    fn tick_forward(&mut self, wall: Micros, out: &mut TickOutput<S::Frame>) {
        let t = self.clock.media_time(wall);
        let mut chosen: Option<S::Frame> = None;
        loop {
            if self.next.is_none() {
                match self.source.poll() {
                    SourcePoll::Frame(f) => self.next = Some(f),
                    SourcePoll::Pending => break,
                    SourcePoll::Eof => {
                        self.eof = true;
                        break;
                    }
                    SourcePoll::Error(e) => {
                        self.fail(e, wall, out);
                        return;
                    }
                }
            }
            match self.next.take_if(|f| f.pts_us() <= t) {
                Some(f) => {
                    if chosen.replace(f).is_some() {
                        self.superseded();
                    }
                }
                None => break,
            }
        }
        if let Some(f) = chosen {
            let interval = if f.duration_us() > 0 { f.duration_us() } else { self.interval };
            let late = t - f.pts_us();
            if late > interval {
                self.superseded();
            } else {
                if self.transport.speed == Speed::Fwd1 {
                    self.diag.av_offset.push(late.abs());
                }
                self.present(&f);
                out.present = Some(f);
            }
        }
        if self.eof && self.next.is_none() && self.current.is_none_or(|(_, end)| t >= end) {
            for fx in self.transport.on_boundary() {
                self.apply(fx, wall);
            }
        }
    }

    fn tick_reverse(&mut self, wall: Micros, out: &mut TickOutput<S::Frame>) {
        let t = self.clock.media_time(wall);
        if t < 0 {
            for fx in self.transport.on_boundary() {
                self.apply(fx, wall);
            }
            return;
        }
        if let Some((pts, _)) = self.current
            && t < pts
        {
            // Frames between the displayed one and the target are skipped on purpose.
            let skipped = ((pts - t) / self.interval).max(1) - 1;
            self.diag.skipped += skipped as u64;
            self.source.seek(t);
            self.next = None;
            self.eof = false;
            self.waiting = Waiting::Reverse;
            // Fast sources answer immediately.
            if let SourcePoll::Frame(f) = self.source.poll() {
                self.waiting = Waiting::None;
                self.present(&f);
                out.present = Some(f);
            }
        }
    }
}
