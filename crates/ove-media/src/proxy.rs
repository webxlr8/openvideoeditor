//! Proxies: smaller copies of heavy video files, used for playback, skim sheets and analysis when the project's
//! "Use proxies" setting is on. Export and agent inspection always read the original.
//!
//! This module decides which files need one and where it lives. A file needs a proxy when it is 4K or larger, when it
//! is 10-bit and long-GOP (each frame depends on others, which makes deep decodes slow), or when it fails a real-time
//! decode check: decoding a short run of frames the way timeline playback does takes longer than playing them.
//! The proxy itself is written by the export pipeline (`ove_render::proxy`), so it uses the same encoders as export.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ffmpeg_the_third::ffi;
use serde::{Deserialize, Serialize};

use crate::error::{MediaError, Result};
use crate::frame::OutputFormat;
use crate::probe::MediaInfo;
use crate::video::{DecoderOptions, FfmpegVideoDecoder, HwAccel, VideoDecoder};

/// Proxy frame height (and the largest width, 1920, for wide sources).
pub const HEIGHT: u32 = 1080;
pub const MAX_WIDTH: u32 = 1920;
/// A keyframe every this many frames, so a seek decodes at most 11 frames.
pub const GOP: u32 = 12;
/// Version of the proxy format and of the eligibility rules; a check of another version is made again.
pub const VERSION: u32 = 1;
/// Sources at least this wide or high count as 4K (3840 × 2160, 4096 × 2160, and portrait 2160 × 3840).
pub const UHD_LONG_SIDE: u32 = 3840;
pub const UHD_SHORT_SIDE: u32 = 2160;
/// Decode must run at least this much faster than real time, leaving room for compositing.
pub const REALTIME_MARGIN: f64 = 1.25;
/// Frame height timeline playback decodes at (the preview height). The real-time check decodes at this size too.
pub const PREVIEW_HEIGHT: u32 = 540;

/// The file name of a media file's proxy: `<contentId>_1080.mp4`.
pub fn file_name(content_id: &str) -> String {
    format!("{content_id}_1080.mp4")
}

/// Where a media file's proxy lives in a project bundle: `proxies/<contentId>_1080.mp4`.
pub fn path(bundle: &Path, content_id: &str) -> PathBuf {
    bundle.join("proxies").join(file_name(content_id))
}

/// The finished proxy of a media file, when there is one.
pub fn existing(bundle: &Path, content_id: &str) -> Option<PathBuf> {
    let p = path(bundle, content_id);
    p.is_file().then_some(p)
}

/// Why a file needs a proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// 4K or larger.
    Resolution,
    /// 10-bit (or deeper) and long-GOP.
    DeepLongGop,
    /// Decoding it as playback does is slower than real time.
    SlowDecode,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Reason::Resolution => "4K or larger",
            Reason::DeepLongGop => "10-bit long-GOP",
            Reason::SlowDecode => "decodes slower than real time",
        }
    }
}

/// The outcome of checking one file (stored beside the project's other derived data, so it is not repeated).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub version: u32,
    /// The reason the file needs a proxy; `None`: it plays well without one.
    pub reason: Option<Reason>,
    /// Decode speed relative to real time, when the real-time check ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decode_speed: Option<f64>,
}

impl Check {
    pub fn needed(&self) -> bool {
        self.reason.is_some()
    }
}

/// True when the codec has frames that depend on other frames (FFmpeg's codec description says it is not
/// intra-only). Unknown codecs count as long-GOP.
pub fn long_gop_codec(codec: &str) -> bool {
    let Ok(name) = std::ffi::CString::new(codec) else { return true };
    // SAFETY: `name` is a NUL-terminated string that outlives the call; the result is null or a pointer into
    // FFmpeg's static descriptor table, read once here.
    unsafe {
        let d = ffi::avcodec_descriptor_get_by_name(name.as_ptr());
        if d.is_null() {
            return true;
        }
        // The constant's integer type differs between platforms' bindings.
        i64::from((*d).props) & i64::from(ffi::AV_CODEC_PROP_INTRA_ONLY) == 0
    }
}

/// The reason a file needs a proxy that its format alone shows (no decoding): size, or bit depth with long GOP.
pub fn needs_by_format(info: &MediaInfo) -> Option<Reason> {
    let v = info.video.as_ref()?;
    let (long, short) = (v.width.max(v.height), v.width.min(v.height));
    if long >= UHD_LONG_SIDE && short >= UHD_SHORT_SIDE {
        return Some(Reason::Resolution);
    }
    let depth = v.pixel_format.as_ref().map_or(8, |p| p.bit_depth);
    if depth >= 10 && long_gop_codec(&v.codec) {
        return Some(Reason::DeepLongGop);
    }
    None
}

/// Decode speed relative to real time: decodes up to `frames` frames from the start the way timeline playback does
/// (software decode, RGBA at the preview height) and compares the time taken with their play time. The decoder's
/// set-up and first frame are not counted.
pub fn decode_speed(path: &Path, info: &MediaInfo, frames: u32) -> Result<f64> {
    let v = info.video.as_ref().ok_or_else(|| MediaError::Decode("the file has no video".into()))?;
    let fps = if v.frame_rate.num > 0 && v.frame_rate.den > 0 {
        v.frame_rate.num as f64 / v.frame_rate.den as f64
    } else {
        30.0
    };
    let opts = DecoderOptions {
        hwaccel: HwAccel::Off,
        output: OutputFormat::Rgba { max_width: 0, max_height: PREVIEW_HEIGHT },
        ..Default::default()
    };
    let mut dec = FfmpegVideoDecoder::open(path, opts)?;
    if dec.next_frame()?.is_none() {
        return Err(MediaError::Decode("the video has no frames".into()));
    }
    let started = Instant::now();
    let mut n = 0u32;
    while n < frames.max(1) {
        match dec.next_frame()? {
            Some(_) => n += 1,
            None => break,
        }
    }
    let took = started.elapsed().as_secs_f64();
    if n == 0 {
        // A one-frame file plays as a still.
        return Ok(f64::INFINITY);
    }
    Ok((n as f64 / fps) / took.max(1e-6))
}

/// Checks whether a file needs a proxy: by format first, then with the real-time decode check (two seconds of frames,
/// at most 60).
pub fn check(path: &Path, info: &MediaInfo) -> Result<Check> {
    if let Some(reason) = needs_by_format(info) {
        return Ok(Check { version: VERSION, reason: Some(reason), decode_speed: None });
    }
    let fps = info
        .video
        .as_ref()
        .map_or(30.0, |v| if v.frame_rate.den > 0 { v.frame_rate.num as f64 / v.frame_rate.den as f64 } else { 30.0 });
    let frames = ((fps * 2.0).round() as u32).clamp(12, 60);
    let speed = decode_speed(path, info, frames)?;
    let reason = (speed < REALTIME_MARGIN).then_some(Reason::SlowDecode);
    Ok(Check { version: VERSION, reason, decode_speed: Some((speed * 100.0).round() / 100.0) })
}

/// The proxy frame size for a source: fits 1920 × 1080 (portrait: 1080 × 1920), keeps the aspect ratio, never
/// upscales, and is even in both directions (4:2:0 encoders need that).
pub fn frame_size(width: u32, height: u32) -> (u32, u32) {
    let (w, h) = (width.max(2) as f64, height.max(2) as f64);
    let (box_w, box_h) =
        if height > width { (HEIGHT as f64, MAX_WIDTH as f64) } else { (MAX_WIDTH as f64, HEIGHT as f64) };
    let k = (box_w / w).min(box_h / h).min(1.0);
    let even = |x: f64| (((x * k).round() as u32) & !1).max(2);
    (even(w), even(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_the_content_id() {
        assert_eq!(file_name("abc"), "abc_1080.mp4");
        assert_eq!(path(Path::new("P.ove"), "abc"), Path::new("P.ove").join("proxies").join("abc_1080.mp4"));
    }

    #[test]
    fn frame_sizes_fit_1080p_and_stay_even() {
        assert_eq!(frame_size(3840, 2160), (1920, 1080));
        assert_eq!(frame_size(4096, 2160), (1920, 1012));
        assert_eq!(frame_size(2160, 3840), (1080, 1920));
        assert_eq!(frame_size(1920, 1080), (1920, 1080));
        assert_eq!(frame_size(1280, 720), (1280, 720), "never upscaled");
        assert_eq!(frame_size(7680, 4320), (1920, 1080));
        let (w, h) = frame_size(5000, 3333);
        assert!(w % 2 == 0 && h % 2 == 0 && w <= 1920 && h <= 1080);
    }

    #[test]
    fn intra_only_codecs_are_not_long_gop() {
        crate::init();
        for c in ["h264", "hevc", "av1", "vp9", "mpeg4"] {
            assert!(long_gop_codec(c), "{c}");
        }
        for c in ["prores", "dnxhd", "mjpeg"] {
            assert!(!long_gop_codec(c), "{c}");
        }
        assert!(long_gop_codec("no-such-codec"));
    }
}
