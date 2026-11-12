//! Durable file primitives (design D2/D11) and the I/O fault layer used by crash and fault-injection tests.
//!
//! Failpoints: every durable step calls [`failpoint`] with a name (`record.write`, `record.flush`, `record.rename`,
//! `record.dirsync`, `snapshot.write`, `snapshot.flush`, `snapshot.rename`, `index.write`). Tests configure them
//! per thread with [`set_failpoint`], or in a child process with the `OVE_FAILPOINTS` environment variable:
//! `name=action[@n];…` where action is `abort` (kill the process), `diskfull`, `denied` or `busy`, and `@n` fires on
//! the n-th hit (1-based, default 1).

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultAction {
    Abort,
    DiskFull,
    Denied,
    /// File held open by another process (Windows sharing violation).
    Busy,
}

#[derive(Debug, Clone, Copy)]
struct Fault {
    action: FaultAction,
    at_hit: u64,
    hits: u64,
}

static ARMED: AtomicBool = AtomicBool::new(false);

fn registry() -> &'static Mutex<HashMap<String, Fault>> {
    static R: OnceLock<Mutex<HashMap<String, Fault>>> = OnceLock::new();
    R.get_or_init(|| {
        let mut m = HashMap::new();
        if let Ok(spec) = std::env::var("OVE_FAILPOINTS") {
            for part in spec.split(';').filter(|s| !s.is_empty()) {
                let Some((name, rest)) = part.split_once('=') else { continue };
                let (act, n) = rest.split_once('@').unwrap_or((rest, "1"));
                let action = match act {
                    "abort" => FaultAction::Abort,
                    "diskfull" => FaultAction::DiskFull,
                    "denied" => FaultAction::Denied,
                    "busy" => FaultAction::Busy,
                    _ => continue,
                };
                m.insert(name.to_string(), Fault { action, at_hit: n.parse().unwrap_or(1), hits: 0 });
            }
            if !m.is_empty() {
                ARMED.store(true, Ordering::SeqCst);
            }
        }
        Mutex::new(m)
    })
}

thread_local! {
    static LOCAL: std::cell::RefCell<HashMap<String, Fault>> = std::cell::RefCell::new(HashMap::new());
}

/// Arms a failpoint for the current thread: fires on the `at_hit`-th hit (1-based), then stays armed.
pub fn set_failpoint(name: &str, action: FaultAction, at_hit: u64) {
    LOCAL.with(|l| l.borrow_mut().insert(name.to_string(), Fault { action, at_hit, hits: 0 }));
    ARMED.store(true, Ordering::SeqCst);
}

/// Disarms every failpoint of the current thread.
pub fn clear_failpoints() {
    LOCAL.with(|l| l.borrow_mut().clear());
}

fn fire(f: &mut Fault) -> io::Result<()> {
    f.hits += 1;
    if f.hits < f.at_hit {
        return Ok(());
    }
    match f.action {
        FaultAction::Abort => std::process::abort(),
        FaultAction::DiskFull => Err(io::Error::new(io::ErrorKind::StorageFull, "No space left on device (injected)")),
        FaultAction::Denied => Err(io::Error::new(io::ErrorKind::PermissionDenied, "Permission denied (injected)")),
        FaultAction::Busy => Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            "The process cannot access the file because it is being used by another process (injected)",
        )),
    }
}

/// Checks a failpoint. Returns the injected error, or aborts the process for `abort`.
pub fn failpoint(name: &str) -> io::Result<()> {
    if !ARMED.load(Ordering::Relaxed) && std::env::var_os("OVE_FAILPOINTS").is_none() {
        return Ok(());
    }
    let local = LOCAL.with(|l| l.borrow_mut().get_mut(name).map(fire));
    if let Some(r) = local {
        return r;
    }
    let mut reg = registry().lock().unwrap_or_else(PoisonError::into_inner);
    match reg.get_mut(name) {
        Some(f) => fire(f),
        None => Ok(()),
    }
}

/// Flushes file data so a crash yields the old or new state. macOS: `F_BARRIERFSYNC` (ordering barrier, cheaper
/// than `F_FULLFSYNC`); Windows: `FlushFileBuffers`; elsewhere `fdatasync`.
pub fn flush_ordered(f: &File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        const F_BARRIERFSYNC: libc::c_int = 85;
        // SAFETY: fcntl on a valid, owned descriptor with an integer command and no pointer arguments.
        let r = unsafe { libc::fcntl(f.as_raw_fd(), F_BARRIERFSYNC) };
        if r == -1 {
            return f.sync_all();
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        f.sync_data()
    }
}

/// Full durability flush (macOS `F_FULLFSYNC` via `sync_all`), used on close and before pruning.
pub fn flush_full(f: &File) -> io::Result<()> {
    f.sync_all()
}

/// Flushes a directory entry so a rename inside it is durable (no-op on Windows, where NTFS journals metadata).
pub fn flush_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// Renames with retries for up to 2 s while the target is held by another process (antivirus, indexers on Windows).
pub fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut delay = Duration::from_millis(10);
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if is_busy(&e) && Instant::now() < deadline => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(250));
            }
            Err(e) => return Err(e),
        }
    }
}

fn is_busy(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::ResourceBusy {
        return true;
    }
    #[cfg(windows)]
    {
        // ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33), ERROR_ACCESS_DENIED (5) during AV scans.
        matches!(e.raw_os_error(), Some(32) | Some(33) | Some(5))
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Names of the failpoints of one atomic write.
#[derive(Debug, Clone, Copy)]
pub struct Points {
    pub write: &'static str,
    pub flush: &'static str,
    pub rename: &'static str,
    pub dirsync: &'static str,
}

pub const RECORD: Points =
    Points { write: "record.write", flush: "record.flush", rename: "record.rename", dirsync: "record.dirsync" };
pub const SNAPSHOT: Points =
    Points { write: "snapshot.write", flush: "snapshot.flush", rename: "snapshot.rename", dirsync: "snapshot.dirsync" };
pub const INDEX: Points =
    Points { write: "index.write", flush: "index.flush", rename: "index.rename", dirsync: "index.dirsync" };

/// Writes `bytes` to `path` atomically: temp file in the same directory, flush, rename, flush the directory.
/// On error the temp file is removed and `path` is untouched.
pub fn write_atomic(path: &Path, bytes: &[u8], points: Points, full: bool) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = temp_name(path);
    let result = (|| {
        failpoint(points.write)?;
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        // Write in two halves so a kill between them leaves a torn temp file (exercised by the crash harness).
        let mid = bytes.len() / 2;
        f.write_all(&bytes[..mid])?;
        failpoint(points.write)?;
        f.write_all(&bytes[mid..])?;
        failpoint(points.flush)?;
        if full {
            flush_full(&f)?
        } else {
            flush_ordered(&f)?
        }
        drop(f);
        failpoint(points.rename)?;
        rename_with_retry(&tmp, path)?;
        failpoint(points.dirsync)?;
        flush_dir(dir)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn temp_name(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".tmp-{}-{}-{name}", std::process::id(), unique()))
}

fn unique() -> u64 {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// Removes leftover temp files from interrupted writes in a directory.
pub fn clean_temp_files(dir: &Path) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(".tmp-") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// Writes a derived (regenerable) file atomically without flushing: temp file + rename.
pub fn write_derived(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = temp_name(path);
    let r = fs::write(&tmp, bytes).and_then(|()| rename_with_retry(&tmp, path));
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    r
}
