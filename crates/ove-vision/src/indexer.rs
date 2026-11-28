//! Building a media file's index: sample, embed, write (tasks 2.1–2.3).
//!
//! [`ensure_index`] reuses a current index (one built by the same model and sampler, found by content id, so a
//! moved or relinked file keeps it), and otherwise samples the file, runs the image encoder on the chosen frames and
//! writes the index file. A damaged index file is discarded and rebuilt. [`cached_scenes`] answers scene detection
//! at its default threshold from an index's cut list without decoding the file again.

use std::path::Path;

use ove_analyze::video::{SceneCut, Scenes};

use crate::error::{Result, VisionError};
use crate::index::{IndexData, Lookup, index_path, lookup, write};
use crate::model::Encoder;
use crate::preprocess::pixel_values;
use crate::sampler::{CUT_THRESHOLD, decode_at, plan, survey};

/// What kind of media is indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    /// A still image: one row, one shot of length 0.
    Image,
}

/// What [`ensure_index`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    /// The existing index was current and was kept.
    pub reused: bool,
    /// A damaged index file was found, discarded and rebuilt.
    pub replaced_damaged: bool,
    pub rows: usize,
    pub shots: usize,
    /// Frames decoded by the sampler.
    pub frames: u64,
}

/// Share of the work done after the first pass (the rest is decoding samples and running the encoder).
const SURVEY_SHARE: f64 = 0.3;

/// Makes sure `source` (content id `content_id`) has a current index in `bundle`. `progress` gets the fraction done
/// and returns false to stop (the result is then [`VisionError::Cancelled`] and nothing is written).
pub fn ensure_index(
    bundle: &Path,
    source: &Path,
    content_id: &str,
    kind: MediaKind,
    encoder: &mut dyn Encoder,
    progress: &mut dyn FnMut(f64) -> bool,
) -> Result<Built> {
    let provenance = encoder.provenance();
    let mut replaced_damaged = false;
    match lookup(bundle, content_id, Some(&provenance)) {
        Lookup::Ready(i) => {
            return Ok(Built {
                reused: true,
                replaced_damaged: false,
                rows: i.rows(),
                shots: i.shots.len(),
                frames: i.frames,
            });
        }
        Lookup::Discarded(_) => replaced_damaged = true,
        Lookup::Missing | Lookup::Stale(_) => {}
    }
    let stop = || VisionError::Cancelled;
    let (data, frames) = match kind {
        MediaKind::Image => {
            let img = decode_still(source)?;
            if !progress(0.5) {
                return Err(stop());
            }
            let emb = encoder.images(&[pixel_values(&img.rgba, img.width as usize, img.height as usize)])?;
            let data = IndexData {
                provenance,
                content_id: content_id.into(),
                duration: 0.0,
                embeddings: emb.into_iter().flatten().collect(),
                times: vec![0.0],
                row_shot: vec![0],
                shots: vec![(0.0, 0.0)],
                cuts: Vec::new(),
                cut_scores: Vec::new(),
                frames: 0,
            };
            (data, 0)
        }
        MediaKind::Video => {
            let s = survey(source)?;
            if !progress(SURVEY_SHARE) {
                return Err(stop());
            }
            let p = plan(&s);
            let times: Vec<f64> = p.picks.iter().map(|k| k.time).collect();
            let mut embeddings = Vec::with_capacity(times.len() * crate::index::DIM);
            // Decode and embed in batches so progress and cancelling stay responsive.
            const BATCH: usize = 16;
            for (b, chunk) in times.chunks(BATCH).enumerate() {
                let images = decode_at(source, chunk)?;
                let pixels: Vec<Vec<f32>> =
                    images.iter().map(|i| pixel_values(&i.rgba, i.width as usize, i.height as usize)).collect();
                for e in encoder.images(&pixels)? {
                    embeddings.extend(e);
                }
                let done = ((b + 1) * BATCH).min(times.len()) as f64 / times.len().max(1) as f64;
                if !progress(SURVEY_SHARE + (1.0 - SURVEY_SHARE) * done) {
                    return Err(stop());
                }
            }
            let data = IndexData {
                provenance,
                content_id: content_id.into(),
                duration: p.duration,
                embeddings,
                times,
                row_shot: p.picks.iter().map(|k| k.shot).collect(),
                shots: p.shots.clone(),
                cuts: p.cuts.iter().map(|c| c.time).collect(),
                cut_scores: p.cuts.iter().map(|c| c.score).collect(),
                frames: p.frames,
            };
            (data, p.frames)
        }
    };
    write(&index_path(bundle, content_id), &data)?;
    Ok(Built { reused: false, replaced_damaged, rows: data.rows(), shots: data.shots.len(), frames })
}

/// A still image: through the media decoder, or for PNG files the decoder cannot read, through the PNG reader.
fn decode_still(source: &Path) -> Result<ove_analyze::Image> {
    match decode_at(source, &[0.0]) {
        Ok(mut v) if !v.is_empty() => Ok(v.remove(0)),
        first => {
            let png = source.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("png"));
            match (png, first) {
                (true, _) => read_png(source),
                (false, Err(e)) => Err(e),
                (false, Ok(_)) => Err(VisionError::Media("no picture".into())),
            }
        }
    }
}

fn read_png(path: &Path) -> Result<ove_analyze::Image> {
    let bad = |e: &dyn std::fmt::Display| VisionError::Media(format!("{}: {e}", path.display()));
    let mut dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().map_err(|e| bad(&e))?;
    let mut buf = vec![0; r.output_buffer_size().ok_or_else(|| bad(&"the picture is too large"))?];
    let info = r.next_frame(&mut buf).map_err(|e| bad(&e))?;
    buf.truncate(info.buffer_size());
    let n = (info.width * info.height) as usize;
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(bad(&"an indexed picture was not expanded")),
    };
    if rgba.len() != n * 4 {
        return Err(bad(&"the picture data is incomplete"));
    }
    Ok(ove_analyze::Image { width: info.width, height: info.height, rgba })
}

/// Scene detection from a current index's cut list: only for the default threshold, and only for an index file that
/// is intact.
pub fn cached_scenes(bundle: &Path, content_id: &str, threshold: f64) -> Option<Scenes> {
    if (threshold - CUT_THRESHOLD).abs() > 1e-9 {
        return None;
    }
    let i = crate::index::Index::open(&index_path(bundle, content_id)).ok()?;
    if i.provenance.sampler != crate::SAMPLER_VERSION || i.frames == 0 {
        return None;
    }
    Some(Scenes {
        cuts: i.cuts.iter().zip(&i.cut_scores).map(|(&time, &score)| SceneCut { time, score }).collect(),
        duration: (i.duration * 1000.0).round() / 1000.0,
        frames: i.frames,
    })
}
