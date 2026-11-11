//! Transport state machine: play/pause, J/K/L shuttle, step, jump, seek, loop. Pure: it only emits effects.

use serde::Serialize;

use crate::clock::Micros;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayState {
    Paused,
    Playing,
}

/// Shuttle speeds. Anything other than 1× plays video only (audio muted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Speed {
    #[serde(rename = "-2")]
    Rev2,
    #[serde(rename = "-1")]
    Rev1,
    #[serde(rename = "1")]
    Fwd1,
    #[serde(rename = "2")]
    Fwd2,
}

impl Speed {
    pub fn factor(self) -> f64 {
        match self {
            Speed::Rev2 => -2.0,
            Speed::Rev1 => -1.0,
            Speed::Fwd1 => 1.0,
            Speed::Fwd2 => 2.0,
        }
    }
    pub fn is_reverse(self) -> bool {
        self.factor() < 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum Command {
    Play,
    Pause,
    TogglePlay,
    /// J: reverse; repeated → faster reverse.
    ShuttleReverse,
    /// K: pause.
    ShuttleStop,
    /// L: forward; repeated → faster forward.
    ShuttleForward,
    StepForward,
    StepBackward,
    GoToStart,
    GoToEnd,
    Seek(Micros),
    SetLoop(bool),
}

/// What the engine must do as a result of a command or event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reposition the frame source; the first frame at the target is shown when it arrives.
    Seek(Micros),
    /// Show the frame after the current one.
    StepForward,
    /// Show the frame before the current one.
    StepBackward,
    /// (Re)start the clock at the current position with the transport's speed.
    Run,
    /// Stop the clock.
    Halt,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transport {
    pub state: PlayState,
    pub speed: Speed,
    pub looping: bool,
    /// Presentation time of the displayed frame.
    pub position: Micros,
    /// Presentation time of the last frame (seek target for End).
    pub last_frame: Micros,
}

impl Transport {
    pub fn new(last_frame: Micros) -> Self {
        Transport {
            state: PlayState::Paused,
            speed: Speed::Fwd1,
            looping: false,
            position: 0,
            last_frame,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.state == PlayState::Playing
    }

    /// Audio is heard only while playing forward at 1×.
    pub fn audio_audible(&self) -> bool {
        self.is_playing() && self.speed == Speed::Fwd1
    }

    fn play(&mut self, speed: Speed) -> Vec<Effect> {
        let was = (self.state, self.speed);
        self.state = PlayState::Playing;
        self.speed = speed;
        if was == (PlayState::Playing, speed) {
            return vec![];
        }
        // Play at the end (forward) or at the start (reverse) wraps around first.
        if !speed.is_reverse() && self.position >= self.last_frame && self.last_frame > 0 {
            self.position = 0;
            return vec![Effect::Seek(0), Effect::Run];
        }
        if speed.is_reverse() && self.position <= 0 {
            self.position = self.last_frame;
            return vec![Effect::Seek(self.last_frame), Effect::Run];
        }
        vec![Effect::Run]
    }

    fn pause(&mut self) -> Vec<Effect> {
        let was_playing = self.is_playing();
        self.state = PlayState::Paused;
        self.speed = Speed::Fwd1;
        if was_playing {
            vec![Effect::Halt]
        } else {
            vec![]
        }
    }

    pub fn handle(&mut self, cmd: Command) -> Vec<Effect> {
        match cmd {
            Command::Play => self.play(Speed::Fwd1),
            Command::Pause | Command::ShuttleStop => self.pause(),
            Command::TogglePlay => {
                if self.is_playing() {
                    self.pause()
                } else {
                    self.play(Speed::Fwd1)
                }
            }
            Command::ShuttleForward => {
                let next = match (self.state, self.speed) {
                    (PlayState::Playing, Speed::Fwd1) | (PlayState::Playing, Speed::Fwd2) => {
                        Speed::Fwd2
                    }
                    _ => Speed::Fwd1,
                };
                self.play(next)
            }
            Command::ShuttleReverse => {
                let next = match (self.state, self.speed) {
                    (PlayState::Playing, Speed::Rev1) | (PlayState::Playing, Speed::Rev2) => {
                        Speed::Rev2
                    }
                    _ => Speed::Rev1,
                };
                self.play(next)
            }
            Command::StepForward | Command::StepBackward => {
                let mut fx = self.pause();
                fx.push(if cmd == Command::StepForward {
                    Effect::StepForward
                } else {
                    Effect::StepBackward
                });
                fx
            }
            Command::GoToStart => self.seek(0),
            Command::GoToEnd => self.seek(self.last_frame),
            Command::Seek(t) => self.seek(t),
            Command::SetLoop(on) => {
                self.looping = on;
                vec![]
            }
        }
    }

    fn seek(&mut self, t: Micros) -> Vec<Effect> {
        let t = t.clamp(0, self.last_frame.max(0));
        self.position = t;
        vec![Effect::Seek(t)]
    }

    /// The engine reached the end (forward) or the start (reverse) of the media while playing.
    pub fn on_boundary(&mut self) -> Vec<Effect> {
        if !self.is_playing() {
            return vec![];
        }
        if self.looping {
            let t = if self.speed.is_reverse() {
                self.last_frame
            } else {
                0
            };
            self.position = t;
            vec![Effect::Seek(t), Effect::Run]
        } else {
            self.pause()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Transport {
        let mut t = Transport::new(10_000_000);
        t.position = 5_000_000;
        t
    }

    #[test]
    fn play_pause_toggle() {
        let mut x = t();
        assert_eq!(x.handle(Command::Play), vec![Effect::Run]);
        assert!(x.is_playing() && x.audio_audible());
        assert_eq!(x.handle(Command::Play), vec![]);
        assert_eq!(x.handle(Command::TogglePlay), vec![Effect::Halt]);
        assert_eq!(x.state, PlayState::Paused);
        assert_eq!(x.handle(Command::Pause), vec![]);
        assert_eq!(x.handle(Command::TogglePlay), vec![Effect::Run]);
    }

    #[test]
    fn jkl_shuttle() {
        let mut x = t();
        x.handle(Command::ShuttleForward);
        assert_eq!((x.state, x.speed), (PlayState::Playing, Speed::Fwd1));
        x.handle(Command::ShuttleForward);
        assert_eq!(x.speed, Speed::Fwd2);
        assert!(!x.audio_audible(), "2× is video only");
        x.handle(Command::ShuttleForward);
        assert_eq!(x.speed, Speed::Fwd2, "2× is the maximum");
        x.handle(Command::ShuttleReverse);
        assert_eq!(x.speed, Speed::Rev1, "J while going forward reverses at 1×");
        assert!(!x.audio_audible());
        x.handle(Command::ShuttleReverse);
        assert_eq!(x.speed, Speed::Rev2);
        x.handle(Command::ShuttleReverse);
        assert_eq!(x.speed, Speed::Rev2);
        x.handle(Command::ShuttleForward);
        assert_eq!(
            x.speed,
            Speed::Fwd1,
            "L while reversing plays forward at 1×"
        );
        assert_eq!(x.handle(Command::ShuttleStop), vec![Effect::Halt]);
        assert_eq!((x.state, x.speed), (PlayState::Paused, Speed::Fwd1));
        x.handle(Command::ShuttleReverse);
        assert_eq!(
            (x.state, x.speed),
            (PlayState::Playing, Speed::Rev1),
            "J from pause"
        );
    }

    #[test]
    fn step_pauses_first() {
        let mut x = t();
        assert_eq!(x.handle(Command::StepForward), vec![Effect::StepForward]);
        x.handle(Command::Play);
        assert_eq!(
            x.handle(Command::StepBackward),
            vec![Effect::Halt, Effect::StepBackward]
        );
        assert_eq!(x.state, PlayState::Paused);
    }

    #[test]
    fn jumps_and_seek_clamp() {
        let mut x = t();
        assert_eq!(x.handle(Command::GoToStart), vec![Effect::Seek(0)]);
        assert_eq!(x.handle(Command::GoToEnd), vec![Effect::Seek(10_000_000)]);
        assert_eq!(x.handle(Command::Seek(-5)), vec![Effect::Seek(0)]);
        assert_eq!(
            x.handle(Command::Seek(99_000_000)),
            vec![Effect::Seek(10_000_000)]
        );
        x.handle(Command::Play);
        assert_eq!(
            x.handle(Command::Seek(3_000_000)),
            vec![Effect::Seek(3_000_000)]
        );
        assert!(x.is_playing(), "seeking keeps playing");
    }

    #[test]
    fn play_at_end_restarts_and_reverse_at_start_wraps() {
        let mut x = t();
        x.position = 10_000_000;
        assert_eq!(x.handle(Command::Play), vec![Effect::Seek(0), Effect::Run]);
        let mut y = t();
        y.position = 0;
        assert_eq!(
            y.handle(Command::ShuttleReverse),
            vec![Effect::Seek(10_000_000), Effect::Run]
        );
    }

    #[test]
    fn boundary_loops_or_pauses() {
        let mut x = t();
        x.handle(Command::Play);
        assert_eq!(x.on_boundary(), vec![Effect::Halt]);
        assert_eq!(x.state, PlayState::Paused);
        x.handle(Command::SetLoop(true));
        x.handle(Command::Play);
        assert_eq!(x.on_boundary(), vec![Effect::Seek(0), Effect::Run]);
        x.handle(Command::ShuttleReverse);
        assert_eq!(x.on_boundary(), vec![Effect::Seek(10_000_000), Effect::Run]);
        x.handle(Command::Pause);
        assert_eq!(x.on_boundary(), vec![]);
    }
}
