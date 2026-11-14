//! Running an agent's program for a short answer: no window, nothing on stdin, a time limit, bounded output.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long an agent's program may take to answer.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// `--version`.
    pub version: Duration,
    /// Registering the MCP server (an agent may start its runtime first).
    pub connect: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { version: Duration::from_secs(5), connect: Duration::from_secs(30) }
    }
}

/// Most output kept from one stream; the rest is read and dropped so the program is never blocked on a full pipe.
const KEEP: usize = 64 * 1024;

pub(crate) struct Ran {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Ran {
    /// What the program printed, for showing to the person: both streams, trimmed, at most 2000 characters.
    pub fn text(&self) -> String {
        let both =
            [self.stdout.trim(), self.stderr.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>();
        both.join("\n").chars().take(2000).collect()
    }
}

fn drain(mut from: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("agent-output".into()).spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        while let Ok(n) = from.read(&mut buf) {
            if n == 0 {
                break;
            }
            let room = KEEP.saturating_sub(kept.len());
            kept.extend_from_slice(&buf[..n.min(room)]);
        }
        let _ = tx.send(String::from_utf8_lossy(&kept).into_owned());
    });
    if spawned.is_err() {
        tracing::warn!("could not start a thread to read an agent's output");
    }
    rx
}

/// Runs `program args` and waits at most `limit`. `None` when it could not be started or did not finish in time
/// (it is stopped then).
pub(crate) fn run_limited(program: &Path, args: &[String], limit: Duration) -> Option<Ran> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: a console program must not flash a window over the app.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(program = %program.display(), error = %e, "an agent's program could not be started");
            return None;
        }
    };
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                // Out of time (or the wait failed): stop it. Its output is not waited for, since something it started
                // may still hold the pipe open.
                let _ = child.kill();
                let _ = child.wait();
                tracing::debug!(program = %program.display(), "an agent's program did not answer in time");
                return None;
            }
        }
    };
    // The program has exited; anything it left running may keep the pipe open, so the wait for its output is short.
    let read = |rx: Option<mpsc::Receiver<String>>| {
        rx.and_then(|rx| rx.recv_timeout(Duration::from_secs(1)).ok()).unwrap_or_default()
    };
    Some(Ran { success: status.success(), stdout: read(out), stderr: read(err) })
}
