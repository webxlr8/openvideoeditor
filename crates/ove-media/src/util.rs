//! Small FFI helpers shared by probe and decode. All `unsafe` in this crate that is not decoder plumbing lives here.

use std::ffi::CStr;
use std::path::Path;
use std::sync::Once;

use ffmpeg_the_third::ffi;
use serde::Serialize;

use crate::error::{MediaError, Result};

/// Microseconds. All public times in this crate use this unit, relative to the stream's start time.
pub type Micros = i64;

pub(crate) const NOPTS: i64 = ffi::AV_NOPTS_VALUE;

/// Initialise FFmpeg once per process and keep its logging quiet (errors only).
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = ffmpeg_the_third::init();
        // SAFETY: setting the global log level is thread-safe in libavutil.
        unsafe { ffi::av_log_set_level(ffi::AV_LOG_FATAL) };
    });
}

/// Version string of the linked FFmpeg libraries (e.g. `9.0.2`).
pub fn ffmpeg_version() -> String {
    // SAFETY: `av_version_info` returns a non-null pointer to a NUL-terminated string compiled into libavutil,
    // valid for the life of the process.
    unsafe { CStr::from_ptr(ffi::av_version_info()).to_string_lossy().into_owned() }
}

/// A rational number as reported by the container or codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rational {
    pub num: i32,
    pub den: i32,
}

impl Rational {
    pub fn as_f64(self) -> Option<f64> {
        (self.num != 0 && self.den != 0).then(|| self.num as f64 / self.den as f64)
    }
}

impl From<ffmpeg_the_third::Rational> for Rational {
    fn from(r: ffmpeg_the_third::Rational) -> Self {
        Rational { num: r.numerator(), den: r.denominator() }
    }
}

/// `ts` in `tb` units → microseconds (rounded to nearest, like `av_rescale_q`).
pub fn ts_to_us(ts: i64, tb: Rational) -> Micros {
    // SAFETY: `av_rescale_q` only does integer arithmetic on its by-value arguments (no pointers, no global state;
    // a zero denominator yields a sentinel value, not undefined behaviour).
    unsafe {
        ffi::av_rescale_q(ts, ffi::AVRational { num: tb.num, den: tb.den }, ffi::AVRational { num: 1, den: 1_000_000 })
    }
}

/// Microseconds → `tb` units (rounded to nearest).
pub fn us_to_ts(us: Micros, tb: Rational) -> i64 {
    // SAFETY: as in `ts_to_us`: integer arithmetic on by-value arguments only.
    unsafe {
        ffi::av_rescale_q(us, ffi::AVRational { num: 1, den: 1_000_000 }, ffi::AVRational { num: tb.num, den: tb.den })
    }
}

/// Description of a pixel format from libavutil's descriptor table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PixelFormatInfo {
    pub name: String,
    pub bit_depth: u8,
    /// "4:2:0", "4:2:2", "4:4:4", "4:1:1", "4:4:0" or "rgb"/"gray".
    pub chroma: String,
    pub has_alpha: bool,
    pub planar: bool,
}

pub(crate) fn pix_fmt_info(fmt: ffi::AVPixelFormat) -> Option<PixelFormatInfo> {
    // SAFETY: av_pix_fmt_desc_get returns a pointer into a static table or null.
    unsafe {
        let d = ffi::av_pix_fmt_desc_get(fmt);
        if d.is_null() {
            return None;
        }
        let d = &*d;
        let name = CStr::from_ptr(d.name).to_string_lossy().into_owned();
        let flags = d.flags;
        let rgb = flags & ffi::AV_PIX_FMT_FLAG_RGB as u64 != 0;
        let chroma = if rgb {
            "rgb".to_string()
        } else if d.nb_components <= 2 {
            "gray".to_string()
        } else {
            match (d.log2_chroma_w, d.log2_chroma_h) {
                (1, 1) => "4:2:0",
                (1, 0) => "4:2:2",
                (0, 0) => "4:4:4",
                (2, 0) => "4:1:1",
                (0, 1) => "4:4:0",
                _ => "other",
            }
            .to_string()
        };
        Some(PixelFormatInfo {
            name,
            bit_depth: d.comp[0].depth as u8,
            chroma,
            has_alpha: flags & ffi::AV_PIX_FMT_FLAG_ALPHA as u64 != 0,
            planar: flags & ffi::AV_PIX_FMT_FLAG_PLANAR as u64 != 0,
        })
    }
}

pub(crate) fn codec_profile_name(id: ffi::AVCodecID, profile: i32) -> Option<String> {
    // SAFETY: avcodec_profile_name returns a static string or null.
    unsafe {
        let p = ffi::avcodec_profile_name(id, profile);
        (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

pub(crate) fn codec_name(id: ffi::AVCodecID) -> String {
    // SAFETY: avcodec_get_name never returns null.
    unsafe { CStr::from_ptr(ffi::avcodec_get_name(id)).to_string_lossy().into_owned() }
}

pub(crate) fn sample_fmt_name(fmt: ffi::AVSampleFormat) -> String {
    // SAFETY: returns static string or null.
    unsafe {
        let p = ffi::av_get_sample_fmt_name(fmt);
        if p.is_null() { "none".into() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
    }
}

pub(crate) fn open_input(path: &Path) -> Result<ffmpeg_the_third::format::context::Input> {
    init();
    if !path.exists() {
        return Err(MediaError::NotFound { path: path.to_owned() });
    }
    ffmpeg_the_third::format::input(path).map_err(|e| MediaError::from_open(path, e))
}

/// Whether the file at `path` is still reachable (deleted, renamed or volume unmounted → offline).
pub(crate) fn check_online(path: &Path) -> Result<()> {
    match std::fs::metadata(path) {
        Ok(_) => Ok(()),
        Err(e) => Err(MediaError::Offline { path: path.to_owned(), reason: e.to_string() }),
    }
}
