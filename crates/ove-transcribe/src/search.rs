//! Transcript search (add-transcript-editing D8): a phrase matched word by word, ignoring case, accents and
//! punctuation, across every cached transcript of a project. Hits have word-accurate source times and list every
//! place the source range is used on the timeline.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::timeline::{SourceClip, word_id};
use crate::transcript::Transcript;

/// A word folded for matching: decomposed, accents removed, lower-cased, punctuation dropped.
pub fn fold(word: &str) -> String {
    word.nfd().filter(|c| !is_combining_mark(*c)).filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The folded words of every transcript, built once per search session.
#[derive(Debug, Clone, Default)]
pub struct Index {
    media: Vec<(String, Transcript, Vec<String>)>,
}

/// Where a hit's source range plays on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Use {
    pub clip_id: String,
    /// Timeline seconds.
    pub start: f64,
    pub end: f64,
}

/// One match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub media_id: String,
    /// The words as transcribed.
    pub text: String,
    /// Source seconds: the start of the first word and the end of the last.
    pub source_start: f64,
    pub source_end: f64,
    pub word_ids: Vec<String>,
    /// Every place on the timeline that plays this range (empty when it is not used).
    pub uses: Vec<Use>,
}

impl Index {
    pub fn new(transcripts: impl IntoIterator<Item = (String, Transcript)>) -> Index {
        let media = transcripts
            .into_iter()
            .map(|(id, t)| {
                let folded = t.words.iter().map(|w| fold(&w.text)).collect();
                (id, t, folded)
            })
            .collect();
        Index { media }
    }

    pub fn words(&self) -> usize {
        self.media.iter().map(|m| m.2.len()).sum()
    }

    /// Every occurrence of `phrase`; `clips` gives the timeline uses.
    pub fn find(&self, phrase: &str, clips: &[SourceClip]) -> Vec<Hit> {
        let want: Vec<String> = phrase.split_whitespace().map(fold).filter(|s| !s.is_empty()).collect();
        let mut hits = Vec::new();
        if want.is_empty() {
            return hits;
        }
        for (media, t, folded) in &self.media {
            // Words that fold to nothing (punctuation) are skipped, so phrases match across them.
            let idx: Vec<usize> = (0..folded.len()).filter(|&i| !folded[i].is_empty()).collect();
            let n = want.len();
            if idx.len() < n {
                continue;
            }
            for k in 0..=idx.len() - n {
                if (0..n).all(|j| folded[idx[k + j]] == want[j]) {
                    let words: Vec<_> = (0..n).map(|j| &t.words[idx[k + j]]).collect();
                    let (a, b) = (words[0].start_us, words[n - 1].end_us);
                    let uses = clips
                        .iter()
                        .filter(|c| &c.media_id == media)
                        .filter_map(|c| {
                            let (lo, hi) = c.source_range();
                            let mid = (a + b) / 2;
                            (mid >= lo && mid < hi).then(|| {
                                let (s, e) = (c.to_timeline(a.max(lo)), c.to_timeline(b.min(hi)));
                                Use {
                                    clip_id: c.item_id.clone(),
                                    start: s.min(e) as f64 / 1e6,
                                    end: s.max(e) as f64 / 1e6,
                                }
                            })
                        })
                        .collect();
                    hits.push(Hit {
                        media_id: media.clone(),
                        text: words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
                        source_start: a as f64 / 1e6,
                        source_end: b as f64 / 1e6,
                        word_ids: words.iter().map(|w| word_id(media, w.start_us)).collect(),
                        uses,
                    });
                }
            }
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Word;

    fn t(words: &[(&str, f64)]) -> Transcript {
        Transcript {
            language: "fr".into(),
            model: "base".into(),
            words: words
                .iter()
                .map(|(w, s)| Word {
                    text: (*w).into(),
                    start_us: (s * 1e6) as i64,
                    end_us: (s * 1e6) as i64 + 300_000,
                    confidence: 0.9,
                })
                .collect(),
            duration_us: 60_000_000,
        }
    }

    #[test]
    fn folding_ignores_case_accents_and_punctuation() {
        assert_eq!(fold("Café,"), "cafe");
        assert_eq!(fold("ÉLÈVE!"), "eleve");
        assert_eq!(fold("naïve"), "naive");
        assert_eq!(fold("—"), "");
    }

    #[test]
    fn accent_insensitive_hit_with_word_times() {
        let i = Index::new([("m".to_string(), t(&[("Un", 1.0), ("café", 1.5), ("noir.", 2.0)]))]);
        let h = i.find("cafe", &[]);
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].source_start, h[0].source_end), (1.5, 1.8));
        assert_eq!(h[0].word_ids, ["m@1500"]);
        assert_eq!(i.find("CAFÉ NOIR", &[])[0].text, "café noir.");
    }

    #[test]
    fn a_phrase_across_a_caption_boundary_is_one_hit() {
        // "the end. Of the day" spans two sentences (two captions); punctuation and case do not matter.
        let i = Index::new([(
            "m".to_string(),
            t(&[("At", 0.0), ("the", 0.4), ("end.", 0.8), ("Of", 1.6), ("the", 2.0), ("day", 2.4)]),
        )]);
        let h = i.find("end of the", &[]);
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].source_start, h[0].source_end), (0.8, 2.3));
        assert!(i.find("of end", &[]).is_empty());
    }

    #[test]
    fn hits_list_their_timeline_uses() {
        let i = Index::new([("m".to_string(), t(&[("hello", 5.0)]))]);
        let clip = |id: &str, start: i64, trim: i64| SourceClip {
            item_id: id.into(),
            media_id: "m".into(),
            path: Default::default(),
            start,
            end: start + 10_000_000,
            trim,
            speed: 1.0,
            reverse: false,
        };
        let h = i.find("hello", &[clip("a", 0, 0), clip("b", 20_000_000, 4_000_000), clip("c", 40_000_000, 8_000_000)]);
        assert_eq!(
            h[0].uses.iter().map(|u| (u.clip_id.as_str(), u.start)).collect::<Vec<_>>(),
            [("a", 5.0), ("b", 21.0)]
        );
    }

    #[test]
    fn a_hundred_thousand_words_search_in_under_10_ms() {
        let words: Vec<(String, f64)> = (0..100_000).map(|i| (format!("word{}", i % 997), i as f64 * 0.3)).collect();
        let tr = t(&words.iter().map(|(w, s)| (w.as_str(), *s)).collect::<Vec<_>>());
        let i = Index::new([("m".to_string(), tr)]);
        assert_eq!(i.words(), 100_000);
        let t0 = std::time::Instant::now();
        let h = i.find("word5 word6 word7", &[]);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        eprintln!("100,000 words: {ms:.2} ms, {} hits", h.len());
        assert_eq!(h.len(), 101);
        assert!(ms < 10.0, "{ms} ms");
    }
}
