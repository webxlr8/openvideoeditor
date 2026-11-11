//! Timeline playback: a frame source that composites the open project's timeline on a worker thread, ahead of the
//! playhead, for the same player (clock, transport, A/V sync) that plays single files.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, TrySendError, channel, sync_channel};
use std::time::Duration;

use ove_core::model::Document;
use ove_playback::{FrameSource, PresentableFrame, SourcePoll};
use ove_render::{Renderer, Rgba};

/// Preview frame rate and height of the composited timeline.
pub const TIMELINE_FPS: f64 = 30.0;
pub const PREVIEW_HEIGHT: u32 = 540;

pub fn frame_us(i: i64) -> i64 {
    (i as f64 * 1_000_000.0 / TIMELINE_FPS).round() as i64
}

#[derive(Debug, Clone)]
pub struct TimelineFrame {
    pub generation: u64,
    pub pts_us: i64,
    pub duration_us: i64,
    pub image: Arc<Rgba>,
}

impl PresentableFrame for TimelineFrame {
    fn pts_us(&self) -> i64 {
        self.pts_us
    }
    fn duration_us(&self) -> i64 {
        self.duration_us
    }
}

enum Ctl {
    Seek(i64, u64),
}

/// Renders frames from a position forward on its own thread.
pub struct TimelineSource {
    ctl: Sender<Ctl>,
    frames: Receiver<TimelineFrame>,
    generation: Arc<AtomicU64>,
    /// Generation + 1 of the last request rendered through to the end (0: none).
    done: Arc<AtomicU64>,
    end_us: i64,
    sent_eof: bool,
    /// Render time of the last frame, for diagnostics.
    pub render_ms: Arc<AtomicU64>,
}

impl TimelineSource {
    pub fn new(doc: Arc<Document>, bundle: PathBuf) -> TimelineSource {
        let (ctl_tx, ctl_rx) = channel::<Ctl>();
        let (tx, rx) = sync_channel::<TimelineFrame>(4);
        let generation = Arc::new(AtomicU64::new(0));
        let render_ms = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicU64::new(0));
        let end_us = doc.duration();
        let (g, rm, dn) = (generation.clone(), render_ms.clone(), done.clone());
        let _ =
            crate::worker::spawn_contained("timeline render", move || render_loop(doc, bundle, ctl_rx, tx, g, rm, dn));
        TimelineSource { ctl: ctl_tx, frames: rx, generation, done, end_us, sent_eof: false, render_ms }
    }

    pub fn duration_us(&self) -> i64 {
        self.end_us
    }
}

fn render_loop(
    doc: Arc<Document>,
    bundle: PathBuf,
    ctl: Receiver<Ctl>,
    out: SyncSender<TimelineFrame>,
    generation: Arc<AtomicU64>,
    render_ms: Arc<AtomicU64>,
    done: Arc<AtomicU64>,
) {
    let mut renderer = Renderer::new(Some(&bundle), PREVIEW_HEIGHT);
    let end = doc.duration();
    let mut gen_now = 0u64;
    let mut next: Option<i64> = Some(0);
    loop {
        // Newest control message wins.
        loop {
            match ctl.try_recv() {
                Ok(Ctl::Seek(t, g)) => {
                    gen_now = g;
                    next = Some(((t as f64 * TIMELINE_FPS / 1e6) + 1e-6).floor() as i64);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let Some(i) = next else {
            match ctl.recv_timeout(Duration::from_millis(50)) {
                Ok(Ctl::Seek(t, g)) => {
                    gen_now = g;
                    next = Some(((t as f64 * TIMELINE_FPS / 1e6) + 1e-6).floor() as i64);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return,
            }
            continue;
        };
        let pts = frame_us(i);
        if pts >= end.max(1) {
            done.store(gen_now + 1, Ordering::SeqCst);
            next = None;
            continue;
        }
        let started = std::time::Instant::now();
        let image = Arc::new(renderer.render(&doc, pts));
        render_ms.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
        let mut frame = TimelineFrame { generation: gen_now, pts_us: pts, duration_us: frame_us(i + 1) - pts, image };
        // Hand it over, but give way to a seek while the queue is full.
        loop {
            match out.try_send(frame) {
                Ok(()) => break,
                Err(TrySendError::Full(f)) => {
                    if generation.load(Ordering::Relaxed) != gen_now {
                        break;
                    }
                    frame = f;
                    std::thread::sleep(Duration::from_millis(3));
                }
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        next = Some(i + 1);
    }
}

impl FrameSource for TimelineSource {
    type Frame = TimelineFrame;

    fn seek(&mut self, t_us: i64) {
        let g = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        while self.frames.try_recv().is_ok() {}
        self.sent_eof = false;
        let _ = self.ctl.send(Ctl::Seek(t_us.clamp(0, self.end_us.max(1)), g));
    }

    fn poll(&mut self) -> SourcePoll<TimelineFrame> {
        let g = self.generation.load(Ordering::SeqCst);
        loop {
            match self.frames.try_recv() {
                Ok(f) if f.generation == g => return SourcePoll::Frame(f),
                Ok(_) => continue,
                Err(TryRecvError::Empty) => {
                    if !self.sent_eof && self.done.load(Ordering::SeqCst) == g + 1 {
                        self.sent_eof = true;
                        return SourcePoll::Eof;
                    }
                    return SourcePoll::Pending;
                }
                Err(TryRecvError::Disconnected) => return SourcePoll::Error("the timeline renderer stopped".into()),
            }
        }
    }
}

/// Converts a composited frame for the viewer.
pub fn to_viewer(f: &TimelineFrame) -> ove_gpu::VideoFrame {
    ove_gpu::VideoFrame::rgba(f.image.w, f.image.h, f.image.data.clone(), f.pts_us)
}
