//! Media probe and decode for preview (FFmpeg, hardware first, software fallback).
//!
//! Public surface used by the playback engine, the desktop app and the benchmark:
//! - [`probe`] → [`MediaInfo`]
//! - [`open_video`] → a [`VideoDecoder`] (hardware with automatic software fallback, exact seek)
//! - [`DecodeWorker`] → the decoder on its own thread feeding a bounded frame queue
//! - [`AudioDecoder`] → decoded, resampled audio in the output device's format

mod audio;
mod convert;
mod error;
mod fallback;
mod frame;
mod probe;
mod util;
mod video;
mod worker;

pub use audio::{AudioChunk, AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
pub use convert::{ColorInfo, ColorMatrix};
pub use error::{MediaError, Result};
pub use fallback::{FallbackDecoder, open_video};
pub use frame::{DecodePath, OutputFormat, PixelFormat, Plane, VideoFrame};
pub use probe::{AudioStreamInfo, MediaInfo, VideoStreamInfo, probe};
pub use util::{Micros, PixelFormatInfo, Rational, init};
pub use video::{DecoderOptions, FfmpegVideoDecoder, HwAccel, VideoDecoder};
pub use worker::{DEFAULT_QUEUE_FRAMES, DecodeWorker, WorkerEvent};
