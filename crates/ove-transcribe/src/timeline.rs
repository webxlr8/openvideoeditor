//! The timeline transcript (add-transcript-editing D1, D2): the recognised words of every clip with sound, mapped
//! through trim, speed and direction into timeline time, each with a word id anchored to its source
//! (`<mediaId>@<sourceStartMs>`) so ids stay valid when the timeline changes. One implementation for the app, the
//! command line, MCP and the assistant.

use std::collections::HashMap;
use std::path::PathBuf;

use ove_core::Document;
use ove_core::time::Micros;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::transcript::{Transcript, Word};

/// Words per page of [`TimelineTranscript`].
pub const PAGE_WORDS: usize = 10_000;

/// A clip whose sound is transcribed: the timeline range it occupies and the part of its media it plays.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceClip {
    pub item_id: String,
    pub media_id: String,
    pub path: PathBuf,
    pub start: Micros,
    pub end: Micros,
    pub trim: Micros,
    pub speed: f64,
    pub reverse: bool,
}

/// Why there are no clips to transcribe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoClips {
    /// The timeline has no clip with sound.
    Timeline,
    /// The clips asked for have no sound.
    Selection,
}

impl std::fmt::Display for NoClips {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NoClips::Timeline => "There is no clip with sound to transcribe. Add a video or audio clip first.",
            NoClips::Selection => "The selected clips have no sound to transcribe.",
        })
    }
}

/// The clips with sound: the given ids, else everything on the Main track, else every clip with sound, in timeline
/// order. `path` resolves a media id to its file.
pub fn source_clips(
    doc: &Document,
    ids: &[String],
    path: &dyn Fn(&str) -> Option<PathBuf>,
) -> Result<Vec<SourceClip>, NoClips> {
    let mut clips: Vec<SourceClip> = if ids.is_empty() {
        let main: Vec<SourceClip> =
            doc.items_on_track("t_main").into_iter().filter_map(|i| to_clip(doc, i, path)).collect();
        if main.is_empty() { all_clips(doc, path) } else { main }
    } else {
        ids.iter().filter_map(|id| doc.item(id)).filter_map(|i| to_clip(doc, i, path)).collect()
    };
    clips.sort_by_key(|c| (c.start, c.item_id.clone()));
    if clips.is_empty() {
        return Err(if ids.is_empty() { NoClips::Timeline } else { NoClips::Selection });
    }
    Ok(clips)
}

/// Every clip with sound on the timeline, in timeline order (all tracks).
pub fn all_clips(doc: &Document, path: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<SourceClip> {
    let mut clips: Vec<SourceClip> = doc.project.items.values().filter_map(|i| to_clip(doc, i, path)).collect();
    clips.sort_by_key(|c| (c.start, c.item_id.clone()));
    clips
}

fn to_clip(doc: &Document, i: &ove_core::Item, path: &dyn Fn(&str) -> Option<PathBuf>) -> Option<SourceClip> {
    if !matches!(i.kind.as_str(), "video" | "audio") || i.fields.get("pending").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let media_id = i.media_id()?.to_string();
    let data = doc.project.media.get(&media_id).and_then(|m| m.data());
    if data.and_then(|d| d.get("type")).and_then(Value::as_str) == Some("image")
        || data.and_then(|d| d.get("hasAudio")).and_then(Value::as_bool) == Some(false)
    {
        return None;
    }
    Some(SourceClip {
        item_id: i.id.clone(),
        path: path(&media_id)?,
        media_id,
        start: i.start,
        end: i.end,
        trim: i.time_field("trimStart"),
        speed: i.fields.get("playbackSpeed").and_then(Value::as_f64).unwrap_or(1.0).max(0.01),
        reverse: i.fields.get("playbackDirection").and_then(Value::as_str) == Some("reverse"),
    })
}

impl SourceClip {
    /// The source range the clip plays, in microseconds.
    pub fn source_range(&self) -> (Micros, Micros) {
        let used = ((self.end - self.start) as f64 * self.speed) as Micros;
        (self.trim, self.trim + used)
    }

    /// A source time as timeline time.
    pub fn to_timeline(&self, src: Micros) -> Micros {
        let (lo, hi) = self.source_range();
        let rel = if self.reverse { hi - src } else { src - lo };
        self.start + (rel as f64 / self.speed) as Micros
    }
}

/// The source words that play in a clip (by their middle), in source order, with both ends clamped to the range.
fn played(clip: &SourceClip, transcript: &Transcript) -> Vec<(Word, Micros, Micros)> {
    let (lo, hi) = clip.source_range();
    let mut seen: Vec<(i64, &str)> = Vec::new();
    let mut out = Vec::new();
    for w in &transcript.words {
        let mid = (w.start_us + w.end_us) / 2;
        if mid < lo || mid >= hi {
            continue;
        }
        // Overlapping recognition windows can give one word twice: same source start (ms) and text.
        let key = (w.start_us / 1000, w.text.as_str());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let (a, b) = (clip.to_timeline(w.start_us.clamp(lo, hi)), clip.to_timeline(w.end_us.clamp(lo, hi)));
        out.push((w.clone(), a.min(b), a.max(b).max(a.min(b) + 20_000)));
    }
    out
}

/// The transcript's words that play in a clip, moved to timeline time (trim, speed and direction applied), in
/// timeline order. Captions are made from these.
pub fn timeline_words(clip: &SourceClip, transcript: &Transcript) -> Vec<Word> {
    let mut out: Vec<Word> = played(clip, transcript)
        .into_iter()
        .map(|(w, s, e)| Word { text: w.text, start_us: s, end_us: e, confidence: w.confidence })
        .collect();
    if clip.reverse {
        out.reverse();
    }
    out
}

/// A word's id: the media and the source start in milliseconds.
pub fn word_id(media_id: &str, source_start_us: i64) -> String {
    format!("{media_id}@{}", source_start_us.div_euclid(1000))
}

/// Splits a word id into media id and source start (ms).
pub fn parse_word_id(id: &str) -> Option<(&str, i64)> {
    let (media, ms) = id.rsplit_once('@')?;
    Some((media, ms.parse().ok()?))
}

/// One word of the timeline transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineWord {
    pub id: String,
    pub text: String,
    /// Timeline seconds.
    pub start: f64,
    pub end: f64,
    pub clip_id: String,
    pub media_id: String,
    /// Source seconds within the media.
    pub source_start: f64,
    pub source_end: f64,
    /// A filler candidate (a word in the language's filler list).
    pub filler: bool,
}

/// A sentence of the timeline transcript (segments mode).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub clip_id: String,
    /// The ids of its words, in order.
    pub word_ids: Vec<String>,
}

/// The timeline transcript of one project revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineTranscript {
    /// The revision the transcript was computed from.
    pub rev: i64,
    /// Words on this page, in timeline order.
    pub words: Vec<TimelineWord>,
    /// Words in the whole transcript.
    pub total_words: usize,
    /// Index of the first word of this page.
    pub offset: usize,
    /// Offset of the next page, when there is one.
    pub next_offset: Option<usize>,
    /// Media used on the timeline that has no transcript yet.
    pub untranscribed: Vec<String>,
}

fn secs(us: Micros) -> f64 {
    us as f64 / 1e6
}

/// Every word of every clip in `clips` whose media has a transcript in `transcripts` (by media id), in timeline
/// order; media without a transcript are listed in `untranscribed`. `language` picks the filler list.
pub fn words(clips: &[SourceClip], transcripts: &HashMap<String, Transcript>) -> (Vec<TimelineWord>, Vec<String>) {
    let mut out = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for clip in clips {
        let Some(t) = transcripts.get(&clip.media_id) else {
            if !missing.contains(&clip.media_id) {
                missing.push(clip.media_id.clone());
            }
            continue;
        };
        for (w, s, e) in played(clip, t) {
            out.push(TimelineWord {
                id: word_id(&clip.media_id, w.start_us),
                filler: crate::fillers::is_filler(&w.text, &t.language),
                text: w.text,
                start: secs(s),
                end: secs(e),
                clip_id: clip.item_id.clone(),
                media_id: clip.media_id.clone(),
                source_start: secs(w.start_us),
                source_end: secs(w.end_us),
            });
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.clip_id.cmp(&b.clip_id)));
    (out, missing)
}

/// The transcript as one page of at most `limit` words from `offset` (limit 0 or over [`PAGE_WORDS`] means
/// [`PAGE_WORDS`]).
pub fn page(
    rev: i64,
    all: Vec<TimelineWord>,
    untranscribed: Vec<String>,
    offset: usize,
    limit: usize,
) -> TimelineTranscript {
    let limit = if limit == 0 { PAGE_WORDS } else { limit.min(PAGE_WORDS) };
    let total = all.len();
    let start = offset.min(total);
    let end = (start + limit).min(total);
    let words = all.into_iter().skip(start).take(end - start).collect();
    TimelineTranscript {
        rev,
        words,
        total_words: total,
        offset: start,
        next_offset: (end < total).then_some(end),
        untranscribed,
    }
}

/// Groups words into sentences: a new segment after a word ending in `.`, `?` or `!`, at a clip change, or after a
/// gap of more than 1 s.
pub fn segments(words: &[TimelineWord]) -> Vec<TranscriptSegment> {
    let mut out: Vec<TranscriptSegment> = Vec::new();
    let mut open = false;
    for w in words {
        let start_new = match out.last() {
            None => true,
            Some(s) => !open || s.clip_id != w.clip_id || w.start - s.end > 1.0,
        };
        if start_new {
            out.push(TranscriptSegment {
                text: w.text.clone(),
                start: w.start,
                end: w.end,
                clip_id: w.clip_id.clone(),
                word_ids: vec![w.id.clone()],
            });
        } else if let Some(s) = out.last_mut() {
            s.text.push(' ');
            s.text.push_str(&w.text);
            s.end = s.end.max(w.end);
            s.word_ids.push(w.id.clone());
        }
        open = !w.text.trim_end().ends_with(['.', '?', '!']);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::ops::{Op, apply_batch};
    use serde_json::json;

    fn doc() -> Document {
        let ops = vec![
            Op::AddTrack { track: json!({"id": "t_main", "name": "Main", "order": 0, "type": "video"}) },
            Op::AddMedia {
                media: json!({"id": "m", "data": {"type": "video", "fileName": "talk.mp4", "duration": 60, "hasAudio": true}}),
                entry: Some(json!({"mediaId": "m", "nativePath": "/media/talk.mp4", "ownedByBundle": false})),
            },
            // Plays source 2 s – 8 s at double speed from timeline 10 s to 13 s.
            Op::AddItem {
                item: json!({"id": "v", "trackId": "t_main", "type": "video", "startTime": 10, "endTime": 13,
                    "mediaItemId": "m", "trimStart": 2, "playbackSpeed": 2}),
            },
        ];
        apply_batch(&Document::new("p", "p", 1), &ops).unwrap().doc
    }

    fn w(t: &str, s: f64, e: f64) -> Word {
        Word { text: t.into(), start_us: (s * 1e6) as i64, end_us: (e * 1e6) as i64, confidence: 0.9 }
    }

    fn transcript() -> Transcript {
        Transcript {
            language: "en".into(),
            model: "base".into(),
            words: vec![
                w("Before", 0.5, 1.0),
                w("Hello", 2.5, 3.0),
                w("there.", 3.0, 3.5),
                // The same word again from an overlapping window.
                w("there.", 3.0004, 3.5),
                w("Um", 4.0, 4.4),
                w("Second", 6.0, 6.6),
                w("line", 6.6, 7.0),
                w("After", 9.0, 9.5),
            ],
            duration_us: 60_000_000,
        }
    }

    fn resolve(_: &str) -> Option<PathBuf> {
        Some(PathBuf::from("/media/talk.mp4"))
    }

    #[test]
    fn words_follow_trim_and_speed() {
        let d = doc();
        let clips = source_clips(&d, &[], &resolve).unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!((clips[0].trim, clips[0].speed), (2_000_000, 2.0));
        let words = timeline_words(&clips[0], &transcript());
        let texts: Vec<_> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Hello", "there.", "Um", "Second", "line"], "duplicates from overlapping windows dropped");
        // Source 2.5 s → 10 s + 0.5 s / 2.
        assert_eq!(words[0].start_us, 10_250_000);
        assert_eq!(words[4].end_us, 12_500_000);
        let rev = SourceClip { reverse: true, ..clips[0].clone() };
        let r = timeline_words(&rev, &transcript());
        assert_eq!(r[0].text, "line");
        assert!(r.windows(2).all(|p| p[0].start_us <= p[1].start_us));
    }

    #[test]
    fn the_spec_scenario_trim_and_speed() {
        // Source words at 10.0 s and 12.0 s; trimmed to source 9.0 s, placed at timeline 0 s, 2× speed.
        let clip = SourceClip {
            item_id: "c".into(),
            media_id: "m".into(),
            path: PathBuf::new(),
            start: 0,
            end: 5_000_000,
            trim: 9_000_000,
            speed: 2.0,
            reverse: false,
        };
        let t = Transcript {
            language: "en".into(),
            model: "base".into(),
            words: vec![w("a", 10.0, 10.2), w("b", 12.0, 12.2)],
            duration_us: 20_000_000,
        };
        let (words, missing) = words(&[clip], &HashMap::from([("m".to_string(), t)]));
        assert!(missing.is_empty());
        assert_eq!(words.iter().map(|w| w.start).collect::<Vec<_>>(), [0.5, 1.5]);
        assert_eq!(words[0].id, "m@10000");
        assert_eq!(parse_word_id(&words[1].id), Some(("m", 12_000)));
    }

    #[test]
    fn ids_fillers_segments_and_pages() {
        let d = doc();
        let clips = source_clips(&d, &[], &resolve).unwrap();
        let (all, missing) = words(&clips, &HashMap::from([("m".to_string(), transcript())]));
        assert!(missing.is_empty());
        assert_eq!(
            all.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["m@2500", "m@3000", "m@4000", "m@6000", "m@6600"]
        );
        assert!(all[2].filler && !all[0].filler);
        let segs = segments(&all);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "Hello there.");
        assert_eq!(segs[1].word_ids, ["m@4000", "m@6000", "m@6600"]);
        let p = page(7, all.clone(), vec![], 0, 2);
        assert_eq!((p.words.len(), p.total_words, p.next_offset), (2, 5, Some(2)));
        let p = page(7, all, vec![], 4, 2);
        assert_eq!((p.words.len(), p.next_offset), (1, None));
        // Media without a transcript is listed, not guessed.
        let (none, missing) = words(&clips, &HashMap::new());
        assert!(none.is_empty());
        assert_eq!(missing, ["m"]);
    }

    #[test]
    fn no_clips_is_explained() {
        let d = apply_batch(&Document::new("p", "p", 1), &[]).unwrap().doc;
        assert_eq!(source_clips(&d, &[], &resolve), Err(NoClips::Timeline));
        assert_eq!(source_clips(&d, &["x".into()], &resolve), Err(NoClips::Selection));
    }
}
