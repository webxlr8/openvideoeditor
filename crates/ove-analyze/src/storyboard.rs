//! Storyboard sheets (unify-agent-tool-layer D6): about 120 sampled frames, near-duplicates dropped, at most 36 tiles
//! in 6 columns, each with its timecode burned in (and, on timeline sheets, the ids of the items visible then).

use std::path::Path;

use ove_core::model::Document;

use crate::grid::label;
use crate::image::{Image, sheet};
use ove_media::VideoDecoder;

use crate::video::{duration_of, frame_to_image, open};
use crate::{Error, Result};

/// Frames sampled before near-duplicates are dropped.
pub const SAMPLES: usize = 120;
/// Most tiles on a sheet.
pub const MAX_TILES: usize = 36;
pub const COLUMNS: u32 = 6;
/// Mean difference of the 8×8 luma grid (0–255) below which a frame repeats the last kept one.
pub const SAME: f64 = 6.0;

/// One kept frame.
#[derive(Debug, Clone)]
pub struct Tile {
    pub at: f64,
    pub image: Image,
    /// Item ids visible at `at` (timeline sheets).
    pub items: Vec<String>,
}

/// The 8×8 mean-luma grid of a frame.
pub fn signature(img: &Image) -> [f64; 64] {
    let mut sum = [0.0f64; 64];
    let mut n = [0.0f64; 64];
    for y in 0..img.height {
        for x in 0..img.width {
            let p = img.px(x, y);
            let cell = ((y * 8 / img.height.max(1)) * 8 + x * 8 / img.width.max(1)) as usize;
            sum[cell] += 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
            n[cell] += 1.0;
        }
    }
    let mut out = [0.0; 64];
    for i in 0..64 {
        out[i] = if n[i] > 0.0 { sum[i] / n[i] } else { 0.0 };
    }
    out
}

pub fn difference(a: &[f64; 64], b: &[f64; 64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f64>() / 64.0
}

/// Keeps a frame when it differs from the last kept one (or shows other items), then, while there are more than 36, drops the frame most
/// like the one before it (so every distinct shot keeps a tile for as long as possible).
pub fn select(tiles: Vec<Tile>) -> Vec<Tile> {
    let mut kept: Vec<(Tile, [f64; 64])> = Vec::new();
    for t in tiles {
        let sig = signature(&t.image);
        // A new picture, or (on timeline sheets) a change in which items are visible.
        if kept.last().is_none_or(|(prev, last)| difference(last, &sig) >= SAME || prev.items != t.items) {
            kept.push((t, sig));
        }
    }
    while kept.len() > MAX_TILES {
        let (at, _) = (1..kept.len())
            .map(|i| (i, difference(&kept[i - 1].1, &kept[i].1)))
            .fold((1, f64::MAX), |best, x| if x.1 < best.1 { x } else { best });
        kept.remove(at);
    }
    kept.into_iter().map(|(t, _)| t).collect()
}

/// `h:mm:ss.d`.
pub fn timecode(t: f64) -> String {
    let tenths = (t.max(0.0) * 10.0).round() as u64;
    let (s, d) = (tenths / 10, tenths % 10);
    format!("{}:{:02}:{:02}.{d}", s / 3600, s / 60 % 60, s % 60)
}

/// The sheet: tiles in 6 columns, each labelled with its timecode and item ids.
pub fn render(tiles: &[Tile]) -> Image {
    let labelled: Vec<Image> = tiles
        .iter()
        .map(|t| {
            let mut img = t.image.clone();
            let s = (img.width / 240).max(1) as i64;
            label(&mut img, &timecode(t.at), 3 * s, 3 * s, s);
            let h = img.height as i64;
            for (row, id) in t.items.iter().take(4).enumerate() {
                label(&mut img, id, 3 * s, h - (row as i64 + 1) * 8 * s, s);
            }
            img
        })
        .collect();
    sheet(&labelled, COLUMNS.min(labelled.len().max(1) as u32))
}

/// Sample times: the centres of `n` equal slices of `[0, duration)`.
pub fn sample_times(duration: f64, n: usize) -> Vec<f64> {
    if duration <= 0.0 {
        return vec![0.0];
    }
    (0..n).map(|i| (i as f64 + 0.5) * duration / n as f64).collect()
}

/// A storyboard of a media file: frames snapped to the decoder's keyframes (fast seeking, no decoding up to the
/// exact time), near-duplicates dropped.
pub fn media(path: &Path, tile_width: u32) -> Result<(Image, Vec<Tile>)> {
    let duration = duration_of(path)?;
    let mut dec = open(path, tile_width)?;
    let mut tiles = Vec::new();
    let mut last_pts = None;
    for t in sample_times(duration, SAMPLES) {
        dec.seek((t * 1_000_000.0) as i64)?;
        let Some(f) = dec.next_frame()? else { continue };
        if last_pts == Some(f.pts_us) {
            continue;
        }
        last_pts = Some(f.pts_us);
        let Some(img) = frame_to_image(&f) else { continue };
        let img = if img.width != tile_width { img.resize_to_width(tile_width) } else { img };
        tiles.push(Tile { at: f.pts_us.max(0) as f64 / 1_000_000.0, image: img, items: vec![] });
    }
    if tiles.is_empty() {
        return Err(Error::NoFrame { path: path.display().to_string(), at: 0.0 });
    }
    let tiles = select(tiles);
    Ok((render(&tiles), tiles))
}

/// Ids of the items with a picture visible at `t` seconds, top track first.
pub fn visible_items(doc: &Document, t: f64) -> Vec<String> {
    let us = (t * 1_000_000.0).round() as i64;
    let mut out = Vec::new();
    for track in &doc.project.tracks {
        for i in doc.items_on_track(&track.id) {
            if i.start <= us && us < i.end && i.kind != "audio" {
                out.push(i.id.clone());
            }
        }
    }
    out
}

/// A storyboard of the timeline: composited frames, near-duplicates dropped, item ids burned in.
pub fn timeline(doc: &Document, bundle: Option<&Path>, tile_width: u32) -> Result<(Image, Vec<Tile>)> {
    let duration = doc.duration() as f64 / 1_000_000.0;
    let times = sample_times(duration, SAMPLES);
    let frames = crate::look::look(doc, bundle, &times, tile_width)?;
    let tiles: Vec<Tile> =
        times.iter().zip(frames).map(|(t, image)| Tile { at: *t, image, items: visible_items(doc, *t) }).collect();
    let tiles = select(tiles);
    Ok((render(&tiles), tiles))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(at: f64, grey: u8) -> Tile {
        Tile { at, image: Image::filled(32, 18, [grey, grey, grey, 255]), items: vec![] }
    }

    #[test]
    fn repeats_are_dropped_and_at_most_36_tiles_stay() {
        // Three shots of 40 identical frames: one tile each.
        let tiles: Vec<Tile> = (0..120).map(|i| tile(i as f64, [10, 120, 240][i / 40])).collect();
        let kept = select(tiles);
        assert_eq!(kept.iter().map(|t| t.at).collect::<Vec<_>>(), vec![0.0, 40.0, 80.0]);
        // 120 all different: 36 remain, first one included.
        let tiles: Vec<Tile> = (0..120).map(|i| tile(i as f64, (i * 2) as u8)).collect();
        let kept = select(tiles);
        assert_eq!(kept.len(), MAX_TILES);
        assert_eq!(kept[0].at, 0.0);
    }

    #[test]
    fn timecodes() {
        assert_eq!(timecode(0.0), "0:00:00.0");
        assert_eq!(timecode(3725.46), "1:02:05.5");
    }
}
