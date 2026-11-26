//! Skim sheets: frames from across a video, 160 px wide, packed into one image with the time of each tile, so the
//! media panel can follow the pointer across a thumbnail without decoding anything.
//!
//! Frame choice: `n` targets evenly spaced over the duration (the middle of each of `n` equal parts). For each
//! target, the tile is the first frame at or after `target − spacing / 2`. To do little decode work, the decoder
//! skips frames nothing refers to, keeps decoding forward while that is cheaper, and jumps to the nearest keyframe
//! when the next tile lies past the next keyframe.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{MediaError, Result};
use crate::frame::{OutputFormat, PixelFormat, VideoFrame};
use crate::util::Micros;
use crate::video::{DecoderOptions, FfmpegVideoDecoder, HwAccel, VideoDecoder};

/// Tile width in pixels.
pub const TILE_WIDTH: u32 = 160;
/// Fewest and most tiles in a sheet.
pub const MIN_TILES: usize = 60;
pub const MAX_TILES: usize = 120;
/// Tiles per row.
pub const COLUMNS: u32 = 10;
/// JPEG quality of the sheet.
pub const QUALITY: u8 = 72;
/// Version of the sheet layout and frame choice; a different version is rebuilt.
pub const VERSION: u32 = 1;

/// The sheet's layout and the time of each tile (`<contentId>.json` beside the image).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkimSheet {
    pub version: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub columns: u32,
    pub rows: u32,
    /// Presentation time of each tile, in µs, in tile order (left to right, then top to bottom).
    pub times_us: Vec<Micros>,
}

/// A rendered sheet: its layout, RGBA pixels and the decode work it took.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub sheet: SkimSheet,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub frames_decoded: u64,
}

/// How many tiles a clip gets: two per second of video, between 60 and 120, and never more than it has frames.
pub fn tile_count(duration_us: Micros, frames: Option<u64>) -> usize {
    let by_time = (duration_us.max(0) as f64 / 500_000.0).round() as usize;
    let n = by_time.clamp(MIN_TILES, MAX_TILES);
    frames.map_or(n, |f| n.min(f.max(1) as usize))
}

/// The target time of each tile: the middle of each of `n` equal parts of the duration.
pub fn targets(duration_us: Micros, n: usize) -> Vec<Micros> {
    let (d, n) = (duration_us.max(0) as i128, n.max(1) as i128);
    (0..n).map(|i| ((2 * i + 1) * d / (2 * n)) as Micros).collect()
}

/// Decodes the tiles of a sheet. `keep_going(fraction)` is called before each tile; returning false stops and
/// gives `Ok(None)`.
pub fn render(
    path: &Path,
    duration_us: Micros,
    n: usize,
    mut keep_going: impl FnMut(f64) -> bool,
) -> Result<Option<Rendered>> {
    let opts = DecoderOptions {
        hwaccel: HwAccel::Off,
        output: OutputFormat::Rgba { max_width: TILE_WIDTH, max_height: 0 },
        ..Default::default()
    };
    let mut dec = FfmpegVideoDecoder::open(path, opts)?;
    dec.skip_non_reference(true);
    let targets = targets(duration_us, n);
    let half = if n > 0 { duration_us.max(0) / (2 * n as Micros) } else { 0 };
    let columns = COLUMNS.min(n.max(1) as u32);
    let rows = (n as u32).div_ceil(columns);
    let mut tiles: Vec<VideoFrame> = Vec::with_capacity(n);
    let mut last: Option<VideoFrame> = None;
    let mut ended = false;
    for (i, &t) in targets.iter().enumerate() {
        if !keep_going(i as f64 / n.max(1) as f64) {
            return Ok(None);
        }
        let want = (t - half).max(0);
        let last_pts = last.as_ref().map(|f| f.pts_us);
        // Jump when the keyframe before the wanted frame lies past what was decoded (or nothing was decoded).
        let jump = match last_pts {
            None => true,
            Some(p) => p < want && dec.keyframe_before(want).is_some_and(|k| k > p),
        };
        if jump && !(ended && last_pts.is_some()) {
            dec.seek_keyframe(want)?;
            last = None;
            ended = false;
        }
        while !ended && last.as_ref().is_none_or(|f| f.pts_us < want) {
            match dec.next_frame()? {
                Some(f) => last = Some(f),
                None => ended = true,
            }
        }
        // Past the last frame: the sheet repeats the last frame decoded.
        let Some(frame) = last.clone().or_else(|| tiles.last().cloned()) else {
            return Err(MediaError::Decode("the video has no frames".into()));
        };
        tiles.push(frame);
    }
    let (tw, th) = tiles.first().map(|f| (f.width, f.height)).unwrap_or((TILE_WIDTH, 90));
    let (w, h) = (tw * columns, th * rows);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for (i, f) in tiles.iter().enumerate() {
        let (cx, cy) = ((i as u32 % columns) * tw, (i as u32 / columns) * th);
        blit(f, &mut rgba, w, cx, cy, tw, th);
    }
    let sheet = SkimSheet {
        version: VERSION,
        tile_width: tw,
        tile_height: th,
        columns,
        rows,
        times_us: tiles.iter().map(|f| f.pts_us).collect(),
    };
    Ok(Some(Rendered { sheet, width: w, height: h, rgba, frames_decoded: dec.frames_decoded() }))
}

/// Copies an RGBA frame into the sheet at (`x`, `y`), clipped to one tile.
fn blit(f: &VideoFrame, sheet: &mut [u8], sheet_w: u32, x: u32, y: u32, tw: u32, th: u32) {
    if f.format != PixelFormat::Rgba8 {
        return;
    }
    let Some(p) = f.planes.first() else { return };
    let (cw, ch) = (f.width.min(tw) as usize, f.height.min(th) as usize);
    for row in 0..ch {
        let src = row * p.stride;
        let dst = (((y as usize + row) * sheet_w as usize) + x as usize) * 4;
        if let (Some(s), Some(d)) = (p.data.get(src..src + cw * 4), sheet.get_mut(dst..dst + cw * 4)) {
            d.copy_from_slice(s);
        }
    }
}

/// Encodes the rendered sheet as a JPEG.
pub fn encode_jpeg(r: &Rendered) -> Result<Vec<u8>> {
    let (w, h) = (
        u16::try_from(r.width).map_err(|_| MediaError::Decode("skim sheet too wide".into()))?,
        u16::try_from(r.height).map_err(|_| MediaError::Decode("skim sheet too tall".into()))?,
    );
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, QUALITY)
        .encode(&r.rgba, w, h, jpeg_encoder::ColorType::Rgba)
        .map_err(|e| MediaError::Decode(format!("could not encode the skim sheet: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_count_follows_duration_within_bounds() {
        assert_eq!(tile_count(5_000_000, None), MIN_TILES);
        assert_eq!(tile_count(45_000_000, None), 90);
        assert_eq!(tile_count(3_600_000_000, None), MAX_TILES);
        assert_eq!(tile_count(60_000_000, Some(12)), 12, "never more tiles than frames");
    }

    #[test]
    fn targets_are_the_middles_of_equal_parts() {
        assert_eq!(targets(60_000_000, 60)[..3], [500_000, 1_500_000, 2_500_000]);
        assert_eq!(*targets(60_000_000, 60).last().unwrap(), 59_500_000);
        assert_eq!(targets(1_000_000, 4), vec![125_000, 375_000, 625_000, 875_000]);
    }
}
