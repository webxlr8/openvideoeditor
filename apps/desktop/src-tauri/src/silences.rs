//! Remove silences (add-captions 1.5): cut the quiet stretches out of a clip with sound and close the gaps, as one
//! revision.
//!
//! - The clip is cut at every stretch quieter than the threshold for at least the minimum length, keeping a little
//!   air at each edge so words are not clipped.
//! - Everything else follows the cuts without being chopped: an item after a cut moves left, an item across a cut
//!   gets shorter (a music bed keeps playing, ending earlier), and a title or caption entirely inside a removed
//!   stretch goes. Items on the clip's own track that overlap it are cut like the clip.

use std::path::Path;

use ove_core::ops::Op;
use ove_core::time::{Micros, micros_to_json};
use ove_core::{Document, Item};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;
use tauri::AppHandle;

use crate::error::{AppError, AppResult, ErrorCode};

/// Shortest item left after a cut.
const MIN_ITEM: Micros = 40_000;

/// The quiet stretches of `clip` as timeline ranges: source silences moved through its trim and speed, kept inside
/// the clip, each shrunk by `keep` at both ends; stretches shorter than `min` after that are dropped.
pub fn timeline_ranges(
    start: Micros,
    end: Micros,
    trim: Micros,
    speed: f64,
    silences: &[(f64, f64)],
    keep: Micros,
    min: Micros,
) -> Vec<(Micros, Micros)> {
    let to_tl = |src_s: f64| start + (((src_s * 1e6) as Micros - trim) as f64 / speed).round() as Micros;
    let mut out: Vec<(Micros, Micros)> = silences
        .iter()
        .map(|&(a, b)| (to_tl(a).max(start) + keep, to_tl(b).min(end) - keep))
        .filter(|(a, b)| b - a >= min)
        .collect();
    out.sort();
    out.dedup();
    out
}

#[derive(Clone)]
struct State {
    item: Item,
    start: Micros,
    end: Micros,
    trim: Micros,
    removed: bool,
    new: bool,
}

fn shift(t: Micros, a: Micros, b: Micros) -> Micros {
    if t <= a {
        t
    } else if t < b {
        a
    } else {
        t - (b - a)
    }
}

/// The ops that remove the timeline `ranges` (sorted, not overlapping) as described in the module docs. `cut_track`
/// is the track whose overlapping media items are cut; `new_id` names the parts a cut creates.
pub fn ripple_ops(
    doc: &Document,
    cut_track: &str,
    ranges: &[(Micros, Micros)],
    new_id: &mut dyn FnMut() -> String,
) -> Vec<Op> {
    let mut items: Vec<State> = doc
        .project
        .items
        .values()
        .map(|i| State {
            item: (**i).clone(),
            start: i.start,
            end: i.end,
            trim: i.time_field("trimStart"),
            removed: false,
            new: false,
        })
        .collect();
    for &(a, b) in ranges.iter().rev() {
        let d = b - a;
        let mut added = Vec::new();
        for s in items.iter_mut().filter(|s| !s.removed) {
            let speed = s.item.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01);
            let src = |t: Micros| (t as f64 * speed).round() as Micros;
            let media = s.item.track_id == cut_track
                && s.item.media_id().is_some()
                && matches!(s.item.kind.as_str(), "video" | "audio");
            if s.end <= a {
                continue;
            }
            if s.start >= b {
                s.start -= d;
                s.end -= d;
                continue;
            }
            if media {
                if s.start >= a && s.end <= b {
                    s.removed = true;
                } else if s.start < a && s.end > b {
                    // Split: the left part keeps the id; the right part starts at the cut with its in-point moved on.
                    let right = State {
                        item: Item { id: new_id(), ..s.item.clone() },
                        start: a,
                        end: s.end - d,
                        trim: s.trim + src(b - s.start),
                        removed: false,
                        new: true,
                    };
                    s.end = a;
                    added.push(right);
                } else if s.start < a {
                    s.end = a;
                } else {
                    s.trim += src(b - s.start);
                    s.start = a;
                    s.end -= d;
                }
            } else {
                let (ns, ne) = (shift(s.start, a, b), shift(s.end, a, b));
                let inside = s.start >= a && s.end <= b;
                if (inside && matches!(s.item.kind.as_str(), "text" | "caption")) || ne - ns < MIN_ITEM {
                    s.removed = true;
                } else {
                    s.start = ns;
                    s.end = ne;
                }
            }
        }
        items.extend(added);
    }

    let mut ops = Vec::new();
    let mut gone = Vec::new();
    for s in &items {
        if s.new {
            if !s.removed {
                let mut j = s.item.to_json();
                j["startTime"] = micros_to_json(s.start);
                j["endTime"] = micros_to_json(s.end);
                j["trimStart"] = micros_to_json(s.trim);
                ops.push(Op::AddItem { item: j });
            }
            continue;
        }
        if s.removed {
            gone.push(s.item.id.clone());
            continue;
        }
        let mut set = Map::new();
        if s.start != s.item.start {
            set.insert("startTime".into(), micros_to_json(s.start));
        }
        if s.end != s.item.end {
            set.insert("endTime".into(), micros_to_json(s.end));
        }
        if s.trim != s.item.time_field("trimStart") {
            set.insert("trimStart".into(), micros_to_json(s.trim));
        }
        if !set.is_empty() {
            ops.push(Op::UpdateItem { id: s.item.id.clone(), set });
        }
    }
    if !gone.is_empty() {
        ops.insert(0, Op::RemoveItems { ids: gone });
    }
    ops
}

/// What Remove silences did.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SilencesRemoved {
    pub view: crate::projects::ProjectView,
    /// Stretches removed.
    pub count: u32,
    /// Seconds removed from the timeline.
    pub seconds: f64,
}

/// Removes the quiet stretches of clip `id` (quieter than `threshold_db` dBFS for at least `min_seconds`, default
/// −40 dB and 0.6 s) and closes the gaps, as one revision.
#[tauri::command]
#[specta::specta]
pub async fn remove_silences(
    app: AppHandle,
    id: String,
    threshold_db: Option<f64>,
    min_seconds: Option<f64>,
) -> AppResult<SilencesRemoved> {
    let threshold = threshold_db.unwrap_or(-40.0).clamp(-90.0, -10.0);
    let min_s = min_seconds.unwrap_or(0.6).clamp(0.2, 10.0);
    crate::projects::serial(&app, "remove_silences", move |app, state| {
        let (doc, bundle) = {
            let g = crate::projects::lock(&state.projects.open)?;
            let p = g.as_ref().ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "No project is open."))?;
            (p.doc().clone(), p.bundle().to_path_buf())
        };
        let clip = crate::captions::source_clips(&doc, &bundle, std::slice::from_ref(&id))?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::new(ErrorCode::InvalidArgument, "That clip has no sound."))?;
        if clip.reverse {
            return Err(AppError::new(ErrorCode::InvalidArgument, "Remove silences works on clips that play forward."));
        }
        let found = ove_analyze::silence(Path::new(&clip.path), threshold, min_s)
            .map_err(|e| AppError::new(ErrorCode::Internal, format!("Could not listen to the clip: {e}")))?;
        let silences: Vec<(f64, f64)> = found.iter().map(|r| (r.start, r.end)).collect();
        // 0.15 s of air kept at each edge of a cut.
        let keep: Micros = 150_000;
        let ranges = timeline_ranges(
            clip.start,
            clip.end,
            clip.trim,
            clip.speed,
            &silences,
            keep,
            (min_s * 1e6) as Micros - 2 * keep,
        );
        if ranges.is_empty() {
            return Err(AppError::new(ErrorCode::InvalidArgument, "No pauses long enough to remove in this clip."));
        }
        let track = doc.item(&id).map(|i| i.track_id.clone()).unwrap_or_default();
        let mut ops = ripple_ops(&doc, &track, &ranges, &mut crate::edits::new_item_id);
        // Parts of one clip stay together for undo and review: one revision.
        let seconds = ranges.iter().map(|(a, b)| (b - a) as f64 / 1e6).sum::<f64>();
        let summary =
            format!("remove {} silence{} ({seconds:.1} s)", ranges.len(), if ranges.len() == 1 { "" } else { "s" });
        ops.retain(|o| !matches!(o, Op::RemoveItems { ids } if ids.is_empty()));
        let view = crate::projects::edit(app, state, ops, &summary)?;
        Ok(SilencesRemoved { view, count: ranges.len() as u32, seconds })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::revision::Source;
    use serde_json::json;
    use std::sync::Arc;

    const S: Micros = 1_000_000;

    #[test]
    fn silences_move_through_trim_and_speed_and_keep_some_air() {
        // A clip at 10 s on the timeline, playing its source from 2 s at 2× speed: source 6-8 s is timeline 12-13 s.
        let r = timeline_ranges(10 * S, 20 * S, 2 * S, 2.0, &[(6.0, 8.0), (30.0, 31.0), (2.0, 2.3)], 100_000, 300_000);
        assert_eq!(r, vec![(12 * S + 100_000, 13 * S - 100_000)], "outside the clip and too short are dropped");
    }

    fn doc() -> (tempfile::TempDir, ove_store::Project) {
        let d = tempfile::tempdir().unwrap();
        let lib = ove_store::Library::open(d.path(), Arc::new(ove_core::SystemClock)).unwrap();
        let mut p = lib.create("Silences").unwrap();
        let ops: Vec<Op> = serde_json::from_value(json!([
            {"op": "addTrack", "track": {"id": "t_text", "name": "Text", "order": 0, "type": "video"}},
            {"op": "addTrack", "track": {"id": "t_main", "name": "Main", "order": 1, "type": "video"}},
            {"op": "addTrack", "track": {"id": "t_music", "name": "Music", "order": 2, "type": "audio"}},
            {"op": "addMedia", "media": {"id": "m", "data": {"type": "video", "fileName": "talk.mp4", "duration": 60}}},
            {"op": "addMedia", "media": {"id": "s", "data": {"type": "audio", "fileName": "song.mp3", "duration": 60}}},
            {"op": "addItem", "item": {"id": "clip", "trackId": "t_main", "type": "video", "mediaItemId": "m", "startTime": 0, "endTime": 10, "trimStart": 1}},
            {"op": "addItem", "item": {"id": "after", "trackId": "t_main", "type": "video", "mediaItemId": "m", "startTime": 10, "endTime": 12}},
            {"op": "addItem", "item": {"id": "music", "trackId": "t_music", "type": "audio", "mediaItemId": "s", "startTime": 0, "endTime": 12}},
            {"op": "addItem", "item": {"id": "in_gap", "trackId": "t_text", "type": "text", "startTime": 3.2, "endTime": 3.8, "textContent": "hm"}},
            {"op": "addItem", "item": {"id": "later", "trackId": "t_text", "type": "text", "startTime": 8, "endTime": 9, "textContent": "end"}}
        ]))
        .unwrap();
        p.apply(ops, Source::Gui, None).unwrap();
        (d, p)
    }

    #[test]
    fn the_clip_is_cut_and_everything_else_follows_without_being_chopped() {
        let (_d, mut p) = doc();
        let mut n = 0;
        // Remove 3-4 s and 6-7 s of the timeline.
        let ops = ripple_ops(p.doc(), "t_main", &[(3 * S, 4 * S), (6 * S, 7 * S)], &mut || {
            n += 1;
            format!("part{n}")
        });
        p.apply(ops, Source::Gui, None).unwrap();
        let doc = p.doc();
        let span = |id: &str| doc.item(id).map(|i| (i.start, i.end, i.time_field("trimStart")));
        // The clip: 0-3 s, then its two later parts with their in-points moved on (trim 1 s at the start).
        assert_eq!(span("clip"), Some((0, 3 * S, S)));
        let mut parts: Vec<_> = ["part1", "part2"].iter().filter_map(|id| span(id)).collect();
        parts.sort();
        assert_eq!(parts, vec![(3 * S, 5 * S, 5 * S), (5 * S, 8 * S, 8 * S)]);
        // The next clip moves left by 2 s; the music keeps playing from the same point and ends 2 s earlier.
        assert_eq!(span("after"), Some((8 * S, 10 * S, 0)));
        assert_eq!(span("music"), Some((0, 10 * S, 0)));
        // A title entirely inside a removed stretch goes; a later one moves.
        assert!(doc.item("in_gap").is_none());
        assert_eq!(span("later").map(|s| (s.0, s.1)), Some((6 * S, 7 * S)));
    }
}
