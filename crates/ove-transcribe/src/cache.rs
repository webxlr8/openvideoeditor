//! Transcript cache per media file in the bundle: `cache/transcripts/<key>.json`, keyed by the file's identity
//! (path, size, modification time), the model and the language.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::recognize::Transcript;

pub fn key(media: &Path, model: &str, language: Option<&str>) -> String {
    let md = std::fs::metadata(media).ok();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    media.display().to_string().hash(&mut h);
    md.as_ref().map(|m| m.len()).hash(&mut h);
    md.and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .hash(&mut h);
    format!("{:016x}-{model}-{}", h.finish(), language.unwrap_or("auto"))
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
