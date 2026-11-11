//! Frames of a media file: one frame, contact sheets, scene cuts.

use std::path::Path;

use ove_media::{ColorMatrix, DecoderOptions, HwAccel, OutputFormat, PixelFormat, VideoDecoder, VideoFrame};
use serde::Serialize;

use crate::image::{Image, sheet};
use crate::{Error, Result};

const US: f64 = 1_000_000.0;

fn open(path: &Path, max_width: u32) -> Result<ove_media::FallbackDecoder> {
    ove_media::init();
    let opts = DecoderOptions {
        hwaccel: HwAccel::Off,
        output: OutputFormat::Rgba { max_width, max_height: 0 },
        inject_hw_failure_after: None,
    };
    Ok(ove_media::open_video(path, opts)?)
}

/// Converts a decoded frame to RGBA: RGBA8 is copied, NV12/P010 are converted with the frame's matrix and range.
pub fn frame_to_image(f: &VideoFrame) -> Option<Image> {
    let (w, h) = (f.width, f.height);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    match f.format {
        PixelFormat::Rgba8 => {
            let p = f.planes.first()?;
            for y in 0..h as usize {
                rgba.extend_from_slice(p.data.get(y * p.stride..y * p.stride + w as usize * 4)?);
            }
        }
        PixelFormat::Nv12 | PixelFormat::P010 => {
            let deep = f.format == PixelFormat::P010;
            let (yp, uvp) = (f.planes.first()?, f.planes.get(1)?);
            // Sample value on the 8-bit code scale (P010 keeps its fraction).
            let read = |p: &ove_media::Plane, col: usize, row: usize| -> Option<f64> {
                Some(if deep {
                    let i = row * p.stride + col * 2;
                    u16::from_le_bytes([*p.data.get(i)?, *p.data.get(i + 1)?]) as f64 / 256.0
                } else {
                    *p.data.get(row * p.stride + col)? as f64
                })
            };
            // (Kr, Kb) of the matrix.
            let (kr, kb) = match f.color.matrix {
                ColorMatrix::Bt601 => (0.299, 0.114),
                ColorMatrix::Bt709 => (0.2126, 0.0722),
                ColorMatrix::Bt2020 => (0.2627, 0.0593),
            };
            let kg = 1.0 - kr - kb;
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let (yv, u, v) = (read(yp, x, y)?, read(uvp, x / 2 * 2, y / 2)?, read(uvp, x / 2 * 2 + 1, y / 2)?);
                    let (luma, cb, cr) = if f.color.full_range {
                        (yv / 255.0, (u - 128.0) / 255.0, (v - 128.0) / 255.0)
                    } else {
                        ((yv - 16.0) / 219.0, (u - 128.0) / 224.0, (v - 128.0) / 224.0)
                    };
                    let r = luma + 2.0 * (1.0 - kr) * cr;
                    let b = luma + 2.0 * (1.0 - kb) * cb;
                    let g = (luma - kr * r - kb * b) / kg;
                    let c = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                    rgba.extend_from_slice(&[c(r), c(g), c(b), 255]);
                }
            }
        }
        PixelFormat::Native(_) => return None,
    }
    Some(Image { width: w, height: h, rgba })
}

/// The frame shown at `at` seconds (the last frame when `at` is past the end), scaled to `width` (aspect kept).
pub fn grab(path: &Path, at: f64, width: Option<u32>) -> Result<Image> {
    let mut dec = open(path, width.unwrap_or(0))?;
    grab_from(&mut dec, path, at, width)
}

fn grab_from(dec: &mut ove_media::FallbackDecoder, path: &Path, at: f64, width: Option<u32>) -> Result<Image> {
    let t = (at.max(0.0) * US) as i64;
    dec.seek(t)?;
    let mut last: Option<VideoFrame> = None;
    while let Some(f) = dec.next_frame()? {
        let end = f.pts_us + if f.duration_us > 0 { f.duration_us } else { 1 };
        let done = end > t;
        last = Some(f);
        if done {
            break;
        }
    }
    let f = last.ok_or_else(|| Error::NoFrame { path: path.display().to_string(), at })?;
    let img = frame_to_image(&f).ok_or_else(|| Error::NoFrame { path: path.display().to_string(), at })?;
    Ok(match width {
        Some(w) if w != img.width => img.resize_to_width(w),
        _ => img,
    })
}

fn duration_of(path: &Path) -> Result<f64> {
    let info = ove_media::probe(path)?;
    Ok(info.video.as_ref().and_then(|v| v.duration_us).or(info.duration_us).unwrap_or(0) as f64 / US)
}

/// Contact sheet options. Frames are `count` evenly spaced times in `[from, to)`, or one every `every` seconds
/// (at most `count` when both are given).
#[derive(Debug, Clone, PartialEq)]
pub struct FilmstripOpts {
    pub count: u32,
    pub every: Option<f64>,
    pub from: f64,
    /// End time; `None` = the end of the media.
    pub to: Option<f64>,
    /// Tiles per row; 0 = up to 5.
    pub columns: u32,
    pub tile_width: u32,
}

impl Default for FilmstripOpts {
    fn default() -> Self {
        FilmstripOpts { count: 8, every: None, from: 0.0, to: None, columns: 0, tile_width: 240 }
    }
}

/// The times a filmstrip samples for a media of `duration` seconds.
pub fn filmstrip_times(opts: &FilmstripOpts, duration: f64) -> Vec<f64> {
    let to = opts.to.unwrap_or(duration).min(duration.max(0.0));
    let from = opts.from.clamp(0.0, to.max(0.0));
    let span = (to - from).max(0.0);
    let round = |t: f64| (t * 1000.0).round() / 1000.0;
    match opts.every {
        Some(e) if e > 0.0 => {
            let cap = if opts.count > 0 { opts.count as usize } else { 1000 };
            (0..)
                .map(|i| from + i as f64 * e)
                .take_while(|t| *t < to || (span == 0.0 && *t == from))
                .take(cap)
                .map(round)
                .collect()
        }
        _ => {
            let n = opts.count.max(1);
            (0..n).map(|i| round(from + (i as f64 + 0.5) * span / n as f64)).collect()
        }
    }
}

/// A grid of frames and the times they show.
pub fn filmstrip(path: &Path, opts: &FilmstripOpts) -> Result<(Image, Vec<f64>)> {
    let times = filmstrip_times(opts, duration_of(path)?);
    let tile = opts.tile_width.max(16);
    let mut dec = open(path, tile)?;
    let mut tiles = Vec::with_capacity(times.len());
    for &t in &times {
        tiles.push(grab_from(&mut dec, path, t, Some(tile))?);
    }
    let columns = if opts.columns == 0 { (tiles.len() as u32).min(5) } else { opts.columns };
    Ok((sheet(&tiles, columns), times))
}

/// A detected hard cut: the time of the first frame of the new shot and how different it is (0–1).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneCut {
    pub time: f64,
    pub score: f64,
}

/// Scene cuts of a media file and its duration.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scenes {
    pub cuts: Vec<SceneCut>,
    pub duration: f64,
    pub frames: u64,
}

const SCENE_WIDTH: u32 = 64;
const BINS: usize = 32;

/// Luma histogram (normalised) and the luma plane of a small frame.
fn luma_signature(img: &Image) -> ([f64; BINS], Vec<u8>) {
    let mut hist = [0f64; BINS];
    let mut luma = Vec::with_capacity((img.width * img.height) as usize);
    for p in img.rgba.chunks_exact(4) {
        let y = (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64).round().clamp(0.0, 255.0) as u8;
        hist[y as usize * BINS / 256] += 1.0;
        luma.push(y);
    }
    let n = luma.len().max(1) as f64;
    for h in &mut hist {
        *h /= n;
    }
    (hist, luma)
}

/// Finds hard cuts. Each frame (decoded at 64 px wide) is compared with the previous one: the score is the mean of
/// the luma-histogram distance and the (scaled, capped) mean absolute pixel difference, both 0–1. A cut is reported
/// where the score exceeds `threshold` (0.3 is a good default) and is the local maximum of its neighbourhood.
pub fn scenes(path: &Path, threshold: f64) -> Result<Scenes> {
    let mut dec = open(path, SCENE_WIDTH)?;
    let mut prev: Option<([f64; BINS], Vec<u8>)> = None;
    let mut scores: Vec<(f64, f64)> = Vec::new();
    let mut frames = 0u64;
    let mut end = 0f64;
    while let Some(f) = dec.next_frame()? {
        frames += 1;
        let t = f.pts_us as f64 / US;
        end = end.max(t + f.duration_us.max(0) as f64 / US);
        let Some(img) = frame_to_image(&f) else { continue };
        let img = if img.width > SCENE_WIDTH { img.resize_to_width(SCENE_WIDTH) } else { img };
        let sig = luma_signature(&img);
        if let Some((ph, pl)) = &prev
            && pl.len() == sig.1.len()
        {
            let hist: f64 = ph.iter().zip(&sig.0).map(|(a, b)| (a - b).abs()).sum::<f64>() / 2.0;
            let mad: f64 = pl.iter().zip(&sig.1).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>()
                / (pl.len() as f64 * 255.0);
            scores.push((t, 0.5 * hist + 0.5 * (mad * 4.0).min(1.0)));
        }
        prev = Some(sig);
    }
    let mut cuts = Vec::new();
    for (i, &(t, s)) in scores.iter().enumerate() {
        let neighbours = scores[i.saturating_sub(2)..(i + 3).min(scores.len())].iter();
        if s > threshold && neighbours.clone().all(|&(_, o)| o <= s) {
            cuts.push(SceneCut { time: (t * 1000.0).round() / 1000.0, score: (s * 1000.0).round() / 1000.0 });
        }
    }
    Ok(Scenes { cuts, duration: (end * 1000.0).round() / 1000.0, frames })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filmstrip_times_even_and_every() {
        let o = FilmstripOpts { count: 4, ..Default::default() };
        assert_eq!(filmstrip_times(&o, 8.0), vec![1.0, 3.0, 5.0, 7.0]);
        let o = FilmstripOpts { count: 0, every: Some(2.5), from: 1.0, to: Some(9.0), ..Default::default() };
        assert_eq!(filmstrip_times(&o, 10.0), vec![1.0, 3.5, 6.0, 8.5]);
        let o = FilmstripOpts { count: 2, every: Some(1.0), ..Default::default() };
        assert_eq!(filmstrip_times(&o, 10.0), vec![0.0, 1.0]);
    }

    #[test]
    fn nv12_white_and_black() {
        let plane = |data: Vec<u8>, stride, w, h| ove_media::Plane { data, stride, width: w, height: h };
        let f = VideoFrame {
            pts_us: 0,
            duration_us: 0,
            width: 2,
            height: 2,
            format: PixelFormat::Nv12,
            planes: vec![plane(vec![235, 16, 235, 16], 2, 2, 2), plane(vec![128, 128], 2, 1, 1)],
            color: ove_media::ColorInfo::default(),
            decode_path: ove_media::DecodePath::Software,
            copies: 0,
        };
        let img = frame_to_image(&f).unwrap();
        assert_eq!(img.px(0, 0), [255, 255, 255, 255]);
        assert_eq!(img.px(1, 1), [0, 0, 0, 255]);
    }
}
