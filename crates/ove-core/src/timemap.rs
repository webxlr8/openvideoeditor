//! Time maps (add-edit-planners-and-links D6): how a ripple edit moved time on each track, recorded on the ripple's
//! revision so a projection printed before the ripple can be rebased onto the document after it.
//!
//! A map holds, per track, steps applied in order. A step `{at, delta}` moves every time `t >= at` by `delta`. A
//! negative delta removes the span `[at + delta, at)`: times inside it collapse to `at + delta`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ops::Op;
use crate::revision::RevisionRecord;
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
    /// Maps the end edge of an item through this step. Unlike a start, an end exactly at the point where time is
    /// opened belongs to the item before the opening and stays.
    pub fn map_end(&self, t: Micros) -> Micros {
        if self.delta > 0 && t == self.at { t } else { self.map(t) }
    }

    /// Maps one time (or the start edge of an item) through this step.
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

    /// The map that undoes this one (for the undo of a ripple): steps reversed, each opening what the original
    /// removed and removing what it opened.
    pub fn inverse(&self) -> TimeMap {
        let tracks = self
            .tracks
            .iter()
            .map(|(t, steps)| {
                (t.clone(), steps.iter().rev().map(|s| Step { at: s.at + s.delta, delta: -s.delta }).collect())
            })
            .collect();
        TimeMap { tracks }
    }

    /// Maps a time (or an item start) on a track. Tracks without steps keep their times.
    pub fn map(&self, track: &str, t: Micros) -> Micros {
        self.tracks.get(track).map_or(t, |steps| steps.iter().fold(t, |t, s| s.map(t)))
    }

    /// Maps an item end on a track (see [`Step::map_end`]).
    pub fn map_end(&self, track: &str, t: Micros) -> Micros {
        self.tracks.get(track).map_or(t, |steps| steps.iter().fold(t, |t, s| s.map_end(t)))
    }
}

/// A committed ripple: its revision, its time map and the items its batch touched (used to rebase projections
/// printed before it and to report conflicts with it as one group).
#[derive(Debug, Clone, PartialEq)]
pub struct Ripple {
    pub rev: i64,
    pub map: TimeMap,
    pub touched: BTreeSet<String>,
}

impl Ripple {
    /// The ripple a revision record holds, if it has a time map.
    pub fn from_record(r: &RevisionRecord) -> Option<Ripple> {
        let map = r.time_map.clone().filter(|m| !m.is_empty())?;
        let mut touched = BTreeSet::new();
        for op in &r.ops {
            match op {
                Op::AddItem { item } => {
                    if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                        touched.insert(id.to_string());
                    }
                }
                Op::UpdateItem { id, .. } | Op::UnsetItemFields { id, .. } => {
                    touched.insert(id.clone());
                }
                Op::RemoveItems { ids } => touched.extend(ids.iter().cloned()),
                _ => {}
            }
        }
        Some(Ripple { rev: r.rev, map, touched })
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
    fn the_inverse_undoes_a_ripple() {
        let mut m = TimeMap::default();
        m.push("v", Step { at: 6_000_000, delta: -2_000_000 });
        m.push("v", Step { at: 1_000_000, delta: 500_000 });
        let inv = m.inverse();
        for t in [0, 900_000, 3_000_000, 6_000_000, 9_000_000] {
            assert_eq!(inv.map("v", m.map("v", t)), t);
        }
    }

    #[test]
    fn insertion_shifts_later_times() {
        let mut m = TimeMap::default();
        m.push("v", Step { at: 2_000_000, delta: 1_000_000 });
        m.push("v", Step { at: 0, delta: 0 });
        assert_eq!(m.tracks["v"].len(), 1);
        assert_eq!(m.map("v", 1_999_999), 1_999_999);
        assert_eq!(m.map("v", 2_000_000), 3_000_000);
        assert_eq!(m.map_end("v", 2_000_000), 2_000_000, "an end at the opening stays");
        assert_eq!(m.map_end("v", 2_000_001), 3_000_001);
    }
}
