//! Partial outputs: a task writes to `<name>.part` beside its final path and renames it on success, so features never
//! see a half-written file. A part file that is not committed is removed when it goes out of scope (cancel, error or
//! panic).

use std::io;
use std::path::{Path, PathBuf};

/// The extension partial outputs carry.
pub const PART_EXT: &str = "part";

/// `<final>.part`: the final file name with `.part` appended.
pub fn part_path(final_path: &Path) -> PathBuf {
    let mut name = final_path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".");
    name.push(PART_EXT);
    final_path.with_file_name(name)
}

/// A partial output on its way to `final_path`.
#[derive(Debug)]
pub struct PartFile {
    part: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl PartFile {
    /// Prepares a part file for `final_path`: creates the parent folder and removes a stale part file.
    pub fn new(final_path: impl Into<PathBuf>) -> io::Result<PartFile> {
        let target = final_path.into();
        if let Some(dir) = target.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let part = part_path(&target);
        match std::fs::remove_file(&part) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        Ok(PartFile { part, target, committed: false })
    }

    /// Where to write.
    pub fn path(&self) -> &Path {
        &self.part
    }

    /// Where the output goes on success.
    pub fn final_path(&self) -> &Path {
        &self.target
    }

    /// Moves the part file to its final path, replacing an older output.
    pub fn commit(mut self) -> io::Result<PathBuf> {
        std::fs::rename(&self.part, &self.target)?;
        self.committed = true;
        Ok(self.target.clone())
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.part);
        }
    }
}

/// Runs `write` against a part file and commits it when `write` succeeds; on error nothing visible is left.
pub fn write_output<E: From<io::Error>>(
    final_path: impl Into<PathBuf>,
    write: impl FnOnce(&Path) -> Result<(), E>,
) -> Result<PathBuf, E> {
    let part = PartFile::new(final_path)?;
    write(part.path())?;
    Ok(part.commit()?)
}

/// Removes leftover part files directly inside `dir` (from a crash or a killed process). Returns how many.
pub fn remove_stale_parts(dir: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == PART_EXT))
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_path_appends_extension() {
        assert_eq!(part_path(Path::new("a/b/sheet.webp")), PathBuf::from("a/b/sheet.webp.part"));
        assert_eq!(part_path(Path::new("proxy")), PathBuf::from("proxy.part"));
    }

    #[test]
    fn commit_renames_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("sub/out.bin");
        let p = PartFile::new(&out).unwrap();
        std::fs::write(p.path(), b"one").unwrap();
        assert!(!out.exists());
        p.commit().unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"one");
        write_output::<io::Error>(&out, |part| std::fs::write(part, b"two")).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"two");
        assert!(!part_path(&out).exists());
    }

    #[test]
    fn dropped_or_failed_part_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.bin");
        {
            let p = PartFile::new(&out).unwrap();
            std::fs::write(p.path(), b"half").unwrap();
        }
        assert!(!out.exists() && !part_path(&out).exists());
        let r = write_output::<io::Error>(&out, |part| {
            std::fs::write(part, b"half")?;
            Err(io::Error::other("encoder failed"))
        });
        assert!(r.is_err());
        assert!(!out.exists() && !part_path(&out).exists());
    }

    #[test]
    fn stale_parts_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.webp.part"), b"x").unwrap();
        std::fs::write(dir.path().join("b.webp"), b"x").unwrap();
        assert_eq!(remove_stale_parts(dir.path()), 1);
        assert!(dir.path().join("b.webp").exists());
    }
}
