//! Transcript cache per media file in the bundle: `cache/transcripts/<key>.json`, keyed by the file's content id
//! (see `ove_store::media::content_id`), the model and the language, so a moved or relinked file keeps its
//! transcripts. Projects from before content ids used a key over the path, size and modification time
//! ([`key`]); [`migrate_legacy`] renames those entries on first open.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::recognize::Transcript;

/// Cache key for a file with a content id: the id, the model and the language.
pub fn content_key(content_id: &str, model: &str, language: Option<&str>) -> String {
    format!("{content_id}-{model}-{}", language.unwrap_or("auto"))
}

/// Renames the legacy cache entries of the file at `media` (keyed by its path, size and modification time) to its
/// content id, keeping the model and language part. Returns how many were moved. An entry already under the content
/// id wins; the legacy one is then removed.
pub fn migrate_legacy(bundle: &Path, media: &Path, content_id: &str) -> usize {
    let legacy = key(media, "", None);
    let Some(prefix) = legacy.strip_suffix("-auto").and_then(|k| k.strip_suffix('-')) else { return 0 };
    let dir = bundle.join("cache").join("transcripts");
    let Ok(rd) = std::fs::read_dir(&dir) else { return 0 };
    let mut moved = 0;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(prefix).and_then(|r| r.strip_prefix('-')) else { continue };
        if !rest.ends_with(".json") {
            continue;
        }
        let target = dir.join(format!("{content_id}-{rest}"));
        let done = if target.exists() {
            std::fs::remove_file(e.path()).is_ok()
        } else {
            std::fs::rename(e.path(), &target).is_ok()
        };
        moved += usize::from(done);
    }
    moved
}

/// Legacy cache key (projects from before content ids): a SHA-256 of the file's path, size and modification time
/// (stable across Rust versions and platforms, unlike `DefaultHasher`), then the model and the language.
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
    fn content_keys_follow_the_content_not_the_path() {
        assert_eq!(content_key("0123abcd", "base", None), "0123abcd-base-auto");
        assert_eq!(content_key("0123abcd", "small", Some("en")), "0123abcd-small-en");
    }

    /// An upgraded project keeps its transcripts: legacy entries move to the content id and load from there, so
    /// nothing is transcribed again.
    #[test]
    fn legacy_entries_migrate_to_the_content_id() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("P.ove");
        let media = dir.path().join("interview.wav");
        std::fs::write(&media, b"sound").unwrap();
        let t = Transcript { language: "en".into(), model: "base".into(), words: vec![], duration_us: 1_000_000 };
        let old_en = key(&media, "base", Some("en"));
        let old_auto = key(&media, "small", None);
        store(&bundle, &old_en, &t);
        store(&bundle, &old_auto, &t);
        // Another file's entry stays.
        let other = dir.path().join("other.wav");
        std::fs::write(&other, b"other").unwrap();
        store(&bundle, &key(&other, "base", None), &t);

        assert_eq!(migrate_legacy(&bundle, &media, "c0ffee"), 2);
        assert_eq!(load(&bundle, &content_key("c0ffee", "base", Some("en"))), Some(t.clone()));
        assert_eq!(load(&bundle, &content_key("c0ffee", "small", None)), Some(t.clone()));
        assert!(load(&bundle, &old_en).is_none(), "moved, not copied");
        assert!(load(&bundle, &key(&other, "base", None)).is_some());
        assert_eq!(migrate_legacy(&bundle, &media, "c0ffee"), 0, "a second open has nothing to move");
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
