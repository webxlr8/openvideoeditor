//! Media registry status on open (spec: Media registry): online, offline or changed, computed without modifying
//! the document.

use std::path::{Path, PathBuf};

use ove_core::model::Document;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaState {
    Online,
    Offline,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaStatus {
    pub media_id: String,
    pub path: PathBuf,
    pub state: MediaState,
}

/// Expands `~/` and resolves bundle-relative paths for owned media (or any relative path).
pub fn resolve_media_path(bundle: &Path, native: &str) -> PathBuf {
    if let Some(rest) = native.strip_prefix("~/").or_else(|| native.strip_prefix("~\\"))
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }
    let p = PathBuf::from(native);
    if p.is_absolute() { p } else { bundle.join(p) }
}

/// Checks every registry entry against the file system.
pub fn media_status(bundle: &Path, doc: &Document) -> Vec<MediaStatus> {
    doc.registry
        .values()
        .map(|e| {
            let path = resolve_media_path(bundle, &e.native_path);
            let state = match std::fs::metadata(&path) {
                Err(_) => MediaState::Offline,
                Ok(md) if !md.is_file() => MediaState::Offline,
                Ok(md) => {
                    let size_changed = e.fields.get("size").and_then(|v| v.as_u64()).is_some_and(|s| s != md.len());
                    let mtime = md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64);
                    let mtime_changed = match (e.fields.get("modifiedAt").and_then(|v| v.as_i64()), mtime) {
                        (Some(a), Some(b)) => a != b,
                        _ => false,
                    };
                    if size_changed || mtime_changed { MediaState::Changed } else { MediaState::Online }
                }
            };
            MediaStatus { media_id: e.media_id.clone(), path, state }
        })
        .collect()
}
