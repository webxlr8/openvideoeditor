//! Ducking music under dialogue (design D7). The result is volume keyframes on the music items, never a hidden
//! effect, so it shows in `timeline.tsx`, can be edited point by point and is one revision.
//!
//! - Speech comes from transcript word timings when the dialogue items have a cached transcript, else from a level
//!   gate with hysteresis on 10 ms windows of the dialogue items as heard (their volume, fades and speed, through the
//!   shared mixer). Spans closer than `hold` merge.
//! - For each music item the gain ramps down (linear in dB, which is how the mixer interpolates) over `attack`
//!   before each span, stays at `depth` during it and ramps back over `release` after it.
//! - `replace` (the default) rewrites the item's volume keyframes only inside the affected range, so ducking again
//!   with another depth replaces the earlier duck; `merge` multiplies the duck with the existing volume curve.
//! - The curve is simplified (Ramer–Douglas–Peucker in dB) to at most 2 keyframes per second of the affected range.

use std::path::Path;
use std::sync::Arc;

use ove_core::model::{Document, Item};
use ove_core::ops::Op;
use ove_core::time::{Micros, json_to_micros, micros_to_json};
use ove_render::audio::{Mixer, RATE, VOLUME_FLOOR_DB, VolumeCurve, percent_to_db};
use serde::Serialize;
use serde_json::{Value, json};

/// Gate opens when a 10 ms window is louder than this (dBFS)…
pub const GATE_OPEN_DB: f64 = -32.0;
/// …and closes after [`GATE_HANG_US`] of windows quieter than this.
pub const GATE_CLOSE_DB: f64 = -42.0;
pub const GATE_HANG_US: i64 = 150_000;
/// Gated spans shorter than this are not speech.
pub const MIN_SPEECH_US: i64 = 120_000;
const WINDOW_US: i64 = 10_000;
/// Simplification target: keyframes per second of the affected range.
const MAX_KEYS_PER_S: f64 = 2.0;

/// A time range on the timeline (µs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub start_us: i64,
    pub end_us: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuckParams {
    /// Gain during speech, dB (negative).
    pub depth_db: f64,
    pub attack_us: i64,
    pub release_us: i64,
    pub hold_us: i64,
}

impl Default for DuckParams {
    fn default() -> Self {
        DuckParams { depth_db: -12.0, attack_us: 200_000, release_us: 400_000, hold_us: 500_000 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechSource {
    /// The transcript when one exists, else the level gate.
    Auto,
    Transcript,
    Level,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Replace,
    Merge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DuckError {
    UnknownItem(String),
    NoSound(String),
    NoTranscript,
    NoSpeech,
}

impl DuckError {
    pub fn code(&self) -> &'static str {
        match self {
            DuckError::UnknownItem(_) => "item.notFound",
            DuckError::NoSound(_) => "duck.noSound",
            DuckError::NoTranscript => "duck.noTranscript",
            DuckError::NoSpeech => "duck.noSpeech",
        }
    }
}

impl std::fmt::Display for DuckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DuckError::UnknownItem(id) => write!(f, "no item or track \"{id}\" in this project"),
            DuckError::NoSound(id) => write!(f, "item \"{id}\" has no sound (only video and audio items can duck)"),
            DuckError::NoTranscript => write!(f, "the dialogue items have no transcript; use the level source"),
            DuckError::NoSpeech => write!(f, "no speech was found in the dialogue items; nothing was changed"),
        }
    }
}

/// Merges spans closer than `hold_us` (and overlapping ones); input in any order.
pub fn merge_spans(mut spans: Vec<Span>, hold_us: i64) -> Vec<Span> {
    spans.retain(|s| s.end_us > s.start_us);
    spans.sort_by_key(|s| s.start_us);
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        match out.last_mut() {
            Some(l) if s.start_us - l.end_us < hold_us => l.end_us = l.end_us.max(s.end_us),
            _ => out.push(s),
        }
    }
    out
}

/// Speech spans from a level gate with hysteresis over window levels (dBFS), window `i` starting at
/// `t0_us + i·window_us`.
pub fn spans_from_levels(levels_db: &[f64], t0_us: i64, window_us: i64) -> Vec<Span> {
    let hang = (GATE_HANG_US / window_us).max(1) as usize;
    let mut out = Vec::new();
    let mut open: Option<usize> = None;
    let mut quiet = 0usize;
    let mut last_loud = 0usize;
    for (i, &l) in levels_db.iter().enumerate() {
        match open {
            None if l > GATE_OPEN_DB => {
                open = Some(i);
                quiet = 0;
                last_loud = i;
            }
            Some(start) => {
                if l < GATE_CLOSE_DB {
                    quiet += 1;
                    if quiet >= hang {
                        out.push(Span {
                            start_us: t0_us + start as i64 * window_us,
                            end_us: t0_us + (last_loud as i64 + 1) * window_us,
                        });
                        open = None;
                    }
                } else {
                    quiet = 0;
                    last_loud = i;
                }
            }
            None => {}
        }
    }
    if let Some(start) = open {
        out.push(Span {
            start_us: t0_us + start as i64 * window_us,
            end_us: t0_us + (last_loud as i64 + 1) * window_us,
        });
    }
    out.retain(|s| s.end_us - s.start_us >= MIN_SPEECH_US);
    out
}

/// Duck gain (dB) at timeline time `t` for merged spans: 0 away from speech, `depth` during it, linear ramps between.
pub fn duck_db(spans: &[Span], p: &DuckParams, t: i64) -> f64 {
    let mut g: f64 = 0.0;
    for s in spans {
        let v = if t >= s.start_us && t <= s.end_us {
            p.depth_db
        } else if t < s.start_us && t > s.start_us - p.attack_us {
            p.depth_db * (t - (s.start_us - p.attack_us)) as f64 / p.attack_us.max(1) as f64
        } else if t > s.end_us && t < s.end_us + p.release_us {
            p.depth_db * ((s.end_us + p.release_us) - t) as f64 / p.release_us.max(1) as f64
        } else {
            0.0
        };
        g = g.min(v);
    }
    g
}

/// Ramer–Douglas–Peucker on a polyline of (time, dB), keeping both ends.
fn rdp(points: &[(i64, f64)], eps: f64) -> Vec<(i64, f64)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    let (mut idx, mut worst) = (0, 0.0);
    for (i, p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        // Vertical distance in dB from the chord at the point's time: the error the person would hear.
        let f = (p.0 - a.0) as f64 / (b.0 - a.0).max(1) as f64;
        let d = (p.1 - (a.1 + (b.1 - a.1) * f)).abs();
        if d > worst {
            worst = d;
            idx = i;
        }
    }
    if worst <= eps {
        return vec![a, b];
    }
    let mut left = rdp(&points[..=idx], eps);
    let right = rdp(&points[idx..], eps);
    left.pop();
    left.extend(right);
    left
}

/// Planned volume keyframes of one music item (item-relative µs, percent).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDuck {
    pub id: String,
    pub keyframes: Vec<(f64, f64)>,
    /// Affected range, item-relative seconds.
    pub from: f64,
    pub to: f64,
}

fn key_points(item: &Item) -> (Vec<(Micros, f64)>, String) {
    let track = item
        .fields
        .get("keyframeTracks")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|t| t.get("property").and_then(Value::as_str) == Some("volume")));
    let easing = track.and_then(|t| t.get("easing")?.as_str()).unwrap_or("linear").to_string();
    let mut keys: Vec<(Micros, f64)> = track
        .and_then(|t| t.get("keyframes")?.as_array())
        .map(|ks| {
            ks.iter().filter_map(|k| Some((json_to_micros(k.get("time")?)?, k.get("value")?.as_f64()?))).collect()
        })
        .unwrap_or_default();
    keys.sort_by_key(|k| k.0);
    (keys, easing)
}

fn curve(keys: &[(Micros, f64)], easing: &str, item: &Item) -> Option<VolumeCurve> {
    if keys.is_empty() {
        return None;
    }
    let tracks = json!([{"property": "volume", "easing": easing,
        "keyframes": keys.iter().map(|(t, v)| json!({"time": micros_to_json(*t), "value": v})).collect::<Vec<_>>()}]);
    let mut probe = item.clone();
    probe.fields.insert("keyframeTracks".into(), tracks);
    VolumeCurve::of(&probe)
}

/// The keyframes for one music item, or `None` when no speech reaches it.
pub fn plan_item(item: &Item, spans: &[Span], p: &DuckParams, mode: Mode) -> Option<ItemDuck> {
    let len = item.end - item.start;
    let near: Vec<Span> = spans
        .iter()
        .copied()
        .filter(|s| s.start_us - p.attack_us < item.end && s.end_us + p.release_us > item.start)
        .collect();
    let (first, last) = (near.first()?, near.last()?);
    let a = (first.start_us - p.attack_us - item.start).max(0);
    let b = (last.end_us + p.release_us - item.start).min(len);
    let (existing, easing) = key_points(item);
    let (inside, outside): (Vec<_>, Vec<_>) = existing.iter().copied().partition(|k| k.0 >= a && k.0 <= b);
    let base_keys = match mode {
        Mode::Replace => outside.clone(),
        Mode::Merge => existing.clone(),
    };
    let base = curve(&base_keys, &easing, item);
    let level = |rel: i64| base.as_ref().map_or(0.0, |c| c.db_at(rel)) + duck_db(&near, p, item.start + rel);
    // Breakpoints: the range ends, the ramp corners and (when merging) the existing keyframes inside the range.
    let mut times = vec![a, b];
    for s in &near {
        for t in [s.start_us - p.attack_us, s.start_us, s.end_us, s.end_us + p.release_us] {
            let rel = t - item.start;
            if rel > a && rel < b {
                times.push(rel);
            }
        }
    }
    if mode == Mode::Merge {
        times.extend(inside.iter().map(|k| k.0));
    }
    times.sort_unstable();
    times.dedup();
    let points: Vec<(i64, f64)> = times.iter().map(|&t| (t, level(t).max(VOLUME_FLOOR_DB))).collect();
    let limit = ((MAX_KEYS_PER_S * (b - a) as f64 / 1e6).ceil() as usize).max(2);
    let mut eps = 0.05;
    let mut simple = rdp(&points, eps);
    while simple.len() > limit && eps < 60.0 {
        eps *= 2.0;
        simple = rdp(&points, eps);
    }
    let pct = |db: f64| if db <= VOLUME_FLOOR_DB { 0.0 } else { (100.0 * 10f64.powf(db / 20.0)).clamp(0.0, 200.0) };
    let mut keys: Vec<(i64, f64)> = outside.iter().map(|k| (k.0, k.1)).collect();
    keys.extend(simple.iter().map(|&(t, db)| (t, (pct(db) * 100.0).round() / 100.0)));
    keys.sort_by_key(|k| k.0);
    Some(ItemDuck {
        id: item.id.clone(),
        keyframes: keys.iter().map(|&(t, v)| (t as f64 / 1e6, v)).collect(),
        from: a as f64 / 1e6,
        to: b as f64 / 1e6,
    })
}

/// The update for one item: its keyframe tracks with the volume track replaced (other properties kept). Volume
/// keyframes interpolate linearly in dB, which the ramps rely on.
pub fn update_op(item: &Item, d: &ItemDuck) -> Op {
    let mut tracks: Vec<Value> = item
        .fields
        .get("keyframeTracks")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|t| t.get("property").and_then(Value::as_str) != Some("volume")).cloned().collect())
        .unwrap_or_default();
    let keys: Vec<Value> = d
        .keyframes
        .iter()
        .map(|(t, v)| json!({"time": micros_to_json((t * 1e6).round() as i64), "value": v}))
        .collect();
    tracks.push(json!({"property": "volume", "keyframes": keys}));
    let mut set = serde_json::Map::new();
    set.insert("keyframeTracks".into(), Value::Array(tracks));
    Op::UpdateItem { id: item.id.clone(), set }
}

/// Item ids named directly or through a track (id or name, any case).
pub fn resolve(doc: &Document, names: &[String]) -> Result<Vec<String>, DuckError> {
    let mut out = Vec::new();
    for n in names {
        if doc.item(n).is_some() {
            out.push(n.clone());
        } else if let Some(t) = doc.project.tracks.iter().find(|t| t.id == *n || t.name.eq_ignore_ascii_case(n)) {
            out.extend(doc.items_on_track(&t.id).iter().map(|i| i.id.clone()));
        } else {
            return Err(DuckError::UnknownItem(n.clone()));
        }
    }
    out.dedup();
    for id in &out {
        let kind = doc.item(id).map(|i| i.kind.as_str());
        if kind != Some("audio") && kind != Some("video") {
            return Err(DuckError::NoSound(id.clone()));
        }
    }
    Ok(out)
}

/// Speech spans from cached transcripts of the dialogue items (`cache/transcripts/<contentId>-*.json`), mapped to
/// timeline time through trim and speed. `None` when no dialogue item has a transcript.
pub fn spans_from_transcripts(doc: &Document, bundle: &Path, dialogue: &[String]) -> Option<Vec<Span>> {
    let dir = bundle.join("cache").join("transcripts");
    let files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).collect();
    let mut found = false;
    let mut spans = Vec::new();
    for id in dialogue {
        let Some(item) = doc.item(id) else { continue };
        if item.fields.get("playbackDirection").and_then(Value::as_str) == Some("reverse") {
            continue;
        }
        let Some(cid) =
            item.media_id().and_then(|m| doc.registry.get(m)).and_then(|e| e.fields.get("contentId")?.as_str())
        else {
            continue;
        };
        let prefix = format!("{cid}-");
        let Some(file) = files.iter().find(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".json"))
        }) else {
            continue;
        };
        let Some(t) = std::fs::read(file).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { continue };
        found = true;
        let speed = item.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01);
        let trim = item.time_field("trimStart");
        let to_tl = |src: i64| item.start + ((src - trim) as f64 / speed) as i64;
        for w in t.get("words").and_then(Value::as_array).into_iter().flatten() {
            let (Some(a), Some(b)) =
                (w.get("start_us").and_then(Value::as_i64), w.get("end_us").and_then(Value::as_i64))
            else {
                continue;
            };
            let (a, b) = (to_tl(a).max(item.start), to_tl(b).min(item.end));
            if b > a {
                spans.push(Span { start_us: a, end_us: b });
            }
        }
    }
    found.then_some(spans)
}

/// 10 ms window levels (dBFS, RMS of both channels) of the dialogue items alone, as heard, from `from_us` to `to_us`.
pub fn dialogue_levels(
    doc: &Document,
    bundle: Option<&Path>,
    dialogue: &[String],
    from_us: i64,
    to_us: i64,
) -> Vec<f64> {
    let mut solo = doc.clone();
    let keep: std::collections::BTreeSet<&str> = dialogue.iter().map(String::as_str).collect();
    let ids: Vec<String> = solo.project.items.keys().cloned().collect();
    for id in ids {
        if !keep.contains(id.as_str())
            && let Some(item) = solo.project.items.get(&id)
        {
            let mut it = (**item).clone();
            it.fields.insert("muted".into(), Value::Bool(true));
            solo.project.items.insert(id, Arc::new(it));
        }
    }
    for t in &mut solo.project.tracks {
        t.fields.remove("muted");
    }
    let mut mixer = Mixer::new(bundle);
    let win = (RATE as i64 * WINDOW_US / 1_000_000) as usize;
    let mut out = Vec::new();
    let mut t = from_us;
    // Mixed in blocks of 50 windows; each window's RMS over both channels.
    while t < to_us {
        let block = mixer.mix(&solo, t, win * 50);
        for w in block.chunks(win * 2) {
            let ms = w.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / w.len().max(1) as f64;
            out.push(if ms <= 0.0 { -120.0 } else { (10.0 * ms.log10()).max(-120.0) });
        }
        t += WINDOW_US * 50;
    }
    out.truncate(((to_us - from_us) / WINDOW_US).max(0) as usize);
    out
}

/// What a duck would do (or did).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuckPlan {
    /// "transcript" or "level".
    pub source: &'static str,
    /// Merged speech spans, seconds.
    pub speech: Vec<(f64, f64)>,
    pub items: Vec<ItemDuck>,
    #[serde(skip)]
    pub ops: Vec<Op>,
}

/// Plans a duck of `music` under `dialogue` (item or track ids or names).
pub fn plan(
    doc: &Document,
    bundle: Option<&Path>,
    music: &[String],
    dialogue: &[String],
    p: &DuckParams,
    source: SpeechSource,
    mode: Mode,
) -> Result<DuckPlan, DuckError> {
    let music = resolve(doc, music)?;
    let dialogue = resolve(doc, dialogue)?;
    let from_words = match (source, bundle) {
        (SpeechSource::Level, _) | (_, None) => None,
        (_, Some(b)) => spans_from_transcripts(doc, b, &dialogue),
    };
    let (spans, used) = match (from_words, source) {
        (Some(s), _) => (s, "transcript"),
        (None, SpeechSource::Transcript) => return Err(DuckError::NoTranscript),
        (None, _) => {
            let items: Vec<&Item> = dialogue.iter().filter_map(|id| doc.item(id)).collect();
            let from = items.iter().map(|i| i.start).min().unwrap_or(0);
            let to = items.iter().map(|i| i.end).max().unwrap_or(0);
            (spans_from_levels(&dialogue_levels(doc, bundle, &dialogue, from, to), from, WINDOW_US), "level")
        }
    };
    let spans = merge_spans(spans, p.hold_us);
    if spans.is_empty() {
        return Err(DuckError::NoSpeech);
    }
    let mut items = Vec::new();
    let mut ops = Vec::new();
    for id in &music {
        let Some(item) = doc.item(id) else { continue };
        if let Some(d) = plan_item(item, &spans, p, mode) {
            ops.push(update_op(item, &d));
            items.push(d);
        }
    }
    if items.is_empty() {
        return Err(DuckError::NoSpeech);
    }
    Ok(DuckPlan {
        source: used,
        speech: spans.iter().map(|s| (s.start_us as f64 / 1e6, s.end_us as f64 / 1e6)).collect(),
        items,
        ops,
    })
}

/// The heard level change (dB) a duck applies at a music item's relative time, for checks.
pub fn planned_db(d: &ItemDuck, rel_s: f64) -> f64 {
    let keys = &d.keyframes;
    if keys.is_empty() {
        return 0.0;
    }
    let at = |i: usize| (keys[i].0, percent_to_db(keys[i].1));
    if rel_s <= keys[0].0 {
        return at(0).1;
    }
    for i in 1..keys.len() {
        let (t1, d1) = at(i);
        if rel_s <= t1 {
            let (t0, d0) = at(i - 1);
            return d0 + (d1 - d0) * (rel_s - t0) / (t1 - t0).max(1e-9);
        }
    }
    at(keys.len() - 1).1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(start: f64, end: f64, keys: Value) -> Item {
        let mut v = json!({"id": "m", "trackId": "t", "type": "audio", "startTime": start, "endTime": end});
        if !keys.is_null() {
            v["keyframeTracks"] = json!([{"property": "volume", "keyframes": keys}, {"property": "opacity", "keyframes": [{"time": 0, "value": 1}]}]);
        }
        Item::from_json(&v, "m").unwrap()
    }

    fn s(a: f64, b: f64) -> Span {
        Span { start_us: (a * 1e6) as i64, end_us: (b * 1e6) as i64 }
    }

    #[test]
    fn spans_merge_within_hold() {
        let m = merge_spans(vec![s(5.0, 6.0), s(1.0, 2.0), s(2.3, 3.0), s(4.0, 4.1)], 500_000);
        assert_eq!(m, vec![s(1.0, 3.0), s(4.0, 4.1), s(5.0, 6.0)]);
        let m = merge_spans(vec![s(1.0, 2.0), s(2.3, 3.0)], 200_000);
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn level_gate_has_hysteresis() {
        // 10 ms windows: quiet, 1 s of speech with a short dip to -38 dB (between the thresholds) and a 50 ms gap
        // below the close threshold, quiet, a 60 ms blip (too short), quiet.
        let mut l = vec![-80.0; 50];
        l.extend(vec![-20.0; 40]);
        l.extend(vec![-38.0; 20]); // stays open: not below -42
        l.extend(vec![-60.0; 5]); // shorter than the 150 ms hang
        l.extend(vec![-20.0; 35]);
        l.extend(vec![-80.0; 60]);
        l.extend(vec![-10.0; 6]);
        l.extend(vec![-80.0; 30]);
        let spans = spans_from_levels(&l, 0, 10_000);
        assert_eq!(spans, vec![s(0.5, 1.5)]);
    }

    #[test]
    fn keyframes_ramp_around_speech_and_replace_earlier_ducks() {
        let p = DuckParams { depth_db: -12.0, attack_us: 200_000, release_us: 400_000, hold_us: 500_000 };
        let spans = vec![s(2.0, 4.0), s(6.0, 7.0)];
        let it = item(0.0, 10.0, Value::Null);
        let d = plan_item(&it, &spans, &p, Mode::Replace).unwrap();
        assert!((d.from - 1.8).abs() < 1e-9 && (d.to - 7.4).abs() < 1e-9);
        // -12 dB during speech, 0 dB outside, ramps at the right times (within 10 ms).
        for t in [2.0, 3.0, 4.0, 6.0, 6.5, 7.0] {
            assert!((planned_db(&d, t) + 12.0).abs() < 0.05, "{t}: {}", planned_db(&d, t));
        }
        for t in [1.8, 4.4, 5.8, 7.4] {
            assert!(planned_db(&d, t).abs() < 0.05, "{t}: {}", planned_db(&d, t));
        }
        assert!((planned_db(&d, 1.9) + 6.0).abs() < 0.05, "half way down the attack");
        let times: Vec<f64> = d.keyframes.iter().map(|k| k.0).collect();
        for want in [1.8, 2.0, 4.0, 4.4, 5.8, 6.0, 7.0, 7.4] {
            assert!(times.iter().any(|t| (t - want).abs() < 0.01), "no keyframe at {want}: {times:?}");
        }
        // Duck again at -6 dB over the -12 dB keyframes: replaced, not added.
        let ducked =
            item(0.0, 10.0, json!(d.keyframes.iter().map(|(t, v)| json!({"time": t, "value": v})).collect::<Vec<_>>()));
        let again = plan_item(&ducked, &spans, &DuckParams { depth_db: -6.0, ..p }, Mode::Replace).unwrap();
        assert!((planned_db(&again, 3.0) + 6.0).abs() < 0.05);
        // Merge multiplies: -12 then -6 is -18.
        let merged = plan_item(&ducked, &spans, &DuckParams { depth_db: -6.0, ..p }, Mode::Merge).unwrap();
        assert!((planned_db(&merged, 3.0) + 18.0).abs() < 0.05);
        // The opacity track survives the update.
        let op = update_op(&ducked, &again);
        let Op::UpdateItem { set, .. } = op else { panic!() };
        let tracks = set["keyframeTracks"].as_array().unwrap();
        assert!(tracks.iter().any(|t| t["property"] == "opacity"));
        assert_eq!(tracks.iter().filter(|t| t["property"] == "volume").count(), 1);
    }

    #[test]
    fn replace_keeps_keyframes_outside_the_range() {
        let p = DuckParams::default();
        let it =
            item(0.0, 20.0, json!([{"time": 0, "value": 50}, {"time": 15, "value": 50}, {"time": 18, "value": 0}]));
        let d = plan_item(&it, &[s(5.0, 6.0)], &p, Mode::Replace).unwrap();
        // Outside keyframes kept; inside, the duck is relative to the 50 % base.
        assert!(d.keyframes.iter().any(|k| k.0 == 0.0 && k.1 == 50.0));
        assert!(d.keyframes.iter().any(|k| k.0 == 18.0 && k.1 == 0.0));
        assert!((planned_db(&d, 5.5) - (percent_to_db(50.0) - 12.0)).abs() < 0.05);
    }

    #[test]
    fn many_short_lines_stay_under_two_keyframes_per_second() {
        let p = DuckParams { depth_db: -12.0, attack_us: 50_000, release_us: 50_000, hold_us: 0 };
        let spans: Vec<Span> = (0..40).map(|i| s(1.0 + i as f64 * 0.5, 1.2 + i as f64 * 0.5)).collect();
        let d = plan_item(&item(0.0, 30.0, Value::Null), &spans, &p, Mode::Replace).unwrap();
        let secs = d.to - d.from;
        assert!(d.keyframes.len() as f64 <= (2.0 * secs).ceil(), "{} keyframes over {secs} s", d.keyframes.len());
    }

    #[test]
    fn music_away_from_speech_is_untouched() {
        assert!(
            plan_item(&item(20.0, 30.0, Value::Null), &[s(2.0, 3.0)], &DuckParams::default(), Mode::Replace).is_none()
        );
    }
}
