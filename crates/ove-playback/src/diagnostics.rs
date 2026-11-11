//! Playback diagnostics: counters and timing statistics for the overlay and the benchmark.

use serde::Serialize;

use crate::clock::{ClockSource, Micros};

/// Collected samples of one measurement (µs). Bounded so long sessions do not grow without limit.
#[derive(Debug, Clone, Default)]
pub struct Samples {
    values: Vec<Micros>,
    count: u64,
    sum: i128,
    max: Micros,
}

const MAX_KEPT: usize = 100_000;

impl Samples {
    pub fn push(&mut self, v: Micros) {
        self.count += 1;
        self.sum += v as i128;
        self.max = if self.count == 1 { v } else { self.max.max(v) };
        if self.values.len() < MAX_KEPT {
            self.values.push(v);
        } else {
            // Reservoir-style replacement keeps percentiles representative.
            let i = (self.count.wrapping_mul(2654435761) % MAX_KEPT as u64) as usize;
            self.values[i] = v;
        }
    }

    pub fn summary(&self) -> Stats {
        if self.count == 0 {
            return Stats::default();
        }
        let mut v = self.values.clone();
        v.sort_unstable();
        let pct = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        Stats {
            count: self.count,
            mean_us: (self.sum / self.count as i128) as Micros,
            p50_us: pct(0.50),
            p95_us: pct(0.95),
            max_us: self.max,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub count: u64,
    pub mean_us: Micros,
    pub p50_us: Micros,
    pub p95_us: Micros,
    pub max_us: Micros,
}

#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    pub presented: u64,
    /// Frames not shown at 1× because they were late by more than one frame interval or superseded.
    pub dropped: u64,
    /// Frames skipped on purpose at shuttle speeds other than 1×.
    pub skipped: u64,
    /// |clock − pts| when a frame is presented at 1× (how far presentation is from the master clock).
    pub av_offset: Samples,
    /// Seek request → first frame of the new position presented.
    pub seek_latency: Samples,
}

/// Serializable snapshot for the overlay / benchmark JSON.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSnapshot {
    pub presented_frames: u64,
    pub dropped_frames: u64,
    pub skipped_frames: u64,
    pub av_offset: Stats,
    pub seek_latency: Stats,
    pub clock: ClockSource,
}

impl Diagnostics {
    pub fn snapshot(&self, clock: ClockSource) -> DiagnosticsSnapshot {
        DiagnosticsSnapshot {
            presented_frames: self.presented,
            dropped_frames: self.dropped,
            skipped_frames: self.skipped,
            av_offset: self.av_offset.summary(),
            seek_latency: self.seek_latency.summary(),
            clock,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats() {
        let mut s = Samples::default();
        assert_eq!(s.summary(), Stats::default());
        for v in 1..=100 {
            s.push(v);
        }
        let st = s.summary();
        assert_eq!(
            (st.count, st.mean_us, st.p50_us, st.p95_us, st.max_us),
            (100, 50, 51, 95, 100)
        );
    }

    #[test]
    fn bounded_memory() {
        let mut s = Samples::default();
        for v in 0..(MAX_KEPT as i64 + 5000) {
            s.push(v % 10);
        }
        assert_eq!(s.values.len(), MAX_KEPT);
        assert_eq!(s.summary().max_us, 9);
    }
}
