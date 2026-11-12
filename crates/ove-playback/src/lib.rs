//! Playback clock, scheduler, A/V sync, transport state machine and diagnostics.
//!
//! Pure and UI-free: the engine is driven by `Player::tick(wall)` with wall time from a [`TimeSource`], so every
//! transport transition and the frame scheduling are unit-tested on simulated time. Frames come from any
//! [`FrameSource`] (the `media` feature adapts `ove_media::DecodeWorker`); audio output is the `audio-output`
//! feature (cpal), with the system clock used when no device is available.
#![forbid(unsafe_code)]

mod audio;
mod clock;
mod diagnostics;
#[cfg(feature = "media")]
mod media;
#[cfg(feature = "audio-output")]
mod output;
mod player;
mod transport;

pub use audio::AudioRing;
pub use clock::{AudioPosition, ClockSource, Micros, PlaybackClock, SimTime, SystemTime, TimeSource};
pub use diagnostics::{Diagnostics, DiagnosticsSnapshot, Samples, Stats};
#[cfg(feature = "media")]
pub use media::WorkerSource;
#[cfg(feature = "audio-output")]
pub use output::{AudioOutput, AudioUnavailable, DEVICE_POLL, OutputStatus};
pub use player::{FrameSource, Player, PlayerConfig, PresentableFrame, SourcePoll, TickOutput};
pub use transport::{Command, Effect, PlayState, Speed, Transport};
