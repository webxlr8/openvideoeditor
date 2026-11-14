//! Audio decoding and resampling to the output device's format.

use std::path::{Path, PathBuf};

use ffmpeg_the_third::ffi;
use ffmpeg_the_third::frame::Audio as AvAudio;
use ffmpeg_the_third::media::Type as MediaType;
use ffmpeg_the_third::{Error as AvError, Packet};

use crate::error::{MediaError, Result};
use crate::util::{Micros, NOPTS, Rational, check_online, open_input, ts_to_us, us_to_ts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    F32,
    S16,
}

/// The device format audio is converted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioOutputSpec {
    pub sample_rate: u32,
    pub channels: u16,
    pub format: SampleFormat,
}

impl Default for AudioOutputSpec {
    fn default() -> Self {
        AudioOutputSpec { sample_rate: 48_000, channels: 2, format: SampleFormat::F32 }
    }
}

/// Interleaved samples in the output format.
#[derive(Debug, Clone, PartialEq)]
pub enum Samples {
    F32(Vec<f32>),
    S16(Vec<i16>),
}

impl Samples {
    pub fn len(&self) -> usize {
        match self {
            Samples::F32(v) => v.len(),
            Samples::S16(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn drain_front(&mut self, n: usize) {
        match self {
            Samples::F32(v) => drop(v.drain(..n.min(v.len()))),
            Samples::S16(v) => drop(v.drain(..n.min(v.len()))),
        }
    }
}

/// A run of output samples starting at `pts_us`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    pub pts_us: Micros,
    /// Sample frames (one sample per channel each).
    pub frames: usize,
    pub samples: Samples,
}

/// Decoding starts this much before a seek target.
const AUDIO_PREROLL_US: Micros = 200_000;

/// Decodes the best audio stream and converts it to an [`AudioOutputSpec`].
pub struct AudioDecoder {
    path: PathBuf,
    input: ffmpeg_the_third::format::context::Input,
    stream_index: usize,
    tb: Rational,
    start_ts: i64,
    dec: ffmpeg_the_third::decoder::Audio,
    spec: AudioOutputSpec,
    swr: Option<Swr>,
    /// Time of the first output sample since open/seek, and output samples produced since then.
    anchor_us: Option<Micros>,
    out_samples: i64,
    /// After a seek: drop output before this time.
    trim_before: Option<Micros>,
    demux_eof: bool,
    eof_sent: bool,
    flushed: bool,
}

impl AudioDecoder {
    pub fn open(path: &Path, spec: AudioOutputSpec) -> Result<Self> {
        let input = open_input(path)?;
        let st = input.streams().best(MediaType::Audio).ok_or(MediaError::NoStream("audio"))?;
        let stream_index = st.index();
        let tb: Rational = st.time_base().into();
        // Media time 0 is shared with the video decoder: the video stream's start when there is one. Audio-only
        // files start at their first audible sample (codec delay such as Opus pre-skip shows as a negative start).
        let start = match input.streams().best(MediaType::Video) {
            Some(v) if v.start_time() != NOPTS => {
                // SAFETY: `av_rescale_q` only does integer arithmetic on its by-value arguments; no pointers.
                unsafe { ffi::av_rescale_q(v.start_time(), v.time_base().into(), st.time_base().into()) }
            }
            _ if st.start_time() == NOPTS => 0,
            _ => st.start_time().max(0),
        };
        let mut ctx = ffmpeg_the_third::codec::context::Context::from_parameters(st.parameters())
            .map_err(|e| MediaError::from_open(path, e))?;
        // SAFETY: `ctx` owns a valid, not yet opened AVCodecContext (from_parameters succeeded), so the pointer is
        // non-null and exclusively ours; `pkt_timebase` is a plain-data field the decoder reads when it is opened.
        unsafe { (*ctx.as_mut_ptr()).pkt_timebase = ffi::AVRational { num: tb.num, den: tb.den } };
        let dec = ctx.decoder().audio().map_err(|e| MediaError::from_open(path, e))?;
        Ok(AudioDecoder {
            path: path.to_owned(),
            input,
            stream_index,
            tb,
            start_ts: start,
            dec,
            spec,
            swr: None,
            anchor_us: None,
            out_samples: 0,
            // Samples before media time 0 (pre-roll) are never played.
            trim_before: Some(0),
            demux_eof: false,
            eof_sent: false,
            flushed: false,
        })
    }

    pub fn spec(&self) -> AudioOutputSpec {
        self.spec
    }

    fn feed(&mut self) -> Result<bool> {
        if self.demux_eof {
            if !self.eof_sent {
                self.eof_sent = true;
                let _ = self.dec.send_eof();
                return Ok(true);
            }
            return Ok(false);
        }
        let mut packet = Packet::empty();
        loop {
            match packet.read(&mut self.input) {
                Ok(()) if packet.stream() == self.stream_index => match self.dec.send_packet(&packet) {
                    Ok(()) | Err(AvError::InvalidData) => return Ok(true),
                    Err(e) => return Err(MediaError::Decode(e.to_string())),
                },
                Ok(()) => continue,
                Err(_) => {
                    check_online(&self.path)?;
                    self.demux_eof = true;
                    return self.feed();
                }
            }
        }
    }

    fn bytes_per_sample(&self) -> usize {
        match self.spec.format {
            SampleFormat::F32 => 4,
            SampleFormat::S16 => 2,
        }
    }

    fn to_samples(&self, bytes: Vec<u8>) -> Samples {
        match self.spec.format {
            SampleFormat::F32 => {
                Samples::F32(bytes.chunks_exact(4).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])).collect())
            }
            SampleFormat::S16 => {
                Samples::S16(bytes.chunks_exact(2).map(|b| i16::from_ne_bytes([b[0], b[1]])).collect())
            }
        }
    }

    fn sample_to_us(&self, n: i64) -> Micros {
        n * 1_000_000 / self.spec.sample_rate as i64
    }

    /// Next run of converted samples, `None` at end of stream.
    pub fn next_chunk(&mut self) -> Result<Option<AudioChunk>> {
        loop {
            if self.flushed {
                return Ok(None);
            }
            let mut f = AvAudio::empty();
            let got = match self.dec.receive_frame(&mut f) {
                Ok(()) => true,
                Err(AvError::Other { errno }) if errno == libc::EAGAIN => {
                    if self.feed()? {
                        continue;
                    }
                    false
                }
                Err(AvError::Eof) => false,
                Err(AvError::InvalidData) => continue,
                Err(e) => return Err(MediaError::Decode(e.to_string())),
            };
            let bytes = if got {
                if self.anchor_us.is_none() {
                    // SAFETY: `f` is a frame the decoder just filled (`got`), so `as_ptr` is a valid AVFrame for the
                    // duration of this read of a plain-data field.
                    let best = unsafe { (*f.as_ptr()).best_effort_timestamp };
                    let ts = if best != NOPTS { best } else { self.start_ts };
                    self.anchor_us = Some(ts_to_us(ts - self.start_ts, self.tb));
                    self.out_samples = 0;
                }
                // SAFETY: `f` holds a decoded audio frame and outlives the call (see `Swr::key`).
                let key = unsafe { Swr::key(f.as_ptr()) };
                // Keep the resampler while the input format is unchanged; otherwise make one for the new format.
                let swr = match self.swr.take_if(|s| s.key == key) {
                    Some(swr) => swr,
                    // SAFETY: `f` holds a decoded audio frame and outlives the call (see `Swr::new`).
                    None => unsafe { Swr::new(f.as_ptr(), self.spec)? },
                };
                let frame_bytes = self.bytes_per_sample() * self.spec.channels as usize;
                let swr = self.swr.insert(swr);
                // SAFETY: `f` holds a decoded frame in the format `swr` was created for (its key matched above).
                unsafe { swr.convert(f.as_ptr(), frame_bytes)? }
            } else {
                // End of stream: drain the resampler's delay once.
                self.flushed = true;
                let frame_bytes = self.bytes_per_sample() * self.spec.channels as usize;
                match self.swr.as_mut() {
                    // SAFETY: a null frame is the documented way to flush `Swr::convert`.
                    Some(swr) => unsafe { swr.convert(std::ptr::null(), frame_bytes)? },
                    None => return Ok(None),
                }
            };
            let frame_bytes = self.bytes_per_sample() * self.spec.channels as usize;
            let mut frames = bytes.len() / frame_bytes;
            if frames == 0 {
                continue;
            }
            let anchor = self.anchor_us.unwrap_or(0);
            let first_sample = self.out_samples;
            self.out_samples += frames as i64;
            let mut pts = anchor + self.sample_to_us(first_sample);
            let mut samples = self.to_samples(bytes);
            if let Some(t) = self.trim_before {
                let end = anchor + self.sample_to_us(self.out_samples);
                if end <= t {
                    continue;
                }
                if t > pts {
                    let drop_frames = (((t - anchor) * self.spec.sample_rate as i64 + 999_999) / 1_000_000
                        - first_sample)
                        .clamp(0, frames as i64) as usize;
                    samples.drain_front(drop_frames * self.spec.channels as usize);
                    frames -= drop_frames;
                    pts = anchor + self.sample_to_us(first_sample + drop_frames as i64);
                }
                self.trim_before = None;
            }
            return Ok(Some(AudioChunk { pts_us: pts, frames, samples }));
        }
    }

    /// Reposition so the next chunk starts at `t_us` (sample-accurate after the decoder's pre-roll is dropped).
    pub fn seek(&mut self, t_us: Micros) -> Result<()> {
        // Start decoding a little before the target: codecs with pre-roll (AAC, Opus) need it, and some demuxers
        // land after the requested time. Output before `t_us` is trimmed sample-accurately.
        let ts = self.start_ts + us_to_ts((t_us - AUDIO_PREROLL_US).max(0), self.tb);
        // SAFETY: `self.input` owns an open AVFormatContext for as long as `self` lives, and `stream_index` is
        // one of its streams.
        let r =
            unsafe { ffi::avformat_seek_file(self.input.as_mut_ptr(), self.stream_index as i32, i64::MIN, ts, ts, 0) };
        if r < 0 {
            check_online(&self.path)?;
            return Err(MediaError::Decode(format!("audio seek failed: {}", AvError::from(r))));
        }
        self.dec.flush();
        self.swr = None;
        self.anchor_us = None;
        self.out_samples = 0;
        self.trim_before = Some(t_us);
        self.demux_eof = false;
        self.eof_sent = false;
        self.flushed = false;
        Ok(())
    }
}

/// swresample context converting decoder frames to the output spec (packed).
struct Swr {
    ctx: *mut ffi::SwrContext,
    key: (i32, i32, i32),
}

// SAFETY: a SwrContext has no thread affinity (no thread-locals, no callbacks). `Swr` owns its context
// exclusively (it is never shared or cloned) and `&mut self` is required to use it, so moving it to another thread
// cannot cause concurrent access.
unsafe impl Send for Swr {}

impl Swr {
    /// (format, channels, rate) of a frame; a change re-creates the resampler.
    ///
    /// # Safety
    ///
    /// `f` must point to a valid, initialised AVFrame for the duration of the call.
    unsafe fn key(f: *const ffi::AVFrame) -> (i32, i32, i32) {
        // SAFETY: the caller guarantees `f` is valid; these are plain-data field reads.
        unsafe { ((*f).format, (*f).ch_layout.nb_channels, (*f).sample_rate) }
    }

    /// A resampler from `f`'s format, layout and rate to `spec`.
    ///
    /// # Safety
    ///
    /// `f` must point to a valid decoded audio frame (initialised `format`, `ch_layout` and `sample_rate`) for the
    /// duration of the call.
    unsafe fn new(f: *const ffi::AVFrame, spec: AudioOutputSpec) -> Result<Swr> {
        // SAFETY: the caller guarantees `f` is valid. Both layouts are zero-initialised (the state
        // `av_channel_layout_default`/`_copy` expect), set before `swr_alloc_set_opts2` reads them and uninitialised
        // once it has copied them. `ctx` is null or a context `swr_alloc_set_opts2` allocated; on failure it is
        // freed (`swr_free` accepts null) and never escapes.
        unsafe {
            let mut out_layout: ffi::AVChannelLayout = std::mem::zeroed();
            ffi::av_channel_layout_default(&mut out_layout, spec.channels as i32);
            let mut in_layout: ffi::AVChannelLayout = std::mem::zeroed();
            if (*f).ch_layout.order == ffi::AVChannelOrder::UNSPEC {
                ffi::av_channel_layout_default(&mut in_layout, (*f).ch_layout.nb_channels);
            } else {
                ffi::av_channel_layout_copy(&mut in_layout, &(*f).ch_layout);
            }
            let out_fmt = match spec.format {
                SampleFormat::F32 => ffi::AVSampleFormat::FLT,
                SampleFormat::S16 => ffi::AVSampleFormat::S16,
            };
            let mut ctx = std::ptr::null_mut();
            let r = ffi::swr_alloc_set_opts2(
                &mut ctx,
                &out_layout,
                out_fmt,
                spec.sample_rate as i32,
                &in_layout,
                ffi::AVSampleFormat((*f).format),
                (*f).sample_rate,
                0,
                std::ptr::null_mut(),
            );
            ffi::av_channel_layout_uninit(&mut out_layout);
            ffi::av_channel_layout_uninit(&mut in_layout);
            if r < 0 || ffi::swr_init(ctx) < 0 {
                ffi::swr_free(&mut ctx);
                return Err(MediaError::Decode("audio resampler could not be created".into()));
            }
            Ok(Swr { ctx, key: Swr::key(f) })
        }
    }

    /// Convert one frame (or flush with a null frame) into packed bytes.
    ///
    /// # Safety
    ///
    /// `f` must be null or point to a valid decoded audio frame in the format this resampler was created for
    /// (same [`Swr::key`]), valid for the duration of the call. `frame_bytes` must be the size of one packed output
    /// sample frame (bytes per sample × output channels).
    unsafe fn convert(&mut self, f: *const ffi::AVFrame, frame_bytes: usize) -> Result<Vec<u8>> {
        // SAFETY: `self.ctx` is an initialised context we own. The output buffer holds `cap` sample frames of
        // `frame_bytes` each, and `swr_convert` writes at most `cap`. The input is null (flush) or, per the
        // caller's contract, a frame whose `extended_data` holds `nb_samples` samples per plane in the format the
        // context was configured with.
        unsafe {
            let in_count = if f.is_null() { 0 } else { (*f).nb_samples };
            let cap = ffi::swr_get_out_samples(self.ctx, in_count);
            if cap <= 0 {
                return Ok(Vec::new());
            }
            let mut buf = vec![0u8; cap as usize * frame_bytes];
            let mut out_ptr = buf.as_mut_ptr();
            let in_ptr = if f.is_null() { std::ptr::null_mut() } else { (*f).extended_data as *mut *const u8 };
            // A raw pointer works with both the FFmpeg 6 (`*mut *mut u8`) and FFmpeg 9 (`*const *mut u8`) signatures.
            let n = ffi::swr_convert(self.ctx, std::ptr::addr_of_mut!(out_ptr), cap, in_ptr, in_count);
            if n < 0 {
                return Err(MediaError::Decode(format!("resample: {}", AvError::from(n))));
            }
            buf.truncate(n as usize * frame_bytes);
            Ok(buf)
        }
    }
}

impl Drop for Swr {
    fn drop(&mut self) {
        // SAFETY: `self.ctx` was allocated by `swr_alloc_set_opts2`, is owned only by `self` and is freed exactly
        // once here (`swr_free` also nulls the pointer).
        unsafe { ffi::swr_free(&mut self.ctx) };
    }
}
