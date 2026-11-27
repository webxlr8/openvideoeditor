//! Snapping (task 2.4): times a drag snaps to (the playhead, item edges, range points, optional beat times, 0), a
//! sticky threshold so a snapped edge does not flicker between nearby targets, and the frame grid.

use ove_core::model::Document;
use ove_core::time::{Micros, snap_to_frame};

/// What a snap target is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Zero,
    Playhead,
    Edge,
    Range,
    Beat,
    /// A bar's first beat: preferred over other beats in reach.
    Downbeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target {
    pub at: Micros,
    pub kind: Kind,
}

/// What a drag may snap to.
#[derive(Debug, Clone, Default)]
pub struct Sources<'a> {
    pub playhead: Option<Micros>,
    /// In and out points of the range selection.
    pub range: Option<(Micros, Micros)>,
    /// Beat times, when beat detection has run.
    pub beats: &'a [Micros],
    /// Downbeat times (the first beat of each bar), also when beat detection has run.
    pub downbeats: &'a [Micros],
    /// Items whose edges are not targets (the items being dragged).
    pub exclude: &'a [String],
}

/// Snap targets, sorted by time.
pub fn targets(doc: &Document, src: &Sources) -> Vec<Target> {
    let mut out = vec![Target { at: 0, kind: Kind::Zero }];
    if let Some(p) = src.playhead {
        out.push(Target { at: p, kind: Kind::Playhead });
    }
    if let Some((a, b)) = src.range {
        out.push(Target { at: a, kind: Kind::Range });
        out.push(Target { at: b, kind: Kind::Range });
    }
    out.extend(src.beats.iter().map(|&at| Target { at, kind: Kind::Beat }));
    out.extend(src.downbeats.iter().map(|&at| Target { at, kind: Kind::Downbeat }));
    for it in doc.project.items.values().filter(|i| !src.exclude.contains(&i.id)) {
        out.push(Target { at: it.start, kind: Kind::Edge });
        out.push(Target { at: it.end, kind: Kind::Edge });
    }
    out.sort();
    out.dedup_by_key(|t| (t.at, t.kind));
    out
}

/// The result of a snap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapped {
    pub at: Micros,
    /// The target it snapped to, if any.
    pub target: Option<Target>,
}

/// Snaps `t` to the nearest target within `threshold`. A target snapped to before (`sticky`) holds while `t` stays
/// within 1.5 × `threshold` of it, so a dragged edge does not jump between two close targets. Without a target the
/// time snaps to the frame grid of `fps`.
pub fn snap(t: Micros, targets: &[Target], threshold: Micros, sticky: Option<Target>, fps: f64) -> Snapped {
    if let Some(s) = sticky
        && (s.at - t).abs() * 2 <= threshold * 3
    {
        return Snapped { at: s.at, target: Some(s) };
    }
    let reach = || targets.iter().filter(|g| (g.at - t).abs() <= threshold);
    let mut best = reach().min_by_key(|g| ((g.at - t).abs(), g.kind));
    // A beat in reach gives way to a downbeat in reach.
    if best.is_some_and(|g| g.kind == Kind::Beat)
        && let Some(d) = reach().filter(|g| g.kind == Kind::Downbeat).min_by_key(|g| (g.at - t).abs())
    {
        best = Some(d);
    }
    match best {
        Some(g) => Snapped { at: g.at, target: Some(*g) },
        None => Snapped { at: snap_to_frame(t.max(0), fps), target: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::ops::{Op, apply_batch};
    use serde_json::json;

    const MS: Micros = 1_000;

    fn doc() -> Document {
        let d = Document::new("p", "Snap", 1);
        apply_batch(
            &d,
            &[
                Op::AddTrack { track: json!({"id": "v", "name": "V", "order": 0, "type": "video"}) },
                Op::AddItem { item: json!({"id": "a", "trackId": "v", "type": "text", "startTime": 1, "endTime": 2}) },
                Op::AddItem { item: json!({"id": "b", "trackId": "v", "type": "text", "startTime": 3, "endTime": 5}) },
            ],
        )
        .expect("valid")
        .doc
    }

    // Ported from ui/src/snap.test.ts: snapping without a target lands exactly on the frame grid.
    #[test]
    fn lands_exactly_on_30_fps_frames() {
        let none: [Target; 0] = [];
        assert_eq!(snap(1_066_700, &none, 0, None, 30.0).at, 1_066_667);
        assert_eq!(snap(2_500_000, &none, 0, None, 30.0).at, 2_500_000);
        assert_eq!(snap(-1_000_000, &none, 0, None, 30.0).at, 0);
        for f in 0..300 {
            let t = ove_core::time::frame_time(f, 30.0);
            assert_eq!(ove_core::time::nearest_frame(snap(t, &none, 0, None, 30.0).at, 30.0), f);
        }
    }

    #[test]
    fn snaps_to_the_nearest_target_within_the_threshold() {
        let d = doc();
        let ex = vec!["a".to_string()];
        let tg = targets(&d, &Sources { playhead: Some(2_500_000), exclude: &ex, ..Default::default() });
        // a's edges are excluded; b's start (3 s) and the playhead (2.5 s) remain.
        assert!(!tg.iter().any(|t| t.at == 1_000_000));
        let s = snap(2_960_000, &tg, 50 * MS, None, 30.0);
        assert_eq!(s.at, 3_000_000);
        assert_eq!(s.target.map(|t| t.kind), Some(Kind::Edge));
        assert_eq!(snap(2_530_000, &tg, 50 * MS, None, 30.0).at, 2_500_000);
        // Too far from any target: the frame grid.
        assert_eq!(snap(2_700_000, &tg, 50 * MS, None, 30.0).at, 2_700_000);
    }

    #[test]
    fn range_points_and_beats_are_targets() {
        let d = doc();
        let beats = [4_200_000];
        let tg = targets(&d, &Sources { range: Some((6_000_000, 8_000_000)), beats: &beats, ..Default::default() });
        assert_eq!(snap(7_980_000, &tg, 50 * MS, None, 25.0).at, 8_000_000);
        assert_eq!(snap(4_230_000, &tg, 50 * MS, None, 25.0).target.map(|t| t.kind), Some(Kind::Beat));
    }

    #[test]
    fn a_downbeat_in_reach_wins_over_a_closer_beat() {
        let d = doc();
        let (beats, downs) = ([10_000_000, 10_500_000], [10_000_000]);
        let tg = targets(&d, &Sources { beats: &beats, downbeats: &downs, ..Default::default() });
        // 10.32 s: the beat at 10.5 s is closer, but the downbeat at 10 s is in reach.
        let s = snap(10_320_000, &tg, 400 * MS, None, 30.0);
        assert_eq!((s.at, s.target.map(|t| t.kind)), (10_000_000, Some(Kind::Downbeat)));
        // Out of the downbeat's reach: the beat.
        assert_eq!(snap(10_450_000, &tg, 100 * MS, None, 30.0).at, 10_500_000);
        // No beat sources: no beat targets.
        assert!(!targets(&d, &Sources::default()).iter().any(|t| matches!(t.kind, Kind::Beat | Kind::Downbeat)));
    }

    #[test]
    fn a_sticky_target_holds_within_one_and_a_half_thresholds() {
        let tg = [Target { at: 1_000_000, kind: Kind::Edge }, Target { at: 1_060_000, kind: Kind::Edge }];
        let first = snap(1_020_000, &tg, 40 * MS, None, 30.0);
        assert_eq!(first.at, 1_000_000);
        // Closer to the second target now, but still within 60 ms of the first: it holds.
        let held = snap(1_045_000, &tg, 40 * MS, first.target, 30.0);
        assert_eq!(held.at, 1_000_000);
        // Past 1.5 × threshold: it lets go and the nearest target wins.
        let moved = snap(1_065_000, &tg, 40 * MS, first.target, 30.0);
        assert_eq!(moved.at, 1_060_000);
    }
}
