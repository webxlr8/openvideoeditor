//! Dead air (add-transcript-editing D5): pauses found with voice activity, judged against the speaker's own level, so
//! a music bed or loud ambience is kept and only quiet pauses go. Also the filler candidates that need the speech
//! mask (D7: voiced sounds no recognised word covers).

use serde::{Deserialize, Serialize};

use crate::cuts::Envelope;
use crate::transcript::Transcript;
use crate::vad::{SpeechMask, THRESHOLD};

/// Settings of the dead-air rule.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadAirOptions {
    /// A pause counts only when it is at least this much below the speech level (dB).
    pub relative_db: f32,
    /// Shortest pause that counts (µs; 0.25–3 s).
    pub min_pause_us: i64,
    /// Kept on each side of a cut, except at the edges of the file (µs; 0–0.5 s).
    pub padding_us: i64,
}

impl Default for DeadAirOptions {
    fn default() -> Self {
        DeadAirOptions { relative_db: 12.0, min_pause_us: 500_000, padding_us: 150_000 }
    }
}

impl DeadAirOptions {
    /// Clamps the settings to their allowed ranges.
    pub fn clamped(self) -> DeadAirOptions {
        DeadAirOptions {
            relative_db: self.relative_db.clamp(0.0, 60.0),
            min_pause_us: self.min_pause_us.clamp(250_000, 3_000_000),
            padding_us: self.padding_us.clamp(0, 500_000),
        }
    }
}

/// Dead air in one media file, in source time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadAir {
    /// Ranges to cut (padding already removed from them), source µs.
    pub ranges: Vec<(i64, i64)>,
    /// False when the voice activity found no speech: nothing is cut then.
    pub speech_found: bool,
    /// The speech level the pauses were measured against (dBFS), when there is speech.
    pub speech_db: Option<f32>,
}

/// Mean power of `[a, b)` in dB.
fn level(env: &Envelope, a: i64, b: i64) -> Option<f32> {
    let (first, last) = ((a / env.cell_us) as usize, ((b - 1).max(a) / env.cell_us) as usize);
    let cells = env.db.get(first..=last.min(env.db.len().saturating_sub(1)))?;
    if cells.is_empty() {
        return None;
    }
    let p = cells.iter().map(|d| 10f32.powf(d / 10.0)).sum::<f32>() / cells.len() as f32;
    Some(10.0 * p.max(1e-12).log10())
}

/// Finds dead air: non-speech runs of at least `min_pause` whose level is at least `relative_db` below the
/// loudness-weighted level of the speech, minus the padding on each side (not at the file's edges).
pub fn detect(mask: &SpeechMask, env: &Envelope, duration_us: i64, opts: DeadAirOptions) -> DeadAir {
    let opts = opts.clamped();
    let speech = mask.ranges(THRESHOLD);
    if speech.is_empty() {
        return DeadAir { ranges: vec![], speech_found: false, speech_db: None };
    }
    // Loudness-weighted speech level: mean power over all speech cells.
    let (mut p, mut n) = (0f32, 0usize);
    for (a, b) in &speech {
        let (first, last) = ((a / env.cell_us) as usize, ((b - 1) / env.cell_us) as usize);
        for d in env.db.iter().take(last + 1).skip(first) {
            p += 10f32.powf(d / 10.0);
            n += 1;
        }
    }
    let speech_db = if n == 0 { None } else { Some(10.0 * (p / n as f32).max(1e-12).log10()) };
    let Some(reference) = speech_db else {
        return DeadAir { ranges: vec![], speech_found: true, speech_db: None };
    };
    let end = duration_us.max(speech.last().map(|r| r.1).unwrap_or(0));
    let mut gaps = Vec::new();
    let mut at = 0;
    for (a, b) in &speech {
        if *a > at {
            gaps.push((at, *a));
        }
        at = *b;
    }
    if end > at {
        gaps.push((at, end));
    }
    let ranges = gaps
        .into_iter()
        .filter(|(a, b)| b - a >= opts.min_pause_us)
        .filter(|(a, b)| level(env, *a, *b).is_none_or(|l| l <= reference - opts.relative_db))
        .filter_map(|(a, b)| {
            let a2 = if a == 0 { a } else { a + opts.padding_us };
            let b2 = if b >= end { b } else { b - opts.padding_us };
            (b2 > a2).then_some((a2, b2))
        })
        .collect();
    DeadAir { ranges, speech_found: true, speech_db: Some(reference) }
}

/// A filler candidate, in source time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filler {
    /// `word` (a listed filler word) or `voiced` (speech no recognised word covers).
    pub kind: String,
    pub text: String,
    pub start_us: i64,
    pub end_us: i64,
    /// Selected for removal by default (never for context-dependent words).
    pub selected: bool,
    /// A word that is a filler only in some contexts ("like", "you know").
    pub context: bool,
}

/// Filler candidates of one media file: listed filler words, context-dependent words (flagged, not selected), and
/// voiced gaps (speech runs of 0.2–1.2 s that no word overlaps) when a speech mask is given.
pub fn fillers(t: &Transcript, mask: Option<&SpeechMask>) -> Vec<Filler> {
    let mut out: Vec<Filler> = Vec::new();
    for (i, w) in t.words.iter().enumerate() {
        let filler = crate::fillers::is_filler(&w.text, &t.language);
        let next = t.words.get(i + 1).map(|n| n.text.as_str()).unwrap_or("");
        let two = format!("{} {}", crate::fillers::normalise(&w.text), crate::fillers::normalise(next));
        let context = crate::fillers::is_context_filler(&w.text, &t.language)
            || crate::fillers::is_context_filler(&two, &t.language);
        if filler || context {
            out.push(Filler {
                kind: "word".into(),
                text: w.text.clone(),
                start_us: w.start_us,
                end_us: w.end_us,
                selected: filler && !context,
                context,
            });
        }
    }
    if let Some(mask) = mask {
        for (a, b) in mask.ranges(THRESHOLD) {
            let len = b - a;
            if !(200_000..=1_200_000).contains(&len) {
                continue;
            }
            if t.words.iter().any(|w| w.start_us < b && w.end_us > a) {
                continue;
            }
            out.push(Filler {
                kind: "voiced".into(),
                text: String::new(),
                start_us: a,
                end_us: b,
                selected: true,
                context: false,
            });
        }
    }
    out.sort_by_key(|f| f.start_us);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Word;
    use crate::vad::CELL_US;

    /// Speech 0–2 s, silence 2–4 s, speech 4–6 s, music 6–10 s, speech 10–12 s.
    fn fixture(music_db: f32) -> (SpeechMask, Envelope) {
        let cells = (12_000_000 / CELL_US) as usize;
        let probs = (0..cells)
            .map(|c| {
                let t = c as i64 * CELL_US;
                if t < 2_000_000 || (4_000_000..6_000_000).contains(&t) || t >= 10_000_000 { 0.9 } else { 0.05 }
            })
            .collect();
        let db = (0..1200)
            .map(|c| {
                let t = c as i64 * 10_000;
                if (2_000_000..4_000_000).contains(&t) {
                    -90.0
                } else if (6_000_000..10_000_000).contains(&t) {
                    music_db
                } else {
                    -20.0
                }
            })
            .collect();
        (SpeechMask { cell_us: CELL_US, probs }, Envelope { cell_us: 10_000, db })
    }

    #[test]
    fn silence_is_removed_and_the_music_bed_kept() {
        let (mask, env) = fixture(-26.0);
        let d = detect(&mask, &env, 12_000_000, DeadAirOptions::default());
        assert!(d.speech_found);
        // The silence minus 0.15 s padding on each side; the music (6 dB below speech) stays.
        let (a, b) = d.ranges[0];
        assert!((a - 2_150_000).abs() <= CELL_US && (b - 3_850_000).abs() <= CELL_US, "{a}..{b}");
        assert_eq!(d.ranges.len(), 1, "the music bed is kept: {:?}", d.ranges);
        // A quiet music bed (20 dB below the speech) counts as dead air.
        let (mask, env) = fixture(-40.0);
        assert_eq!(detect(&mask, &env, 12_000_000, DeadAirOptions::default()).ranges.len(), 2);
    }

    #[test]
    fn no_speech_removes_nothing_and_says_so() {
        let mask = SpeechMask { cell_us: CELL_US, probs: vec![0.01; 100] };
        let env = Envelope { cell_us: 10_000, db: vec![-90.0; 320] };
        let d = detect(&mask, &env, 3_200_000, DeadAirOptions::default());
        assert!(!d.speech_found && d.ranges.is_empty());
    }

    #[test]
    fn min_pause_and_padding_follow_the_settings() {
        let (mask, env) = fixture(-26.0);
        let o = DeadAirOptions { min_pause_us: 2_500_000, ..Default::default() };
        assert!(detect(&mask, &env, 12_000_000, o).ranges.is_empty(), "the 2 s pause is shorter than the minimum");
        let o = DeadAirOptions { padding_us: 0, ..Default::default() };
        let (a, b) = detect(&mask, &env, 12_000_000, o).ranges[0];
        assert!((a - 2_000_000).abs() <= CELL_US && (b - 4_000_000).abs() <= CELL_US);
        assert_eq!(
            DeadAirOptions { min_pause_us: 1, padding_us: 9_000_000, relative_db: 12.0 }.clamped().min_pause_us,
            250_000
        );
    }

    #[test]
    fn filler_candidates_flag_context_words_and_find_voiced_gaps() {
        let w = |t: &str, s: f64, e: f64| Word {
            text: t.into(),
            start_us: (s * 1e6) as i64,
            end_us: (e * 1e6) as i64,
            confidence: 0.9,
        };
        let t = Transcript {
            language: "en".into(),
            model: "base".into(),
            words: vec![
                w("So", 0.0, 0.2),
                w("um,", 0.3, 0.6),
                w("I", 0.7, 0.8),
                w("like", 0.9, 1.1),
                w("cats.", 1.2, 1.5),
            ],
            duration_us: 5_000_000,
        };
        // A voiced sound at 2.0–2.5 s that no word covers.
        let probs = (0..100).map(|c| if (62..78).contains(&c) || c < 47 { 0.9 } else { 0.1 }).collect();
        let f = fillers(&t, Some(&SpeechMask { cell_us: CELL_US, probs }));
        let selected: Vec<&str> = f.iter().filter(|x| x.selected).map(|x| x.kind.as_str()).collect();
        assert_eq!(selected, ["word", "voiced"]);
        assert!(f.iter().any(|x| x.text == "like" && x.context && !x.selected));
        assert!(f.iter().any(|x| x.text == "So" && x.context && !x.selected));
        assert!(!f.iter().any(|x| x.text == "cats." || x.text == "I"), "no content word");
    }
}
