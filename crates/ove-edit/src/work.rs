//! The working copy a planner edits, and the ripple primitives every ripple verb is built from.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::Value;

use ove_core::diff::diff_documents;
use ove_core::model::{Document, Item, Track};
use ove_core::time::{Micros, fmt_secs, frame_len, snap_to_frame};
use ove_core::timemap::{Step, TimeMap};

use crate::retime::{self, Edges};
use crate::{Plan, Refusal};

/// Two edges closer than this are the same point (rounding of frame times at non-integer frame lengths).
pub const TOUCH: Micros = 1;

/// Two overlapping item ids, and the sorted pair of the items they were cut from (when both existed before).
type OverlapPair = (String, String, Option<(String, String)>);

pub struct Work<'a> {
    pub orig: &'a Document,
    pub doc: Document,
    pub fps: f64,
    ids: &'a mut dyn FnMut() -> String,
    pub map: TimeMap,
    pub limited: BTreeSet<String>,
    /// New item id → the item it was cut from.
    origin: BTreeMap<String, String>,
    /// (link value, cut time) → the link value the right halves of that cut share.
    right_links: BTreeMap<(String, Micros), String>,
}

pub fn missing(id: &str) -> Refusal {
    Refusal::new("item.missing", vec![id.to_string()], format!("item {id} does not exist"))
}

impl<'a> Work<'a> {
    pub fn new(doc: &'a Document, ids: &'a mut dyn FnMut() -> String) -> Self {
        Work {
            orig: doc,
            doc: doc.clone(),
            fps: doc.fps(),
            ids,
            map: TimeMap::default(),
            limited: BTreeSet::new(),
            origin: BTreeMap::new(),
            right_links: BTreeMap::new(),
        }
    }

    pub fn snap(&self, t: Micros) -> Micros {
        snap_to_frame(t, self.fps)
    }

    /// Shortest item a planner leaves: one frame.
    pub fn min_len(&self) -> Micros {
        frame_len(self.fps).max(1)
    }

    pub fn item(&self, id: &str) -> Result<Item, Refusal> {
        self.doc.item(id).cloned().ok_or_else(|| missing(id))
    }

    pub fn put(&mut self, it: Item) {
        self.doc.project.items.insert(it.id.clone(), Arc::new(it));
    }

    pub fn remove(&mut self, id: &str) {
        self.doc.project.items.remove(id);
    }

    pub fn track(&self, id: &str) -> Result<&Track, Refusal> {
        self.doc.track(id).ok_or_else(|| Refusal::new("track.missing", vec![], format!("track {id} does not exist")))
    }

    fn flag(&self, track: &str, key: &str, default: bool) -> bool {
        self.doc.track(track).and_then(|t| t.fields.get(key)).and_then(Value::as_bool).unwrap_or(default)
    }

    pub fn locked(&self, track: &str) -> bool {
        self.flag(track, "locked", false)
    }

    pub fn sync_locked(&self, track: &str) -> bool {
        self.flag(track, "syncLock", true)
    }

    /// Refuses an edit of an item on a locked track.
    pub fn editable(&self, it: &Item) -> Result<(), Refusal> {
        if self.locked(&it.track_id) {
            return Err(Refusal::new(
                "track.locked",
                vec![it.id.clone()],
                format!("{} is on a locked track; unlock the track to edit it", it.id),
            ));
        }
        Ok(())
    }

    /// A new item id not used in the document.
    pub fn fresh_id(&mut self) -> String {
        loop {
            let id = (self.ids)();
            if !self.doc.project.items.contains_key(&id) && !self.orig.project.items.contains_key(&id) {
                return id;
            }
        }
    }

    /// A new link value not used in the document.
    fn fresh_link(&mut self) -> String {
        loop {
            let id = format!("l_{}", (self.ids)().trim_start_matches("i_"));
            let used =
                self.doc.project.items.values().any(|i| i.fields.get("link").and_then(Value::as_str) == Some(&id));
            if !used {
                return id;
            }
        }
    }

    /// The item and its link partners (the item first), unless `linked` is false.
    pub fn group(&self, it: &Item, linked: bool) -> Vec<Item> {
        let mut out = vec![it.clone()];
        let Some(link) = it.fields.get("link").and_then(Value::as_str).filter(|_| linked) else { return out };
        out.extend(
            self.doc
                .project
                .items
                .values()
                .filter(|o| o.id != it.id && o.fields.get("link").and_then(Value::as_str) == Some(link))
                .map(|o| (**o).clone()),
        );
        out
    }

    /// Cuts the item `id` in two at `at` (strictly inside it). Returns the id of the new right part. Right parts of
    /// one link group cut at the same time share a new link value.
    pub fn split_at(&mut self, id: &str, at: Micros) -> Result<Option<String>, Refusal> {
        let it = self.item(id)?;
        if at <= it.start || at >= it.end {
            return Ok(None);
        }
        let left = retime::cut(&it, it.start, at, Edges::LEFT);
        let mut right = retime::cut(&it, at, it.end, Edges::RIGHT);
        right.id = self.fresh_id();
        if let Some(link) = it.fields.get("link").and_then(Value::as_str).map(str::to_string) {
            let key = (link, at);
            let new_link = match self.right_links.get(&key) {
                Some(l) => l.clone(),
                None => {
                    let l = self.fresh_link();
                    self.right_links.insert(key, l.clone());
                    l
                }
            };
            right.fields.insert("link".into(), new_link.into());
        }
        let origin = self.origin.get(id).cloned().unwrap_or_else(|| id.to_string());
        self.origin.insert(right.id.clone(), origin);
        let rid = right.id.clone();
        self.put(left);
        self.put(right);
        Ok(Some(rid))
    }

    fn items_on(&self, track: &str) -> Vec<Item> {
        self.doc.items_on_track(track).into_iter().cloned().collect()
    }

    /// Tracks a ripple at `point` moves: the targets, every sync-locked track, and (when `linked`) every track that
    /// holds a link partner of an item the ripple affects, until no track is added.
    pub fn ripple_tracks(&self, targets: &BTreeSet<String>, point: Micros, linked: bool) -> BTreeSet<String> {
        let mut set: BTreeSet<String> = targets.clone();
        set.extend(self.doc.project.tracks.iter().filter(|t| self.sync_locked(&t.id)).map(|t| t.id.clone()));
        if !linked {
            return set;
        }
        loop {
            let links: BTreeSet<&str> = self
                .doc
                .project
                .items
                .values()
                .filter(|i| set.contains(&i.track_id) && i.end > point)
                .filter_map(|i| i.fields.get("link").and_then(Value::as_str))
                .collect();
            let more: BTreeSet<String> = self
                .doc
                .project
                .items
                .values()
                .filter(|i| !set.contains(&i.track_id))
                .filter(|i| i.fields.get("link").and_then(Value::as_str).is_some_and(|l| links.contains(l)))
                .map(|i| i.track_id.clone())
                .collect();
            if more.is_empty() {
                return set;
            }
            set.extend(more);
        }
    }

    /// Refuses a ripple at `point` over `tracks` when it would move or cut an item on a locked track.
    pub fn check_ripple(&self, tracks: &BTreeSet<String>, point: Micros) -> Result<(), Refusal> {
        let culprits: Vec<String> = tracks
            .iter()
            .filter(|t| self.locked(t))
            .flat_map(|t| self.doc.items_on_track(t))
            .filter(|i| i.end > point)
            .map(|i| i.id.clone())
            .collect();
        if culprits.is_empty() {
            return Ok(());
        }
        Err(Refusal::new(
            "ripple.blocked",
            culprits.clone(),
            format!(
                "the ripple would move {} on a locked track; unlock the track or turn its sync lock off",
                culprits.join(", ")
            ),
        ))
    }

    /// Opens `len` of empty time at `at` on each track: items that start at or after `at` move later, items that
    /// cross `at` are cut there and their right part moves.
    pub fn open_gap(&mut self, tracks: &BTreeSet<String>, at: Micros, len: Micros) -> Result<(), Refusal> {
        if len <= 0 {
            return Ok(());
        }
        for t in tracks {
            for it in self.items_on(t) {
                if it.start >= at {
                    self.put(retime::shifted(&it, len));
                } else if it.end > at
                    && let Some(rid) = self.split_at(&it.id, at)?
                {
                    let r = self.item(&rid)?;
                    self.put(retime::shifted(&r, len));
                }
            }
            self.map.push(t, Step { at, delta: len });
        }
        Ok(())
    }

    /// Removes `[a, b)` from each track. With `close`, later items move earlier by `b − a` (a ripple); without, the
    /// span is left empty (a lift). Items inside the span are removed; items crossing an end are trimmed; an item
    /// covering the whole span is cut in two.
    pub fn remove_span(&mut self, tracks: &BTreeSet<String>, a: Micros, b: Micros, close: bool) -> Result<(), Refusal> {
        if b <= a {
            return Ok(());
        }
        let len = b - a;
        let shift = if close { -len } else { 0 };
        for t in tracks {
            for it in self.items_on(t) {
                if it.end <= a {
                    continue;
                }
                if it.start >= b {
                    if close {
                        self.put(retime::shifted(&it, shift));
                    }
                } else if it.start >= a && it.end <= b {
                    self.remove(&it.id);
                } else if it.start < a && it.end > b {
                    let rid = self.split_at(&it.id, b)?;
                    let left = self.item(&it.id)?;
                    self.put(retime::cut(&left, left.start, a, Edges::TRIM));
                    if let Some(rid) = rid {
                        let r = self.item(&rid)?;
                        self.put(retime::shifted(&r, shift));
                    }
                } else if it.start < a {
                    self.put(retime::cut(&it, it.start, a, Edges::TRIM));
                } else {
                    let r = retime::cut(&it, b, it.end, Edges::TRIM);
                    self.put(retime::shifted(&r, shift));
                }
            }
            if close {
                self.map.push(t, Step { at: b, delta: -len });
            }
        }
        Ok(())
    }

    /// Ripple-removes `[a, b)` from the targets and their followers, after the locked-track check.
    pub fn ripple_remove(
        &mut self,
        targets: &BTreeSet<String>,
        a: Micros,
        b: Micros,
        linked: bool,
    ) -> Result<(), Refusal> {
        let tracks = self.ripple_tracks(targets, a, linked);
        self.check_ripple(&tracks, a)?;
        self.remove_span(&tracks, a, b, true)
    }

    /// Ripple-opens `len` at `at` on the targets and their followers, after the locked-track check.
    pub fn ripple_open(
        &mut self,
        targets: &BTreeSet<String>,
        at: Micros,
        len: Micros,
        linked: bool,
    ) -> Result<(), Refusal> {
        let tracks = self.ripple_tracks(targets, at, linked);
        self.check_ripple(&tracks, at)?;
        self.open_gap(&tracks, at, len)
    }

    fn origin_of(&self, id: &str) -> Option<String> {
        if let Some(o) = self.origin.get(id) {
            return Some(o.clone());
        }
        self.orig.project.items.contains_key(id).then(|| id.to_string())
    }

    /// Pairs of items that overlap on a track, by origin id (sorted pair).
    fn overlaps(
        doc: &Document,
        tracks: &BTreeSet<String>,
        origin: &dyn Fn(&str) -> Option<String>,
    ) -> Vec<OverlapPair> {
        let mut out = Vec::new();
        for t in tracks {
            let items = doc.items_on_track(t);
            for (i, a) in items.iter().enumerate() {
                for b in &items[i + 1..] {
                    if b.start >= a.end {
                        break;
                    }
                    let key = match (origin(&a.id), origin(&b.id)) {
                        (Some(x), Some(y)) => Some(if x <= y { (x, y) } else { (y, x) }),
                        _ => None,
                    };
                    out.push((a.id.clone(), b.id.clone(), key));
                }
            }
        }
        out
    }

    /// Ops, summary and checks: refuses a plan that creates an overlap unless `crossfade` allows it.
    pub fn finish(self, summary: String, crossfade: bool) -> Result<Plan, Refusal> {
        let ops = diff_documents(self.orig, &self.doc);
        let mut touched: BTreeSet<String> = BTreeSet::new();
        let mut tracks: BTreeSet<String> = BTreeSet::new();
        for (id, it) in &self.doc.project.items {
            match self.orig.project.items.get(id) {
                Some(o) if o == it => {}
                Some(o) => {
                    touched.insert(id.clone());
                    tracks.insert(it.track_id.clone());
                    tracks.insert(o.track_id.clone());
                }
                None => {
                    touched.insert(id.clone());
                    tracks.insert(it.track_id.clone());
                }
            }
        }
        for (id, o) in &self.orig.project.items {
            if !self.doc.project.items.contains_key(id) {
                touched.insert(id.clone());
                tracks.insert(o.track_id.clone());
            }
        }
        if !crossfade {
            let before: BTreeSet<(String, String)> = Self::overlaps(self.orig, &tracks, &|id| Some(id.to_string()))
                .into_iter()
                .filter_map(|p| p.2)
                .collect();
            let created: Vec<(String, String)> = Self::overlaps(&self.doc, &tracks, &|id| self.origin_of(id))
                .into_iter()
                .filter(|(_, _, key)| key.as_ref().is_none_or(|k| k.0 == k.1 || !before.contains(k)))
                .map(|(a, b, _)| (a, b))
                .collect();
            if let Some((a, b)) = created.first() {
                return Err(Refusal::new(
                    "overlap.created",
                    vec![a.clone(), b.clone()],
                    format!("the edit would make {a} and {b} overlap; ask for a crossfade to allow it"),
                ));
            }
        }
        Ok(Plan {
            ops,
            summary,
            touched: touched.into_iter().collect(),
            time_map: self.map,
            limited: self.limited.into_iter().collect(),
            result: self.doc,
        })
    }
}

/// `n item(s)`.
pub fn count(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// Seconds for summaries, e.g. `2.5s`.
pub fn at(t: Micros) -> String {
    fmt_secs(t)
}
