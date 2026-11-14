//! Transcript cache per media file in the bundle: `cache/transcripts/<key>.json`, keyed by the file's identity
//! (path, size, modification time), the model and the language.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::recognize::Transcript;

/// Cache key: a SHA-256 of the file's identity (stable across Rust versions and platforms, unlike `DefaultHasher`),
/// then the model and the language.
pub fn key(media: &Path, model: &str, language: Option<&str>) -> String {
    let md = std::fs::metadata(media).ok();
    let modified_ms = md
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    let id = identity(&media.to_string_lossy(), md.map(|m| m.len()), modified_ms);
    format!("{id}-{model}-{}", language.unwrap_or("auto"))
}

/// First 16 hex digits of SHA-256 over the length-prefixed path, then the size and modification time (each a
/// presence byte and a little-endian u64).
fn identity(path: &str, len: Option<u64>, modified_ms: Option<u64>) -> String {
    let mut h = Sha256::new();
    h.update((path.len() as u64).to_le_bytes());
    h.update(path.as_bytes());
    for v in [len, modified_ms] {
        match v {
            Some(v) => {
                h.update([1]);
                h.update(v.to_le_bytes());
            }
            None => h.update([0]),
        }
    }
    h.finalize()[..8].iter().map(|b| format!("{b:02x}")).collect()
}

fn path(bundle: &Path, key: &str) -> PathBuf {
    bundle.join("cache").join("transcripts").join(format!("{key}.json"))
}

pub fn load(bundle: &Path, key: &str) -> Option<Transcript> {
    serde_json::from_slice(&std::fs::read(path(bundle, key)).ok()?).ok()
}

pub fn store(bundle: &Path, key: &str, t: &Transcript) {
    let p = path(bundle, key);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec(t).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_a_fixed_function_of_its_inputs() {
        // Pinned (computed independently with Python's hashlib): a toolchain or platform change must not invalidate
        // every bundle's transcript cache.
        assert_eq!(identity("/clips/a.mov", Some(1234), Some(1_700_000_000_000)), "b73f255817f84924");
        assert_eq!(identity("/clips/a.mov", None, None), "36e67dc6e93c7d17");
        assert_ne!(identity("/clips/a.mov", Some(1234), Some(1)), identity("/clips/a.mov", Some(1234), Some(2)));
        assert_ne!(identity("/clips/a.mov", Some(1), None), identity("/clips/a.mov", None, Some(1)));
    }

    #[test]
    fn key_changes_when_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.wav");
        std::fs::write(&f, b"one").unwrap();
        let k1 = key(&f, "base", None);
        assert_eq!(k1, key(&f, "base", None));
        assert!(k1.ends_with("-base-auto"), "{k1}");
        std::fs::write(&f, b"three").unwrap();
        assert_ne!(k1, key(&f, "base", None));
        assert_ne!(key(&f, "base", Some("en")), key(&f, "small", Some("en")));
    }
}
