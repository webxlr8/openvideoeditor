//! Time maps (add-edit-planners-and-links D6): how a ripple edit moved time on each track, recorded on the ripple's
//! revision so a projection printed before the ripple can be rebased onto the document after it.
//!
//! A map holds, per track, steps applied in order. A step `{at, delta}` moves every time `t >= at` by `delta`. A
//! negative delta removes the span `[at + delta, at)`: times inside it collapse to `at + delta`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::time::Micros;

/// One step of a time map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    /// Times at or after this point move, in microseconds.
    pub at: Micros,
    /// How far they move, in microseconds (negative: earlier).
    pub delta: Micros,
}

impl Step {
    /// Maps one time through this step.
    pub fn map(&self, t: Micros) -> Micros {
        if t >= self.at {
            t + self.delta
        } else if self.delta < 0 && t > self.at + self.delta {
            self.at + self.delta
        } else {
            t
        }
    }
}

/// Per-track steps of one ripple edit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeMap {
    pub tracks: BTreeMap<String, Vec<Step>>,
}

impl TimeMap {
    pub fn is_empty(&self) -> bool {
        self.tracks.values().all(Vec::is_empty)
    }

    /// Adds a step for a track (after the steps already recorded for it).
    pub fn push(&mut self, track: &str, step: Step) {
        if step.delta != 0 {
            self.tracks.entry(track.to_string()).or_default().push(step);
        }
    }

    /// Maps a time on a track. Tracks without steps keep their times.
    pub fn map(&self, track: &str, t: Micros) -> Micros {
        self.tracks.get(track).map_or(t, |steps| steps.iter().fold(t, |t, s| s.map(t)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_collapses_and_shifts() {
        let mut m = TimeMap::default();
        // Remove [4 s, 6 s).
        m.push("v", Step { at: 6_000_000, delta: -2_000_000 });
        assert_eq!(m.map("v", 3_000_000), 3_000_000);
        assert_eq!(m.map("v", 5_000_000), 4_000_000);
        assert_eq!(m.map("v", 6_000_000), 4_000_000);
        assert_eq!(m.map("v", 9_000_000), 7_000_000);
        assert_eq!(m.map("a", 9_000_000), 9_000_000);
    }

    #[test]
    fn insertion_shifts_later_times() {
        let mut m = TimeMap::default();
        m.push("v", Step { at: 2_000_000, delta: 1_000_000 });
        m.push("v", Step { at: 0, delta: 0 });
        assert_eq!(m.tracks["v"].len(), 1);
        assert_eq!(m.map("v", 1_999_999), 1_999_999);
        assert_eq!(m.map("v", 2_000_000), 3_000_000);
    }
}
