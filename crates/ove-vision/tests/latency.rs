//! Visual query latency at 10,000 and 100,000 sampled frames (add-semantic-media-search task 3.3): the target is
//! 50 ms or less at 100,000 rows. The workspace builds ove-vision and the f16 conversion optimised in test builds too
//! (Cargo.toml profiles), so the number is the shipped one.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code: a failure here is a failed test")]

use std::time::Instant;

use ove_vision::index::{DIM, Index, IndexData, Provenance, index_path, normalise, write};
use ove_vision::rank::{Scoring, Source, rank};

/// A deterministic pseudo-random unit vector.
fn unit(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut v: Vec<f32> = (0..DIM)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 2000) as f32 / 1000.0 - 1.0
        })
        .collect();
    normalise(&mut v);
    v
}

/// `files` index files of `rows` rows each (shots of 4 rows).
fn build(dir: &std::path::Path, files: usize, rows: usize) -> Vec<Index> {
    (0..files)
        .map(|f| {
            let cid = format!("{f:032x}");
            let shots = rows.div_ceil(4);
            let d = IndexData {
                provenance: Provenance {
                    model: "bench".into(),
                    model_hash: "0".into(),
                    sampler: 1,
                    preprocessing: "bench".into(),
                },
                content_id: cid.clone(),
                duration: rows as f64 * 4.0,
                embeddings: (0..rows).flat_map(|r| unit((f * rows + r) as u64)).collect(),
                times: (0..rows).map(|r| r as f64 * 4.0).collect(),
                row_shot: (0..rows).map(|r| (r / 4) as u32).collect(),
                shots: (0..shots).map(|s| (s as f64 * 16.0, s as f64 * 16.0 + 16.0)).collect(),
                cuts: Vec::new(),
                cut_scores: Vec::new(),
                frames: 0,
            };
            let p = index_path(dir, &cid);
            write(&p, &d).unwrap();
            Index::open(&p).unwrap()
        })
        .collect()
}

fn median_ms(mut f: impl FnMut()) -> f64 {
    let mut t: Vec<f64> = (0..7)
        .map(|_| {
            let t0 = Instant::now();
            f();
            t0.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    t.sort_by(f64::total_cmp);
    t[3]
}

#[test]
fn query_latency_at_10k_and_100k_rows() {
    let dir = tempfile::tempdir().unwrap();
    for (files, rows) in [(10, 1000), (100, 1000)] {
        let t0 = Instant::now();
        let indexes = build(dir.path(), files, rows);
        let built = t0.elapsed();
        // Opening (map and checksum) is paid once per search session, not per query.
        let t0 = Instant::now();
        let reopened: Vec<Index> =
            indexes.iter().map(|i| Index::open(&index_path(dir.path(), &i.content_id)).unwrap()).collect();
        let open_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let ids: Vec<String> = (0..files).map(|f| format!("m{f}")).collect();
        let sources: Vec<Source> =
            reopened.iter().zip(&ids).map(|(index, id)| Source { media_id: id, index }).collect();
        let q = unit(u64::MAX / 3);
        let mut n = 0;
        let ms = median_ms(|| n = rank(&sources, &q, Scoring::Text { min: 0.0 }, 50).len());
        let total = files * rows;
        eprintln!(
            "{total} rows in {files} files: query {ms:.1} ms (median of 7), open {open_ms:.0} ms, written in {built:?}, {n} results"
        );
        assert_eq!(n, 50);
        if total == 100_000 {
            assert!(ms <= 50.0, "{ms} ms at 100,000 rows");
        }
    }
}
