//! Automatic software fallback, at open time and mid-stream.

use std::path::Path;

use crate::error::{MediaError, Result};
use crate::frame::{DecodePath, VideoFrame};
use crate::util::Micros;
use crate::video::{DecoderOptions, FfmpegVideoDecoder, HwAccel, VideoDecoder};

type Factory = Box<dyn FnMut() -> Result<Box<dyn VideoDecoder>> + Send>;

/// Wraps a (usually hardware) primary decoder. When it fails with a hardware error — at any point — a software
/// decoder is created and playback resumes at the frame after the last one delivered, so the caller sees one
/// uninterrupted sequence of frames (at most one hitch while the software decoder catches up).
pub struct FallbackDecoder {
    current: Box<dyn VideoDecoder>,
    software: Option<Factory>,
    last_pts: Option<Micros>,
    fallbacks: u32,
    last_reason: Option<String>,
}

impl FallbackDecoder {
    pub fn new(primary: Box<dyn VideoDecoder>, software: Factory) -> Self {
        FallbackDecoder { current: primary, software: Some(software), last_pts: None, fallbacks: 0, last_reason: None }
    }

    /// How many times the decoder switched to software after a hardware failure.
    pub fn fallback_count(&self) -> u32 {
        self.fallbacks
    }

    /// The hardware error that caused the last switch.
    pub fn fallback_reason(&self) -> Option<&str> {
        self.last_reason.as_deref()
    }

    fn switch_to_software(&mut self, reason: String) -> Result<()> {
        let Some(mut make) = self.software.take() else {
            return Err(MediaError::Hardware(reason));
        };
        let mut sw = make()?;
        if let Some(p) = self.last_pts {
            sw.seek(p)?;
            // `seek(p)` re-delivers the frame at `p`; drop it and anything not after it.
            loop {
                match sw.next_frame()? {
                    Some(f) if f.pts_us <= p => continue,
                    Some(f) => {
                        self.current = Box::new(Prepend { first: Some(f), inner: sw });
                        break;
                    }
                    None => {
                        self.current = sw;
                        break;
                    }
                }
            }
        } else {
            self.current = sw;
        }
        self.fallbacks += 1;
        self.last_reason = Some(reason);
        Ok(())
    }
}

impl VideoDecoder for FallbackDecoder {
    fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        match self.current.next_frame() {
            Ok(f) => {
                if let Some(f) = &f {
                    self.last_pts = Some(f.pts_us);
                }
                Ok(f)
            }
            Err(MediaError::Hardware(reason)) => {
                self.switch_to_software(reason)?;
                self.next_frame()
            }
            Err(e) => Err(e),
        }
    }

    fn seek(&mut self, t_us: Micros) -> Result<()> {
        self.last_pts = None;
        match self.current.seek(t_us) {
            Err(MediaError::Hardware(reason)) => {
                self.switch_to_software(reason)?;
                self.current.seek(t_us)
            }
            other => other,
        }
    }

    fn decode_path(&self) -> DecodePath {
        self.current.decode_path()
    }
}

/// A decoder with one frame pushed back in front.
struct Prepend {
    first: Option<VideoFrame>,
    inner: Box<dyn VideoDecoder>,
}

impl VideoDecoder for Prepend {
    fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        match self.first.take() {
            Some(f) => Ok(Some(f)),
            None => self.inner.next_frame(),
        }
    }
    fn seek(&mut self, t_us: Micros) -> Result<()> {
        self.first = None;
        self.inner.seek(t_us)
    }
    fn decode_path(&self) -> DecodePath {
        self.inner.decode_path()
    }
}

/// Open the best video stream of `path`: hardware decoder first (per `opts.hwaccel`), software if hardware is not
/// available, does not support the stream, or fails later.
pub fn open_video(path: &Path, opts: DecoderOptions) -> Result<FallbackDecoder> {
    let sw_opts = DecoderOptions { hwaccel: HwAccel::Off, inject_hw_failure_after: None, ..opts.clone() };
    let sw_path = path.to_owned();
    let software: Factory =
        Box::new(move || Ok(Box::new(FfmpegVideoDecoder::open(&sw_path, sw_opts.clone())?) as Box<dyn VideoDecoder>));
    let primary: Box<dyn VideoDecoder> = match FfmpegVideoDecoder::open(path, opts.clone()) {
        Ok(d) => Box::new(d),
        // Opening with the hardware device failed for a reason other than the file itself: retry in software.
        Err(MediaError::Unsupported { .. } | MediaError::Decode(_) | MediaError::Hardware(_))
            if opts.hwaccel != HwAccel::Off =>
        {
            Box::new(FfmpegVideoDecoder::open(path, DecoderOptions { hwaccel: HwAccel::Off, ..opts })?)
        }
        Err(e) => return Err(e),
    };
    Ok(FallbackDecoder::new(primary, software))
}
