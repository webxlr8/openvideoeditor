//! Scripted answers for the native file dialogs, for the end-to-end suite (`apps/desktop/e2e-app`).
//!
//! WebDriver drives the web view only; a native open or save dialog would block the run. In debug builds, and only
//! when `OVE_E2E_DIALOGS=<dir>` is set, the dialogs are not shown and answer from that folder instead:
//!
//! - a save dialog answers `<dir>/save/<suggested file name>`;
//! - an open dialog answers the paths listed in `<dir>/open.txt` (one per line), or "cancelled" when the file is
//!   missing or empty. The runner rewrites the file before each import.
//!
//! Release builds never read the variable.

use std::path::{Path, PathBuf};

/// The environment variable naming the answer folder.
pub const ENV: &str = "OVE_E2E_DIALOGS";

fn answer_dir() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os(ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The scripted answer to a save dialog suggesting `file_name`; `None` when dialogs are not scripted.
pub fn save(file_name: &str) -> Option<Option<PathBuf>> {
    answer_dir().map(|dir| save_in(&dir, file_name))
}

/// The scripted answer to an open dialog (empty: cancelled); `None` when dialogs are not scripted.
pub fn open() -> Option<Vec<PathBuf>> {
    answer_dir().map(|dir| open_in(&dir))
}

fn save_in(dir: &Path, file_name: &str) -> Option<PathBuf> {
    let folder = dir.join("save");
    std::fs::create_dir_all(&folder).ok()?;
    let name = Path::new(file_name).file_name()?;
    let path = folder.join(name);
    tracing::info!(path = %path.display(), "save dialog answered by {ENV}");
    Some(path)
}

fn open_in(dir: &Path) -> Vec<PathBuf> {
    let text = std::fs::read_to_string(dir.join("open.txt")).unwrap_or_default();
    let paths: Vec<PathBuf> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect();
    tracing::info!(count = paths.len(), "open dialog answered by {ENV}");
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_answers_inside_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let p = save_in(dir.path(), "My cut.mp4").unwrap();
        assert_eq!(p, dir.path().join("save").join("My cut.mp4"));
        assert!(dir.path().join("save").is_dir());
        // A suggested name cannot climb out of the folder.
        let p = save_in(dir.path(), "../../escape.mp4").unwrap();
        assert_eq!(p, dir.path().join("save").join("escape.mp4"));
    }

    #[test]
    fn open_reads_the_listed_paths() {
        let dir = tempfile::tempdir().unwrap();
        assert!(open_in(dir.path()).is_empty(), "no list: cancelled");
        std::fs::write(dir.path().join("open.txt"), "/media/a.mp4\n\n  /media/b.wav \n").unwrap();
        assert_eq!(open_in(dir.path()), vec![PathBuf::from("/media/a.mp4"), PathBuf::from("/media/b.wav")]);
    }
}
