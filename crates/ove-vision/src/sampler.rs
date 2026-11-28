//! Shot-aware frame sampling (design D2).
//!
//! One decode at 64 px wide gives, per frame, the cut score scene detection uses (so the cut list can answer scene
//! detection later), a sharpness value (variance of the Laplacian of the luma) and whether the frame is near-black
//! or flat. Shots run from cut to cut. Each shot gets one sample point, plus one more for every 8 seconds of its
//! length, evenly spread; at each point the sharpest usable frame within half a second (and inside the shot) is
//! chosen. A shot with no usable frame near its points uses its sharpest usable frame anywhere; a shot with none at
//! all (black or flat throughout) gets no sample. The chosen frames are then decoded at 256 px in one forward pass.

use std::path::Path;

use ove_analyze::Image;
use ove_analyze::video::{CutScorer, SCENE_WIDTH, SceneCut, frame_to_image, open_for_cuts, pick_cuts};
use ove_media::{DecoderOptions, HwAccel, OutputFormat, VideoDecoder};

use crate::error::{Result, VisionError};

/// Cut threshold (the scene-detection default).
pub const CUT_THRESHOLD: f64 = 0.3;
/// One more sample per this many seconds of shot.
pub const SAMPLE_EVERY: f64 = 8.0;
/// How far from a sample point the sharpest frame is looked for.
pub const PICK_WINDOW: f64 = 0.5;
/// Width of the decoded sample frames.
pub const SAMPLE_WIDTH: u32 = 256;
/// Mean luma (0–255) below which a frame counts as near-black.
const BLACK_LUMA: f64 = 16.0;
/// Luma standard deviation (0–255) below which a frame counts as flat.
const FLAT_DEVIATION: f64 = 4.0;
/// Decoding continues forward instead of seeking when the next frame is at most this far ahead.
const SEEK_GAP: f64 = 4.0;

const US: f64 = 1_000_000.0;

fn media_err(e: impl std::fmt::Display) -> VisionError {
    VisionError::Media(e.to_string())
}

/// What the first pass learns about one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameInfo {
    pub time: f64,
    pub sharpness: f64,
    /// Near-black or flat: never chosen.
    pub skip: bool,
}

/// The first pass over a video.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Survey {
    pub frames: Vec<FrameInfo>,
    /// Cut scores between consecutive frames: (time of the later frame, score).
    pub scores: Vec<(f64, f64)>,
    pub duration: f64,
}

/// A chosen frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pick {
    pub time: f64,
    pub shot: u32,
}

/// Shots and the frames chosen to represent them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    pub cuts: Vec<SceneCut>,
    pub shots: Vec<(f64, f64)>,
    pub picks: Vec<Pick>,
    pub frames: u64,
    pub duration: f64,
}

/// Sharpness and the skip flag of a small frame.
pub fn frame_quality(img: &Image) -> (f64, bool) {
    let (w, h) = (img.width as usize, img.height as usize);
    let luma: Vec<f64> = img
        .rgba
        .chunks_exact(4)
        .map(|p| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]))
        .collect();
    let n = luma.len().max(1) as f64;
    let mean = luma.iter().sum::<f64>() / n;
    let deviation = (luma.iter().map(|y| (y - mean) * (y - mean)).sum::<f64>() / n).sqrt();
    let mut lap = Vec::with_capacity(w.saturating_sub(2) * h.saturating_sub(2));
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let c = luma[y * w + x];
            lap.push(
                4.0 * c - luma[y * w + x - 1] - luma[y * w + x + 1] - luma[(y - 1) * w + x] - luma[(y + 1) * w + x],
            );
        }
    }
    let m = lap.len().max(1) as f64;
    let lm = lap.iter().sum::<f64>() / m;
    let sharpness = lap.iter().map(|v| (v - lm) * (v - lm)).sum::<f64>() / m;
    (sharpness, mean < BLACK_LUMA || deviation < FLAT_DEVIATION)
}

/// Decodes every frame of `path` at 64 px wide: cut scores, sharpness and skip flags.
pub fn survey(path: &Path) -> Result<Survey> {
    let mut dec = open_for_cuts(path).map_err(media_err)?;
    let mut scorer = CutScorer::new();
    let mut out = Survey::default();
    while let Some(f) = dec.next_frame().map_err(media_err)? {
        let t = f.pts_us as f64 / US;
        out.duration = out.duration.max(t + f.duration_us.max(0) as f64 / US);
        let Some(img) = frame_to_image(&f) else { continue };
        let img = if img.width > SCENE_WIDTH { img.resize_to_width(SCENE_WIDTH) } else { img };
        if let Some(s) = scorer.push(&img) {
            out.scores.push((t, s));
        }
        let (sharpness, skip) = frame_quality(&img);
        out.frames.push(FrameInfo { time: t, sharpness, skip });
    }
    Ok(out)
}

/// The number of sample points of a shot `length` seconds long.
pub fn points_for(length: f64) -> usize {
    1 + (length.max(0.0) / SAMPLE_EVERY).floor() as usize
}

/// Shots and picks from a survey.
pub fn plan(s: &Survey) -> Plan {
    let cuts = pick_cuts(&s.scores, CUT_THRESHOLD);
    let start = 0.0;
    let mut bounds: Vec<f64> = std::iter::once(start).chain(cuts.iter().map(|c| c.time)).collect();
    bounds.push(s.duration.max(start));
    let shots: Vec<(f64, f64)> = bounds.windows(2).map(|w| (w[0], w[1])).filter(|(a, b)| b > a).collect();
    let mut picks = Vec::new();
    for (i, &(a, b)) in shots.iter().enumerate() {
        // Frames of this shot (frames are in time order).
        let lo = s.frames.partition_point(|f| f.time < a - 1e-6);
        let hi = s.frames.partition_point(|f| f.time < b - 1e-6);
        let frames = &s.frames[lo..hi];
        let usable = || frames.iter().filter(|f| !f.skip);
        let sharpest = |it: &mut dyn Iterator<Item = &FrameInfo>| -> Option<f64> {
            it.max_by(|x, y| x.sharpness.total_cmp(&y.sharpness).then(y.time.total_cmp(&x.time))).map(|f| f.time)
        };
        let n = points_for(b - a);
        let mut chosen: Vec<f64> = Vec::new();
        for k in 0..n {
            let p = a + (k as f64 + 0.5) / n as f64 * (b - a);
            let mut near = usable().filter(|f| (f.time - p).abs() <= PICK_WINDOW + 1e-9);
            if let Some(t) = sharpest(&mut near)
                && !chosen.contains(&t)
            {
                chosen.push(t);
            }
        }
        if chosen.is_empty()
            && let Some(t) = sharpest(&mut usable())
        {
            chosen.push(t);
        }
        picks.extend(chosen.into_iter().map(|time| Pick { time, shot: i as u32 }));
    }
    Plan { cuts, shots, picks, frames: s.frames.len() as u64, duration: s.duration }
}

/// Decodes the frames at `times` (ascending) at 256 px wide, in one forward pass that seeks only across gaps.
pub fn decode_at(path: &Path, times: &[f64]) -> Result<Vec<Image>> {
    ove_media::init();
    let opts = DecoderOptions {
        hwaccel: HwAccel::Off,
        output: OutputFormat::Rgba { max_width: SAMPLE_WIDTH, max_height: SAMPLE_WIDTH },
        inject_hw_failure_after: None,
    };
    let mut dec = ove_media::open_video(path, opts).map_err(media_err)?;
    let mut out = Vec::with_capacity(times.len());
    let mut pending: Option<ove_media::VideoFrame> = None;
    let mut position = f64::NEG_INFINITY;
    for &t in times {
        let target = (t * US).round() as i64;
        if t < position || t - position > SEEK_GAP {
            dec.seek(target).map_err(media_err)?;
            pending = None;
        }
        // The frame on screen at `t`: the last frame that starts at or before it.
        let mut shown: Option<ove_media::VideoFrame> = None;
        loop {
            let f = match pending.take() {
                Some(f) => f,
                None => match dec.next_frame().map_err(media_err)? {
                    Some(f) => f,
                    None => break,
                },
            };
            position = f.pts_us as f64 / US;
            if f.pts_us <= target {
                shown = Some(f);
            } else {
                pending = Some(f);
                break;
            }
        }
        let f = shown.or_else(|| pending.clone()).ok_or_else(|| media_err(format!("no frame at {t:.3} s")))?;
        out.push(frame_to_image(&f).ok_or_else(|| media_err(format!("frame at {t:.3} s has no pixels")))?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn survey_of(frames: &[(f64, f64, bool)], cuts: &[f64], duration: f64) -> Survey {
        let frames: Vec<FrameInfo> =
            frames.iter().map(|&(time, sharpness, skip)| FrameInfo { time, sharpness, skip }).collect();
        let scores = frames.iter().skip(1).map(|f| (f.time, if cuts.contains(&f.time) { 0.9 } else { 0.01 })).collect();
        Survey { frames, scores, duration }
    }

    fn steady(from: f64, to: f64, fps: f64) -> Vec<(f64, f64, bool)> {
        let n = ((to - from) * fps).round() as usize;
        (0..n).map(|i| (from + i as f64 / fps, 1.0 + (i % 7) as f64, false)).collect()
    }

    #[test]
    fn points_follow_the_per_shot_rule() {
        assert_eq!(points_for(0.5), 1);
        assert_eq!(points_for(7.99), 1);
        assert_eq!(points_for(8.0), 2);
        assert_eq!(points_for(20.0), 3);
    }

    #[test]
    fn shots_run_between_cuts_and_get_their_samples() {
        let mut f = steady(0.0, 3.0, 10.0);
        f.extend(steady(3.0, 20.0, 10.0));
        let s = survey_of(&f, &[3.0], 20.0);
        let p = plan(&s);
        assert_eq!(p.shots, [(0.0, 3.0), (3.0, 20.0)]);
        let per_shot = |i| p.picks.iter().filter(|k| k.shot == i).count();
        assert_eq!((per_shot(0), per_shot(1)), (1, 3));
        for k in &p.picks {
            let (a, b) = p.shots[k.shot as usize];
            assert!(k.time >= a && k.time < b);
        }
    }

    #[test]
    fn the_sharpest_frame_near_each_point_is_chosen() {
        let mut f = steady(0.0, 4.0, 10.0);
        // The midpoint is 2.0; the sharpest frame within 0.5 s is 2.3, a sharper one at 3.0 is too far.
        for x in &mut f {
            x.1 = 1.0;
        }
        f[23].1 = 50.0;
        f[30].1 = 99.0;
        let p = plan(&survey_of(&f, &[], 4.0));
        assert_eq!(p.picks.len(), 1);
        assert!((p.picks[0].time - 2.3).abs() < 1e-9);
    }

    #[test]
    fn black_and_flat_frames_are_skipped() {
        let mut f = steady(0.0, 4.0, 10.0);
        for x in &mut f[10..30] {
            x.2 = true;
        }
        let p = plan(&survey_of(&f, &[], 4.0));
        // Nothing usable near the midpoint: the shot falls back to its sharpest usable frame.
        assert_eq!(p.picks.len(), 1);
        assert!(!(1.0..3.0).contains(&p.picks[0].time));
        // A shot that is black throughout gets no sample.
        let mut f = steady(0.0, 2.0, 10.0);
        for x in &mut f {
            x.2 = true;
        }
        assert!(plan(&survey_of(&f, &[], 2.0)).picks.is_empty());
    }

    #[test]
    fn quality_tells_black_flat_and_detailed_frames_apart() {
        let black = Image::filled(64, 36, [3, 3, 3, 255]);
        let flat = Image::filled(64, 36, [120, 60, 200, 255]);
        let mut detail = Image::filled(64, 36, [0, 0, 0, 255]);
        for (i, p) in detail.rgba.chunks_exact_mut(4).enumerate() {
            let v = if (i % 64 / 4 + i / 64 / 4) % 2 == 0 { 230 } else { 20 };
            p.copy_from_slice(&[v, v, v, 255]);
        }
        assert!(frame_quality(&black).1);
        assert!(frame_quality(&flat).1);
        let (sharp, skip) = frame_quality(&detail);
        assert!(!skip && sharp > 1000.0, "{sharp}");
        let soft = detail.resize(16, 9).resize(64, 36);
        assert!(frame_quality(&soft).0 < sharp);
    }
}
