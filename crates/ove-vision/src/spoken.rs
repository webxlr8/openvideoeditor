//! Spoken search (design D5): ranked phrase search over cached transcripts.
//!
//! Words are folded as transcript search folds them (case, accents and punctuation ignored; `ove-transcribe`'s
//! [`fold`]). The query's quoted parts (`"the price is"`) must match consecutive words, also across caption segments;
//! its other words match on their own. Transcript segments (the caption segmentation) are the documents BM25 ranks:
//! a phrase counts as one term. Each hit is one segment's matches: the range runs from the first matched word's start
//! to the last matched word's end (a phrase that runs into the next segment keeps its own last word), padded by
//! 150 ms on each side and kept inside the media.

use std::collections::HashMap;

use ove_transcribe::search::fold;
use ove_transcribe::segment::{SegmentOptions, segment};
use ove_transcribe::transcript::Transcript;
use serde::{Deserialize, Serialize};

/// Padding added on each side of a hit's word range.
pub const PAD_US: i64 = 150_000;
/// BM25 term-frequency saturation.
const K1: f64 = 1.2;
/// BM25 length normalisation.
const B: f64 = 0.75;

/// One term of a query: a single folded word, or a phrase of several.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Term(pub Vec<String>);

/// Splits a query into terms: quoted parts are phrases, other words single terms. An unclosed quote runs to the end.
pub fn parse_query(q: &str) -> Vec<Term> {
    let mut terms: Vec<Term> = Vec::new();
    for (i, part) in q.split('"').enumerate() {
        let words: Vec<String> = part.split_whitespace().map(fold).filter(|w| !w.is_empty()).collect();
        if words.is_empty() {
            continue;
        }
        if i % 2 == 1 {
            terms.push(Term(words));
        } else {
            terms.extend(words.into_iter().map(|w| Term(vec![w])));
        }
    }
    let mut seen = std::collections::HashSet::new();
    terms.retain(|t| seen.insert(t.clone()));
    terms
}

/// One hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpokenHit {
    pub media_id: String,
    /// The words of the hit's range, as transcribed.
    pub text: String,
    /// The segment the hit is in, for context.
    pub context: String,
    /// Source seconds, padded.
    pub start: f64,
    pub end: f64,
    /// Unpadded word range, seconds.
    pub word_start: f64,
    pub word_end: f64,
    pub score: f64,
}

/// A term (by its index in the query) and its matches in one segment: (first word, last word) word indices.
type TermMatches = (usize, Vec<(usize, usize)>);

struct Doc {
    media: usize,
    /// Word indices `[from, to)` in the media's word list.
    from: usize,
    to: usize,
    /// Folded, non-empty words in the segment.
    len: usize,
}

struct Media {
    id: String,
    transcript: Transcript,
    /// Folded word per transcript word ("" for punctuation-only words).
    folded: Vec<String>,
    /// Indices of non-empty folded words.
    content: Vec<usize>,
}

/// The transcripts of a project, segmented and folded once per search session.
pub struct SpokenIndex {
    media: Vec<Media>,
    docs: Vec<Doc>,
    avg_len: f64,
}

impl std::fmt::Debug for SpokenIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpokenIndex").field("media", &self.media.len()).field("segments", &self.docs.len()).finish()
    }
}

impl SpokenIndex {
    pub fn new(transcripts: impl IntoIterator<Item = (String, Transcript)>) -> SpokenIndex {
        let mut media = Vec::new();
        let mut docs = Vec::new();
        for (m, (id, t)) in transcripts.into_iter().enumerate() {
            let folded: Vec<String> = t.words.iter().map(|w| fold(&w.text)).collect();
            let content = (0..folded.len()).filter(|&i| !folded[i].is_empty()).collect();
            let mut from = 0;
            for s in segment(&t.words, &SegmentOptions::default()) {
                let to = from + s.words.len();
                let len = folded[from..to].iter().filter(|w| !w.is_empty()).count();
                docs.push(Doc { media: m, from, to, len });
                from = to;
            }
            media.push(Media { id, transcript: t, folded, content });
        }
        let avg_len = docs.iter().map(|d| d.len as f64).sum::<f64>() / docs.len().max(1) as f64;
        SpokenIndex { media, docs, avg_len }
    }

    pub fn words(&self) -> usize {
        self.media.iter().map(|m| m.content.len()).sum()
    }

    /// Occurrences of a term in one media file: (first word index, last word index), skipping punctuation-only words.
    fn occurrences(&self, m: &Media, t: &Term) -> Vec<(usize, usize)> {
        let n = t.0.len();
        if m.content.len() < n {
            return Vec::new();
        }
        (0..=m.content.len() - n)
            .filter(|&k| (0..n).all(|j| m.folded[m.content[k + j]] == t.0[j]))
            .map(|k| (m.content[k], m.content[k + n - 1]))
            .collect()
    }

    /// Ranked hits for `query`, best first, at most `limit`.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SpokenHit> {
        let terms = parse_query(query);
        if terms.is_empty() {
            return Vec::new();
        }
        // Per document: per term, the matches that start in it.
        let mut per_doc: HashMap<usize, Vec<TermMatches>> = HashMap::new();
        let mut df = vec![0usize; terms.len()];
        for (mi, m) in self.media.iter().enumerate() {
            let docs: Vec<usize> = (0..self.docs.len()).filter(|&d| self.docs[d].media == mi).collect();
            for (ti, t) in terms.iter().enumerate() {
                let occ = self.occurrences(m, t);
                let mut by_doc: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
                for o in occ {
                    // Segments of one media are contiguous and in order.
                    let k = docs.partition_point(|&d| self.docs[d].from <= o.0);
                    if let Some(&d) = k.checked_sub(1).and_then(|k| docs.get(k))
                        && o.0 < self.docs[d].to
                    {
                        by_doc.entry(d).or_default().push(o);
                    }
                }
                df[ti] += by_doc.len();
                for (d, os) in by_doc {
                    per_doc.entry(d).or_default().push((ti, os));
                }
            }
        }
        let n_docs = self.docs.len() as f64;
        let mut hits: Vec<SpokenHit> = per_doc
            .into_iter()
            .map(|(d, matches)| {
                let doc = &self.docs[d];
                let m = &self.media[doc.media];
                let mut score = 0.0;
                for (ti, os) in &matches {
                    let n = df[*ti] as f64;
                    let idf = ((n_docs - n + 0.5) / (n + 0.5) + 1.0).ln();
                    let tf = os.len() as f64;
                    let norm = K1 * (1.0 - B + B * doc.len as f64 / self.avg_len.max(1.0));
                    // A phrase is rarer and more specific than its words: weight it by its length.
                    score += idf * tf * (K1 + 1.0) / (tf + norm) * terms[*ti].0.len() as f64;
                }
                let first = matches.iter().flat_map(|(_, os)| os.iter().map(|o| o.0)).min().unwrap_or(doc.from);
                let last = matches.iter().flat_map(|(_, os)| os.iter().map(|o| o.1)).max().unwrap_or(first);
                let words = &m.transcript.words;
                let (a, b) = (words[first].start_us, words[last].end_us);
                let limit_us = m.transcript.duration_us.max(b);
                let join =
                    |r: std::ops::Range<usize>| words[r].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                SpokenHit {
                    media_id: m.id.clone(),
                    text: join(first..last + 1),
                    context: join(doc.from..doc.to),
                    start: (a - PAD_US).max(0) as f64 / 1e6,
                    end: (b + PAD_US).min(limit_us) as f64 / 1e6,
                    word_start: a as f64 / 1e6,
                    word_end: b as f64 / 1e6,
                    score: (score * 1000.0).round() / 1000.0,
                }
            })
            .collect();
        hits.sort_by(|x, y| {
            y.score
                .total_cmp(&x.score)
                .then_with(|| x.media_id.cmp(&y.media_id))
                .then_with(|| x.word_start.total_cmp(&y.word_start))
        });
        hits.truncate(limit);
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_transcribe::transcript::Word;

    /// Words with start times; each lasts 300 ms.
    fn t(words: &[(&str, f64)]) -> Transcript {
        Transcript {
            language: "en".into(),
            model: "base".into(),
            words: words
                .iter()
                .map(|(w, s)| Word {
                    text: (*w).into(),
                    start_us: (s * 1e6).round() as i64,
                    end_us: (s * 1e6).round() as i64 + 300_000,
                    confidence: 0.9,
                })
                .collect(),
            duration_us: 120_000_000,
        }
    }

    #[test]
    fn queries_split_into_phrases_and_words() {
        assert_eq!(
            parse_query(r#"Café "the Price is" boat"#),
            [
                Term(vec!["cafe".into()]),
                Term(vec!["the".into(), "price".into(), "is".into()]),
                Term(vec!["boat".into()])
            ]
        );
        assert_eq!(parse_query(r#""open quote"#), [Term(vec!["open".into(), "quote".into()])]);
        assert!(parse_query(" \" , \" ").is_empty());
    }

    #[test]
    fn a_phrase_hit_is_its_words_padded_by_150_ms() {
        let i = SpokenIndex::new([(
            "m".to_string(),
            t(&[("So", 10.0), ("the", 10.4), ("price", 10.8), ("is", 11.2), ("right.", 11.6)]),
        )]);
        let h = i.search(r#""the price is""#, 10);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].text, "the price is");
        assert_eq!((h[0].word_start, h[0].word_end), (10.4, 11.5));
        assert_eq!((h[0].start, h[0].end), (10.25, 11.65));
        // Out of order is not the phrase.
        assert!(i.search(r#""price the""#, 10).is_empty());
    }

    #[test]
    fn accents_case_and_punctuation_are_ignored() {
        let i = SpokenIndex::new([("m".to_string(), t(&[("Un", 1.0), ("CAFÉ,", 1.4), ("crème", 1.8), ("!", 2.2)]))]);
        let h = i.search(r#""cafe creme""#, 10);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].text, "CAFÉ, crème");
        assert_eq!(i.search("Crème", 10)[0].word_start, 1.8);
    }

    #[test]
    fn a_phrase_crosses_a_segment_boundary() {
        // A sentence end splits the segments between "end." and "Of"; the phrase still matches as one hit.
        let i = SpokenIndex::new([(
            "m".to_string(),
            t(&[("At", 0.0), ("the", 0.4), ("end.", 0.8), ("Of", 1.6), ("the", 2.0), ("day", 2.4)]),
        )]);
        let h = i.search(r#""end of the day""#, 10);
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].word_start, h[0].word_end), (0.8, 2.7));
        assert_eq!(h[0].text, "end. Of the day");
    }

    #[test]
    fn rarer_and_repeated_terms_rank_higher() {
        // Two media: "price" appears twice in one segment of b and once in a; "boat" is rare.
        let a = t(&[
            ("the", 0.0),
            ("price", 0.4),
            ("went", 0.8),
            ("up.", 1.2),
            ("The", 5.0),
            ("boat", 5.4),
            ("sank.", 5.8),
        ]);
        let b = t(&[("price", 0.0), ("after", 0.4), ("price.", 0.8), ("the", 5.0), ("end.", 5.4)]);
        let i = SpokenIndex::new([("a".to_string(), a), ("b".to_string(), b)]);
        let h = i.search("price", 10);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].media_id, "b", "{h:?}");
        let h = i.search("the boat", 10);
        assert_eq!((h[0].media_id.as_str(), h[0].text.as_str()), ("a", "The boat"));
        assert!(h.iter().all(|x| x.score > 0.0));
    }

    #[test]
    fn padding_stays_inside_the_media() {
        let mut tr = t(&[("hello", 0.05), ("bye", 119.8)]);
        tr.duration_us = 120_000_000;
        let i = SpokenIndex::new([("m".to_string(), tr)]);
        assert_eq!(i.search("hello", 5)[0].start, 0.0);
        assert_eq!(i.search("bye", 5)[0].end, 120.1);
    }

    #[test]
    fn a_hundred_thousand_words_search_quickly() {
        let words: Vec<(String, f64)> = (0..100_000).map(|i| (format!("word{}", i % 997), i as f64 * 0.3)).collect();
        let tr = t(&words.iter().map(|(w, s)| (w.as_str(), *s)).collect::<Vec<_>>());
        let t0 = std::time::Instant::now();
        let i = SpokenIndex::new([("m".to_string(), tr)]);
        let build = t0.elapsed();
        let t0 = std::time::Instant::now();
        let h = i.search(r#""word5 word6" word7"#, 50);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        eprintln!("100,000 words: index {build:?}, search {ms:.1} ms, {} hits", h.len());
        assert_eq!(h.len(), 50);
        assert!(ms < 300.0, "{ms} ms");
    }
}
