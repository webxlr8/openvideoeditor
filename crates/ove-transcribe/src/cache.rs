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

/// A cached transcript of `media` made with any model and language (the largest model first, then the newest file).
pub fn load_any(bundle: &Path, media: &Path) -> Option<Transcript> {
    let id = key(media, "", None);
    let id = id.split('-').next()?.to_string();
    let dir = bundle.join("cache").join("transcripts");
    let rank =
        |name: &str| ["small", "base", "tiny"].iter().position(|m| name.contains(&format!("-{m}-"))).unwrap_or(3);
    let mut found: Vec<(usize, std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (name.starts_with(&format!("{id}-")) && name.ends_with(".json")).then(|| {
                let modified = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                (rank(&name), modified, e.path())
            })
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    found.iter().find_map(|(_, _, p)| serde_json::from_slice(&std::fs::read(p).ok()?).ok())
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
    fn any_cached_transcript_of_a_file_is_found_largest_model_first() {
        let d = tempfile::tempdir().unwrap();
        let media = d.path().join("clip.wav");
        std::fs::write(&media, b"RIFF").unwrap();
        let other = d.path().join("other.wav");
        std::fs::write(&other, b"RIFFF").unwrap();
        let t = |model: &str| Transcript { language: "en".into(), model: model.into(), words: vec![], duration_us: 1 };
        assert!(load_any(d.path(), &media).is_none());
        store(d.path(), &key(&media, "tiny", None), &t("tiny"));
        store(d.path(), &key(&media, "base", Some("en")), &t("base"));
        store(d.path(), &key(&other, "small", None), &t("small"));
        assert_eq!(load_any(d.path(), &media).unwrap().model, "base", "base before tiny; another file's ignored");
    }

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
