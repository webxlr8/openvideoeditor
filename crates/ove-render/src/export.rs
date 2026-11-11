//! Export: render every frame, mix the audio, encode H.264 + AAC into MP4.
//!
//! Video encoder preference: the platform's hardware encoder (VideoToolbox on macOS; Media Foundation, NVENC,
//! Quick Sync or AMF on Windows), then OpenH264 (BSD), then libx264 if the FFmpeg build has it (GPL: development
//! machines only, never in a shipped build), then MPEG-4 Part 2 as a last resort. Audio: FFmpeg's native AAC.

use std::ffi::CString;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use ffmpeg_the_third::ffi;
use ove_core::model::Document;
use ove_core::time::Micros;
use serde::{Deserialize, Serialize};

use crate::audio::{CHANNELS, Mixer, RATE};
use crate::render::Renderer;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSettings {
    /// Output height in pixels (width follows the aspect ratio).
    pub height: u32,
    pub fps: u32,
    /// Video bit rate in bits per second; 0 = automatic from the frame size.
    pub bitrate: u64,
    /// Encoder name to force (testing); `None` picks automatically.
    pub encoder: Option<String>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings { height: 1080, fps: 30, bitrate: 0, encoder: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub frame: u64,
    pub total_frames: u64,
    pub fraction: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub frames: u64,
    pub duration_seconds: f64,
    pub video_encoder: String,
    pub seconds_taken: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}")]
    Refused(String),
    #[error("export was cancelled")]
    Cancelled,
    #[error("encoding failed: {0}")]
    Ffmpeg(String),
}

fn err(what: &str, code: i32) -> ExportError {
    let mut buf = [0 as std::ffi::c_char; 256];
    // SAFETY: buffer is valid for its length; av_strerror NUL-terminates.
    unsafe { ffi::av_strerror(code, buf.as_mut_ptr(), buf.len()) };
    // SAFETY: NUL-terminated by av_strerror.
    let msg = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
    ExportError::Ffmpeg(format!("{what}: {msg}"))
}

/// Encoder names to try, best first.
pub fn encoder_candidates() -> Vec<&'static str> {
    let mut v = Vec::new();
    if cfg!(target_os = "macos") {
        v.push("h264_videotoolbox");
    }
    if cfg!(windows) {
        v.extend(["h264_nvenc", "h264_qsv", "h264_amf", "h264_mf"]);
    }
    v.extend(["libopenh264", "libx264", "mpeg4"]);
    v
}

struct Stream {
    ctx: *mut ffi::AVCodecContext,
    st: *mut ffi::AVStream,
}

struct Muxer {
    oc: *mut ffi::AVFormatContext,
    video: Stream,
    audio: Stream,
    sws: *mut ffi::SwsContext,
    frame: *mut ffi::AVFrame,
    aframe: *mut ffi::AVFrame,
    pkt: *mut ffi::AVPacket,
    encoder: String,
    header_written: bool,
}

impl Drop for Muxer {
    fn drop(&mut self) {
        // SAFETY: every pointer is owned by this muxer and freed once.
        unsafe {
            ffi::av_packet_free(&mut self.pkt);
            ffi::av_frame_free(&mut self.frame);
            ffi::av_frame_free(&mut self.aframe);
            if !self.sws.is_null() {
                ffi::sws_freeContext(self.sws);
            }
            ffi::avcodec_free_context(&mut self.video.ctx);
            ffi::avcodec_free_context(&mut self.audio.ctx);
            if !self.oc.is_null() {
                if !(*self.oc).pb.is_null() {
                    ffi::avio_closep(&mut (*self.oc).pb);
                }
                ffi::avformat_free_context(self.oc);
            }
        }
    }
}

const SWS_BILINEAR: i32 = 2;
const SWS_CS_ITU709: i32 = 1;
const SWS_CS_ITU601: i32 = 5;

impl Muxer {
    /// Opens the output, the encoders and writes the header.
    unsafe fn open(path: &Path, w: u32, h: u32, s: &ExportSettings) -> Result<Muxer, ExportError> {
        // SAFETY: plain FFmpeg C API use; every allocation is checked and owned by the returned muxer.
        unsafe {
            let cpath =
                CString::new(path.to_string_lossy().as_bytes()).map_err(|_| ExportError::Refused("bad path".into()))?;
            let mut m = Muxer {
                oc: ptr::null_mut(),
                video: Stream { ctx: ptr::null_mut(), st: ptr::null_mut() },
                audio: Stream { ctx: ptr::null_mut(), st: ptr::null_mut() },
                sws: ptr::null_mut(),
                frame: ffi::av_frame_alloc(),
                aframe: ffi::av_frame_alloc(),
                pkt: ffi::av_packet_alloc(),
                encoder: String::new(),
                header_written: false,
            };
            let mp4 = CString::new("mp4").unwrap();
            let r = ffi::avformat_alloc_output_context2(&mut m.oc, ptr::null(), mp4.as_ptr(), cpath.as_ptr());
            if r < 0 || m.oc.is_null() {
                return Err(err("output", r));
            }
            // Constant types differ between FFmpeg versions' bindings, hence the casts.
            #[allow(clippy::unnecessary_cast)]
            let global = ((*(*m.oc).oformat).flags & ffi::AVFMT_GLOBALHEADER as i32) != 0;

            // Video encoder: first candidate that opens.
            let names: Vec<String> = match &s.encoder {
                Some(n) => vec![n.clone()],
                None => encoder_candidates().into_iter().map(str::to_string).collect(),
            };
            let bitrate = if s.bitrate > 0 {
                s.bitrate
            } else {
                (w as u64 * h as u64 * s.fps as u64 / 10).clamp(2_000_000, 60_000_000)
            };
            let mut opened = false;
            for name in &names {
                let cname = CString::new(name.as_str()).unwrap();
                let codec = ffi::avcodec_find_encoder_by_name(cname.as_ptr());
                if codec.is_null() {
                    continue;
                }
                let mut c = ffi::avcodec_alloc_context3(codec);
                (*c).width = w as i32;
                (*c).height = h as i32;
                (*c).time_base = ffi::AVRational { num: 1, den: s.fps as i32 };
                (*c).framerate = ffi::AVRational { num: s.fps as i32, den: 1 };
                (*c).gop_size = (s.fps * 2) as i32;
                (*c).max_b_frames = 0;
                (*c).bit_rate = bitrate as i64;
                (*c).pix_fmt = if name.contains("qsv") || name.contains("_mf") {
                    ffi::AVPixelFormat::NV12
                } else {
                    ffi::AVPixelFormat::YUV420P
                };
                (*c).colorspace = ffi::AVColorSpace::BT709;
                (*c).color_primaries = ffi::AVColorPrimaries::BT709;
                (*c).color_trc = ffi::AVColorTransferCharacteristic::BT709;
                (*c).color_range = ffi::AVColorRange::MPEG;
                if global {
                    (*c).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
                }
                let mut opts: *mut ffi::AVDictionary = ptr::null_mut();
                if name == "libx264" {
                    let (k, v) = (CString::new("preset").unwrap(), CString::new("veryfast").unwrap());
                    ffi::av_dict_set(&mut opts, k.as_ptr(), v.as_ptr(), 0);
                }
                let r = ffi::avcodec_open2(c, codec, &mut opts);
                ffi::av_dict_free(&mut opts);
                if r < 0 {
                    tracing::info!(encoder = name, "encoder unavailable, trying the next");
                    ffi::avcodec_free_context(&mut c);
                    continue;
                }
                m.video.ctx = c;
                m.encoder = name.clone();
                opened = true;
                break;
            }
            if !opened {
                return Err(ExportError::Ffmpeg(format!(
                    "no H.264 encoder could be opened (tried {})",
                    names.join(", ")
                )));
            }
            m.video.st = ffi::avformat_new_stream(m.oc, ptr::null());
            ffi::avcodec_parameters_from_context((*m.video.st).codecpar, m.video.ctx);
            (*m.video.st).time_base = (*m.video.ctx).time_base;

            // Audio: native AAC, stereo 48 kHz, planar float.
            let aac = ffi::avcodec_find_encoder(ffi::AVCodecID::AAC);
            if aac.is_null() {
                return Err(ExportError::Ffmpeg("no AAC encoder".into()));
            }
            let a = ffi::avcodec_alloc_context3(aac);
            (*a).sample_fmt = ffi::AVSampleFormat::FLTP;
            (*a).sample_rate = RATE as i32;
            ffi::av_channel_layout_default(&mut (*a).ch_layout, CHANNELS as i32);
            (*a).bit_rate = 192_000;
            (*a).time_base = ffi::AVRational { num: 1, den: RATE as i32 };
            if global {
                (*a).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
            m.audio.ctx = a;
            let r = ffi::avcodec_open2(a, aac, ptr::null_mut());
            if r < 0 {
                return Err(err("AAC encoder", r));
            }
            m.audio.st = ffi::avformat_new_stream(m.oc, ptr::null());
            ffi::avcodec_parameters_from_context((*m.audio.st).codecpar, a);
            (*m.audio.st).time_base = (*a).time_base;

            #[allow(clippy::unnecessary_cast)]
            let r = ffi::avio_open(&mut (*m.oc).pb, cpath.as_ptr(), ffi::AVIO_FLAG_WRITE as i32);
            if r < 0 {
                return Err(err("open output file", r));
            }
            let r = ffi::avformat_write_header(m.oc, ptr::null_mut());
            if r < 0 {
                return Err(err("write header", r));
            }
            m.header_written = true;

            // Frame buffers and the RGBA → YUV converter.
            let vf = m.frame;
            (*vf).format = (*m.video.ctx).pix_fmt.0;
            (*vf).width = w as i32;
            (*vf).height = h as i32;
            let r = ffi::av_frame_get_buffer(vf, 0);
            if r < 0 {
                return Err(err("frame buffer", r));
            }
            m.sws = ffi::sws_getContext(
                w as i32,
                h as i32,
                ffi::AVPixelFormat::RGBA,
                w as i32,
                h as i32,
                (*m.video.ctx).pix_fmt,
                SWS_BILINEAR,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if m.sws.is_null() {
                return Err(ExportError::Ffmpeg("no colour converter".into()));
            }
            ffi::sws_setColorspaceDetails(
                m.sws,
                ffi::sws_getCoefficients(SWS_CS_ITU601),
                1,
                ffi::sws_getCoefficients(SWS_CS_ITU709),
                0,
                0,
                1 << 16,
                1 << 16,
            );
            let af = m.aframe;
            (*af).format = ffi::AVSampleFormat::FLTP.0;
            (*af).sample_rate = RATE as i32;
            ffi::av_channel_layout_default(&mut (*af).ch_layout, CHANNELS as i32);
            let fs = (*m.audio.ctx).frame_size;
            (*af).nb_samples = if fs > 0 { fs } else { 1024 };
            let r = ffi::av_frame_get_buffer(af, 0);
            if r < 0 {
                return Err(err("audio frame buffer", r));
            }
            Ok(m)
        }
    }

    unsafe fn drain(&mut self, video: bool) -> Result<(), ExportError> {
        // SAFETY: valid open encoder, stream and packet.
        unsafe {
            let s = if video { &self.video } else { &self.audio };
            loop {
                let r = ffi::avcodec_receive_packet(s.ctx, self.pkt);
                if r == ffi::AVERROR(libc::EAGAIN) || r == ffi::AVERROR_EOF {
                    return Ok(());
                }
                if r < 0 {
                    return Err(err("encode", r));
                }
                ffi::av_packet_rescale_ts(self.pkt, (*s.ctx).time_base, (*s.st).time_base);
                (*self.pkt).stream_index = (*s.st).index;
                let r = ffi::av_interleaved_write_frame(self.oc, self.pkt);
                if r < 0 {
                    return Err(err("write", r));
                }
            }
        }
    }

    unsafe fn video_frame(&mut self, rgba: &[u8], w: u32, h: u32, pts: i64) -> Result<(), ExportError> {
        // SAFETY: frame and converter sized for w×h; the source slice holds w×h×4 bytes.
        unsafe {
            let r = ffi::av_frame_make_writable(self.frame);
            if r < 0 {
                return Err(err("frame", r));
            }
            let src = [rgba.as_ptr(), ptr::null(), ptr::null(), ptr::null()];
            let stride = [(w * 4) as i32, 0, 0, 0];
            ffi::sws_scale(
                self.sws,
                src.as_ptr(),
                stride.as_ptr(),
                0,
                h as i32,
                (*self.frame).data.as_ptr(),
                (*self.frame).linesize.as_ptr(),
            );
            (*self.frame).pts = pts;
            let r = ffi::avcodec_send_frame(self.video.ctx, self.frame);
            if r < 0 {
                return Err(err("send video frame", r));
            }
            self.drain(true)
        }
    }

    /// One audio frame of exactly `nb_samples` interleaved stereo samples.
    unsafe fn audio_frame(&mut self, interleaved: &[f32], pts: i64) -> Result<(), ExportError> {
        // SAFETY: frame buffer has nb_samples per plane; we write exactly that many.
        unsafe {
            let r = ffi::av_frame_make_writable(self.aframe);
            if r < 0 {
                return Err(err("audio frame", r));
            }
            let n = (*self.aframe).nb_samples as usize;
            for c in 0..CHANNELS {
                let plane = (*self.aframe).data[c] as *mut f32;
                for i in 0..n {
                    *plane.add(i) = interleaved.get(i * CHANNELS + c).copied().unwrap_or(0.0);
                }
            }
            (*self.aframe).pts = pts;
            let r = ffi::avcodec_send_frame(self.audio.ctx, self.aframe);
            if r < 0 {
                return Err(err("send audio frame", r));
            }
            self.drain(false)
        }
    }

    unsafe fn finish(&mut self) -> Result<(), ExportError> {
        // SAFETY: flush both encoders, then trailer.
        unsafe {
            ffi::avcodec_send_frame(self.video.ctx, ptr::null());
            self.drain(true)?;
            ffi::avcodec_send_frame(self.audio.ctx, ptr::null());
            self.drain(false)?;
            let r = ffi::av_write_trailer(self.oc);
            if r < 0 {
                return Err(err("trailer", r));
            }
            Ok(())
        }
    }
}

/// Exports the whole timeline. `progress` is called after every frame; set `cancel` to stop (the partial file is
/// removed).
pub fn export(
    doc: &Document,
    bundle: Option<&Path>,
    out: &Path,
    settings: &ExportSettings,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<ExportResult, ExportError> {
    let pending = doc.project.items.values().filter(|i| i.is_pending()).count();
    if pending > 0 {
        return Err(ExportError::Refused(format!(
            "{pending} pending placeholder{} still need media; fill or remove them before exporting",
            if pending == 1 { "" } else { "s" }
        )));
    }
    let duration = doc.duration();
    if duration <= 0 {
        return Err(ExportError::Refused("the timeline is empty".into()));
    }
    if settings.fps == 0 || settings.fps > 120 || settings.height < 16 || settings.height > 4320 {
        return Err(ExportError::Refused("frame rate must be 1–120 and height 16–4320".into()));
    }
    ove_media::init();
    let started = std::time::Instant::now();
    let mut renderer = Renderer::new(bundle, settings.height);
    let (w, h) = renderer.size(doc);
    let mut mixer = Mixer::new(bundle);
    let total = ((duration as i128 * settings.fps as i128 + 999_999) / 1_000_000) as u64;
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // SAFETY: the muxer owns every FFmpeg object and frees them on drop.
    let mut mux = unsafe { Muxer::open(out, w, h, settings)? };
    let result = (|| -> Result<(), ExportError> {
        let a_n = unsafe { (*mux.aframe).nb_samples } as usize;
        let mut audio_pts: i64 = 0;
        for i in 0..total {
            if cancel.load(Ordering::Relaxed) {
                return Err(ExportError::Cancelled);
            }
            let t: Micros = (i as i128 * 1_000_000 / settings.fps as i128) as Micros;
            let frame = renderer.render(doc, t);
            // SAFETY: frame is w×h RGBA.
            unsafe { mux.video_frame(&frame.data, w, h, i as i64)? };
            // Audio up to the end of this video frame.
            let video_end_samples = ((i + 1) as i128 * RATE as i128 / settings.fps as i128) as i64;
            while audio_pts + a_n as i64 <= video_end_samples {
                let t0 = audio_pts * 1_000_000 / RATE as i64;
                let samples = mixer.mix(doc, t0, a_n);
                // SAFETY: frame has a_n samples per channel.
                unsafe { mux.audio_frame(&samples, audio_pts)? };
                audio_pts += a_n as i64;
            }
            progress(Progress { frame: i + 1, total_frames: total, fraction: (i + 1) as f64 / total as f64 });
        }
        // SAFETY: flush and trailer.
        unsafe { mux.finish() }
    })();
    let encoder = mux.encoder.clone();
    drop(mux);
    if let Err(e) = result {
        let _ = std::fs::remove_file(out);
        return Err(e);
    }
    Ok(ExportResult {
        path: out.display().to_string(),
        width: w,
        height: h,
        frames: total,
        duration_seconds: total as f64 / settings.fps as f64,
        video_encoder: encoder,
        seconds_taken: started.elapsed().as_secs_f64(),
    })
}
