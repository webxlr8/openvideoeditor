//! One file of a model: an HTTP download into `<name>.part`, resumed with `Range` after an interruption, hashed while
//! it streams, and renamed to its final name only when the size and SHA-256 match the pins.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::{ModelError, Result};
use crate::manifest::ModelFile;

/// Attempts in a row that receive no bytes before the download gives up (the partial file is kept for next time).
const MAX_ATTEMPTS_WITHOUT_PROGRESS: u32 = 5;
/// How often a cancel is checked while waiting for the network.
const CANCEL_POLL: Duration = Duration::from_millis(100);
const CHUNK: usize = 1 << 16;

/// The HTTP client for model downloads: the platform's certificate verifier; a connection silent for 30 s counts as
/// interrupted and is resumed with a new request. Network reads run on a helper thread, so a cancel never waits
/// for them.
pub fn agent() -> ureq::Agent {
    ove_guard::http::agent_builder()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .build()
}

/// What the network thread reports.
enum Msg {
    Head { status: u16, range_start: Option<u64> },
    Status(u16),
    Data(Vec<u8>),
    End,
    Failed(String),
}

/// Sends the request and streams the body on a helper thread. The thread ends by itself when the receiver is gone.
fn start_request(agent: &ureq::Agent, url: &str, from: u64) -> mpsc::Receiver<Msg> {
    let (tx, rx) = mpsc::sync_channel::<Msg>(8);
    let mut request = agent.get(url);
    if from > 0 {
        request = request.set("Range", &format!("bytes={from}-"));
    }
    std::thread::spawn(move || {
        let response = match request.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => {
                let _ = tx.send(Msg::Status(code));
                return;
            }
            Err(e) => {
                let _ = tx.send(Msg::Failed(e.to_string()));
                return;
            }
        };
        if tx.send(Msg::Head { status: response.status(), range_start: range_start(&response) }).is_err() {
            return;
        }
        let mut reader = response.into_reader();
        loop {
            let mut buf = vec![0u8; CHUNK];
            let msg = match reader.read(&mut buf) {
                Ok(0) => Msg::End,
                Ok(n) => {
                    buf.truncate(n);
                    Msg::Data(buf)
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => Msg::Failed(format!("the connection was interrupted ({e})")),
            };
            let last = !matches!(msg, Msg::Data(_));
            if tx.send(msg).is_err() || last {
                return;
            }
        }
    });
    rx
}

/// The next message, checking the cancel flag while waiting. `None` when the network thread is gone.
fn next(rx: &mpsc::Receiver<Msg>, cancel: &AtomicBool) -> Result<Option<Msg>> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ModelError::Cancelled);
        }
        match rx.recv_timeout(CANCEL_POLL) {
            Ok(m) => return Ok(Some(m)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
        }
    }
}

/// The partial file next to `dest`.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

/// What one file download needs.
pub struct FileJob<'a> {
    /// Model id, for messages.
    pub model: &'a str,
    pub file: &'a ModelFile,
    /// Where to fetch it (the pinned URL, or a mirror).
    pub url: &'a str,
    /// Final path; the partial file is `dest` + `.part`.
    pub dest: &'a Path,
}

/// Downloads one file. `on_bytes(done)` reports the bytes of this file on disk so far (including resumed bytes).
/// On cancel the partial file is deleted; on a hash or size mismatch the file is deleted and `Corrupt` returned; on a
/// network failure the partial file is kept so the next attempt resumes.
pub fn fetch_file(
    agent: &ureq::Agent,
    job: &FileJob<'_>,
    on_bytes: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<()> {
    let part = part_path(job.dest);
    if let Some(dir) = job.dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io_err("create", dir, e))?;
    }
    let result = fetch_into(agent, job, &part, on_bytes, cancel);
    if let Err(ModelError::Cancelled | ModelError::Corrupt { .. }) = &result {
        let _ = std::fs::remove_file(&part);
    }
    result
}

fn fetch_into(
    agent: &ureq::Agent,
    job: &FileJob<'_>,
    part: &Path,
    on_bytes: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<()> {
    let want = job.file.bytes;
    let (mut hasher, mut have) = resume_state(part, want)?;
    on_bytes(have);
    let mut failures = 0u32;
    let mut last_error = String::new();
    while have < want {
        if cancel.load(Ordering::Relaxed) {
            return Err(ModelError::Cancelled);
        }
        if failures >= MAX_ATTEMPTS_WITHOUT_PROGRESS {
            return Err(ModelError::Download(format!(
                "could not download {} for {}: {last_error}; the received part is kept and the next attempt resumes",
                job.file.name, job.model
            )));
        }
        if failures > 0 {
            wait(Duration::from_millis((250 * u64::from(failures)).min(1000)), cancel)?;
        }
        let rx = start_request(agent, job.url, have);
        let (status, start) = match next(&rx, cancel)? {
            Some(Msg::Head { status, range_start }) => (status, range_start),
            // The server says the range is past the end: the part is complete as far as it knows; verify it.
            Some(Msg::Status(416)) if have > 0 => break,
            Some(Msg::Status(code)) => {
                return Err(ModelError::Download(format!(
                    "could not download {} for {}: the server answered {code}",
                    job.file.name, job.model
                )));
            }
            Some(Msg::Failed(e)) => {
                failures += 1;
                last_error = e;
                continue;
            }
            Some(Msg::Data(_) | Msg::End) | None => {
                failures += 1;
                last_error = "the request failed".into();
                continue;
            }
        };
        let mut file = if have > 0 && status == 206 && start == Some(have) {
            OpenOptions::new().append(true).open(part).map_err(|e| io_err("write", part, e))?
        } else {
            // The server ignored the range (or this is a fresh start): begin again from byte 0.
            hasher = Sha256::new();
            have = 0;
            on_bytes(0);
            File::create(part).map_err(|e| io_err("write", part, e))?
        };
        let before = have;
        loop {
            match next(&rx, cancel)? {
                Some(Msg::Data(chunk)) => {
                    if have + chunk.len() as u64 > want {
                        return Err(ModelError::Corrupt { model: job.model.into(), file: job.file.name.clone() });
                    }
                    file.write_all(&chunk).map_err(|e| io_err("write", part, e))?;
                    hasher.update(&chunk);
                    have += chunk.len() as u64;
                    on_bytes(have);
                }
                Some(Msg::End) => break,
                Some(Msg::Failed(e)) => {
                    last_error = e;
                    break;
                }
                Some(Msg::Head { .. } | Msg::Status(_)) | None => {
                    last_error = "the connection was interrupted".into();
                    break;
                }
            }
        }
        file.sync_all().map_err(|e| io_err("write", part, e))?;
        if have == before {
            failures += 1;
            if last_error.is_empty() {
                last_error = "the server sent no data".into();
            }
        } else {
            failures = 0;
        }
    }
    if have != want || format!("{:x}", hasher.finalize()) != job.file.sha256 {
        return Err(ModelError::Corrupt { model: job.model.into(), file: job.file.name.clone() });
    }
    std::fs::rename(part, job.dest).map_err(|e| io_err("install", job.dest, e))
}

/// Hash state and length of an existing partial file (empty when none). A part longer than the file is dropped.
fn resume_state(part: &Path, want: u64) -> Result<(Sha256, u64)> {
    let mut hasher = Sha256::new();
    let Ok(meta) = std::fs::metadata(part) else { return Ok((hasher, 0)) };
    if meta.len() > want {
        std::fs::remove_file(part).map_err(|e| io_err("remove", part, e))?;
        return Ok((hasher, 0));
    }
    let mut f = File::open(part).map_err(|e| io_err("read", part, e))?;
    let mut buf = vec![0u8; CHUNK];
    let mut have = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| io_err("read", part, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        have += n as u64;
    }
    Ok((hasher, have))
}

/// The first byte of a `Content-Range: bytes <start>-<end>/<total>` header.
fn range_start(r: &ureq::Response) -> Option<u64> {
    let v = r.header("content-range")?;
    v.trim().strip_prefix("bytes ")?.split('-').next()?.trim().parse().ok()
}

fn wait(d: Duration, cancel: &AtomicBool) -> Result<()> {
    let step = Duration::from_millis(50);
    let mut left = d;
    while !left.is_zero() {
        if cancel.load(Ordering::Relaxed) {
            return Err(ModelError::Cancelled);
        }
        let s = left.min(step);
        std::thread::sleep(s);
        left -= s;
    }
    Ok(())
}

/// SHA-256 of a whole file, lower-case hex.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = File::open(path).map_err(|e| io_err("read", path, e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| io_err("read", path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn io_err(what: &str, path: &Path, e: std::io::Error) -> ModelError {
    ModelError::Io(format!("could not {what} {}: {e}", path.display()))
}
