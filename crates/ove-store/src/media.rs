//! Media registry status on open (spec: Media registry): online, offline or changed, computed without modifying
//! the document. Content identity (spec: media-derivatives, Content identity): a stable id per media file that
//! derived data is keyed by, so moving, renaming or relinking a file keeps its derived data.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use ove_core::model::Document;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

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

/// Bytes read from each sampled region.
const SAMPLE: u64 = 1024 * 1024;

/// Hex characters kept from the hash (128 bits).
const ID_HEX: usize = 32;

/// The content id of a file: SHA-256 over its size and its first, middle and last MiB (the whole file when it is
/// 3 MiB or smaller), as 32 hex characters. Fast for any file size; the same for every copy of the file wherever it
/// lives.
pub fn content_id(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    let mut h = Sha256::new();
    h.update(b"ove-content-v1\0");
    h.update(size.to_le_bytes());
    if size <= 3 * SAMPLE {
        std::io::copy(&mut f, &mut HashWriter(&mut h))?;
    } else {
        for start in [0, size / 2 - SAMPLE / 2, size - SAMPLE] {
            f.seek(SeekFrom::Start(start))?;
            std::io::copy(&mut (&mut f).take(SAMPLE), &mut HashWriter(&mut h))?;
        }
    }
    Ok(hex(&h.finalize()))
}

/// A content id over every byte of the file. Used only when a file changed in place but its sampled id did not (an
/// edit between the sampled regions that kept the size), so its derived data is still rebuilt.
pub fn full_content_id(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    h.update(b"ove-content-full-v1\0");
    h.update(f.metadata()?.len().to_le_bytes());
    std::io::copy(&mut f, &mut HashWriter(&mut h))?;
    Ok(hex(&h.finalize()))
}

struct HashWriter<'a>(&'a mut Sha256);

impl std::io::Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s.truncate(ID_HEX);
    s
}

/// A file's size and modification time (ms since the epoch): the quick "has it changed" check.
pub fn file_stamp(path: &Path) -> Option<(u64, Option<i64>)> {
    let md = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let mtime =
        md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
    Some((md.len(), mtime))
}

/// The registry fields that identify a file: `size`, `modifiedAt` and `contentId`. Written at import and relink.
pub fn identity_fields(path: &Path) -> Map<String, Value> {
    let mut m = Map::new();
    if let Some((size, mtime)) = file_stamp(path) {
        m.insert("size".into(), size.into());
        if let Some(t) = mtime {
            m.insert("modifiedAt".into(), t.into());
        }
    }
    match content_id(path) {
        Ok(id) => {
            m.insert("contentId".into(), id.into());
        }
        Err(e) => tracing::warn!(path = %path.display(), "no content id: {e}"),
    }
    m
}

/// The identity fields to store for a registry entry whose file is at `path`, or `None` when they are current.
///
/// Nothing is read when the size and modification time match the entry and it has a content id. Otherwise the
/// content id is computed again. When the file at the same path changed (size or time differ) and its sampled id did
/// not, the full-content id is used so derived data is rebuilt. A file that is moved or copied elsewhere keeps its
/// sampled id (relink stores [`identity_fields`] of the new file).
pub fn refresh_identity(path: &Path, entry: &Map<String, Value>) -> Option<Map<String, Value>> {
    let (size, mtime) = file_stamp(path)?;
    let old_id = entry.get("contentId").and_then(Value::as_str);
    let same_size = entry.get("size").and_then(Value::as_u64).is_none_or(|s| s == size);
    let same_time = match (entry.get("modifiedAt").and_then(Value::as_i64), mtime) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    if same_size && same_time && old_id.is_some() {
        return None;
    }
    let mut fields = identity_fields(path);
    let changed_in_place = !(same_size && same_time);
    let sampled = fields.get("contentId").and_then(Value::as_str);
    if changed_in_place
        && old_id.is_some()
        && sampled == old_id
        && let Ok(full) = full_content_id(path)
    {
        fields.insert("contentId".into(), full.into());
    }
    Some(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
    }

    /// Deterministic content of `len` bytes.
    fn content(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| ((i as u64).wrapping_mul(2_654_435_761) >> 13) as u8 ^ seed).collect()
    }

    fn set_mtime(path: &Path, secs_ago: u64) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago)).unwrap();
    }

    #[test]
    fn rename_move_and_copy_keep_the_id() {
        let dir = tempfile::tempdir().unwrap();
        for len in [10usize, 5 * 1024 * 1024] {
            let a = dir.path().join("clip.mov");
            write(&a, &content(len, 1));
            let id = content_id(&a).unwrap();
            assert_eq!(id.len(), 32);
            let renamed = dir.path().join("renamed.mov");
            std::fs::rename(&a, &renamed).unwrap();
            assert_eq!(content_id(&renamed).unwrap(), id);
            std::fs::create_dir_all(dir.path().join("elsewhere")).unwrap();
            let moved = dir.path().join("elsewhere/clip.mov");
            std::fs::rename(&renamed, &moved).unwrap();
            assert_eq!(content_id(&moved).unwrap(), id);
            // A copy with a new modification time has the same id too.
            let copy = dir.path().join("copy.mov");
            std::fs::copy(&moved, &copy).unwrap();
            set_mtime(&copy, 3600);
            assert_eq!(content_id(&copy).unwrap(), id);
            std::fs::remove_file(&moved).unwrap();
            std::fs::remove_file(&copy).unwrap();
        }
    }

    #[test]
    fn a_content_change_alters_the_id() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("clip.mov");
        let big = content(5 * 1024 * 1024, 2);
        write(&p, &big);
        let id = content_id(&p).unwrap();
        // A different file under the same name.
        write(&p, &content(5 * 1024 * 1024, 3));
        assert_ne!(content_id(&p).unwrap(), id);
        // One byte in a sampled region (the last MiB).
        let mut edited = big.clone();
        let n = edited.len();
        edited[n - 10] ^= 0xff;
        write(&p, &edited);
        assert_ne!(content_id(&p).unwrap(), id);
        // The same bytes plus one more: the size is hashed.
        let mut longer = big.clone();
        longer.push(0);
        write(&p, &longer);
        assert_ne!(content_id(&p).unwrap(), id);
        // A small file is hashed whole: any byte counts.
        write(&p, b"abc");
        let small = content_id(&p).unwrap();
        write(&p, b"abd");
        assert_ne!(content_id(&p).unwrap(), small);
    }

    #[test]
    fn unchanged_files_are_not_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("clip.mov");
        write(&p, &content(1000, 4));
        let entry = identity_fields(&p);
        assert!(entry.contains_key("contentId") && entry.contains_key("size") && entry.contains_key("modifiedAt"));
        assert_eq!(refresh_identity(&p, &entry), None);
        // An entry from before content ids gets one.
        let mut old = entry.clone();
        old.remove("contentId");
        assert_eq!(refresh_identity(&p, &old).unwrap()["contentId"], entry["contentId"]);
        // Missing file: nothing to refresh.
        assert_eq!(refresh_identity(&dir.path().join("gone.mov"), &entry), None);
    }

    /// The collision risk in design.md: an edit between the sampled regions that keeps the size. The sampled id
    /// cannot see it; the size and time check does, and the refresh then uses the full-content id.
    #[test]
    fn an_edit_outside_the_samples_with_a_new_time_still_changes_the_id() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("clip.mov");
        let mut bytes = content(8 * 1024 * 1024, 5);
        write(&p, &bytes);
        set_mtime(&p, 7200);
        let entry = identity_fields(&p);
        let old = entry["contentId"].as_str().unwrap().to_string();
        bytes[2 * 1024 * 1024] ^= 0xff; // between the first and middle samples
        write(&p, &bytes);
        assert_eq!(content_id(&p).unwrap(), old, "the sampled id cannot see this edit");
        let refreshed = refresh_identity(&p, &entry).expect("size or time changed");
        let new = refreshed["contentId"].as_str().unwrap();
        assert_ne!(new, old);
        assert_eq!(new, full_content_id(&p).unwrap());
    }

    /// Older readers keep the field: a registry field they do not know round-trips through read and write.
    #[test]
    fn content_id_round_trips_as_an_unknown_field() {
        let doc = ove_core::Document::new("p", "Old reader", 1);
        let mut json = doc.to_json();
        json["media"] = serde_json::json!([{"mediaId": "m1", "nativePath": "/x/clip.mov", "ownedByBundle": false,
            "contentId": "0123456789abcdef0123456789abcdef", "fieldFromTheFuture": {"a": 1}}]);
        json["project"]["media"] = serde_json::json!([{"id": "m1", "data": {"type": "video"}}]);
        let read = ove_core::Document::from_json(&json).unwrap();
        let back = read.to_json();
        assert_eq!(back["media"][0]["contentId"], "0123456789abcdef0123456789abcdef");
        assert_eq!(back["media"][0]["fieldFromTheFuture"], serde_json::json!({"a": 1}));
    }
}
