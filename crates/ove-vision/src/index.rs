//! The index of one media file (design D3): `<bundle>/cache/embeddings/<contentId>.safetensors`.
//!
//! A plain safetensors file, so common numeric tools can read it: `embeddings` f16 `[N, 768]` (one row per sampled
//! frame, L2-normalised), `times` f64 `[N]` (seconds), `row_shot` u32 `[N]`, `shots` f64 `[S, 2]` (start, end
//! seconds), `cuts` f64 `[C]` and `cut_scores` f64 `[C]` (so scene detection can answer from the index). The metadata names the model, the sampler version, the preprocessing, the duration
//! and a CRC-32 of the tensor bytes. Files are read through a memory map; a damaged file, or one made by another
//! model or sampler, is reported so the caller can discard and rebuild it.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use half::f16;
use half::slice::HalfFloatSliceExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::error::{Result, VisionError};

/// Embedding width.
pub const DIM: usize = 768;

/// The format name in the metadata.
pub const FORMAT: &str = "ove-media-index-1";

/// The bundle folder that holds index files.
pub fn index_dir(bundle: &Path) -> PathBuf {
    bundle.join("cache").join("embeddings")
}

/// The index file of a media file with content id `content_id`.
pub fn index_path(bundle: &Path, content_id: &str) -> PathBuf {
    index_dir(bundle).join(format!("{content_id}.safetensors"))
}

/// What an index was built with; an index whose fields differ from the current ones is rebuilt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub model: String,
    /// SHA-256 of the image encoder file.
    pub model_hash: String,
    pub sampler: u32,
    pub preprocessing: String,
}

/// The content of an index, in memory (for writing, and as read by tests).
#[derive(Debug, Clone, PartialEq)]
pub struct IndexData {
    pub provenance: Provenance,
    pub content_id: String,
    pub duration: f64,
    /// Row-major `[N, DIM]`.
    pub embeddings: Vec<f32>,
    pub times: Vec<f64>,
    pub row_shot: Vec<u32>,
    pub shots: Vec<(f64, f64)>,
    pub cuts: Vec<f64>,
    /// The score of each cut (scene detection reports it).
    pub cut_scores: Vec<f64>,
    /// Frames decoded by the sampler (0 for a still image).
    pub frames: u64,
}

impl IndexData {
    pub fn rows(&self) -> usize {
        self.times.len()
    }

    fn check(&self) -> std::result::Result<(), String> {
        let n = self.times.len();
        if self.embeddings.len() != n * DIM {
            return Err(format!("{} embedding values for {n} rows", self.embeddings.len()));
        }
        if self.row_shot.len() != n {
            return Err("row_shot does not match the rows".into());
        }
        if self.row_shot.iter().any(|&s| s as usize >= self.shots.len()) {
            return Err("a row names a shot that does not exist".into());
        }
        if !self.times.iter().all(|t| t.is_finite()) || !self.cuts.iter().all(|t| t.is_finite()) {
            return Err("a time is not a number".into());
        }
        if self.cut_scores.len() != self.cuts.len() {
            return Err("cut_scores does not match the cuts".into());
        }
        if self.shots.iter().any(|(a, b)| !(a.is_finite() && b.is_finite() && a <= b)) {
            return Err("a shot range is invalid".into());
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TensorInfo {
    dtype: String,
    shape: Vec<usize>,
    data_offsets: [usize; 2],
}

/// Writes `data` to `path` atomically (a temporary file renamed into place).
pub fn write(path: &Path, data: &IndexData) -> Result<()> {
    let bad = |reason: String| VisionError::Index { path: path.display().to_string(), reason };
    data.check().map_err(bad)?;
    let n = data.rows();
    let mut f16s = vec![f16::ZERO; data.embeddings.len()];
    f16s.convert_from_f32_slice(&data.embeddings);
    let shots: Vec<f64> = data.shots.iter().flat_map(|&(a, b)| [a, b]).collect();
    // f64 tensors first so every tensor stays aligned to its element size.
    let mut parts: Vec<(&str, &str, Vec<usize>, Vec<u8>)> = vec![
        ("times", "F64", vec![n], data.times.iter().flat_map(|v| v.to_le_bytes()).collect()),
        ("shots", "F64", vec![data.shots.len(), 2], shots.iter().flat_map(|v| v.to_le_bytes()).collect()),
        ("cuts", "F64", vec![data.cuts.len()], data.cuts.iter().flat_map(|v| v.to_le_bytes()).collect()),
        (
            "cut_scores",
            "F64",
            vec![data.cut_scores.len()],
            data.cut_scores.iter().flat_map(|v| v.to_le_bytes()).collect(),
        ),
        ("embeddings", "F16", vec![n, DIM], f16s.iter().flat_map(|v| v.to_le_bytes()).collect()),
        ("row_shot", "U32", vec![n], data.row_shot.iter().flat_map(|v| v.to_le_bytes()).collect()),
    ];
    let mut header = Map::new();
    let mut offset = 0usize;
    let mut crc = crc32fast::Hasher::new();
    for (name, dtype, shape, bytes) in &mut parts {
        crc.update(bytes);
        let end = offset + bytes.len();
        header.insert(
            (*name).to_string(),
            json!(TensorInfo { dtype: (*dtype).into(), shape: std::mem::take(shape), data_offsets: [offset, end] }),
        );
        offset = end;
    }
    let p = &data.provenance;
    let meta: BTreeMap<&str, String> = [
        ("format", FORMAT.to_string()),
        ("model", p.model.clone()),
        ("modelHash", p.model_hash.clone()),
        ("sampler", p.sampler.to_string()),
        ("preprocessing", p.preprocessing.clone()),
        ("contentId", data.content_id.clone()),
        ("duration", data.duration.to_string()),
        ("frames", data.frames.to_string()),
        ("crc32", format!("{:08x}", crc.finalize())),
    ]
    .into_iter()
    .collect();
    header.insert("__metadata__".into(), json!(meta));
    let mut h = serde_json::to_vec(&Value::Object(header)).map_err(|e| bad(e.to_string()))?;
    while (8 + h.len()) % 8 != 0 {
        h.push(b' ');
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("safetensors.part");
    {
        let mut f = std::io::BufWriter::new(File::create(&tmp)?);
        f.write_all(&(h.len() as u64).to_le_bytes())?;
        f.write_all(&h)?;
        for (_, _, _, bytes) in &parts {
            f.write_all(bytes)?;
        }
        f.flush()?;
        f.get_ref().sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// An index file opened through a memory map.
pub struct Index {
    map: memmap2::Mmap,
    path: PathBuf,
    pub provenance: Provenance,
    pub content_id: String,
    pub duration: f64,
    rows: usize,
    embeddings: (usize, usize),
    times: Vec<f64>,
    row_shot: Vec<u32>,
    pub shots: Vec<(f64, f64)>,
    pub cuts: Vec<f64>,
    pub cut_scores: Vec<f64>,
    pub frames: u64,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index").field("path", &self.path).field("rows", &self.rows).finish()
    }
}

fn f64s(b: &[u8]) -> Vec<f64> {
    b.chunks_exact(8).map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])).collect()
}

impl Index {
    /// Opens and checks an index file: the layout, the tensor shapes and the CRC of the tensor bytes.
    pub fn open(path: &Path) -> Result<Index> {
        let bad = |reason: &str| VisionError::Index { path: path.display().to_string(), reason: reason.into() };
        let file = File::open(path)?;
        // SAFETY: the map is read-only and index files are only ever replaced by renaming a new file into place,
        // never written in place, so the mapped bytes do not change while mapped (macOS keeps the old file for the
        // map after a rename; Windows refuses to replace a mapped file, so the writer fails and retries later).
        let map = unsafe { memmap2::Mmap::map(&file) }?;
        if map.len() < 8 {
            return Err(bad("the file is too short"));
        }
        let hlen = u64::from_le_bytes([map[0], map[1], map[2], map[3], map[4], map[5], map[6], map[7]]);
        let hlen = usize::try_from(hlen).map_err(|_| bad("the header length is invalid"))?;
        if hlen > 1 << 20 || 8 + hlen > map.len() {
            return Err(bad("the header length is invalid"));
        }
        let header: Map<String, Value> =
            serde_json::from_slice(&map[8..8 + hlen]).map_err(|_| bad("the header is not valid"))?;
        let base = 8 + hlen;
        let data_len = map.len() - base;
        let meta = header.get("__metadata__").and_then(Value::as_object).ok_or_else(|| bad("no metadata"))?;
        let m = |k: &str| meta.get(k).and_then(Value::as_str).map(str::to_string);
        if m("format").as_deref() != Some(FORMAT) {
            return Err(bad("not a media index of this version"));
        }
        let tensor = |name: &str, dtype: &str, elem: usize| -> Result<(Vec<usize>, usize, usize)> {
            let t: TensorInfo = header
                .get(name)
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .ok_or_else(|| bad(&format!("no {name} tensor")))?;
            let [a, b] = t.data_offsets;
            let count: usize = t.shape.iter().product();
            if t.dtype != dtype || a > b || b > data_len || b - a != count * elem || a % elem != 0 {
                return Err(bad(&format!("the {name} tensor is damaged")));
            }
            Ok((t.shape, base + a, base + b))
        };
        let (ts, t0, t1) = tensor("times", "F64", 8)?;
        let (ss, s0, s1) = tensor("shots", "F64", 8)?;
        let (cs, c0, c1) = tensor("cuts", "F64", 8)?;
        let (ks, k0, k1) = tensor("cut_scores", "F64", 8)?;
        let (es, e0, e1) = tensor("embeddings", "F16", 2)?;
        let (rs, r0, r1) = tensor("row_shot", "U32", 4)?;
        let rows = ts.first().copied().unwrap_or(0);
        if es != [rows, DIM] || rs != [rows] || ss.len() != 2 || ss[1] != 2 || ks != cs {
            return Err(bad("the tensor shapes do not agree"));
        }
        let mut crc = crc32fast::Hasher::new();
        let mut ranges = [(t0, t1), (s0, s1), (c0, c1), (k0, k1), (e0, e1), (r0, r1)];
        ranges.sort();
        for (a, b) in ranges {
            crc.update(&map[a..b]);
        }
        if m("crc32").as_deref() != Some(format!("{:08x}", crc.finalize()).as_str()) {
            return Err(bad("the checksum does not match (the file is damaged)"));
        }
        let shots_flat = f64s(&map[s0..s1]);
        let row_shot: Vec<u32> =
            map[r0..r1].chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let shots: Vec<(f64, f64)> = shots_flat.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        if row_shot.iter().any(|&s| s as usize >= shots.len()) {
            return Err(bad("a row names a shot that does not exist"));
        }
        let provenance = Provenance {
            model: m("model").unwrap_or_default(),
            model_hash: m("modelHash").unwrap_or_default(),
            sampler: m("sampler").and_then(|s| s.parse().ok()).unwrap_or(0),
            preprocessing: m("preprocessing").unwrap_or_default(),
        };
        Ok(Index {
            times: f64s(&map[t0..t1]),
            cuts: f64s(&map[c0..c1]),
            cut_scores: f64s(&map[k0..k1]),
            frames: m("frames").and_then(|s| s.parse().ok()).unwrap_or(0),
            shots,
            row_shot,
            rows,
            embeddings: (e0, e1),
            content_id: m("contentId").unwrap_or_default(),
            duration: m("duration").and_then(|s| s.parse().ok()).unwrap_or(0.0),
            provenance,
            path: path.to_path_buf(),
            map,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn time(&self, row: usize) -> f64 {
        self.times[row]
    }

    pub fn shot_of(&self, row: usize) -> usize {
        self.row_shot[row] as usize
    }

    /// The raw f16 bytes of the embedding matrix (little-endian).
    fn embedding_bytes(&self) -> &[u8] {
        &self.map[self.embeddings.0..self.embeddings.1]
    }

    /// One row's embedding as f32.
    pub fn embedding(&self, row: usize) -> Vec<f32> {
        let b = &self.embedding_bytes()[row * DIM * 2..(row + 1) * DIM * 2];
        b.chunks_exact(2).map(|c| f16::from_le_bytes([c[0], c[1]]).to_f32()).collect()
    }

    /// Dot products of every row with `query` (length [`DIM`]), streamed over the mapped matrix.
    pub fn scores(&self, query: &[f32]) -> Vec<f32> {
        scores(self.embedding_bytes(), query)
    }
}

/// Dot products of each f16 row in `bytes` with `query`, converting a block of rows at a time.
pub fn scores(bytes: &[u8], query: &[f32]) -> Vec<f32> {
    const BLOCK: usize = 64;
    let rows = bytes.len() / (DIM * 2);
    let mut out = Vec::with_capacity(rows);
    let mut halves = vec![f16::ZERO; BLOCK * DIM];
    let mut floats = vec![0f32; BLOCK * DIM];
    let mut r = 0;
    while r < rows {
        let k = BLOCK.min(rows - r);
        let src = &bytes[r * DIM * 2..(r + k) * DIM * 2];
        for (h, c) in halves[..k * DIM].iter_mut().zip(src.chunks_exact(2)) {
            *h = f16::from_bits(u16::from_le_bytes([c[0], c[1]]));
        }
        halves[..k * DIM].convert_to_f32_slice(&mut floats[..k * DIM]);
        for row in floats[..k * DIM].chunks_exact(DIM) {
            out.push(dot(row, query));
        }
        r += k;
    }
    out
}

/// Dot product in eight lanes, so the compiler can vectorise it.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let (ac, bc) = (a.chunks_exact(8), b.chunks_exact(8));
    let tail: f32 = ac.remainder().iter().zip(bc.remainder()).map(|(x, y)| x * y).sum();
    for (x, y) in ac.zip(bc) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    acc.iter().sum::<f32>() + tail
}

/// Scales a vector to unit length (left as it is when it is all zeros).
pub fn normalise(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        for x in v {
            *x /= n;
        }
    }
}

/// The outcome of looking for a media file's index.
#[derive(Debug)]
pub enum Lookup {
    Ready(Index),
    /// No index yet.
    Missing,
    /// An index built by another model or sampler; it is rebuilt.
    Stale(Index),
    /// A damaged file, which has been deleted so it is rebuilt.
    Discarded(String),
}

/// Opens the index of `content_id` and checks it against the current `provenance`. A damaged file is deleted.
pub fn lookup(bundle: &Path, content_id: &str, provenance: Option<&Provenance>) -> Lookup {
    let path = index_path(bundle, content_id);
    if !path.exists() {
        return Lookup::Missing;
    }
    match Index::open(&path) {
        Ok(i) if provenance.is_some_and(|p| *p != i.provenance) => Lookup::Stale(i),
        Ok(i) => Lookup::Ready(i),
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            Lookup::Discarded(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample(rows: usize) -> IndexData {
        let mut embeddings = Vec::with_capacity(rows * DIM);
        for r in 0..rows {
            let mut v: Vec<f32> = (0..DIM).map(|i| ((r * 31 + i * 7) % 97) as f32 - 48.0).collect();
            normalise(&mut v);
            embeddings.extend(v);
        }
        IndexData {
            provenance: Provenance {
                model: "test".into(),
                model_hash: "0".repeat(64),
                sampler: 1,
                preprocessing: "test".into(),
            },
            content_id: "abc".into(),
            duration: 12.5,
            embeddings,
            times: (0..rows).map(|r| r as f64 * 0.5).collect(),
            row_shot: (0..rows).map(|r| (r / 2) as u32).collect(),
            shots: (0..rows.div_ceil(2)).map(|s| (s as f64, s as f64 + 1.0)).collect(),
            cuts: (1..rows.div_ceil(2)).map(|s| s as f64).collect(),
            cut_scores: (1..rows.div_ceil(2)).map(|_| 0.5).collect(),
            frames: rows as u64 * 10,
        }
    }

    #[test]
    fn round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = index_path(dir.path(), "abc");
        let d = sample(5);
        write(&p, &d).unwrap();
        let i = Index::open(&p).unwrap();
        assert_eq!(i.rows(), 5);
        assert_eq!(i.provenance, d.provenance);
        assert_eq!((i.content_id.as_str(), i.duration), ("abc", 12.5));
        assert_eq!(i.shots, d.shots);
        assert_eq!(i.cuts, d.cuts);
        assert_eq!((i.cut_scores.clone(), i.frames), (d.cut_scores.clone(), d.frames));
        assert_eq!(i.time(3), 1.5);
        assert_eq!(i.shot_of(3), 1);
        // f16 keeps about three decimal digits.
        for r in 0..5 {
            let got = i.embedding(r);
            for (a, b) in got.iter().zip(&d.embeddings[r * DIM..(r + 1) * DIM]) {
                assert!((a - b).abs() < 1e-3, "{a} {b}");
            }
        }
        let q = d.embeddings[2 * DIM..3 * DIM].to_vec();
        let s = i.scores(&q);
        assert!((s[2] - 1.0).abs() < 2e-3);
        assert!(s.iter().all(|&x| x <= s[2] + 1e-3));
    }

    #[test]
    fn the_file_is_plain_safetensors() {
        let dir = tempfile::tempdir().unwrap();
        let p = index_path(dir.path(), "abc");
        write(&p, &sample(3)).unwrap();
        let b = std::fs::read(&p).unwrap();
        let n = u64::from_le_bytes(b[..8].try_into().unwrap()) as usize;
        assert_eq!((8 + n) % 8, 0);
        let h: Value = serde_json::from_slice(&b[8..8 + n]).unwrap();
        assert_eq!(h["embeddings"]["dtype"], "F16");
        assert_eq!(h["embeddings"]["shape"], json!([3, DIM]));
        assert_eq!(h["__metadata__"]["format"], FORMAT);
        let end = h.as_object().unwrap().values().filter_map(|v| v["data_offsets"][1].as_u64()).max().unwrap();
        assert_eq!(8 + n + end as usize, b.len());
    }

    #[test]
    fn a_damaged_file_is_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let p = index_path(dir.path(), "abc");
        write(&p, &sample(4)).unwrap();
        let mut b = std::fs::read(&p).unwrap();
        let last = b.len() - 10;
        b[last] ^= 0xff;
        std::fs::write(&p, &b).unwrap();
        assert!(Index::open(&p).unwrap_err().to_string().contains("checksum"));
        assert!(matches!(lookup(dir.path(), "abc", None), Lookup::Discarded(_)));
        assert!(!p.exists(), "the damaged file is removed so it is rebuilt");
        assert!(matches!(lookup(dir.path(), "abc", None), Lookup::Missing));
        // Truncated and garbage files too.
        write(&p, &sample(4)).unwrap();
        let b = std::fs::read(&p).unwrap();
        std::fs::write(&p, &b[..b.len() / 2]).unwrap();
        assert!(Index::open(&p).is_err());
        std::fs::write(&p, b"nonsense").unwrap();
        assert!(Index::open(&p).is_err());
    }

    #[test]
    fn another_model_makes_the_index_stale() {
        let dir = tempfile::tempdir().unwrap();
        let d = sample(2);
        write(&index_path(dir.path(), "abc"), &d).unwrap();
        assert!(matches!(lookup(dir.path(), "abc", Some(&d.provenance)), Lookup::Ready(_)));
        let mut other = d.provenance.clone();
        other.sampler += 1;
        assert!(matches!(lookup(dir.path(), "abc", Some(&other)), Lookup::Stale(_)));
    }

    #[test]
    fn an_empty_index_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = sample(0);
        d.shots.clear();
        d.cuts.clear();
        d.cut_scores.clear();
        let p = index_path(dir.path(), "abc");
        write(&p, &d).unwrap();
        assert_eq!(Index::open(&p).unwrap().rows(), 0);
    }

    #[test]
    fn inconsistent_data_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = sample(2);
        d.row_shot[0] = 99;
        assert!(write(&index_path(dir.path(), "abc"), &d).is_err());
    }
}
