//! Visual search over index files (design D4).
//!
//! The query embedding is compared with every sampled frame (streamed dot products over the mapped matrices). Each
//! shot scores by its best frame only. For a text query the confidence is the model's calibrated match probability
//! and results below [`MIN_CONFIDENCE`] are dropped; for "more like this" (a frame as the query) the confidence is the
//! image–image cosine and results below [`MIN_SIMILARITY`] are dropped. The suggested window is up to 2 seconds
//! either side of the best frame, inside the shot.

use serde::{Deserialize, Serialize};

use crate::index::Index;
use crate::model::confidence;

/// Text results below this confidence are omitted.
pub const MIN_CONFIDENCE: f32 = 0.1;
/// "More like this" results below this cosine are omitted (image–image cosines run high: unrelated synthetic pictures
/// reach 0.78–0.94; to be re-fitted on the labelled set).
pub const MIN_SIMILARITY: f32 = 0.8;
/// Half the suggested window, seconds.
pub const WINDOW_HALF: f64 = 2.0;
/// Below this many rows in total, ranking runs on one thread.
const PARALLEL_ROWS: usize = 20_000;

/// How scores turn into confidence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scoring {
    /// A text query: calibrated probability, omitted below `min`.
    Text { min: f32 },
    /// A frame as the query: cosine, omitted below `min`.
    Similar { min: f32 },
}

impl Scoring {
    pub const TEXT: Scoring = Scoring::Text { min: MIN_CONFIDENCE };
    pub const SIMILAR: Scoring = Scoring::Similar { min: MIN_SIMILARITY };

    fn confidence(self, cosine: f32) -> f32 {
        match self {
            Scoring::Text { .. } => confidence(cosine),
            Scoring::Similar { .. } => cosine.clamp(0.0, 1.0),
        }
    }

    fn min(self) -> f32 {
        match self {
            Scoring::Text { min } | Scoring::Similar { min } => min,
        }
    }
}

/// One media file's index, under its media id.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    pub media_id: &'a str,
    pub index: &'a Index,
}

/// A ranked shot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Moment {
    pub media_id: String,
    pub content_id: String,
    pub shot: usize,
    /// Shot range, source seconds.
    pub shot_start: f64,
    pub shot_end: f64,
    /// Time of the best frame.
    pub best: f64,
    /// Suggested in and out, source seconds.
    pub window_start: f64,
    pub window_end: f64,
    /// 0–1.
    pub confidence: f32,
    pub cosine: f32,
}

/// The suggested window around `best` inside the shot `(a, b)`.
pub fn window(best: f64, a: f64, b: f64) -> (f64, f64) {
    ((best - WINDOW_HALF).max(a), (best + WINDOW_HALF).min(b))
}

fn rank_one(src: &Source, query: &[f32], scoring: Scoring) -> Vec<Moment> {
    let i = src.index;
    let scores = i.scores(query);
    let mut best: Vec<Option<(usize, f32)>> = vec![None; i.shots.len()];
    for (row, &s) in scores.iter().enumerate() {
        let shot = i.shot_of(row);
        if best[shot].is_none_or(|(_, b)| s > b) {
            best[shot] = Some((row, s));
        }
    }
    best.into_iter()
        .enumerate()
        .filter_map(|(shot, b)| {
            let (row, cosine) = b?;
            let conf = scoring.confidence(cosine);
            (conf >= scoring.min()).then(|| {
                let (a, e) = i.shots[shot];
                let t = i.time(row);
                let (ws, we) = window(t, a, e);
                Moment {
                    media_id: src.media_id.to_string(),
                    content_id: i.content_id.clone(),
                    shot,
                    shot_start: a,
                    shot_end: e,
                    best: t,
                    window_start: ws,
                    window_end: we,
                    confidence: conf,
                    cosine,
                }
            })
        })
        .collect()
}

/// Ranks the shots of `sources` for `query` (a unit-length embedding), best first, at most `limit`.
pub fn rank(sources: &[Source], query: &[f32], scoring: Scoring, limit: usize) -> Vec<Moment> {
    let rows: usize = sources.iter().map(|s| s.index.rows()).sum();
    let mut all: Vec<Moment> = if rows < PARALLEL_ROWS || sources.len() < 2 {
        sources.iter().flat_map(|s| rank_one(s, query, scoring)).collect()
    } else {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).clamp(1, 8);
        let per = sources.len().div_ceil(threads);
        std::thread::scope(|scope| {
            let handles: Vec<_> = sources
                .chunks(per)
                .map(|chunk| scope.spawn(move || chunk.iter().flat_map(|s| rank_one(s, query, scoring)).collect()))
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_else(|_| Vec::<Moment>::new())).collect()
        })
    };
    all.sort_by(|a, b| {
        b.cosine.total_cmp(&a.cosine).then_with(|| a.media_id.cmp(&b.media_id)).then_with(|| a.shot.cmp(&b.shot))
    });
    all.truncate(limit);
    all
}

/// The query for "more like this": the embedding of the sampled frame of `index` nearest to `time`, preferring
/// frames of the shot that contains `time`.
pub fn frame_query(index: &Index, time: f64) -> Option<Vec<f32>> {
    let shot = index.shots.iter().position(|&(a, b)| time >= a && (time < b || (a == b && time == a)));
    let row = (0..index.rows())
        .filter(|&r| shot.is_none_or(|s| index.shot_of(r) == s))
        .min_by(|&x, &y| (index.time(x) - time).abs().total_cmp(&(index.time(y) - time).abs()))
        .or_else(|| {
            (0..index.rows()).min_by(|&x, &y| (index.time(x) - time).abs().total_cmp(&(index.time(y) - time).abs()))
        })?;
    Some(index.embedding(row))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{DIM, IndexData, Provenance, index_path, normalise, write};

    fn unit(seed: usize) -> Vec<f32> {
        let mut v: Vec<f32> = (0..DIM).map(|i| (((i + 1) * (seed + 3) * 7919) % 1000) as f32 / 1000.0 - 0.5).collect();
        normalise(&mut v);
        v
    }

    /// Mixes `v` towards `toward` so their cosine is about `c`.
    fn blend(v: &[f32], toward: &[f32], c: f32) -> Vec<f32> {
        let mut out: Vec<f32> = v.iter().zip(toward).map(|(a, b)| c * b + (1.0 - c) * a).collect();
        normalise(&mut out);
        out
    }

    /// Three shots: 0–10 s (rows at 2, 5, 8), 10–12 s (row 11), 12–30 s (rows 15, 24).
    fn sample(dir: &std::path::Path, query: &[f32]) -> Index {
        let rows = [
            (2.0, 0, blend(&unit(1), query, 0.10)),
            (5.0, 0, blend(&unit(2), query, 0.60)),
            (8.0, 0, blend(&unit(3), query, 0.20)),
            (11.0, 1, unit(4)),
            (15.0, 2, blend(&unit(5), query, 0.40)),
            (24.0, 2, blend(&unit(6), query, 0.30)),
        ];
        let d = IndexData {
            provenance: Provenance { model: "t".into(), model_hash: "0".into(), sampler: 1, preprocessing: "t".into() },
            content_id: "c".into(),
            duration: 30.0,
            embeddings: rows.iter().flat_map(|r| r.2.clone()).collect(),
            times: rows.iter().map(|r| r.0).collect(),
            row_shot: rows.iter().map(|r| r.1).collect(),
            shots: vec![(0.0, 10.0), (10.0, 12.0), (12.0, 30.0)],
            cuts: vec![10.0, 12.0],
            cut_scores: vec![0.9, 0.9],
            frames: 750,
        };
        let p = index_path(dir, "c");
        write(&p, &d).unwrap();
        Index::open(&p).unwrap()
    }

    #[test]
    fn each_shot_counts_once_by_its_best_frame() {
        let dir = tempfile::tempdir().unwrap();
        let q = unit(99);
        let i = sample(dir.path(), &q);
        let r = rank(&[Source { media_id: "m", index: &i }], &q, Scoring::Text { min: 0.0 }, 10);
        assert_eq!(r.len(), 3);
        assert_eq!((r[0].shot, r[0].best), (0, 5.0));
        assert_eq!((r[1].shot, r[1].best), (2, 15.0));
        assert!(r[0].cosine > r[1].cosine && r[1].cosine > r[2].cosine);
        assert!(r.iter().all(|m| (0.0..=1.0).contains(&m.confidence)));
    }

    #[test]
    fn windows_are_four_seconds_at_most_and_inside_the_shot() {
        assert_eq!(window(5.0, 0.0, 10.0), (3.0, 7.0));
        assert_eq!(window(1.0, 0.0, 10.0), (0.0, 3.0));
        assert_eq!(window(11.0, 10.0, 12.0), (10.0, 12.0));
        assert_eq!(window(0.0, 0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn results_below_the_threshold_are_omitted() {
        let dir = tempfile::tempdir().unwrap();
        let q = unit(99);
        let i = sample(dir.path(), &q);
        let all = rank(&[Source { media_id: "m", index: &i }], &q, Scoring::Text { min: 0.0 }, 10);
        let cut = all[1].confidence;
        let kept = rank(&[Source { media_id: "m", index: &i }], &q, Scoring::Text { min: cut }, 10);
        assert_eq!(kept.len(), 2);
        // The opposite of the query matches nothing.
        let away: Vec<f32> = q.iter().map(|x| -x).collect();
        assert!(rank(&[Source { media_id: "m", index: &i }], &away, Scoring::TEXT, 10).is_empty());
    }

    #[test]
    fn more_like_this_puts_the_source_shot_first() {
        let dir = tempfile::tempdir().unwrap();
        let q = unit(99);
        let i = sample(dir.path(), &q);
        let frame = frame_query(&i, 15.5).unwrap();
        let r = rank(&[Source { media_id: "m", index: &i }], &frame, Scoring::Similar { min: 0.0 }, 10);
        assert_eq!((r[0].shot, r[0].best), (2, 15.0));
        assert!(r[0].confidence > 0.99);
        // A time inside a shot picks that shot's frame even when another shot's frame is nearer.
        let f = frame_query(&i, 10.1).unwrap();
        assert!(crate::index::dot(&f, &i.embedding(3)) > 0.999);
    }

    #[test]
    fn many_sources_rank_the_same_in_parallel() {
        let dir = tempfile::tempdir().unwrap();
        let q = unit(99);
        let i = sample(dir.path(), &q);
        let sources: Vec<Source> = (0..4000).map(|_| Source { media_id: "m", index: &i }).collect();
        let r = rank(&sources, &q, Scoring::Text { min: 0.0 }, 5);
        assert_eq!(r.len(), 5);
        assert!(r.iter().all(|m| m.shot == 0 && m.best == 5.0));
    }
}
