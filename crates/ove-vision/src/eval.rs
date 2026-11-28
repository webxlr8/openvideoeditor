//! Retrieval quality on a labelled query set (task 3.1): recall@k and mean reciprocal rank.
//!
//! Each query lists the shots that answer it. Recall@k is the share of a query's relevant shots found in its first k
//! results, averaged over queries; MRR averages 1 / (rank of the first relevant shot), 0 when none is returned.

use serde::{Deserialize, Serialize};

/// A shot: media id and shot number.
pub type ShotRef = (String, usize);

/// One labelled query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Judged {
    pub query: String,
    pub relevant: Vec<ShotRef>,
}

/// The quality numbers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    pub queries: usize,
    pub recall_at_5: f64,
    pub recall_at_10: f64,
    pub mrr: f64,
}

fn recall(ranked: &[ShotRef], relevant: &[ShotRef], k: usize) -> f64 {
    if relevant.is_empty() {
        return 0.0;
    }
    let top = &ranked[..k.min(ranked.len())];
    relevant.iter().filter(|r| top.contains(r)).count() as f64 / relevant.len() as f64
}

/// Metrics for ranked results, one list per judged query, in the same order.
pub fn metrics(judged: &[Judged], ranked: &[Vec<ShotRef>]) -> Metrics {
    let n = judged.len().min(ranked.len());
    let mut m = Metrics { queries: n, recall_at_5: 0.0, recall_at_10: 0.0, mrr: 0.0 };
    for (j, r) in judged.iter().zip(ranked) {
        m.recall_at_5 += recall(r, &j.relevant, 5);
        m.recall_at_10 += recall(r, &j.relevant, 10);
        m.mrr += r.iter().position(|x| j.relevant.contains(x)).map_or(0.0, |p| 1.0 / (p + 1) as f64);
    }
    if n > 0 {
        m.recall_at_5 /= n as f64;
        m.recall_at_10 /= n as f64;
        m.mrr /= n as f64;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(m: &str, i: usize) -> ShotRef {
        (m.to_string(), i)
    }

    #[test]
    fn recall_and_mrr_by_hand() {
        let judged = vec![
            Judged { query: "a".into(), relevant: vec![s("x", 0)] },
            Judged { query: "b".into(), relevant: vec![s("x", 1), s("y", 0)] },
            Judged { query: "c".into(), relevant: vec![s("z", 9)] },
        ];
        let mut long: Vec<ShotRef> = (0..12).map(|i| s("w", i)).collect();
        long[6] = s("y", 0);
        let ranked = vec![
            vec![s("x", 0), s("x", 1)], // first: rank 1
            long,                       // one of two relevant at rank 7
            vec![s("q", 0)],            // none
        ];
        let m = metrics(&judged, &ranked);
        assert_eq!(m.queries, 3);
        assert!((m.recall_at_5 - (1.0 + 0.0 + 0.0) / 3.0).abs() < 1e-12);
        assert!((m.recall_at_10 - (1.0 + 0.5 + 0.0) / 3.0).abs() < 1e-12);
        assert!((m.mrr - (1.0 + 1.0 / 7.0 + 0.0) / 3.0).abs() < 1e-12);
    }
}
