//! Text query embeddings cached on disk by query text (design D4): `<bundle>/cache/embeddings/queries/<key>.f32`,
//! the key a hash of the model and the lower-cased query. A repeated search needs no model call.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::index::{DIM, index_dir, normalise};
use crate::model::Encoder;

fn path(bundle: &Path, model: &str, query: &str) -> PathBuf {
    let mut h = Sha256::new();
    h.update(model.as_bytes());
    h.update([0]);
    h.update(query.trim().to_lowercase().as_bytes());
    let hex: String = h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect();
    index_dir(bundle).join("queries").join(format!("{hex}.f32"))
}

/// The cached embedding of `query`, if any.
pub fn cached(bundle: &Path, model: &str, query: &str) -> Option<Vec<f32>> {
    let b = std::fs::read(path(bundle, model, query)).ok()?;
    (b.len() == DIM * 4).then(|| b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

/// The embedding of `query`: from the cache, else from `encoder` (and then cached; a failed write is ignored).
pub fn embed(bundle: &Path, encoder: &mut dyn Encoder, query: &str) -> crate::Result<Vec<f32>> {
    let model = encoder.provenance().model_hash;
    if let Some(v) = cached(bundle, &model, query) {
        return Ok(v);
    }
    let mut v = encoder.text(query)?;
    normalise(&mut v);
    let p = path(bundle, &model, query);
    if let Some(dir) = p.parent()
        && std::fs::create_dir_all(dir).is_ok()
    {
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        let tmp = p.with_extension("part");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Provenance;

    struct Counting(usize);

    impl Encoder for Counting {
        fn images(&mut self, _: &[Vec<f32>]) -> crate::Result<Vec<Vec<f32>>> {
            Ok(Vec::new())
        }
        fn text(&mut self, t: &str) -> crate::Result<Vec<f32>> {
            self.0 += 1;
            let mut v = vec![0f32; DIM];
            v[t.len() % DIM] = 2.0;
            Ok(v)
        }
        fn provenance(&self) -> Provenance {
            Provenance { model: "c".into(), model_hash: "h".into(), sampler: 1, preprocessing: "p".into() }
        }
    }

    #[test]
    fn a_repeated_query_is_answered_from_disk() {
        let d = tempfile::tempdir().unwrap();
        let mut e = Counting(0);
        let a = embed(d.path(), &mut e, "Boat at dusk").unwrap();
        let b = embed(d.path(), &mut e, "  boat at DUSK ").unwrap();
        assert_eq!(e.0, 1);
        assert_eq!(a, b);
        assert!((a.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-6);
        embed(d.path(), &mut e, "another").unwrap();
        assert_eq!(e.0, 2);
    }
}
