//! Splits recognised words into caption segments: at sentence ends, long pauses, a maximum length and duration.

use serde::{Deserialize, Serialize};

use crate::recognize::Word;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentOptions {
    /// Characters per caption (one or two short lines).
    pub max_chars: usize,
    pub max_duration_us: i64,
    /// A silence at least this long starts a new caption.
    pub pause_us: i64,
}

impl Default for SegmentOptions {
    fn default() -> Self {
        SegmentOptions { max_chars: 42, max_duration_us: 3_500_000, pause_us: 600_000 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub text: String,
    pub start_us: i64,
    pub end_us: i64,
    pub words: Vec<Word>,
}

fn ends_sentence(w: &str) -> bool {
    w.ends_with(['.', '?', '!', '…']) || w.ends_with("。")
}

fn ends_clause(w: &str) -> bool {
    w.ends_with([',', ';', ':', '—'])
}

pub fn segment(words: &[Word], opts: &SegmentOptions) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut cur: Vec<Word> = Vec::new();
    let flush = |cur: &mut Vec<Word>, out: &mut Vec<Segment>| {
        if cur.is_empty() {
            return;
        }
        let text = cur.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        out.push(Segment {
            text,
            start_us: cur[0].start_us,
            end_us: cur[cur.len() - 1].end_us,
            words: std::mem::take(cur),
        });
    };
    for w in words {
        if let Some(last) = cur.last() {
            let chars: usize = cur.iter().map(|x| x.text.chars().count() + 1).sum::<usize>() + w.text.chars().count();
            let too_long = chars > opts.max_chars;
            let too_slow = w.end_us - cur[0].start_us > opts.max_duration_us;
            let pause = w.start_us - last.end_us >= opts.pause_us;
            // Prefer to break after a clause once the caption is reasonably full.
            let soft = ends_clause(&last.text) && chars > opts.max_chars * 2 / 3;
            if too_long || too_slow || pause || ends_sentence(&last.text) || soft {
                flush(&mut cur, &mut out);
            }
        }
        cur.push(w.clone());
    }
    flush(&mut cur, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(t: &str, s: f64, e: f64) -> Word {
        Word { text: t.into(), start_us: (s * 1e6) as i64, end_us: (e * 1e6) as i64, confidence: 1.0 }
    }

    #[test]
    fn breaks_at_sentences_pauses_and_length() {
        let words = vec![
            w("Hello", 0.0, 0.4),
            w("there.", 0.4, 0.8),
            w("This", 0.9, 1.1),
            w("is", 1.1, 1.2),
            w("a", 1.2, 1.3),
            w("test", 1.3, 1.6),
            w("after", 3.0, 3.3),
            w("a", 3.3, 3.4),
            w("pause", 3.4, 3.8),
        ];
        let s = segment(&words, &SegmentOptions::default());
        let texts: Vec<_> = s.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["Hello there.", "This is a test", "after a pause"]);
        assert_eq!(s[1].start_us, 900_000);
        assert_eq!(s[1].end_us, 1_600_000);
        let long: Vec<Word> = (0..30).map(|i| w("word", i as f64 * 0.2, i as f64 * 0.2 + 0.18)).collect();
        let s = segment(&long, &SegmentOptions::default());
        assert!(s.len() >= 3);
        assert!(s.iter().all(|x| x.text.len() <= 42 && x.end_us - x.start_us <= 3_500_000));
    }
}
