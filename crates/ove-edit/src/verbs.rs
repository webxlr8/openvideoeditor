//! The planners, one function per verb.

use std::collections::BTreeSet;

use serde_json::Value;

use ove_core::model::{Document, Item};
use ove_core::time::{Micros, frame_time, json_to_micros, micros_to_json, nearest_frame};

use crate::retime::{self, Edges};
use crate::work::{TOUCH, Work, at, count};
use crate::{Edge, Plan, Range, Refusal, Request};

/// Plans a request; `ids` supplies new item ids (it is called again when an id is taken).
pub fn plan_with(doc: &Document, req: &Request, ids: &mut dyn FnMut() -> String) -> Result<Plan, Refusal> {
    let mut w = Work::new(doc, ids);
    match req {
        Request::Split { items, tracks, at, linked } => split(w, items, tracks, *at, *linked),
        Request::Trim { item, edge, to, linked, crossfade } => trim(w, item, *edge, *to, *linked, *crossfade),
        Request::RippleTrim { item, edge, to, linked } => ripple_trim(w, item, *edge, *to, *linked),
        Request::RippleDelete { items, tracks, ranges, linked } => {
            if items.is_empty() {
                ripple_delete_ranges(w, tracks, ranges, *linked)
            } else {
                ripple_delete_items(w, items, *linked)
            }
        }
        Request::Delete { items, linked } => delete(w, items, *linked),
        Request::Insert { track, at, items } => place(w, track, *at, items, true),
        Request::Overwrite { track, at, items } => place(w, track, *at, items, false),
        Request::Lift { tracks, range } => lift(w, tracks, *range),
        Request::Extract { tracks, range } => {
            let r = snap_range(&w, *range)?;
            let targets = target_tracks(&w, tracks)?;
            w.ripple_remove(&targets, r.start, r.end, true)?;
            let s = format!("extract {}–{}", at(r.start), at(r.end));
            w.finish(s, false)
        }
        Request::CloseGap { track, at: t } => close_gap(w, track, *t),
        Request::Slip { item, delta, linked } => slip(w, item, *delta, *linked),
        Request::Slide { item, delta, linked } => slide(w, item, *delta, *linked),
        Request::Roll { left, right, to, linked } => roll(w, left, right, *to, *linked),
        Request::Move { item, start, track, linked, crossfade } => {
            let crossfade = *crossfade;
            move_item(&mut w, item, *start, track.as_deref(), *linked)?;
            w.finish(format!("move {item}"), crossfade)
        }
    }
}

fn target_tracks(w: &Work, tracks: &[String]) -> Result<BTreeSet<String>, Refusal> {
    if tracks.is_empty() {
        return Ok(w.doc.project.tracks.iter().filter(|t| !w.locked(&t.id)).map(|t| t.id.clone()).collect());
    }
    for t in tracks {
        w.track(t)?;
    }
    Ok(tracks.iter().cloned().collect())
}

fn snap_range(w: &Work, r: Range) -> Result<Range, Refusal> {
    let (a, b) = (w.snap(r.start.max(0)), w.snap(r.end.max(0)));
    if b <= a {
        return Err(Refusal::new("range.empty", vec![], "the range must end after it starts"));
    }
    Ok(Range { start: a, end: b })
}

/// Snaps a delta to a whole number of frames.
fn snap_delta(w: &Work, d: Micros) -> Micros {
    frame_time(nearest_frame(d, w.fps), w.fps)
}

fn limit(w: &mut Work, id: &str, wanted: Micros, got: Micros) {
    if wanted != got {
        w.limited.insert(id.to_string());
    }
}

// ---------------------------------------------------------------------------------------------------------------

fn split(mut w: Work, items: &[String], tracks: &[String], t: Micros, linked: bool) -> Result<Plan, Refusal> {
    let t = w.snap(t);
    let mut ids: Vec<String> = Vec::new();
    if items.is_empty() {
        let targets = target_tracks(&w, tracks)?;
        ids.extend(
            w.doc
                .project
                .items
                .values()
                .filter(|i| targets.contains(&i.track_id) && !w.locked(&i.track_id) && i.start < t && t < i.end)
                .map(|i| i.id.clone()),
        );
    } else {
        for id in items {
            let it = w.item(id)?;
            w.editable(&it)?;
            for m in w.group(&it, linked) {
                if m.start < t && t < m.end && !ids.contains(&m.id) && !w.locked(&m.track_id) {
                    ids.push(m.id);
                }
            }
        }
    }
    if ids.is_empty() {
        return Err(Refusal::new("edit.outside", items.to_vec(), "Move the playhead inside the clip to split it."));
    }
    for id in &ids {
        w.split_at(id, t)?;
    }
    let s = format!("split {} at {}", count(ids.len(), "item"), at(t));
    w.finish(s, false)
}

/// Clamped new edge time of one item for a trim by `delta`.
fn trim_edge(w: &Work, it: &Item, edge: Edge, delta: Micros) -> Micros {
    let dur = retime::media_duration(&w.doc, it);
    let (lo, hi) = retime::edge_limits(it, dur, edge == Edge::Start);
    match edge {
        Edge::Start => {
            let mut s = (it.start + delta).min(it.end - w.min_len()).max(0);
            if let Some(lo) = lo {
                s = s.max(lo.min(it.start));
            }
            s
        }
        Edge::End => {
            let mut e = (it.end + delta).max(it.start + w.min_len());
            if let Some(hi) = hi {
                e = e.min(hi.max(it.end));
            }
            e
        }
    }
}

fn trim(mut w: Work, id: &str, edge: Edge, to: Micros, linked: bool, crossfade: bool) -> Result<Plan, Refusal> {
    let it = w.item(id)?;
    w.editable(&it)?;
    let to = w.snap(to);
    let delta = to - if edge == Edge::Start { it.start } else { it.end };
    let group = w.group(&it, linked);
    for m in &group {
        if w.locked(&m.track_id) {
            continue;
        }
        let e = trim_edge(&w, m, edge, delta);
        let wanted = if edge == Edge::Start { m.start + delta } else { m.end + delta };
        limit(&mut w, &m.id, wanted, e);
        let next = match edge {
            Edge::Start => retime::cut(m, e, m.end, Edges::TRIM),
            Edge::End => retime::cut(m, m.start, e, Edges::TRIM),
        };
        w.put(next);
    }
    w.finish(format!("trim {id}"), crossfade)
}

fn ripple_trim(mut w: Work, id: &str, edge: Edge, to: Micros, linked: bool) -> Result<Plan, Refusal> {
    let it = w.item(id)?;
    w.editable(&it)?;
    let to = w.snap(to);
    // A ripple trim of the start edge keeps the item's start where it is: moving the edge to `to` removes (or adds)
    // `to − start` of the item's head and the rest of the track follows.
    let delta = match edge {
        Edge::Start => {
            let d = to - it.start;
            let e = trim_edge(&w, &it, Edge::Start, d);
            limit(&mut w, id, it.start + d, e);
            e - it.start
        }
        Edge::End => {
            let e = trim_edge(&w, &it, Edge::End, to - it.end);
            limit(&mut w, id, to, e);
            e - it.end
        }
    };
    if delta == 0 {
        return w.finish(format!("ripple trim {id}"), false);
    }
    let mut targets: BTreeSet<String> = BTreeSet::from([it.track_id.clone()]);
    if linked {
        targets.extend(w.group(&it, true).into_iter().map(|m| m.track_id));
    }
    match (edge, delta > 0) {
        // Shorter at the end: remove [new end, end).
        (Edge::End, false) => w.ripple_remove(&targets, it.end + delta, it.end, linked)?,
        // Longer at the end: open space after the item, then extend it (and partners that end with it).
        (Edge::End, true) => {
            w.ripple_open(&targets, it.end, delta, linked)?;
            for m in w.group(&it, linked) {
                if (m.end - it.end).abs() <= TOUCH {
                    let cur = w.item(&m.id)?;
                    w.put(retime::cut(&cur, cur.start, cur.end + delta, Edges::TRIM));
                }
            }
        }
        // Head removed: remove [start, start + delta).
        (Edge::Start, true) => w.ripple_remove(&targets, it.start, it.start + delta, linked)?,
        // Head revealed: open space at the start, then extend the shifted item back to the old start.
        (Edge::Start, false) => {
            let d = -delta;
            w.ripple_open(&targets, it.start, d, linked)?;
            for m in w.group(&it, linked) {
                if (m.start - it.start).abs() <= TOUCH {
                    let cur = w.item(&m.id)?;
                    w.put(retime::cut(&cur, cur.start - d, cur.end, Edges::TRIM));
                }
            }
        }
    }
    w.finish(format!("ripple trim {id}"), false)
}

fn ripple_delete_items(mut w: Work, items: &[String], linked: bool) -> Result<Plan, Refusal> {
    let mut selected: Vec<Item> = Vec::new();
    for id in items {
        let it = w.item(id)?;
        w.editable(&it)?;
        for m in w.group(&it, linked) {
            if !selected.iter().any(|s| s.id == m.id) {
                w.editable(&m)?;
                selected.push(m);
            }
        }
    }
    let targets: BTreeSet<String> = selected.iter().map(|s| s.track_id.clone()).collect();
    // The time to remove: the union of the selected spans, minus time other items on the same tracks still use.
    let mut spans: Vec<(Micros, Micros)> = selected.iter().map(|s| (s.start, s.end)).collect();
    spans.sort();
    let mut merged: Vec<(Micros, Micros)> = Vec::new();
    for (a, b) in spans {
        match merged.last_mut() {
            Some(l) if a <= l.1 => l.1 = l.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    let keep: Vec<(Micros, Micros)> = w
        .doc
        .project
        .items
        .values()
        .filter(|i| targets.contains(&i.track_id) && !selected.iter().any(|s| s.id == i.id))
        .map(|i| (i.start, i.end))
        .collect();
    let mut ranges: Vec<(Micros, Micros)> = Vec::new();
    for (a, b) in merged {
        let mut pieces = vec![(a, b)];
        for &(ka, kb) in &keep {
            pieces = pieces
                .into_iter()
                .flat_map(|(pa, pb)| {
                    if kb <= pa || ka >= pb {
                        vec![(pa, pb)]
                    } else {
                        [(pa, ka.max(pa)), (kb.min(pb), pb)].into_iter().filter(|(x, y)| y > x).collect()
                    }
                })
                .collect();
        }
        ranges.extend(pieces);
    }
    for s in &selected {
        w.remove(&s.id);
    }
    ranges.sort();
    let removed: Micros = ranges.iter().map(|(a, b)| b - a).sum();
    for (a, b) in ranges.into_iter().rev() {
        w.ripple_remove(&targets, a, b, linked)?;
    }
    let s = format!("ripple delete {} ({} closed)", count(selected.len(), "item"), at(removed));
    w.finish(s, false)
}

fn ripple_delete_ranges(mut w: Work, tracks: &[String], ranges: &[Range], linked: bool) -> Result<Plan, Refusal> {
    if ranges.is_empty() {
        return Err(Refusal::new("request.invalid", vec![], "give the items or the ranges to ripple-delete"));
    }
    let targets = target_tracks(&w, tracks)?;
    let mut rs: Vec<Range> = ranges.iter().map(|r| snap_range(&w, *r)).collect::<Result<_, _>>()?;
    rs.sort_by_key(|r| r.start);
    for pair in rs.windows(2) {
        if pair[1].start < pair[0].end {
            return Err(Refusal::new("range.overlap", vec![], "ranges must not overlap"));
        }
    }
    for r in rs.iter().rev() {
        w.ripple_remove(&targets, r.start, r.end, linked)?;
    }
    let total: Micros = rs.iter().map(|r| r.end - r.start).sum();
    let s = format!("ripple delete {} ({})", count(rs.len(), "range"), at(total));
    w.finish(s, false)
}

fn delete(mut w: Work, items: &[String], linked: bool) -> Result<Plan, Refusal> {
    let mut n = 0;
    for id in items {
        let it = w.item(id)?;
        w.editable(&it)?;
        for m in w.group(&it, linked) {
            if w.doc.item(&m.id).is_some() {
                w.editable(&m)?;
                w.remove(&m.id);
                n += 1;
            }
        }
    }
    w.finish(format!("delete {}", count(n, "item")), false)
}

/// Insert (`ripple`) or overwrite new items at `t`.
fn place(mut w: Work, track: &str, t: Micros, items: &[Value], ripple: bool) -> Result<Plan, Refusal> {
    let t = w.snap(t.max(0));
    w.track(track)?;
    if items.is_empty() {
        return Err(Refusal::new("request.invalid", vec![], "give at least one item to place"));
    }
    let mut parsed = Vec::new();
    for v in items {
        let mut v = v.clone();
        let Some(o) = v.as_object_mut() else {
            return Err(Refusal::new("request.invalid", vec![], "each item must be an object"));
        };
        o.entry("trackId").or_insert_with(|| track.into());
        let (Some(a), Some(b)) =
            (o.get("startTime").and_then(json_to_micros), o.get("endTime").and_then(json_to_micros))
        else {
            return Err(Refusal::new("request.invalid", vec![], "each item needs startTime and endTime"));
        };
        if b <= a {
            return Err(Refusal::new("request.invalid", vec![], "each item must end after it starts"));
        }
        let tr = o.get("trackId").and_then(Value::as_str).unwrap_or(track).to_string();
        w.track(&tr)?;
        if w.locked(&tr) {
            return Err(Refusal::new("track.locked", vec![], format!("track {tr} is locked")));
        }
        parsed.push((v, tr, a, b));
    }
    let base = parsed.iter().map(|p| p.2).min().unwrap_or(0);
    let len = w.snap(parsed.iter().map(|p| p.3).max().unwrap_or(0) - base);
    let targets: BTreeSet<String> = parsed.iter().map(|p| p.1.clone()).collect();
    if ripple {
        w.ripple_open(&targets, t, len, true)?;
    } else {
        w.remove_span(&targets, t, t + len, false)?;
    }
    let n = parsed.len();
    for (mut v, _, a, b) in parsed {
        let id = match v.get("id").and_then(Value::as_str) {
            Some(id) if w.doc.item(id).is_none() => id.to_string(),
            _ => w.fresh_id(),
        };
        v["id"] = id.into();
        v["startTime"] = micros_to_json(t + w.snap(a - base));
        v["endTime"] = micros_to_json(t + w.snap(b - base));
        let item = Item::from_json(&v, "item").map_err(|e| Refusal::new("request.invalid", vec![], e.to_string()))?;
        w.put(item);
    }
    let verb = if ripple { "insert" } else { "overwrite" };
    w.finish(format!("{verb} {} at {}", count(n, "item"), at(t)), false)
}

fn lift(mut w: Work, tracks: &[String], range: Range) -> Result<Plan, Refusal> {
    let r = snap_range(&w, range)?;
    let targets = target_tracks(&w, tracks)?;
    let culprits: Vec<String> = targets
        .iter()
        .filter(|t| w.locked(t))
        .flat_map(|t| w.doc.items_on_track(t))
        .filter(|i| i.end > r.start && i.start < r.end)
        .map(|i| i.id.clone())
        .collect();
    if !culprits.is_empty() {
        return Err(Refusal::new("track.locked", culprits, "the range covers items on a locked track"));
    }
    w.remove_span(&targets, r.start, r.end, false)?;
    w.finish(format!("lift {}–{}", at(r.start), at(r.end)), false)
}

fn close_gap(mut w: Work, track: &str, t: Micros) -> Result<Plan, Refusal> {
    w.track(track)?;
    let items = w.doc.items_on_track(track);
    if let Some(i) = items.iter().find(|i| i.start <= t && t < i.end) {
        return Err(Refusal::new(
            "gap.none",
            vec![i.id.clone()],
            format!("there is no gap at {}: {} is there", at(t), i.id),
        ));
    }
    let a = items.iter().filter(|i| i.end <= t).map(|i| i.end).max().unwrap_or(0);
    let Some(b) = items.iter().filter(|i| i.start > t).map(|i| i.start).min() else {
        return Err(Refusal::new("gap.none", vec![], format!("there is no gap at {}: nothing follows", at(t))));
    };
    let targets = BTreeSet::from([track.to_string()]);
    let tracks = w.ripple_tracks(&targets, a, true);
    // Closing a gap must not remove anything: sync-locked tracks need the same span empty.
    let culprits: Vec<String> = tracks
        .iter()
        .flat_map(|tr| w.doc.items_on_track(tr))
        .filter(|i| i.end > a && i.start < b)
        .map(|i| i.id.clone())
        .collect();
    if !culprits.is_empty() {
        return Err(Refusal::new(
            "ripple.blocked",
            culprits.clone(),
            format!("closing the gap would cut {} on a sync-locked track", culprits.join(", ")),
        ));
    }
    w.check_ripple(&tracks, a)?;
    w.remove_span(&tracks, a, b, true)?;
    w.finish(format!("close gap {}–{}", at(a), at(b)), false)
}

fn slip(mut w: Work, id: &str, delta: Micros, linked: bool) -> Result<Plan, Refusal> {
    let it = w.item(id)?;
    w.editable(&it)?;
    if !retime::has_source(&it) || it.media_id().is_none() {
        return Err(Refusal::new("slip.noSource", vec![id.to_string()], format!("{id} has no source media to slip")));
    }
    let wanted = snap_delta(&w, delta);
    let group: Vec<Item> =
        w.group(&it, linked).into_iter().filter(|m| retime::has_source(m) && !w.locked(&m.track_id)).collect();
    let mut d = wanted;
    let mut limiting = Vec::new();
    for m in &group {
        let (lo, hi) = retime::slip_limits(m, retime::media_duration(&w.doc, m));
        let c = d.max(lo).min(hi.unwrap_or(Micros::MAX));
        if c != d {
            limiting.push(m.id.clone());
            d = c;
        }
    }
    for m in &group {
        if let Some(s) = retime::slipped(m, d) {
            w.put(s);
        }
    }
    if d != wanted {
        w.limited.extend(limiting);
    }
    w.finish(format!("slip {id} by {}", at(d)), false)
}

/// Neighbour bounds of a slide of `it` on its track: `(min delta, max delta, adjacent previous, adjacent next)`.
fn slide_bounds(w: &Work, it: &Item) -> (Micros, Micros, Option<Item>, Option<Item>) {
    let items = w.doc.items_on_track(&it.track_id);
    let prev = items.iter().filter(|o| o.id != it.id && o.end <= it.start + TOUCH).max_by_key(|o| o.end);
    let next = items.iter().filter(|o| o.id != it.id && o.start >= it.end - TOUCH).min_by_key(|o| o.start);
    let min_len = w.min_len();
    let (mut lo, mut hi) = (-it.start, Micros::MAX);
    let mut adj_prev = None;
    let mut adj_next = None;
    match prev {
        Some(p) if (p.end - it.start).abs() <= TOUCH => {
            lo = lo.max(-(p.end - p.start - min_len));
            let (_, phi) = retime::edge_limits(p, retime::media_duration(&w.doc, p), false);
            if let Some(phi) = phi {
                hi = hi.min(phi - p.end);
            }
            adj_prev = Some((*p).clone());
        }
        Some(p) => lo = lo.max(p.end - it.start),
        None => {}
    }
    match next {
        Some(n) if (n.start - it.end).abs() <= TOUCH => {
            hi = hi.min(n.end - n.start - min_len);
            let (nlo, _) = retime::edge_limits(n, retime::media_duration(&w.doc, n), true);
            if let Some(nlo) = nlo {
                lo = lo.max(nlo - n.start);
            }
            adj_next = Some((*n).clone());
        }
        Some(n) => hi = hi.min(n.start - it.end),
        None => {}
    }
    (lo, hi, adj_prev, adj_next)
}

fn slide(mut w: Work, id: &str, delta: Micros, linked: bool) -> Result<Plan, Refusal> {
    let it = w.item(id)?;
    w.editable(&it)?;
    let wanted = snap_delta(&w, delta);
    let group: Vec<Item> = w.group(&it, linked).into_iter().filter(|m| !w.locked(&m.track_id)).collect();
    let mut d = wanted;
    let mut plans = Vec::new();
    for m in &group {
        let (lo, hi, p, n) = slide_bounds(&w, m);
        let c = d.clamp(lo.min(0), hi.max(0));
        if c != d {
            w.limited.insert(m.id.clone());
            d = c;
        }
        plans.push((m.clone(), p, n));
    }
    for (m, p, n) in plans {
        if let Some(p) = p {
            w.put(retime::cut(&p, p.start, p.end + d, Edges::TRIM));
        }
        if let Some(n) = n {
            w.put(retime::cut(&n, n.start + d, n.end, Edges::TRIM));
        }
        w.put(retime::shifted(&m, d));
    }
    w.finish(format!("slide {id} by {}", at(d)), false)
}

fn roll(mut w: Work, left: &str, right: &str, to: Micros, linked: bool) -> Result<Plan, Refusal> {
    let l = w.item(left)?;
    let r = w.item(right)?;
    w.editable(&l)?;
    w.editable(&r)?;
    if l.track_id != r.track_id || (l.end - r.start).abs() > TOUCH {
        return Err(Refusal::new(
            "roll.notAdjacent",
            vec![left.to_string(), right.to_string()],
            format!("{left} and {right} do not meet at an edit point"),
        ));
    }
    let wanted = w.snap(to) - l.end;
    // Pairs to roll: the given pair and, when linked, partner pairs that meet at the same point.
    let mut pairs = vec![(l.clone(), r.clone())];
    if linked {
        let rlink = r.fields.get("link").and_then(Value::as_str);
        for lp in w.group(&l, true).into_iter().skip(1) {
            if (lp.end - l.end).abs() > TOUCH || w.locked(&lp.track_id) {
                continue;
            }
            let rp = w.doc.items_on_track(&lp.track_id).into_iter().find(|o| {
                (o.start - lp.end).abs() <= TOUCH
                    && rlink.is_some()
                    && o.fields.get("link").and_then(Value::as_str) == rlink
            });
            if let Some(rp) = rp {
                pairs.push((lp.clone(), rp.clone()));
            }
        }
    }
    let mut d = wanted;
    for (a, b) in &pairs {
        let (_, ahi) = retime::edge_limits(a, retime::media_duration(&w.doc, a), false);
        let (blo, _) = retime::edge_limits(b, retime::media_duration(&w.doc, b), true);
        let mut lo = -(a.end - a.start - w.min_len());
        let mut hi = b.end - b.start - w.min_len();
        if let Some(ahi) = ahi {
            hi = hi.min(ahi - a.end);
        }
        if let Some(blo) = blo {
            lo = lo.max(blo - b.start);
        }
        let c = d.clamp(lo.min(0), hi.max(0));
        if c != d {
            w.limited.insert(a.id.clone());
            w.limited.insert(b.id.clone());
            d = c;
        }
    }
    for (a, b) in pairs {
        w.put(retime::cut(&a, a.start, a.end + d, Edges::TRIM));
        w.put(retime::cut(&b, b.start + d, b.end, Edges::TRIM));
    }
    w.finish(format!("roll {left}|{right} to {}", at(l.end + d)), false)
}

fn move_item(w: &mut Work, id: &str, start: Micros, track: Option<&str>, linked: bool) -> Result<(), Refusal> {
    let it = w.item(id)?;
    w.editable(&it)?;
    if let Some(t) = track {
        w.track(t)?;
        if w.locked(t) {
            return Err(Refusal::new("track.locked", vec![id.to_string()], format!("track {t} is locked")));
        }
    }
    let group: Vec<Item> = w.group(&it, linked).into_iter().filter(|m| !w.locked(&m.track_id)).collect();
    let wanted = w.snap(start.max(0)) - it.start;
    let earliest = group.iter().map(|m| m.start).min().unwrap_or(it.start);
    let d = wanted.max(-earliest);
    if d != wanted {
        w.limited.insert(id.to_string());
    }
    for m in group {
        let mut moved = retime::shifted(&m, d);
        if m.id == id
            && let Some(t) = track
        {
            moved.track_id = t.to_string();
        }
        w.put(moved);
    }
    Ok(())
}
