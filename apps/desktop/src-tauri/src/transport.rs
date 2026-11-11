//! Transport state for the viewer: play/pause, step, J/K/L shuttle, loop, seek, volume.
//!
//! Interim engine used by the shell until ove-playback is wired in; the command and state types
//! are the bridge contract and stay. Time is in frames of a constant-rate source.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Transport commands sent by the UI (buttons and keys).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TransportCommand {
    Play,
    Pause,
    TogglePlay,
    /// One frame forward (positive) or back (negative); pauses.
    Step {
        frames: i32,
    },
    GoToStart,
    GoToEnd,
    /// L key: 1× forward, then 2×.
    ShuttleForward,
    /// J key: 1× reverse, then 2×.
    ShuttleReverse,
    /// Seek to an exact frame (clamped to the media).
    SeekFrame {
        frame: u32,
    },
    SetLoop {
        enabled: bool,
    },
}

/// Snapshot of the transport, sent with every change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    pub has_media: bool,
    pub frame: u32,
    pub total_frames: u32,
    pub frame_rate: f64,
    pub playing: bool,
    /// −2, −1, 1 or 2 while playing; 0 when paused.
    pub speed: f64,
    pub looping: bool,
    pub volume: f64,
    pub muted: bool,
    /// False when no audio output device is available (video then runs on the system clock).
    pub audio_available: bool,
}

/// The transport state machine. `tick` advances one frame in the current direction.
#[derive(Debug, Clone)]
pub struct Transport {
    pub state: PlaybackState,
}

impl Default for Transport {
    fn default() -> Self {
        Transport {
            state: PlaybackState {
                has_media: false,
                frame: 0,
                total_frames: 0,
                frame_rate: 30.0,
                playing: false,
                speed: 0.0,
                looping: false,
                volume: 1.0,
                muted: false,
                audio_available: false,
            },
        }
    }
}

impl Transport {
    pub fn load(&mut self, total_frames: u32, frame_rate: f64) {
        let s = &mut self.state;
        s.has_media = total_frames > 0;
        s.total_frames = total_frames;
        s.frame_rate = frame_rate;
        s.frame = 0;
        s.playing = false;
        s.speed = 0.0;
    }

    fn last(&self) -> u32 {
        self.state.total_frames.saturating_sub(1)
    }

    fn set_speed(&mut self, speed: f64) {
        self.state.speed = speed;
        self.state.playing = speed != 0.0;
    }

    pub fn apply(&mut self, cmd: TransportCommand) {
        if !self.state.has_media {
            if let TransportCommand::SetLoop { enabled } = cmd {
                self.state.looping = enabled;
            }
            return;
        }
        match cmd {
            TransportCommand::Play => {
                if self.state.frame >= self.last() && !self.state.looping {
                    self.state.frame = 0;
                }
                self.set_speed(1.0);
            }
            TransportCommand::Pause => self.set_speed(0.0),
            TransportCommand::TogglePlay => {
                if self.state.playing {
                    self.apply(TransportCommand::Pause)
                } else {
                    self.apply(TransportCommand::Play)
                }
            }
            TransportCommand::Step { frames } => {
                self.set_speed(0.0);
                let target = i64::from(self.state.frame) + i64::from(frames);
                self.state.frame = target.clamp(0, i64::from(self.last())) as u32;
            }
            TransportCommand::GoToStart => {
                self.set_speed(0.0);
                self.state.frame = 0;
            }
            TransportCommand::GoToEnd => {
                self.set_speed(0.0);
                self.state.frame = self.last();
            }
            TransportCommand::ShuttleForward => {
                let next = if self.state.speed >= 1.0 { 2.0 } else { 1.0 };
                self.set_speed(next);
            }
            TransportCommand::ShuttleReverse => {
                let next = if self.state.speed <= -1.0 { -2.0 } else { -1.0 };
                self.set_speed(next);
            }
            TransportCommand::SeekFrame { frame } => self.state.frame = frame.min(self.last()),
            TransportCommand::SetLoop { enabled } => self.state.looping = enabled,
        }
    }

    pub fn set_volume(&mut self, volume: f64, muted: bool) {
        self.state.volume = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 1.0 };
        self.state.muted = muted;
    }

    /// Advances one frame in the playing direction. Returns true if the frame changed.
    pub fn tick(&mut self) -> bool {
        if !self.state.playing || !self.state.has_media {
            return false;
        }
        let last = self.last();
        let before = self.state.frame;
        if self.state.speed > 0.0 {
            if before >= last {
                if self.state.looping {
                    self.state.frame = 0;
                } else {
                    self.set_speed(0.0);
                }
            } else {
                self.state.frame += 1;
            }
        } else if before == 0 {
            if self.state.looping {
                self.state.frame = last;
            } else {
                self.set_speed(0.0);
            }
        } else {
            self.state.frame -= 1;
        }
        self.state.frame != before
    }

    /// Interval between ticks at the current speed.
    pub fn tick_interval(&self) -> std::time::Duration {
        let rate = self.state.frame_rate.max(1.0) * self.state.speed.abs().max(1.0);
        std::time::Duration::from_secs_f64(1.0 / rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TransportCommand::*;

    fn loaded() -> Transport {
        let mut t = Transport::default();
        t.load(100, 25.0);
        t
    }

    #[test]
    fn ten_steps_advance_exactly_ten_frames() {
        let mut t = loaded();
        t.apply(SeekFrame { frame: 40 });
        for _ in 0..10 {
            t.apply(Step { frames: 1 });
        }
        assert_eq!(t.state.frame, 50);
        assert!(!t.state.playing);
        t.apply(Step { frames: -60 });
        assert_eq!(t.state.frame, 0);
    }

    #[test]
    fn jkl_shuttle() {
        let mut t = loaded();
        t.apply(ShuttleForward);
        assert_eq!(t.state.speed, 1.0);
        t.apply(ShuttleForward);
        assert_eq!(t.state.speed, 2.0);
        t.apply(ShuttleForward);
        assert_eq!(t.state.speed, 2.0);
        t.apply(ShuttleReverse);
        assert_eq!(t.state.speed, -1.0);
        t.apply(ShuttleReverse);
        assert_eq!(t.state.speed, -2.0);
        t.apply(Pause);
        assert!(!t.state.playing);
        assert_eq!(t.state.speed, 0.0);
    }

    #[test]
    fn plays_to_end_and_stops_or_loops() {
        let mut t = loaded();
        t.apply(GoToEnd);
        assert_eq!(t.state.frame, 99);
        t.apply(Play);
        assert_eq!(t.state.frame, 0, "play at the end restarts");
        t.apply(SeekFrame { frame: 98 });
        assert!(t.tick());
        assert!(!t.tick());
        assert!(!t.state.playing, "stops at the last frame");
        t.apply(SetLoop { enabled: true });
        t.apply(Play);
        assert!(t.tick());
        assert_eq!(t.state.frame, 0, "loops to the start");
    }

    #[test]
    fn reverse_stops_at_start_or_loops_to_end() {
        let mut t = loaded();
        t.apply(SeekFrame { frame: 1 });
        t.apply(ShuttleReverse);
        assert!(t.tick());
        assert!(!t.tick());
        assert!(!t.state.playing);
        t.apply(SetLoop { enabled: true });
        t.apply(ShuttleReverse);
        t.tick();
        assert_eq!(t.state.frame, 99);
    }

    #[test]
    fn toggle_home_end_seek_clamp() {
        let mut t = loaded();
        t.apply(TogglePlay);
        assert!(t.state.playing);
        t.apply(TogglePlay);
        assert!(!t.state.playing);
        t.apply(SeekFrame { frame: 10_000 });
        assert_eq!(t.state.frame, 99);
        t.apply(GoToStart);
        assert_eq!(t.state.frame, 0);
    }

    #[test]
    fn no_media_ignores_transport() {
        let mut t = Transport::default();
        t.apply(Play);
        assert!(!t.state.playing);
        assert!(!t.tick());
    }

    #[test]
    fn volume_is_clamped() {
        let mut t = loaded();
        t.set_volume(3.0, true);
        assert_eq!((t.state.volume, t.state.muted), (1.0, true));
        t.set_volume(f64::NAN, false);
        assert_eq!(t.state.volume, 1.0);
    }

    #[test]
    fn double_speed_halves_interval() {
        let mut t = loaded();
        t.apply(ShuttleForward);
        let one = t.tick_interval();
        t.apply(ShuttleForward);
        assert_eq!(t.tick_interval() * 2, one);
    }
}
