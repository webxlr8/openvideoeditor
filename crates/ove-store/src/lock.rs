//! Single-writer locking (design D8): an OS advisory lock on `<bundle>/.lock`, held while the project is open for
//! writing. The OS releases it when the holder dies, so stale detection never relies on pid heuristics.
//!
//! Holder info (pid, process name, start time, host) is written to `<bundle>/.lock.holder` rather than into `.lock`
//! itself, because Windows byte-range locks are mandatory and would stop other processes from reading it.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, StoreError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Holder {
    pub pid: u32,
    pub process: String,
    pub started_at: i64,
    pub host: String,
}

impl Holder {
    pub fn current() -> Self {
        Holder {
            pid: std::process::id(),
            process: std::env::current_exe()
                .ok()
                .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "unknown".into()),
            started_at: ove_core::revision::Clock::now_ms(&ove_core::SystemClock),
            host: hostname(),
        }
    }

    pub fn describe(&self) -> String {
        format!("{} (pid {}) on {}", self.process, self.pid, self.host)
    }
}

fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: `buf` is valid for writes of `buf.len()` bytes and `gethostname` writes at most that many; the
        // result is read back through a bounds-checked search for the NUL.
        if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
        "localhost".into()
    }
    #[cfg(not(unix))]
    {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".into())
    }
}

/// A held write lock. Released on drop (and by the OS if the process dies).
#[derive(Debug)]
pub struct WriteLock {
    file: File,
    holder_path: PathBuf,
    /// Set when a lock left by a dead process was taken over.
    pub took_over_from: Option<Holder>,
}

impl WriteLock {
    pub fn acquire(bundle: &Path) -> Result<WriteLock> {
        let path = bundle.join(".lock");
        let holder_path = bundle.join(".lock.holder");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| StoreError::io("open lock file", &path, e))?;
        let previous: Option<Holder> = std::fs::read(&holder_path).ok().and_then(|b| serde_json::from_slice(&b).ok());
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                let holder = previous.map(|h| h.describe()).unwrap_or_else(|| "another process".into());
                return Err(StoreError::Locked { holder });
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(StoreError::io("lock", &path, e)),
        }
        // We hold the lock. Any holder info left behind belongs to a process that died without releasing.
        let took_over_from = previous;
        if let Some(h) = &took_over_from {
            tracing::warn!(bundle = %bundle.display(), previous = %h.describe(), "took over a stale project lock");
        }
        let me = Holder::current();
        serde_json::to_vec(&me)
            .map_err(std::io::Error::from)
            .and_then(|bytes| std::fs::write(&holder_path, bytes))
            .map_err(|e| StoreError::io("write lock holder", &holder_path, e))?;
        Ok(WriteLock { file, holder_path, took_over_from })
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.holder_path);
        let _ = self.file.unlock();
    }
}
