//! Media probe and decode for preview (FFmpeg, hardware first, software fallback).
//!
//! Public surface used by the playback engine, the desktop app and the benchmark:
//! - [`probe`] → [`MediaInfo`]
//! - [`open_video`] → a [`VideoDecoder`] (hardware with automatic software fallback, exact seek)
//! - [`DecodeWorker`] → the decoder on its own thread feeding a bounded frame queue
//! - [`AudioDecoder`] → decoded, resampled audio in the output device's format
//! - [`tonemap`] → HDR (PQ / HLG) to SDR conversion used for RGBA output

mod audio;
mod convert;
mod error;
mod fallback;
mod frame;
mod probe;
pub mod tonemap;
mod util;
mod video;
mod worker;

pub use audio::{AudioChunk, AudioDecoder, AudioOutputSpec, SampleFormat, Samples};
pub use convert::{ColorInfo, ColorMatrix, Transfer};
pub use error::{MediaError, Result};
pub use fallback::{FallbackDecoder, open_video};
pub use frame::{DecodePath, OutputFormat, PixelFormat, Plane, VideoFrame};
pub use probe::{AudioStreamInfo, MediaInfo, VideoStreamInfo, probe};
pub use util::{Micros, PixelFormatInfo, Rational, ffmpeg_version, init};
pub use video::{DecoderOptions, FfmpegVideoDecoder, HwAccel, VideoDecoder};
pub use worker::{DEFAULT_QUEUE_FRAMES, DecodeWorker, WorkerEvent};
