//! Sync placement (design D5): turns per-target matches into one batch of item moves.
//!
//! - Each target is matched to the reference by timecode (mode `timecode`, or `auto` when both media carry
//!   timecode) or by sound ([`crate::sync::align`]).
//! - The most confident match is placed first; placed targets become extra anchors for targets that did not match
//!   the reference directly.
//! - A drift between 20 and 500 ppm retimes an audio-only target; video items keep their speed.
//! - If any synced item would start before 0, every synced item (the reference too) moves right by the same amount.
//! - A move that would overlap another item on the same track is refused and names that item.
//!
//! The plan is a list of `updateItem` ops, applied as one revision; a dry run returns the same plan unapplied.

use std::collections::{BTreeMap, BTreeSet};

use ove_core::model::{Document, Item};
use ove_core::ops::Op;
use ove_core::time::{Micros, micros_to_json};
use ove_media::SourceTimecode;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::sync::{Match, Refusal, Source, SyncParams, align};
use crate::{Error, Result};

/// How targets are matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Timecode when both media carry it, else sound.
    Auto,
    Timecode,
    Audio,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "auto" => Some(Mode::Auto),
            "timecode" => Some(Mode::Timecode),
            "audio" | "sound" => Some(Mode::Audio),
            _ => None,
        }
    }
}

/// Audio access for the planner: the cached onset envelope and raw audio of a media item.
pub trait MediaAudio {
    /// The onset envelope (analysis cache; analysed once when missing).
    fn envelope(&mut self, media_id: &str) -> Result<Vec<f32>>;
    /// Raw mono audio for the fine step.
    fn source(&self, media_id: &str) -> Result<Box<dyn Source>>;
}

/// Media audio of a project bundle: envelopes from the import analysis cache (`cache/waveforms`, analysed once when
/// missing), raw audio decoded from the media files.
pub struct BundleAudio {
    bundle: std::path::PathBuf,
    /// Media id → (file, content id).
    files: BTreeMap<String, (std::path::PathBuf, Option<String>)>,
}

impl BundleAudio {
    pub fn new(bundle: &std::path::Path, files: BTreeMap<String, (std::path::PathBuf, Option<String>)>) -> BundleAudio {
        BundleAudio { bundle: bundle.to_owned(), files }
    }

    fn file(&self, id: &str) -> Result<&(std::path::PathBuf, Option<String>)> {
        let f = self.files.get(id).ok_or_else(|| Error::Invalid(format!("media {id} has no file")))?;
        if !f.0.is_file() {
            return Err(Error::Invalid(format!("{} is offline; reconnect or relink it first", f.0.display())));
        }
        Ok(f)
    }
}

impl MediaAudio for BundleAudio {
    fn envelope(&mut self, id: &str) -> Result<Vec<f32>> {
        let (path, key) = self.file(id)?.clone();
        match key.filter(|k| crate::analysis::valid_key(k)) {
            Some(k) => {
                crate::analysis::ensure(&self.bundle, &k, &path)?;
                let (_, onset) = crate::analysis::read_peaks(&self.bundle, &k)
                    .map_err(|e| Error::Invalid(format!("cannot read the analysis of {}: {e}", path.display())))?;
                Ok(crate::sync::envelope(&onset))
            }
            None => match crate::analysis::analyse_file(&path, &mut |_| true)? {
                Some(a) => Ok(crate::sync::envelope(&a.onset)),
                None => Err(Error::Invalid("analysis cancelled".into())),
            },
        }
    }

    fn source(&self, id: &str) -> Result<Box<dyn Source>> {
        Ok(Box::new(crate::sync::FileSource::new(&self.file(id)?.0)))
    }
}

/// One target's outcome.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetResult {
    pub item: String,
    /// placed, unmatched or refused.
    pub status: &'static str,
    /// timecode or audio.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<&'static str>,
    /// The item it was matched against (the reference, or a placed target).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// Reference media time (s) at which the target media's time 0 sits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_seconds: Option<f64>,
    /// 0–1 (1 for timecode).
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift_ppm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift_applied: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub residual_ms: Option<f64>,
    /// New timeline start and end, seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    /// Timeline move, seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_by: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The item in the way, when the move would overlap it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl TargetResult {
    fn new(item: &str) -> TargetResult {
        TargetResult {
            item: item.to_string(),
            status: "unmatched",
            method: None,
            anchor: None,
            offset_seconds: None,
            confidence: 0.0,
            drift_ppm: None,
            drift_applied: None,
            residual_ms: None,
            start: None,
            end: None,
            moved_by: None,
            speed: None,
            reason: None,
            blocked_by: None,
            detail: None,
        }
    }
}

/// The whole plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlan {
    pub reference: String,
    pub targets: Vec<TargetResult>,
    /// How far every synced item moved right so none starts before 0, seconds.
    pub group_shift: f64,
    pub placed: usize,
    #[serde(skip)]
    pub ops: Vec<Op>,
}

/// A match in reference media time: target media time `u = (1 + ppm·1e-6) · (t − offset)`.
#[derive(Debug, Clone, PartialEq)]
struct Rel {
    offset: f64,
    ppm: f64,
}

fn tc_of(doc: &Document, item: &Item) -> Option<SourceTimecode> {
    let m = doc.project.media.get(item.media_id()?)?;
    serde_json::from_value(m.data()?.get("sourceTimecode")?.clone()).ok()
}

fn speed(item: &Item) -> f64 {
    item.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01)
}

fn reversed(item: &Item) -> bool {
    item.fields.get("playbackDirection").and_then(Value::as_str) == Some("reverse")
}

/// True for an audio item, or a video item whose media is audio only.
fn audio_only(doc: &Document, item: &Item) -> bool {
    item.kind == "audio"
        || item
            .media_id()
            .and_then(|m| doc.project.media.get(m))
            .and_then(|m| m.data()?.get("type")?.as_str().map(|t| t == "audio"))
            .unwrap_or(false)
}

fn has_sound(item: &Item) -> bool {
    matches!(item.kind.as_str(), "audio" | "video") && item.media_id().is_some()
}

fn secs(us: Micros) -> f64 {
    us as f64 / 1e6
}

fn link_of(item: &Item) -> Option<&str> {
    item.fields.get("link").and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// A candidate placement.
struct Cand {
    id: String,
    rel: Rel,
    confidence: f64,
}

/// Envelopes, loaded once per media.
struct Envelopes<'a> {
    media: &'a mut dyn MediaAudio,
    cache: BTreeMap<String, Vec<f32>>,
}

impl Envelopes<'_> {
    fn get(&mut self, id: &str) -> Result<Vec<f32>> {
        if let Some(e) = self.cache.get(id) {
            return Ok(e.clone());
        }
        let e = self.media.envelope(id)?;
        self.cache.insert(id.to_string(), e.clone());
        Ok(e)
    }
}

/// Sound match of `target`'s media against `anchor`'s media.
fn sound_match(
    doc: &Document,
    envs: &mut Envelopes<'_>,
    anchor: &Item,
    target: &Item,
    params: &SyncParams,
) -> Result<std::result::Result<Match, Refusal>> {
    let (am, tm) = (anchor.media_id().unwrap_or_default(), target.media_id().unwrap_or_default());
    let ea = envs.get(am)?;
    let et = envs.get(tm)?;
    let mut sa = envs.media.source(am)?;
    let mut st = envs.media.source(tm)?;
    let p = SyncParams { retime: params.retime && audio_only(doc, target), ..params.clone() };
    align(&ea, sa.as_mut(), &et, st.as_mut(), &p)
}

fn record_match(res: &mut TargetResult, m: &Match, anchor: &str) {
    res.method = Some("audio");
    res.anchor = Some(anchor.to_string());
    res.confidence = (m.coarse.confidence * 1000.0).round() / 1000.0;
    res.reason = None;
    res.detail = None;
    if let Some(d) = &m.drift {
        res.drift_ppm = Some(d.ppm);
        res.drift_applied = Some(d.applied);
        res.residual_ms = Some(d.residual_ms);
        if let Some(n) = &d.note {
            res.reason = Some(n.clone());
        }
    }
}

/// Plans a sync of `targets` to `reference` (item ids).
pub fn plan(
    doc: &Document,
    reference: &str,
    targets: &[String],
    mode: Mode,
    params: &SyncParams,
    media: &mut dyn MediaAudio,
) -> Result<SyncPlan> {
    let r = doc.item(reference).ok_or_else(|| Error::Invalid(format!("no item {reference}")))?;
    if !has_sound(r) {
        return Err(Error::Invalid(format!("the reference {reference} is not a video or audio item with media")));
    }
    if reversed(r) {
        return Err(Error::Invalid(format!("the reference {reference} plays in reverse; sync needs forward items")));
    }
    let mut envs = Envelopes { media, cache: BTreeMap::new() };
    let mut order: Vec<String> = Vec::new();
    let mut results: BTreeMap<String, TargetResult> = BTreeMap::new();
    let mut cands: Vec<Cand> = Vec::new();
    let mut by_sound: Vec<String> = Vec::new();

    for t in targets {
        if t == reference || results.contains_key(t) {
            continue;
        }
        order.push(t.clone());
        let mut res = TargetResult::new(t);
        match doc.item(t) {
            None => {
                res.status = "refused";
                res.reason = Some(format!("no item {t}"));
            }
            Some(item) if !has_sound(item) => {
                res.status = "refused";
                res.reason = Some("not a video or audio item with media".into());
            }
            Some(item) if reversed(item) => {
                res.status = "refused";
                res.reason = Some("plays in reverse; sync needs forward items".into());
            }
            Some(item) => {
                let tcs = (tc_of(doc, r), tc_of(doc, item));
                let use_tc = match mode {
                    Mode::Timecode => true,
                    Mode::Auto => tcs.0.is_some() && tcs.1.is_some(),
                    Mode::Audio => false,
                };
                match (use_tc, tcs) {
                    (true, (Some(a), Some(b))) => {
                        let offset = a.offset_us(&b) as f64 / 1e6;
                        res.method = Some("timecode");
                        res.confidence = 1.0;
                        res.anchor = Some(reference.to_string());
                        cands.push(Cand { id: t.clone(), rel: Rel { offset, ppm: 0.0 }, confidence: 1.0 });
                    }
                    (true, _) => res.reason = Some("timecode mode needs timecode on both media".into()),
                    (false, _) if item.media_id() == r.media_id() => {
                        // The same file: it lines up at offset 0.
                        res.method = Some("audio");
                        res.confidence = 1.0;
                        res.anchor = Some(reference.to_string());
                        cands.push(Cand { id: t.clone(), rel: Rel { offset: 0.0, ppm: 0.0 }, confidence: 1.0 });
                    }
                    (false, _) => by_sound.push(t.clone()),
                }
            }
        }
        results.insert(t.clone(), res);
    }

    // Sound: each target against the reference.
    let mut waiting: Vec<String> = Vec::new();
    for t in &by_sound {
        let Some(item) = doc.item(t) else { continue };
        let res = results.get_mut(t).ok_or_else(|| Error::Invalid("lost target".into()))?;
        match sound_match(doc, &mut envs, r, item, params)? {
            Ok(m) => {
                record_match(res, &m, reference);
                cands.push(Cand {
                    id: t.clone(),
                    rel: Rel { offset: m.offset_s, ppm: m.speed_ppm },
                    confidence: m.coarse.confidence,
                });
            }
            Err(f) => {
                res.confidence = f.coarse.as_ref().map_or(0.0, |c| (c.confidence * 1000.0).round() / 1000.0);
                res.reason = Some(f.reason.clone());
                res.detail = serde_json::to_value(&f).ok();
                waiting.push(t.clone());
            }
        }
    }
    // Anchor chaining: targets that did not match the reference try the placed sound targets, most confident first.
    loop {
        let mut progress = false;
        let mut anchors: Vec<(String, Rel, f64)> = cands
            .iter()
            .filter(|c| results.get(&c.id).and_then(|r| r.method) == Some("audio"))
            .map(|c| (c.id.clone(), c.rel.clone(), c.confidence))
            .collect();
        anchors.sort_by(|a, b| b.2.total_cmp(&a.2));
        let mut still = Vec::new();
        for t in waiting {
            let Some(item) = doc.item(&t) else { continue };
            let mut done = false;
            for (aid, arel, _) in &anchors {
                let Some(anchor) = doc.item(aid) else { continue };
                if let Ok(m) = sound_match(doc, &mut envs, anchor, item, params)? {
                    let ea = 1.0 + arel.ppm * 1e-6;
                    let rel = Rel {
                        offset: arel.offset + m.offset_s / ea,
                        ppm: ((1.0 + m.speed_ppm * 1e-6) * ea - 1.0) * 1e6,
                    };
                    if let Some(res) = results.get_mut(&t) {
                        record_match(res, &m, aid);
                    }
                    cands.push(Cand { id: t.clone(), rel, confidence: m.coarse.confidence });
                    done = true;
                    progress = true;
                    break;
                }
            }
            if !done {
                still.push(t);
            }
        }
        waiting = still;
        if !progress || waiting.is_empty() {
            break;
        }
    }

    // Placement: most confident first.
    cands.sort_by(|a, b| b.confidence.total_cmp(&a.confidence).then(a.id.cmp(&b.id)));
    let rs = speed(r);
    let ref_trim = secs(r.time_field("trimStart"));
    let ref_start = secs(r.start);
    struct Move {
        owner: String,
        id: String,
        start: Micros,
        end: Micros,
        speed: Option<f64>,
    }
    let mut refused: BTreeSet<String> = BTreeSet::new();
    let (moves, shift) = loop {
        let mut moves: Vec<Move> = Vec::new();
        let mut moving: BTreeSet<String> = BTreeSet::new();
        moving.insert(r.id.clone());
        if let Some(l) = link_of(r) {
            doc.project.items.values().filter(|i| link_of(i) == Some(l)).for_each(|i| {
                moving.insert(i.id.clone());
            });
        }
        let ref_group: Vec<String> = moving.iter().cloned().collect();
        for c in cands.iter().filter(|c| !refused.contains(&c.id)) {
            let Some(item) = doc.item(&c.id) else { continue };
            let e = 1.0 + c.rel.ppm * 1e-6;
            let old_speed = speed(item);
            let new_speed = if audio_only(doc, item) { e * rs } else { old_speed };
            let trim = secs(item.time_field("trimStart"));
            let start_s = ref_start + (c.rel.offset + trim / e - ref_trim) / rs;
            let start = (start_s * 1e6).round() as Micros;
            let span = (item.end - item.start) as f64 * old_speed;
            let end = start + (span / new_speed).round() as Micros;
            let delta = start - item.start;
            let speed_change = ((new_speed - old_speed).abs() > 1e-12).then_some(new_speed);
            moves.push(Move { owner: c.id.clone(), id: c.id.clone(), start, end, speed: speed_change });
            moving.insert(c.id.clone());
            if let Some(l) = link_of(item) {
                for p in doc.project.items.values().filter(|i| link_of(i) == Some(l) && i.id != item.id) {
                    if moving.insert(p.id.clone()) {
                        moves.push(Move {
                            owner: c.id.clone(),
                            id: p.id.clone(),
                            start: p.start + delta,
                            end: p.end + delta,
                            speed: None,
                        });
                    }
                }
            }
        }
        // Nothing before 0: the whole synced group moves right.
        let first = moves.iter().min_by_key(|m| m.start).map(|m| (m.start, m.owner.clone()));
        let (min, min_owner) = first.unwrap_or((0, String::new()));
        let shift = if min < 0 { -min } else { 0 };
        if shift > 0 {
            for m in moves.iter_mut() {
                m.start += shift;
                m.end += shift;
            }
            for id in &ref_group {
                if let Some(i) = doc.item(id) {
                    moves.push(Move {
                        owner: reference.to_string(),
                        id: id.clone(),
                        start: i.start + shift,
                        end: i.end + shift,
                        speed: None,
                    });
                }
            }
        }
        // Overlaps with items that stay, or with another target's move on the same track.
        let mut blocked: Option<(String, String)> = None;
        'check: for (k, m) in moves.iter().enumerate() {
            let Some(item) = doc.item(&m.id) else { continue };
            for other in doc.items_on_track(&item.track_id) {
                if other.id == m.id {
                    continue;
                }
                let (os, oe) = match moves.iter().position(|x| x.id == other.id) {
                    Some(j) if j < k => (moves[j].start, moves[j].end),
                    Some(_) => continue,
                    None if moving.contains(&other.id) => (other.start + shift, other.end + shift),
                    None => (other.start, other.end),
                };
                if m.start < oe && os < m.end {
                    // The reference group gives way only through the group shift: the target that forced it is refused.
                    let owner = if m.owner == reference { min_owner.clone() } else { m.owner.clone() };
                    blocked = Some((owner, other.id.clone()));
                    break 'check;
                }
            }
        }
        match blocked {
            Some((owner, other)) => {
                refused.insert(owner.clone());
                if let Some(res) = results.get_mut(&owner) {
                    let name = doc.item(&other).and_then(|i| i.fields.get("name")?.as_str()).unwrap_or("");
                    res.status = "refused";
                    res.blocked_by = Some(other.clone());
                    res.reason = Some(if name.is_empty() {
                        format!("the synced position overlaps item {other} on the same track")
                    } else {
                        format!("the synced position overlaps “{name}” ({other}) on the same track")
                    });
                }
            }
            None => break (moves, shift),
        }
    };

    let mut ops = Vec::new();
    let mut placed = 0;
    for m in &moves {
        let Some(item) = doc.item(&m.id) else { continue };
        if m.start == item.start && m.end == item.end && m.speed.is_none() {
            if m.id == m.owner
                && let Some(res) = results.get_mut(&m.id)
            {
                placed += 1;
                res.status = "placed";
                res.start = Some(secs(m.start));
                res.end = Some(secs(m.end));
                res.moved_by = Some(0.0);
                res.speed = Some(speed(item));
            }
            continue;
        }
        let mut set = Map::new();
        set.insert("startTime".into(), micros_to_json(m.start));
        set.insert("endTime".into(), micros_to_json(m.end));
        if let Some(s) = m.speed {
            set.insert("playbackSpeed".into(), Value::from(s));
        }
        ops.push(Op::UpdateItem { id: m.id.clone(), set });
        if m.id == m.owner
            && let Some(res) = results.get_mut(&m.id)
        {
            placed += 1;
            res.status = "placed";
            res.start = Some(secs(m.start));
            res.end = Some(secs(m.end));
            res.moved_by = Some(secs(m.start - item.start));
            res.speed = Some(m.speed.unwrap_or_else(|| speed(item)));
        }
    }
    for c in &cands {
        if let Some(res) = results.get_mut(&c.id) {
            res.offset_seconds = Some((c.rel.offset * 1e6).round() / 1e6);
        }
    }
    let targets = order.into_iter().filter_map(|id| results.remove(&id)).collect();
    Ok(SyncPlan { reference: reference.to_string(), targets, group_shift: secs(shift), placed, ops })
}
