//! Text tokenisation for the image–text model's text encoder.
//!
//! The model's tokenizer is a SentencePiece byte-pair model (`tokenizer.model`, a protocol-buffer file). Encoding
//! follows the SentencePiece BPE rules: the text is lower-cased (the model was trained on lower-case text), spaces
//! become `▁`, the text is split into characters, and adjacent symbols are merged while their concatenation is a
//! vocabulary piece, the highest-scoring pair first (leftmost on ties). Characters with no piece fall back to their
//! UTF-8 bytes (`<0xNN>` pieces). Pieces marked user-defined (runs of spaces, for example) are matched whole before
//! merging. The ids are followed by the end-of-sequence id and padded with the pad id to the model's fixed length;
//! a longer text is cut so that the end-of-sequence id is kept.

use std::collections::HashMap;

use crate::error::{Result, VisionError};

/// The text encoder's input length.
pub const TEXT_LENGTH: usize = 64;

/// Piece types of the SentencePiece model file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PieceType {
    Normal,
    Unknown,
    Control,
    UserDefined,
    Unused,
    Byte,
}

impl PieceType {
    fn from_proto(v: u64) -> PieceType {
        match v {
            2 => PieceType::Unknown,
            3 => PieceType::Control,
            4 => PieceType::UserDefined,
            5 => PieceType::Unused,
            6 => PieceType::Byte,
            _ => PieceType::Normal,
        }
    }
}

/// A loaded tokenizer.
#[derive(Debug, Clone)]
pub struct Tokenizer {
    /// Piece text to (id, score) for pieces that take part in merging.
    pieces: HashMap<String, (u32, f32)>,
    /// User-defined pieces, longest first, matched whole before merging.
    user_defined: Vec<(String, u32)>,
    /// Id of each byte's `<0xNN>` piece.
    bytes: [u32; 256],
    unk: u32,
    eos: u32,
    pad: u32,
    vocab: usize,
}

const SPACE: char = '\u{2581}';

impl Tokenizer {
    /// Reads a SentencePiece model file.
    pub fn from_model_bytes(data: &[u8]) -> Result<Tokenizer> {
        let bad = |why: &str| VisionError::Tokenizer(format!("the tokenizer model is damaged: {why}"));
        let mut pieces = HashMap::new();
        let mut user_defined = Vec::new();
        let mut bytes = [u32::MAX; 256];
        let (mut unk, mut eos, mut pad) = (None, None, None);
        let mut id = 0u32;
        let mut r = Proto::new(data);
        while let Some((field, value)) = r.next_field().map_err(bad)? {
            let Value::Bytes(msg) = value else { continue };
            if field != 1 {
                continue;
            }
            let (mut text, mut score, mut ty) = (None, 0f32, PieceType::Normal);
            let mut p = Proto::new(msg);
            while let Some((f, v)) = p.next_field().map_err(bad)? {
                match (f, v) {
                    (1, Value::Bytes(b)) => {
                        text = Some(std::str::from_utf8(b).map_err(|_| bad("a piece is not UTF-8"))?.to_string())
                    }
                    (2, Value::Fixed32(x)) => score = f32::from_bits(x),
                    (3, Value::Varint(x)) => ty = PieceType::from_proto(x),
                    _ => {}
                }
            }
            let text = text.ok_or_else(|| bad("a piece has no text"))?;
            match ty {
                PieceType::Normal => {
                    pieces.insert(text, (id, score));
                }
                PieceType::UserDefined => {
                    pieces.insert(text.clone(), (id, score));
                    user_defined.push((text, id));
                }
                PieceType::Byte => {
                    let hex = text.strip_prefix("<0x").and_then(|s| s.strip_suffix('>'));
                    let b = hex.and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or_else(|| bad("a byte piece"))?;
                    bytes[b as usize] = id;
                }
                PieceType::Unknown => unk = Some(id),
                PieceType::Control => match text.as_str() {
                    "<eos>" | "</s>" => eos = Some(id),
                    "<pad>" => pad = Some(id),
                    _ => {}
                },
                PieceType::Unused => {}
            }
            id += 1;
        }
        user_defined.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.1.cmp(&b.1)));
        let unk = unk.ok_or_else(|| bad("no unknown piece"))?;
        if bytes.contains(&u32::MAX) {
            return Err(bad("byte pieces are missing"));
        }
        Ok(Tokenizer {
            pieces,
            user_defined,
            bytes,
            unk,
            eos: eos.ok_or_else(|| bad("no end-of-sequence piece"))?,
            pad: pad.ok_or_else(|| bad("no padding piece"))?,
            vocab: id as usize,
        })
    }

    /// Reads a SentencePiece model file from disk.
    pub fn from_file(path: &std::path::Path) -> Result<Tokenizer> {
        let data = std::fs::read(path)
            .map_err(|e| VisionError::Tokenizer(format!("could not read {}: {e}", path.display())))?;
        Tokenizer::from_model_bytes(&data)
    }

    /// Number of pieces in the vocabulary.
    pub fn vocab_size(&self) -> usize {
        self.vocab
    }

    pub fn eos(&self) -> u32 {
        self.eos
    }

    pub fn pad(&self) -> u32 {
        self.pad
    }

    /// The ids of `text` without end-of-sequence or padding.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let normalised: String = text.chars().map(|c| if c == ' ' { SPACE } else { c }).collect();
        let mut out = Vec::new();
        // User-defined pieces split the text; the parts between them are merged on their own.
        let text = normalised.as_str();
        let mut plain_start = 0usize;
        let mut i = 0usize;
        while i < text.len() {
            let tail = &text[i..];
            if let Some((p, id)) = self.user_defined.iter().find(|(p, _)| tail.starts_with(p.as_str())) {
                self.merge(&text[plain_start..i], &mut out);
                out.push(*id);
                i += p.len();
                plain_start = i;
                continue;
            }
            i += tail.chars().next().map(char::len_utf8).unwrap_or(1);
        }
        self.merge(&text[plain_start..], &mut out);
        out
    }

    /// Byte-pair merging of one part of the text.
    fn merge(&self, text: &str, out: &mut Vec<u32>) {
        if text.is_empty() {
            return;
        }
        // Symbols as byte ranges into `text`, linked left to right.
        let mut start: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        let n = start.len();
        start.push(text.len());
        let mut end: Vec<usize> = start[1..].to_vec();
        start.truncate(n);
        let mut alive = vec![true; n];
        let mut next: Vec<usize> = (1..=n).collect();
        let mut prev: Vec<Option<usize>> = (0..n).map(|i| i.checked_sub(1)).collect();
        // Candidate merges, best score first, then leftmost. Stale entries are skipped when popped.
        let mut heap = std::collections::BinaryHeap::new();
        let push =
            |heap: &mut std::collections::BinaryHeap<Candidate>, l: usize, r: usize, s: &[usize], e: &[usize]| {
                if let Some(&(_, score)) = self.pieces.get(&text[s[l]..e[r]]) {
                    heap.push(Candidate { score, left: l, right: r, len: e[r] - s[l] });
                }
            };
        for i in 0..n.saturating_sub(1) {
            push(&mut heap, i, i + 1, &start, &end);
        }
        while let Some(c) = heap.pop() {
            let (l, r) = (c.left, c.right);
            if !alive[l] || !alive[r] || next[l] != r || end[r] - start[l] != c.len {
                continue;
            }
            end[l] = end[r];
            alive[r] = false;
            next[l] = next[r];
            if next[l] < n {
                prev[next[l]] = Some(l);
            }
            if let Some(p) = prev[l] {
                push(&mut heap, p, l, &start, &end);
            }
            if next[l] < n {
                push(&mut heap, l, next[l], &start, &end);
            }
        }
        let mut i = 0;
        while i < n {
            let piece = &text[start[i]..end[i]];
            match self.pieces.get(piece) {
                Some(&(id, _)) => out.push(id),
                None => {
                    // Byte fallback; an unknown byte cannot happen (every byte has a piece), but keep unk as a guard.
                    for b in piece.bytes() {
                        out.push(*self.bytes.get(b as usize).unwrap_or(&self.unk));
                    }
                }
            }
            i = next[i];
        }
    }

    /// The text encoder's input: the lower-cased text's ids, the end-of-sequence id, then padding, exactly
    /// [`TEXT_LENGTH`] ids. A longer text is cut and keeps its end-of-sequence id.
    pub fn encode_for_model(&self, text: &str) -> Vec<i64> {
        let lower = text.to_lowercase();
        let mut ids = self.encode(&lower);
        ids.truncate(TEXT_LENGTH - 1);
        ids.push(self.eos);
        let mut out: Vec<i64> = ids.into_iter().map(i64::from).collect();
        out.resize(TEXT_LENGTH, i64::from(self.pad));
        out
    }
}

#[derive(Debug, PartialEq)]
struct Candidate {
    score: f32,
    left: usize,
    right: usize,
    len: usize,
}

impl Eq for Candidate {}

impl Ord for Candidate {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.score.total_cmp(&o.score).then_with(|| o.left.cmp(&self.left))
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

/// A protocol-buffer field value.
enum Value<'a> {
    Varint(u64),
    Fixed32(u32),
    Fixed64,
    Bytes(&'a [u8]),
}

/// A minimal protocol-buffer reader: enough for the tokenizer model's pieces.
struct Proto<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Proto<'a> {
    fn new(data: &'a [u8]) -> Proto<'a> {
        Proto { data, pos: 0 }
    }

    fn varint(&mut self) -> std::result::Result<u64, &'static str> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = *self.data.get(self.pos).ok_or("a number runs past the end")?;
            self.pos += 1;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("a number is too long")
    }

    fn take(&mut self, n: usize) -> std::result::Result<&'a [u8], &'static str> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or("a field runs past the end")?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn next_field(&mut self) -> std::result::Result<Option<(u64, Value<'a>)>, &'static str> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        let key = self.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        let v = match wire {
            0 => Value::Varint(self.varint()?),
            1 => {
                self.take(8)?;
                Value::Fixed64
            }
            2 => {
                let n = usize::try_from(self.varint()?).map_err(|_| "a length is too large")?;
                Value::Bytes(self.take(n)?)
            }
            5 => {
                let b = self.take(4)?;
                Value::Fixed32(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            _ => return Err("an unsupported field type"),
        };
        Ok(Some((field, v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a tiny SentencePiece model: (piece, score, type).
    fn model(pieces: &[(&str, f32, u64)]) -> Vec<u8> {
        fn varint(mut v: u64, out: &mut Vec<u8>) {
            loop {
                let b = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    out.push(b);
                    return;
                }
                out.push(b | 0x80);
            }
        }
        let mut out = Vec::new();
        let mut all: Vec<(String, f32, u64)> =
            vec![("<pad>".into(), 0.0, 3), ("<eos>".into(), 0.0, 3), ("<unk>".into(), 0.0, 2)];
        all.extend((0..=255u8).map(|b| (format!("<0x{b:02X}>"), 0.0, 6)));
        all.extend(pieces.iter().map(|(p, s, t)| (p.to_string(), *s, *t)));
        for (p, s, t) in all {
            let mut m = Vec::new();
            m.push(0x0a);
            varint(p.len() as u64, &mut m);
            m.extend_from_slice(p.as_bytes());
            m.push(0x15);
            m.extend_from_slice(&s.to_le_bytes());
            m.push(0x18);
            varint(t, &mut m);
            out.push(0x0a);
            varint(m.len() as u64, &mut out);
            out.extend(m);
        }
        out
    }

    #[test]
    fn merges_take_the_best_pair_first() {
        // "abc": "bc" scores higher than "ab", so the result is a + bc, not ab + c.
        let t = Tokenizer::from_model_bytes(&model(&[
            ("a", -1.0, 1),
            ("b", -1.0, 1),
            ("c", -1.0, 1),
            ("ab", -5.0, 1),
            ("bc", -2.0, 1),
        ]))
        .unwrap();
        let base = 3 + 256;
        assert_eq!(t.encode("abc"), [base as u32, base as u32 + 4]);
    }

    #[test]
    fn unknown_characters_fall_back_to_bytes_and_spaces_become_markers() {
        let t = Tokenizer::from_model_bytes(&model(&[("\u{2581}", -1.0, 1), ("x", -1.0, 1)])).unwrap();
        let base = 3 + 256;
        // "é" is two UTF-8 bytes: <0xC3> <0xA9>.
        assert_eq!(t.encode("x é"), [base as u32 + 1, base as u32, 3 + 0xC3, 3 + 0xA9]);
    }

    #[test]
    fn user_defined_pieces_are_matched_whole() {
        let sp2 = "\u{2581}\u{2581}";
        let t = Tokenizer::from_model_bytes(&model(&[("\u{2581}", -1.0, 1), ("a", -1.0, 1), (sp2, 0.0, 4)])).unwrap();
        let base = 3 + 256;
        assert_eq!(t.encode("  a"), [base as u32 + 2, base as u32 + 1]);
    }

    #[test]
    fn model_input_is_padded_and_keeps_its_end_marker() {
        let t = Tokenizer::from_model_bytes(&model(&[("a", -1.0, 1)])).unwrap();
        let ids = t.encode_for_model("A");
        assert_eq!(ids.len(), TEXT_LENGTH);
        assert_eq!(&ids[..3], &[259, 1, 0]);
        let long = t.encode_for_model(&"a".repeat(200));
        assert_eq!(long.len(), TEXT_LENGTH);
        assert_eq!(long[TEXT_LENGTH - 1], 1);
    }

    #[test]
    fn a_damaged_model_is_refused() {
        let mut m = model(&[("a", -1.0, 1)]);
        m.truncate(m.len() - 3);
        assert!(Tokenizer::from_model_bytes(&m).is_err());
        assert!(Tokenizer::from_model_bytes(&[0xff]).is_err());
    }
}
