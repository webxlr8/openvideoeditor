//! Writes proxies (change add-background-tasks-and-media-derivatives, design D7): a 1080p H.264 8-bit copy of a video
//! file with a keyframe every 12 frames, through the export pipeline, so it uses the same encoders (hardware first)
//! and the same atomic write as an export. Frames come from `ove-media`'s RGBA output, which tone-maps HDR sources
//! to SDR with the shared tone-mapping. The proxy carries no sound: playback reads sound from the original, which is
//! cheap to decode, so only the picture is replaced.
//!
//! Which files need a proxy, and where it lives, is decided in `ove_media::proxy`.

use std::cell::RefCell;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use ove_core::time::Micros;
use ove_media::{DecoderOptions, HwAccel, OutputFormat, VideoDecoder, VideoFrame};
use serde::Serialize;

use crate::canvas::Rgba;
use crate::export::{
    ExportError, ExportSettings, Picture, Quality, SourceSpec, VideoCodec, VideoSettings, export_frames,
};

/// The project field of the "Use proxies" setting (set with `updateProject`; absent means off).
pub const SETTING: &str = "useProxies";

/// Whether the project's "Use proxies" setting is on.
pub fn enabled(doc: &ove_core::model::Document) -> bool {
    doc.project.extra.get(SETTING).and_then(serde_json::Value::as_bool) == Some(true)
}

/// What a finished proxy is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyResult {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frames: u64,
    pub encoder: Option<String>,
    pub seconds_taken: f64,
}

/// Reads a video forward and gives, for each time asked, the last frame at or before it.
struct Forward {
    dec: Box<dyn VideoDecoder>,
    current: Rgba,
    current_pts: Micros,
    next: Option<VideoFrame>,
    width: u32,
    height: u32,
}

fn to_rgba(f: VideoFrame, width: u32, height: u32) -> Result<Rgba, ExportError> {
    if f.format != ove_media::PixelFormat::Rgba8 {
        return Err(ExportError::Ffmpeg("the decoder did not return RGBA frames".into()));
    }
    let plane = f.planes.into_iter().next().ok_or_else(|| ExportError::Ffmpeg("an empty frame".into()))?;
    let row = width as usize * 4;
    if f.width == width && f.height == height && plane.stride == row {
        return Ok(Rgba { w: width, h: height, data: plane.data });
    }
    // Odd sizes lose their last column or row (4:2:0 encoders need even sizes).
    let mut data = Vec::with_capacity(row * height as usize);
    for r in plane.data.chunks(plane.stride).take(height as usize) {
        data.extend_from_slice(&r[..row.min(r.len())]);
    }
    if data.len() != row * height as usize {
        return Err(ExportError::Ffmpeg("a decoded frame is smaller than the first".into()));
    }
    Ok(Rgba { w: width, h: height, data })
}

impl Forward {
    fn open(source: &Path, max_w: u32, max_h: u32) -> Result<Forward, ExportError> {
        let opts = DecoderOptions {
            hwaccel: HwAccel::Auto,
            output: OutputFormat::Rgba { max_width: max_w, max_height: max_h },
            inject_hw_failure_after: None,
        };
        let mut dec: Box<dyn VideoDecoder> =
            Box::new(ove_media::open_video(source, opts).map_err(|e| ExportError::Ffmpeg(e.to_string()))?);
        let first = dec
            .next_frame()
            .map_err(|e| ExportError::Ffmpeg(e.to_string()))?
            .ok_or_else(|| ExportError::Refused("the video has no frames".into()))?;
        let (width, height) = ((first.width & !1).max(2), (first.height & !1).max(2));
        let current_pts = first.pts_us;
        let current = to_rgba(first, width, height)?;
        let next = dec.next_frame().map_err(|e| ExportError::Ffmpeg(e.to_string()))?;
        Ok(Forward { dec, current, current_pts, next, width, height })
    }

    /// The frame showing at `t` (µs from the stream's first frame).
    fn at(&mut self, t: Micros) -> Result<Rgba, ExportError> {
        while let Some(n) = self.next.take() {
            // Frame times are rounded to whole µs on both sides: allow a millisecond.
            if n.pts_us > t + 1_000 {
                self.next = Some(n);
                break;
            }
            self.current_pts = n.pts_us;
            self.current = to_rgba(n, self.width, self.height)?;
            self.next = self.dec.next_frame().map_err(|e| ExportError::Ffmpeg(e.to_string()))?;
        }
        Ok(self.current.clone())
    }
}

/// Writes the proxy of `source` to `out` (replaced only by a complete file). `keep_going(fraction)` is called after
/// every frame; returning false cancels ([`ExportError::Cancelled`]).
pub fn make(source: &Path, out: &Path, mut keep_going: impl FnMut(f64) -> bool) -> Result<ProxyResult, ExportError> {
    ove_media::init();
    let info = ove_media::probe(source).map_err(|e| ExportError::Ffmpeg(e.to_string()))?;
    let v = info.video.as_ref().ok_or_else(|| ExportError::Refused("the file has no video".into()))?;
    let rate = if v.frame_rate.num > 0 && v.frame_rate.den > 0 {
        v.frame_rate.num as f64 / v.frame_rate.den as f64
    } else {
        30.0
    };
    let fps = (rate.round() as u32).clamp(1, 120);
    let duration = info
        .duration_us
        .or(v.duration_us)
        .filter(|d| *d > 0)
        .ok_or_else(|| ExportError::Refused("the video's length is unknown".into()))?;
    let (max_w, max_h) = ove_media::proxy::frame_size(v.width, v.height);
    let mut frames = Forward::open(source, max_w, max_h)?;
    // Opening the decoder can take a while (hardware set-up): stop here when cancelled meanwhile.
    if !keep_going(0.0) {
        return Err(ExportError::Cancelled);
    }
    let start_pts = frames.current_pts;
    let spec = SourceSpec {
        width: frames.width,
        height: frames.height,
        fps,
        duration,
        hdr: false,
        keyframe_interval: Some(ove_media::proxy::GOP),
    };
    let settings = ExportSettings {
        video: Some(VideoSettings { codec: VideoCodec::H264, quality: Quality::High, ..VideoSettings::default() }),
        audio: None,
        ..ExportSettings::default()
    };
    let cancel = AtomicBool::new(false);
    let failed: RefCell<Option<ExportError>> = RefCell::new(None);
    let (w, h) = (frames.width, frames.height);
    let frame = |t: Micros| match frames.at(start_pts + t) {
        Ok(img) => Picture::Rgba8(img),
        Err(e) => {
            *failed.borrow_mut() = Some(e);
            cancel.store(true, Ordering::SeqCst);
            Picture::Rgba8(Rgba { w, h, data: vec![0; (w * h * 4) as usize] })
        }
    };
    let result = export_frames(
        spec,
        &settings,
        out,
        frame,
        |_, n| vec![0.0; n * 2],
        &cancel,
        |p| {
            if !keep_going(p.fraction) {
                cancel.store(true, Ordering::SeqCst);
            }
        },
    );
    if let Some(e) = failed.into_inner() {
        return Err(e);
    }
    let r = result?;
    Ok(ProxyResult {
        path: r.path,
        width: r.width,
        height: r.height,
        fps: r.fps,
        frames: r.frames,
        encoder: r.video_encoder,
        seconds_taken: r.seconds_taken,
    })
}
