//! Revision pins: a queued export holds the revision it renders, so pruning keeps that revision reconstructable until
//! the export has run (change extend-export-formats-and-queue, design D6). A pin is a small file in the bundle,
//! `cache/pins/<key>.rev`, holding the revision number; any process can add or remove one without the write lock.

use std::path::{Path, PathBuf};

fn dir(bundle: &Path) -> PathBuf {
    bundle.join("cache").join("pins")
}

/// Keys become file names: letters, digits, `_` and `-` only.
fn safe(key: &str) -> bool {
    !key.is_empty() && key.len() <= 80 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Pins `rev` under `key` (replacing an earlier pin with that key).
pub fn pin(bundle: &Path, key: &str, rev: i64) -> std::io::Result<()> {
    if !safe(key) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("bad pin key {key:?}")));
    }
    std::fs::create_dir_all(dir(bundle))?;
    std::fs::write(dir(bundle).join(format!("{key}.rev")), rev.to_string())
}

/// Removes the pin `key` (nothing when there is none).
pub fn unpin(bundle: &Path, key: &str) {
    if safe(key) {
        let _ = std::fs::remove_file(dir(bundle).join(format!("{key}.rev")));
    }
}

/// Every pinned revision of the bundle.
pub fn pinned(bundle: &Path) -> Vec<i64> {
    std::fs::read_dir(dir(bundle))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "rev"))
                .filter_map(|e| std::fs::read_to_string(e.path()).ok()?.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_round_trip() {
        let d = tempfile::tempdir().unwrap();
        pin(d.path(), "export-x_1", 42).unwrap();
        pin(d.path(), "export-x_2", 7).unwrap();
        let mut p = pinned(d.path());
        p.sort();
        assert_eq!(p, [7, 42]);
        unpin(d.path(), "export-x_1");
        assert_eq!(pinned(d.path()), [7]);
        assert!(pin(d.path(), "../x", 1).is_err());
    }
}
