//! Removing words from the timeline (add-transcript-editing D3, D4): selected word runs become timeline ranges with
//! part of the natural pause kept on each side, edges moved to the quietest 10 ms near the word boundary, and the
//! ranges go to the shared ripple-delete planner (`ove-edit`), so the cut is ordinary ops with a time map, one
//! revision. Caption items drop the cut words and keep the rest aligned; each seam gets a 10 ms audio fade.

use std::collections::{BTreeMap, BTreeSet};

use ove_core::Document;
use ove_core::diff::diff_documents;
use ove_core::model::Item;
use ove_core::time::{Micros, json_to_micros, micros_to_json};
use ove_edit::{Plan, Range, Refusal, Request};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::fillers::normalise;
use crate::timeline::{SourceClip, TimelineWord};

/// How much of the natural pause a cut keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pause {
    Tight,
    #[default]
    Balanced,
    Loose,
}

impl Pause {
    /// The longest pause a cut leaves at a seam.
    pub fn keep_us(self) -> Micros {
        match self {
            Pause::Tight => 60_000,
            Pause::Balanced => 150_000,
            Pause::Loose => 320_000,
        }
    }
}

/// Edges move at most this far to reach a quiet point.
pub const SNAP_US: Micros = 40_000;
/// Fade at each seam.
pub const SEAM_FADE_US: Micros = 10_000;

/// What to remove: word ids, inclusive id ranges (`from`, `to` in timeline order) and exact phrases.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub ranges: Vec<IdRange>,
    /// Every occurrence of each phrase (case, accents and punctuation ignored).
    #[serde(default)]
    pub matches: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdRange {
    pub from: String,
    pub to: String,
}

/// The loudness of a media file's audio in 10 ms cells (dBFS), for edge snapping and dead-air levels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub cell_us: Micros,
    pub db: Vec<f32>,
}

impl Envelope {
    /// 10 ms RMS levels of 16 kHz mono samples.
    pub fn from_samples(samples: &[f32]) -> Envelope {
        const CELL: usize = 160;
        let db = samples
            .chunks(CELL)
            .map(|c| {
                let ms = c.iter().map(|s| s * s).sum::<f32>() / c.len().max(1) as f32;
                10.0 * ms.max(1e-12).log10()
            })
            .collect();
        Envelope { cell_us: 10_000, db }
    }

    pub fn at(&self, t: Micros) -> Option<f32> {
        if t < 0 {
            return None;
        }
        self.db.get((t / self.cell_us) as usize).copied()
    }

    /// The middle of the quietest cell whose middle lies in `[lo, hi]` (source time), or `None` without data.
    pub fn quietest(&self, lo: Micros, hi: Micros) -> Option<Micros> {
        if hi < lo {
            return None;
        }
        let first = lo.max(0) / self.cell_us;
        let last = hi.max(0) / self.cell_us;
        (first..=last)
            .map(|c| (c * self.cell_us + self.cell_us / 2, c))
            .filter(|(mid, _)| *mid >= lo && *mid <= hi)
            .filter_map(|(mid, c)| self.db.get(c as usize).map(|d| (mid, *d)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(mid, _)| mid)
    }
}

/// Why a removal was refused. Nothing changes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutError {
    pub code: String,
    pub message: String,
    /// The word ids or items that caused it.
    pub culprits: Vec<String>,
}

impl CutError {
    fn new(code: &str, message: impl Into<String>, culprits: Vec<String>) -> CutError {
        CutError { code: code.into(), message: message.into(), culprits }
    }
}

impl From<Refusal> for CutError {
    fn from(r: Refusal) -> Self {
        CutError { code: r.code, message: r.message, culprits: r.culprits }
    }
}

impl std::fmt::Display for CutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// Indices into `words` (timeline order) of the selected words. An id that is not on the timeline is refused and
/// named; nothing else in the request is used then.
pub fn select(words: &[TimelineWord], sel: &Selection) -> Result<Vec<usize>, CutError> {
    let index: BTreeMap<&str, usize> = words.iter().enumerate().map(|(i, w)| (w.id.as_str(), i)).collect();
    let mut missing = Vec::new();
    let mut out: BTreeSet<usize> = BTreeSet::new();
    for id in &sel.ids {
        match index.get(id.as_str()) {
            Some(i) => {
                out.insert(*i);
            }
            None => missing.push(id.clone()),
        }
    }
    for r in &sel.ranges {
        match (index.get(r.from.as_str()), index.get(r.to.as_str())) {
            (Some(a), Some(b)) => out.extend(*a.min(b)..=*a.max(b)),
            (a, b) => {
                if a.is_none() {
                    missing.push(r.from.clone());
                }
                if b.is_none() {
                    missing.push(r.to.clone());
                }
            }
        }
    }
    let norm: Vec<String> = words.iter().map(|w| normalise(&w.text)).collect();
    for phrase in &sel.matches {
        let want: Vec<String> = phrase.split_whitespace().map(normalise).filter(|s| !s.is_empty()).collect();
        if want.is_empty() {
            continue;
        }
        for start in 0..norm.len().saturating_sub(want.len() - 1) {
            if norm[start..start + want.len()] == want[..] {
                out.extend(start..start + want.len());
            }
        }
    }
    if !missing.is_empty() {
        return Err(CutError::new(
            "transcript.wordMissing",
            format!("these words are not on the timeline: {}", missing.join(", ")),
            missing,
        ));
    }
    if out.is_empty() {
        return Err(CutError::new("request.invalid", "nothing selected: give word ids, id ranges or a phrase", vec![]));
    }
    Ok(out.into_iter().collect())
}

/// One planned cut, in timeline time before the cut.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutRange {
    pub clip_id: String,
    #[serde(with = "secs")]
    pub start: Micros,
    #[serde(with = "secs")]
    pub end: Micros,
    pub word_ids: Vec<String>,
}

mod secs {
    use ove_core::time::{Micros, micros_to_json};
    pub fn serialize<S: serde::Serializer>(v: &Micros, s: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&micros_to_json(*v), s)
    }
}

impl SourceClip {
    /// A timeline time as source time.
    pub fn to_source(&self, tl: Micros) -> Micros {
        let (lo, hi) = self.source_range();
        let rel = ((tl - self.start) as f64 * self.speed) as Micros;
        if self.reverse { hi - rel } else { lo + rel }
    }
}

fn us(s: f64) -> Micros {
    (s * 1e6).round() as Micros
}

/// The timeline ranges to cut for the selected words: runs of selected words per clip, widened by the pause rule
/// and moved to quiet points (`envelope` gives a media's levels; `None` keeps the computed edges).
pub fn cut_ranges(
    words: &[TimelineWord],
    selected: &[usize],
    clips: &[SourceClip],
    pause: Pause,
    envelope: &dyn Fn(&str) -> Option<Envelope>,
) -> Vec<CutRange> {
    let chosen: BTreeSet<usize> = selected.iter().copied().collect();
    let keep_half = pause.keep_us() / 2;
    let mut out = Vec::new();
    let mut envelopes: BTreeMap<String, Option<Envelope>> = BTreeMap::new();
    for clip in clips {
        let in_clip: Vec<usize> = (0..words.len()).filter(|&i| words[i].clip_id == clip.item_id).collect();
        let mut k = 0;
        while k < in_clip.len() {
            if !chosen.contains(&in_clip[k]) {
                k += 1;
                continue;
            }
            let first = k;
            while k + 1 < in_clip.len() && chosen.contains(&in_clip[k + 1]) {
                k += 1;
            }
            let (a, b) = (&words[in_clip[first]], &words[in_clip[k]]);
            let prev_end = if first > 0 { us(words[in_clip[first - 1]].end) } else { clip.start };
            let next_start = if k + 1 < in_clip.len() { us(words[in_clip[k + 1]].start) } else { clip.end };
            let (run_start, run_end) = (us(a.start), us(b.end));
            let gap_before = (run_start - prev_end).max(0);
            let gap_after = (next_start - run_end).max(0);
            // Each side keeps up to half the pause next to the neighbouring word; the rest of the gap goes.
            let mut start = (prev_end + (gap_before / 2).min(keep_half)).min(run_start);
            let mut end = (next_start - (gap_after / 2).min(keep_half)).max(run_end);
            let env = envelopes.entry(clip.media_id.clone()).or_insert_with(|| envelope(&clip.media_id));
            if let Some(env) = env {
                start = snap(clip, env, start, prev_end, run_start);
                end = snap(clip, env, end, run_end, next_start);
            }
            let word_ids = (first..=k).map(|j| words[in_clip[j]].id.clone()).collect();
            out.push(CutRange {
                clip_id: clip.item_id.clone(),
                start: start.max(clip.start),
                end: end.min(clip.end),
                word_ids,
            });
            k += 1;
        }
    }
    out.sort_by_key(|r| r.start);
    out
}

/// Moves a timeline edge to the quietest 10 ms within ±[`SNAP_US`], never past `lo` or `hi` (neighbouring words).
fn snap(clip: &SourceClip, env: &Envelope, edge: Micros, lo: Micros, hi: Micros) -> Micros {
    let (a, b) = ((edge - SNAP_US).max(lo), (edge + SNAP_US).min(hi));
    if b <= a {
        return edge;
    }
    let (sa, sb) = (clip.to_source(a), clip.to_source(b));
    match env.quietest(sa.min(sb), sa.max(sb)) {
        Some(src) => {
            let t = clip.to_timeline(src);
            t.clamp(a, b)
        }
        None => edge,
    }
}

/// A cut ready to apply: the ops (one batch), its time map and what it does.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutPlan {
    pub plan: Plan,
    pub ranges: Vec<CutRange>,
    pub removed_words: Vec<String>,
    /// Caption items shortened and removed.
    pub captions_changed: Vec<String>,
    pub captions_removed: Vec<String>,
}

/// Plans removing `ranges` from `doc` as one ripple delete on the tracks of the cut clips (and, by the planner's
/// rule, sync-locked tracks and link partners), then makes captions follow and fades the seams.
pub fn plan_cut(
    doc: &Document,
    ranges: Vec<CutRange>,
    removed_words: Vec<String>,
    ids: &mut dyn FnMut() -> String,
) -> Result<CutPlan, CutError> {
    if ranges.is_empty() {
        return Err(CutError::new("request.invalid", "nothing to cut", vec![]));
    }
    // Merge touching or overlapping ranges.
    let mut merged: Vec<CutRange> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(m) if r.start <= m.end => {
                m.end = m.end.max(r.end);
                m.word_ids.extend(r.word_ids);
            }
            _ => merged.push(r),
        }
    }
    let tracks: BTreeSet<String> =
        merged.iter().filter_map(|r| doc.item(&r.clip_id).map(|i| i.track_id.clone())).collect();
    let req = Request::RippleDelete {
        items: vec![],
        tracks: tracks.iter().cloned().collect(),
        ranges: merged.iter().map(|r| Range { start: r.start, end: r.end }).collect(),
        linked: true,
    };
    let mut plan = ove_edit::plan_with(doc, &req, ids)?;
    let track = tracks.iter().next().cloned().unwrap_or_default();
    let map = |t: Micros| plan.time_map.map(&track, t);
    let inside = |t: Micros| merged.iter().any(|r| t >= r.start && t < r.end);

    let mut result = plan.result.clone();
    // Captions follow: rebuilt from the items before the cut.
    let caption_tracks: BTreeSet<String> =
        doc.project.tracks.iter().filter(|t| t.kind == "caption").map(|t| t.id.clone()).collect();
    let (mut changed, mut removed) = (Vec::new(), Vec::new());
    if !caption_tracks.is_empty() {
        let in_result: Vec<String> = result
            .project
            .items
            .values()
            .filter(|i| caption_tracks.contains(&i.track_id))
            .map(|i| i.id.clone())
            .collect();
        for id in in_result {
            result.project.items.remove(&id);
        }
        for it in doc.project.items.values().filter(|i| caption_tracks.contains(&i.track_id)) {
            match follow_caption(it, &map, &inside) {
                Some(new) => {
                    if new != **it {
                        changed.push(it.id.clone());
                    }
                    result.project.items.insert(new.id.clone(), std::sync::Arc::new(new));
                }
                None => removed.push(it.id.clone()),
            }
        }
    }
    // A short fade on both sides of each seam, on the cut tracks.
    let fps = doc.fps();
    let seams: BTreeSet<Micros> =
        merged.iter().map(|r| map(ove_core::time::snap_to_frame(r.start.max(0), fps))).collect();
    let fade = json!({"duration": micros_to_json(SEAM_FADE_US), "easing": "linear"});
    let faded: Vec<Item> = result
        .project
        .items
        .values()
        .filter(|it| tracks.contains(&it.track_id) && matches!(it.kind.as_str(), "video" | "audio"))
        .filter_map(|it| {
            let mut n = (**it).clone();
            if seams.contains(&n.end) && !n.fields.contains_key("audioFadeOut") {
                n.fields.insert("audioFadeOut".into(), fade.clone());
            }
            if seams.contains(&n.start) && n.start > 0 && !n.fields.contains_key("audioFadeIn") {
                n.fields.insert("audioFadeIn".into(), fade.clone());
            }
            (n != **it).then_some(n)
        })
        .collect();
    for n in faded {
        result.project.items.insert(n.id.clone(), std::sync::Arc::new(n));
    }
    plan.ops = diff_documents(doc, &result);
    let n = removed_words.len();
    plan.summary = format!(
        "remove {n} word{} ({} cut{})",
        if n == 1 { "" } else { "s" },
        merged.len(),
        if merged.len() == 1 { "" } else { "s" }
    );
    plan.result = result;
    Ok(CutPlan { plan, ranges: merged, removed_words, captions_changed: changed, captions_removed: removed })
}

/// The caption item after the cut, or `None` when it has no word left. Word times are relative to the item start
/// plus their segment's start.
fn follow_caption(it: &Item, map: &dyn Fn(Micros) -> Micros, inside: &dyn Fn(Micros) -> bool) -> Option<Item> {
    let segs = it.fields.get("captionSegments").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut words: Vec<(String, Micros, Micros)> = Vec::new();
    for s in &segs {
        let base = it.start + s.get("startTime").and_then(json_to_micros).unwrap_or(0);
        for w in s.get("wordTimings").and_then(Value::as_array).into_iter().flatten() {
            let (Some(a), Some(b)) = (w.get("start").and_then(json_to_micros), w.get("end").and_then(json_to_micros))
            else {
                continue;
            };
            let text = w.get("word").and_then(Value::as_str).unwrap_or_default().to_string();
            words.push((text, base + a, base + b));
        }
    }
    if words.is_empty() {
        // A caption without word timings follows its own span.
        if inside(it.start) && inside(it.end - 1) {
            return None;
        }
        let mut n = it.clone();
        n.start = map(it.start);
        n.end = map(it.end).max(n.start + 1);
        return Some(n);
    }
    let kept: Vec<(String, Micros, Micros)> =
        words.into_iter().filter(|(_, a, b)| !inside((a + b) / 2)).map(|(t, a, b)| (t, map(a), map(b))).collect();
    if kept.is_empty() {
        return None;
    }
    let mut n = it.clone();
    let start = if inside(it.start) { kept[0].1 } else { map(it.start).min(kept[0].1) };
    let end = map(it.end).max(kept[kept.len() - 1].2).max(start + 1);
    let text = kept.iter().map(|w| w.0.as_str()).collect::<Vec<_>>().join(" ");
    let rel = |t: Micros| micros_to_json(t - start);
    let timings: Vec<Value> =
        kept.iter().map(|(w, a, b)| json!({"word": w, "start": rel(*a), "end": rel(*b)})).collect();
    n.start = start;
    n.end = end;
    n.fields.insert("captionText".into(), json!(text));
    let seg_id = segs.first().and_then(|s| s.get("id")).cloned().unwrap_or(json!("s1"));
    n.fields.insert(
        "captionSegments".into(),
        json!([{"id": seg_id, "text": text, "startTime": 0, "endTime": rel(end), "wordTimings": timings}]),
    );
    Some(n)
}
