//! Media clock: audio-master while audio plays at 1×, system monotonic clock otherwise.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use serde::Serialize;

/// Microseconds.
pub type Micros = i64;

/// A monotonic wall clock in µs. The engine never reads `Instant` directly so tests can drive time.
pub trait TimeSource: Send + Sync {
    fn now_us(&self) -> Micros;
}

/// Real monotonic time since creation.
#[derive(Debug, Clone)]
pub struct SystemTime(Instant);

impl SystemTime {
    pub fn new() -> Self {
        SystemTime(Instant::now())
    }
}

impl Default for SystemTime {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeSource for SystemTime {
    fn now_us(&self) -> Micros {
        self.0.elapsed().as_micros() as Micros
    }
}

/// Manually advanced time for tests and simulations.
#[derive(Debug, Clone, Default)]
pub struct SimTime(Arc<AtomicI64>);

impl SimTime {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn advance(&self, us: Micros) {
        self.0.fetch_add(us, Ordering::SeqCst);
    }
    pub fn set(&self, us: Micros) {
        self.0.store(us, Ordering::SeqCst);
    }
}

impl TimeSource for SimTime {
    fn now_us(&self) -> Micros {
        self.0.load(Ordering::SeqCst)
    }
}

/// What the audio device reported in its last callback: the media time of the first sample written then, the wall
/// time of the callback, and the output latency (time until that sample is heard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioPosition {
    pub media_us: Micros,
    pub at_wall_us: Micros,
    pub latency_us: Micros,
}

impl AudioPosition {
    /// Media time of the sample being heard at `wall`.
    pub fn heard_at(&self, wall: Micros) -> Micros {
        self.media_us - self.latency_us + (wall - self.at_wall_us)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClockSource {
    Audio,
    System,
}

/// Maps wall time to media time.
#[derive(Debug, Clone)]
pub struct PlaybackClock {
    anchor_media: Micros,
    anchor_wall: Micros,
    speed: f64,
    running: bool,
    audio: Option<AudioPosition>,
    audio_enabled: bool,
    last: Option<Micros>,
}

impl Default for PlaybackClock {
    fn default() -> Self {
        PlaybackClock {
            anchor_media: 0,
            anchor_wall: 0,
            speed: 1.0,
            running: false,
            audio: None,
            audio_enabled: false,
            last: None,
        }
    }
}

impl PlaybackClock {
    /// Run from `media` at `wall` with `speed` (negative = reverse).
    pub fn start(&mut self, media: Micros, wall: Micros, speed: f64) {
        self.anchor_media = media;
        self.anchor_wall = wall;
        self.speed = speed;
        self.running = true;
        self.audio = None;
        self.last = None;
    }

    /// Freeze at the current media time.
    pub fn pause(&mut self, wall: Micros) {
        let now = self.media_time(wall);
        self.anchor_media = now;
        self.anchor_wall = wall;
        self.running = false;
        self.audio = None;
        self.last = None;
    }

    /// Jump to `media` without changing running state.
    pub fn set_position(&mut self, media: Micros, wall: Micros) {
        self.anchor_media = media;
        self.anchor_wall = wall;
        self.audio = None;
        self.last = None;
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Whether audio may master the clock (playing at 1× with an output device).
    pub fn set_audio_enabled(&mut self, enabled: bool) {
        self.audio_enabled = enabled;
        if !enabled {
            self.audio = None;
        }
    }

    /// Feed the latest audio device position.
    pub fn on_audio_position(&mut self, pos: AudioPosition) {
        if self.audio_enabled && self.running {
            self.audio = Some(pos);
        }
    }

    pub fn source(&self) -> ClockSource {
        if self.running && self.audio_enabled && self.audio.is_some() && self.speed == 1.0 {
            ClockSource::Audio
        } else {
            ClockSource::System
        }
    }

    /// Media time at `wall`. While running forward it never goes backwards (callback jitter is absorbed).
    pub fn media_time(&mut self, wall: Micros) -> Micros {
        if !self.running {
            return self.anchor_media;
        }
        let t = match (self.source(), self.audio) {
            (ClockSource::Audio, Some(a)) => a.heard_at(wall).max(self.anchor_media),
            _ => self.anchor_media + ((wall - self.anchor_wall) as f64 * self.speed).round() as Micros,
        };
        let t = match self.last {
            Some(l) if self.speed > 0.0 && t < l => l,
            Some(l) if self.speed < 0.0 && t > l => l,
            _ => t,
        };
        self.last = Some(t);
        t
    }

    /// Offset between the audio clock and the system-time extrapolation (diagnostics), if audio is mastering.
    pub fn audio_drift(&self, wall: Micros) -> Option<Micros> {
        let a = self.audio?;
        let sys = self.anchor_media + ((wall - self.anchor_wall) as f64 * self.speed).round() as Micros;
        Some(a.heard_at(wall) - sys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_runs_at_speed() {
        let mut c = PlaybackClock::default();
        assert_eq!(c.media_time(500), 0);
        c.start(1_000_000, 0, 1.0);
        assert_eq!(c.media_time(40_000), 1_040_000);
        c.start(1_000_000, 0, 2.0);
        assert_eq!(c.media_time(40_000), 1_080_000);
        c.start(1_000_000, 0, -1.0);
        assert_eq!(c.media_time(40_000), 960_000);
        c.pause(50_000);
        assert_eq!(c.media_time(900_000), 950_000);
        assert_eq!(c.source(), ClockSource::System);
    }

    #[test]
    fn audio_masters_at_1x_with_latency_correction() {
        let mut c = PlaybackClock::default();
        c.set_audio_enabled(true);
        c.start(0, 0, 1.0);
        // Device wrote media 100 ms at wall 90 ms with 20 ms output latency: heard now = 80 ms.
        c.on_audio_position(AudioPosition { media_us: 100_000, at_wall_us: 90_000, latency_us: 20_000 });
        assert_eq!(c.source(), ClockSource::Audio);
        assert_eq!(c.media_time(90_000), 80_000);
        assert_eq!(c.media_time(100_000), 90_000);
        assert_eq!(c.audio_drift(100_000), Some(-10_000));
    }

    #[test]
    fn audio_jitter_never_moves_time_backwards() {
        let mut c = PlaybackClock::default();
        c.set_audio_enabled(true);
        c.start(0, 0, 1.0);
        c.on_audio_position(AudioPosition { media_us: 50_000, at_wall_us: 40_000, latency_us: 0 });
        assert_eq!(c.media_time(45_000), 55_000);
        // A late callback reports a slightly earlier position.
        c.on_audio_position(AudioPosition { media_us: 52_000, at_wall_us: 46_000, latency_us: 0 });
        assert_eq!(c.media_time(46_000), 55_000);
        assert_eq!(c.media_time(50_000), 56_000);
    }

    #[test]
    fn audio_ignored_off_1x_or_when_disabled() {
        let mut c = PlaybackClock::default();
        c.start(0, 0, 2.0);
        c.set_audio_enabled(true);
        c.on_audio_position(AudioPosition { media_us: 5, at_wall_us: 0, latency_us: 0 });
        assert_eq!(c.source(), ClockSource::System);
        c.set_audio_enabled(false);
        c.start(0, 0, 1.0);
        c.on_audio_position(AudioPosition { media_us: 5, at_wall_us: 0, latency_us: 0 });
        assert_eq!(c.source(), ClockSource::System);
    }

    #[test]
    fn sim_time_is_shared() {
        let t = SimTime::new();
        let t2 = t.clone();
        t.advance(10);
        t2.advance(5);
        assert_eq!(t.now_us(), 15);
        let s = SystemTime::new();
        assert!(s.now_us() >= 0);
    }
}
