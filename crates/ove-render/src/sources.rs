//! Decoded-frame sources for the compositor: one decoder per (item, size), read sequentially and sought only on
//! jumps, so playback and export decode each frame once. Images are decoded once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ove_core::model::Document;
use ove_core::time::Micros;
use ove_media::{DecoderOptions, FallbackDecoder, HwAccel, OutputFormat, VideoDecoder, VideoFrame};

use crate::canvas::Rgba;

/// Resolves a media item's file: `~/`, absolute, or relative to the bundle.
pub fn media_path(doc: &Document, bundle: Option<&Path>, media_id: &str) -> Option<PathBuf> {
    let e = doc.registry.get(media_id)?;
    let native = e.native_path.as_str();
    if let Some(rest) = native.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return Some(PathBuf::from(home).join(rest));
    }
    let p = PathBuf::from(native);
    Some(if p.is_absolute() { p } else { bundle.map(|b| b.join(&p)).unwrap_or(p) })
}

fn to_rgba(f: VideoFrame) -> Option<Rgba> {
    if f.format != ove_media::PixelFormat::Rgba8 {
        return None;
    }
    let p = f.planes.into_iter().next()?;
    let row = f.width as usize * 4;
    let data = if p.stride == row {
        p.data
    } else {
        p.data.chunks(p.stride).take(f.height as usize).flat_map(|r| r[..row].iter().copied()).collect()
    };
    Some(Rgba { w: f.width, h: f.height, data })
}

struct Entry {
    dec: Option<FallbackDecoder>,
    /// (pts, end, image) of the frame shown last.
    current: Option<(Micros, Micros, Arc<Rgba>)>,
    image: bool,
    eof: bool,
    /// Not read or sought yet: positioned at the start of the stream.
    fresh: bool,
    used: u64,
}

/// Cache of open decoders keyed by item and decode height.
pub struct Sources {
    entries: HashMap<(String, u32), Entry>,
    tick: u64,
    /// How far ahead (µs) decoding forward is preferred over seeking.
    pub forward_window: Micros,
    pub hwaccel: HwAccel,
}

impl Default for Sources {
    fn default() -> Self {
        Sources { entries: HashMap::new(), tick: 0, forward_window: 2_000_000, hwaccel: HwAccel::Off }
    }
}

impl Sources {
    /// Frame of `path` for item `key` at source time `src_us`, decoded to fit `max_h` pixels high.
    pub fn frame(&mut self, key: &str, path: &Path, src_us: Micros, max_h: u32, image: bool) -> Option<Arc<Rgba>> {
        self.tick += 1;
        let tick = self.tick;
        let hw = self.hwaccel.clone();
        let e = self.entries.entry((key.to_string(), max_h)).or_insert_with(|| {
            let opts = DecoderOptions {
                hwaccel: hw,
                output: OutputFormat::Rgba { max_width: 0, max_height: max_h },
                inject_hw_failure_after: None,
            };
            let dec = ove_media::open_video(path, opts)
                .inspect_err(|e| tracing::warn!(path = %path.display(), error = %e, "cannot decode media"))
                .ok();
            Entry { dec, current: None, image, eof: false, fresh: true, used: tick }
        });
        e.used = tick;
        let dec = e.dec.as_mut()?;
        if let Some((pts, end, img)) = &e.current
            && (e.image || (*pts <= src_us && src_us < *end) || (e.eof && src_us >= *pts))
        {
            return Some(img.clone());
        }
        let forward = e.image
            || (e.fresh && src_us < self.forward_window)
            || e.current.as_ref().is_some_and(|(pts, _, _)| src_us > *pts && src_us - *pts < self.forward_window);
        e.fresh = false;
        if !forward {
            if dec.seek(src_us).is_err() {
                return e.current.as_ref().map(|c| c.2.clone());
            }
            e.eof = false;
        }
        loop {
            match dec.next_frame() {
                Ok(Some(f)) => {
                    let pts = f.pts_us;
                    let dur = if f.duration_us > 0 { f.duration_us } else { 33_333 };
                    let img = to_rgba(f)?;
                    e.current = Some((pts, pts + dur, Arc::new(img)));
                    if e.image || pts + dur > src_us {
                        break;
                    }
                }
                Ok(None) => {
                    // Past the end of the source: hold the last frame.
                    e.eof = true;
                    break;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "decode error");
                    break;
                }
            }
        }
        e.current.as_ref().map(|c| c.2.clone())
    }

    /// Drops decoders not used during the last `keep` frame requests.
    pub fn trim(&mut self, keep: u64) {
        let tick = self.tick;
        self.entries.retain(|_, e| tick - e.used <= keep);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
