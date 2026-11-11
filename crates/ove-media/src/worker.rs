//! A decoder on its own thread feeding a bounded frame queue, with seek generations and panic containment.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::error::MediaError;
use crate::frame::{DecodePath, VideoFrame};
use crate::util::Micros;
use crate::video::VideoDecoder;

/// Default queue depth (frames). At 4K NV12 this is ~100 MB of decoded frames.
pub const DEFAULT_QUEUE_FRAMES: usize = 8;

/// What the worker delivers. Every event carries the seek generation it belongs to; [`DecodeWorker`] drops events
/// from older generations, so after `seek()` the caller only sees frames of the new position.
#[derive(Debug)]
pub enum WorkerEvent {
    Frame(VideoFrame),
    /// End of stream reached; the worker waits for the next seek.
    Eof,
    /// Decoding stopped with an error (including a caught panic). The worker waits for the next seek.
    Error(MediaError),
}

enum Cmd {
    Seek { t_us: Micros, generation: u64 },
    Stop,
}

/// Owns a decode thread. Dropping it stops the thread.
pub struct DecodeWorker {
    cmd: mpsc::Sender<Cmd>,
    events: Receiver<(u64, WorkerEvent)>,
    generation: u64,
    path: Arc<Mutex<DecodePath>>,
    handle: Option<JoinHandle<()>>,
}

impl DecodeWorker {
    /// Start decoding from the decoder's current position with a queue of `capacity` frames.
    pub fn spawn(decoder: Box<dyn VideoDecoder>, capacity: usize) -> DecodeWorker {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::sync_channel(capacity.max(1));
        let path = Arc::new(Mutex::new(decoder.decode_path()));
        let path2 = path.clone();
        let handle = std::thread::Builder::new()
            .name("ove-decode".into())
            .spawn(move || run(decoder, cmd_rx, ev_tx, path2))
            .expect("spawn decode thread");
        DecodeWorker {
            cmd: cmd_tx,
            events: ev_rx,
            generation: 0,
            path,
            handle: Some(handle),
        }
    }

    /// Flush queued frames and reposition; the next frame is the one whose interval contains `t_us`.
    pub fn seek(&mut self, t_us: Micros) {
        self.generation += 1;
        let _ = self.cmd.send(Cmd::Seek {
            t_us,
            generation: self.generation,
        });
        // Drop whatever is queued now; later stale events are filtered by generation.
        while self.events.try_recv().is_ok() {}
    }

    /// Next event of the current generation, waiting up to `timeout`. `None` on timeout.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Option<WorkerEvent> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.events.recv_timeout(left) {
                Ok((g, ev)) if g == self.generation => return Some(ev),
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) => return None,
                Err(RecvTimeoutError::Disconnected) => {
                    return Some(WorkerEvent::Error(MediaError::WorkerPanicked(
                        "decode thread exited".into(),
                    )));
                }
            }
        }
    }

    /// Next event of the current generation if one is ready.
    pub fn try_recv(&mut self) -> Option<WorkerEvent> {
        loop {
            match self.events.try_recv() {
                Ok((g, ev)) if g == self.generation => return Some(ev),
                Ok(_) => continue,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    return Some(WorkerEvent::Error(MediaError::WorkerPanicked(
                        "decode thread exited".into(),
                    )));
                }
            }
        }
    }

    /// The decode path currently in use (changes on fallback).
    pub fn decode_path(&self) -> DecodePath {
        self.path
            .lock()
            .map(|p| p.clone())
            .unwrap_or(DecodePath::Software)
    }
}

impl Drop for DecodeWorker {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Stop);
        // Unblock a worker waiting on a full queue.
        while self.events.try_recv().is_ok() {}
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn panic_message(p: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".into()
    }
}

fn run(
    mut decoder: Box<dyn VideoDecoder>,
    cmd: Receiver<Cmd>,
    tx: SyncSender<(u64, WorkerEvent)>,
    path: Arc<Mutex<DecodePath>>,
) {
    let mut generation = 0u64;
    let mut idle = false; // after EOF / error: wait for a command
    loop {
        // Commands first; block only when idle.
        let next = if idle {
            match cmd.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        } else {
            match cmd.try_recv() {
                Ok(c) => Some(c),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        };
        if let Some(c) = next {
            match c {
                Cmd::Stop => return,
                Cmd::Seek {
                    t_us,
                    generation: g,
                } => {
                    generation = g;
                    idle = false;
                    let r = catch_unwind(AssertUnwindSafe(|| decoder.seek(t_us)));
                    let err = match r {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(e),
                        Err(p) => Some(MediaError::WorkerPanicked(panic_message(p))),
                    };
                    if let Some(e) = err {
                        idle = true;
                        let fatal = matches!(e, MediaError::WorkerPanicked(_));
                        if tx.send((generation, WorkerEvent::Error(e))).is_err() || fatal {
                            return;
                        }
                    }
                    continue;
                }
            }
        }

        let r = catch_unwind(AssertUnwindSafe(|| decoder.next_frame()));
        if let Ok(mut p) = path.lock() {
            *p = decoder.decode_path();
        }
        let (ev, fatal) = match r {
            Ok(Ok(Some(f))) => (WorkerEvent::Frame(f), false),
            Ok(Ok(None)) => {
                idle = true;
                (WorkerEvent::Eof, false)
            }
            Ok(Err(e)) => {
                idle = true;
                (WorkerEvent::Error(e), false)
            }
            Err(p) => (
                WorkerEvent::Error(MediaError::WorkerPanicked(panic_message(p))),
                true,
            ),
        };
        // Send without blocking command handling: while the queue is full, keep watching for seek/stop.
        let mut msg = (generation, ev);
        loop {
            match tx.try_send(msg) {
                Ok(()) => break,
                Err(TrySendError::Disconnected(_)) => return,
                Err(TrySendError::Full(m)) => {
                    msg = m;
                    match cmd.recv_timeout(Duration::from_millis(2)) {
                        Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                        Ok(Cmd::Seek {
                            t_us,
                            generation: g,
                        }) => {
                            // Re-queue the seek for the top of the loop and drop the stale frame.
                            generation = g;
                            idle = false;
                            let r = catch_unwind(AssertUnwindSafe(|| decoder.seek(t_us)));
                            match r {
                                Ok(Ok(())) => {}
                                Ok(Err(e)) => {
                                    idle = true;
                                    let _ = tx.send((generation, WorkerEvent::Error(e)));
                                }
                                Err(p) => {
                                    let _ = tx.send((
                                        generation,
                                        WorkerEvent::Error(MediaError::WorkerPanicked(
                                            panic_message(p),
                                        )),
                                    ));
                                    return;
                                }
                            }
                            break;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                }
            }
        }
        if fatal {
            return;
        }
    }
}
