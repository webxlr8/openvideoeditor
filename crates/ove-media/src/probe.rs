//! Media probing: container, streams, duration, frame rate (with VFR detection), codec/profile, audio layout.

use std::path::{Path, PathBuf};

use ffmpeg_the_third::ffi;
use ffmpeg_the_third::media::Type as MediaType;
use serde::Serialize;

use crate::error::Result;
use crate::util::{
    Micros, NOPTS, PixelFormatInfo, Rational, codec_name, codec_profile_name, open_input, pix_fmt_info,
    sample_fmt_name, ts_to_us,
};

/// Everything the viewer and properties panel need about a file, without decoding it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub path: PathBuf,
    /// Demuxer short name(s), e.g. `mov,mp4,m4a,3gp,3g2,mj2`.
    pub container: String,
    pub duration_us: Option<Micros>,
    pub bit_rate: Option<i64>,
    pub video: Option<VideoStreamInfo>,
    pub audio: Option<AudioStreamInfo>,
    pub stream_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoStreamInfo {
    pub stream_index: usize,
    pub codec: String,
    pub profile: Option<String>,
    pub width: u32,
    pub height: u32,
    pub pixel_format: Option<PixelFormatInfo>,
    /// Average frame rate (`avg_frame_rate`).
    pub frame_rate: Rational,
    /// Lowest common frame rate that represents all timestamps (`r_frame_rate`).
    pub base_frame_rate: Rational,
    /// True when frame intervals differ (phone recordings, screen captures).
    pub variable_frame_rate: bool,
    pub frame_count: Option<u64>,
    pub time_base: Rational,
    pub duration_us: Option<Micros>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStreamInfo {
    pub stream_index: usize,
    pub codec: String,
    pub profile: Option<String>,
    pub sample_rate: u32,
    pub channels: u32,
    /// Layout description, e.g. `stereo`, `5.1(side)`, `mono`.
    pub layout: String,
    pub sample_format: String,
    pub time_base: Rational,
    pub duration_us: Option<Micros>,
}

/// How many timestamps are inspected for VFR detection when the container has no index.
const VFR_SCAN_PACKETS: usize = 240;

/// Probe a file. Reads the container header and, for VFR detection, the container's sample index (or up to
/// [`VFR_SCAN_PACKETS`] packet headers when there is none); no frames are decoded.
pub fn probe(path: &Path) -> Result<MediaInfo> {
    let mut input = open_input(path)?;
    let container = input.format().name().to_string();
    let duration = input.duration();
    let duration_us = (duration != NOPTS && duration > 0).then_some(duration); // AV_TIME_BASE is 1 µs
    let bit_rate = (input.bit_rate() > 0).then_some(input.bit_rate());
    let stream_count = input.nb_streams() as usize;

    let mut video = None;
    let mut audio = None;
    let best_v = input.streams().best(MediaType::Video).map(|s| s.index());
    let best_a = input.streams().best(MediaType::Audio).map(|s| s.index());

    if let Some(idx) = best_v
        && let Some(st) = input.stream(idx)
    {
        let par = st.parameters();
        // SAFETY: parameters pointer is valid for the stream's lifetime; fields are plain data.
        let (codec_id, profile, fmt, w, h) = unsafe {
            let p = &*par.as_ptr();
            (p.codec_id, p.profile, p.format, p.width as u32, p.height as u32)
        };
        let tb: Rational = st.time_base().into();
        let frames = st.frames();
        let dur = st.duration();
        let index_ts = index_timestamps(&st);
        video = Some(VideoStreamInfo {
            stream_index: idx,
            codec: codec_name(codec_id),
            profile: codec_profile_name(codec_id, profile),
            width: w,
            height: h,
            pixel_format: pix_fmt_info(ffi::AVPixelFormat(fmt)),
            frame_rate: st.avg_frame_rate().into(),
            base_frame_rate: st.rate().into(),
            variable_frame_rate: false,
            frame_count: (frames > 0).then_some(frames as u64),
            time_base: tb,
            duration_us: (dur != NOPTS && dur > 0).then(|| ts_to_us(dur, tb)),
        });
        let ts = if index_ts.len() >= 3 { index_ts } else { scan_timestamps(&mut input, idx) };
        if let Some(v) = video.as_mut() {
            v.variable_frame_rate = is_variable(&ts);
            if v.frame_count.is_none() && ts.len() > 1 && ts.len() < VFR_SCAN_PACKETS {
                v.frame_count = Some(ts.len() as u64);
            }
        }
    }

    if let Some(idx) = best_a
        && let Some(st) = input.stream(idx)
    {
        let par = st.parameters();
        let tb: Rational = st.time_base().into();
        let dur = st.duration();
        // SAFETY: as for the video stream, the parameters outlive this block and are plain data; `ch_layout` is
        // initialised by the demuxer, and `av_channel_layout_describe` writes at most `buf.len()` bytes including
        // the terminating NUL, so `CStr::from_ptr` reads a terminated string inside `buf`.
        let (codec_id, profile, fmt, rate, channels, layout) = unsafe {
            let p = &*par.as_ptr();
            let mut buf = [0 as std::ffi::c_char; 128];
            let n = ffi::av_channel_layout_describe(&p.ch_layout, buf.as_mut_ptr(), buf.len());
            let layout = if n > 0 {
                std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
            } else {
                format!("{} channels", p.ch_layout.nb_channels)
            };
            (p.codec_id, p.profile, p.format, p.sample_rate as u32, p.ch_layout.nb_channels as u32, layout)
        };
        audio = Some(AudioStreamInfo {
            stream_index: idx,
            codec: codec_name(codec_id),
            profile: codec_profile_name(codec_id, profile),
            sample_rate: rate,
            channels,
            layout,
            sample_format: sample_fmt_name(ffi::AVSampleFormat(fmt)),
            time_base: tb,
            duration_us: (dur != NOPTS && dur > 0).then(|| ts_to_us(dur, tb)),
        });
    }

    Ok(MediaInfo { path: path.to_owned(), container, duration_us, bit_rate, video, audio, stream_count })
}

/// Timestamps from the demuxer's sample index (MOV/MP4 `stts`, MKV cues are sparse and give < 3 entries' use).
fn index_timestamps(st: &ffmpeg_the_third::format::stream::Stream) -> Vec<i64> {
    // SAFETY: the stream pointer is valid; the index API only reads demuxer-owned memory.
    unsafe {
        let ptr = st.as_ptr() as *mut ffi::AVStream;
        let n = ffi::avformat_index_get_entries_count(ptr);
        let mut out = Vec::with_capacity(n.max(0) as usize);
        for i in 0..n {
            let e = ffi::avformat_index_get_entry(ptr, i);
            if !e.is_null() {
                out.push((*e).timestamp);
            }
        }
        // Sparse indexes (only keyframes, e.g. Matroska cues) cannot tell frame intervals apart.
        let n_frames = (*ptr).nb_frames;
        if n_frames > 0 && (out.len() as i64) < n_frames / 2 {
            out.clear();
        }
        out
    }
}

fn scan_timestamps(input: &mut ffmpeg_the_third::format::context::Input, idx: usize) -> Vec<i64> {
    let mut ts = Vec::new();
    let mut packet = ffmpeg_the_third::Packet::empty();
    while ts.len() < VFR_SCAN_PACKETS {
        match packet.read(input) {
            Ok(()) => {
                if packet.stream() == idx
                    && let Some(t) = packet.pts().or(packet.dts())
                {
                    ts.push(t);
                }
            }
            Err(_) => break,
        }
    }
    ts
}

/// VFR if frame intervals differ by more than one tick and 1 % (rounding jitter in 1/600 or 1/90000 timescales
/// is not variable frame rate).
pub(crate) fn is_variable(ts: &[i64]) -> bool {
    if ts.len() < 3 {
        return false;
    }
    let mut sorted = ts.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    // Ignore the last interval: containers often shorten the final frame.
    let deltas: Vec<i64> = sorted.windows(2).map(|w| w[1] - w[0]).collect();
    let body = if deltas.len() > 2 { &deltas[..deltas.len() - 1] } else { &deltas[..] };
    let (min, max) = body.iter().fold((i64::MAX, i64::MIN), |(lo, hi), &d| (lo.min(d), hi.max(d)));
    max - min > 1 && (max - min) as f64 > 0.01 * min as f64
}

#[cfg(test)]
mod tests {
    use super::is_variable;

    #[test]
    fn constant_intervals_are_not_vfr() {
        assert!(!is_variable(&[0, 20, 40, 60, 80, 100]));
        // 29.97 in a 1/600 timescale alternates 20/20/20/21 ticks: not VFR.
        assert!(!is_variable(&[0, 20, 40, 60, 81, 101, 121]));
        // B-frame reordering does not matter.
        assert!(!is_variable(&[0, 60, 20, 40, 120, 80, 100]));
    }

    #[test]
    fn irregular_intervals_are_vfr() {
        assert!(is_variable(&[0, 27, 54, 60, 87, 114, 120]));
        assert!(is_variable(&[0, 1000, 2000, 4000, 5000, 6000]));
    }
}
