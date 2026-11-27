//! Timeline playback: a frame source that composites the open project's timeline on a worker thread, ahead of the
//! playhead, for the same player (clock, transport, A/V sync) that plays single files.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, TrySendError, channel, sync_channel};
use std::time::Duration;

use ove_core::model::Document;
use ove_playback::{FrameSource, PresentableFrame, SourcePoll};
use ove_render::Rgba;

/// Preview height of the composited timeline on the CPU backend. The preview frame rate is the timeline's own
/// (`fps`, default 30).
pub const PREVIEW_HEIGHT: u32 = 540;
/// Preview height when the GPU backend composites (design: preview resolution).
pub const GPU_PREVIEW_HEIGHT: u32 = 1080;

/// The preview height for a backend.
pub fn preview_height(backend: &str) -> u32 {
    if backend == "gpu" { GPU_PREVIEW_HEIGHT } else { PREVIEW_HEIGHT }
}

/// Start of frame `i` on the timeline's frame grid.
pub fn frame_us(i: i64, fps: f64) -> i64 {
    ove_core::time::frame_time(i, fps)
}

/// The frame showing at `t_us`.
fn frame_at(t_us: i64, fps: f64) -> i64 {
    ((t_us as f64 * fps / 1e6) + 1e-6).floor() as i64
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
    /// The compositing backend and why, for diagnostics ("GPU, <adapter>" or "CPU — <reason>").
    pub backend: Arc<std::sync::Mutex<String>>,
}

impl TimelineSource {
    pub fn new(doc: Arc<Document>, bundle: PathBuf) -> TimelineSource {
        let (ctl_tx, ctl_rx) = channel::<Ctl>();
        let (tx, rx) = sync_channel::<TimelineFrame>(4);
        let generation = Arc::new(AtomicU64::new(0));
        let render_ms = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicU64::new(0));
        let backend = Arc::new(std::sync::Mutex::new("starting".to_string()));
        let end_us = doc.duration();
        let (g, rm, dn, be) = (generation.clone(), render_ms.clone(), done.clone(), backend.clone());
        let _ = crate::worker::spawn_contained("timeline render", move || {
            render_loop(doc, bundle, ctl_rx, tx, g, Diag { render_ms: rm, backend: be }, dn)
        });
        TimelineSource { ctl: ctl_tx, frames: rx, generation, done, end_us, sent_eof: false, render_ms, backend }
    }

    pub fn duration_us(&self) -> i64 {
        self.end_us
    }
}

/// What the render loop reports for the diagnostics overlay.
struct Diag {
    render_ms: Arc<AtomicU64>,
    backend: Arc<std::sync::Mutex<String>>,
}

fn render_loop(
    doc: Arc<Document>,
    bundle: PathBuf,
    ctl: Receiver<Ctl>,
    out: SyncSender<TimelineFrame>,
    generation: Arc<AtomicU64>,
    diag: Diag,
    done: Arc<AtomicU64>,
) {
    let (backend, status) = crate::compositing::backend();
    let mut renderer = ove_render::Renderer::with_backend(Some(&bundle), preview_height(&status.backend), backend);
    *crate::worker::relock(&diag.backend) = crate::compositing::label(&status);
    let mut fell_back = false;
    // Playback reads proxies when the project says so; export and agent frames never do.
    renderer.set_use_proxies(ove_render::proxy::enabled(&doc));
    let end = doc.duration();
    let fps = doc.fps();
    let mut gen_now = 0u64;
    let mut next: Option<i64> = Some(0);
    loop {
        // Newest control message wins.
        loop {
            match ctl.try_recv() {
                Ok(Ctl::Seek(t, g)) => {
                    gen_now = g;
                    next = Some(frame_at(t, fps));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let Some(i) = next else {
            match ctl.recv_timeout(Duration::from_millis(50)) {
                Ok(Ctl::Seek(t, g)) => {
                    gen_now = g;
                    next = Some(frame_at(t, fps));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return,
            }
            continue;
        };
        let pts = frame_us(i, fps);
        if pts >= end.max(1) {
            done.store(gen_now + 1, Ordering::SeqCst);
            next = None;
            continue;
        }
        let started = std::time::Instant::now();
        let image = Arc::new(renderer.render(&doc, pts));
        diag.render_ms.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
        if !fell_back && let Some(why) = renderer.fallback() {
            // A GPU device loss: the renderer carries on with the CPU backend; say so in the overlay.
            fell_back = true;
            crate::compositing::fell_back(why);
            *crate::worker::relock(&diag.backend) = format!("CPU — {why}");
        }
        let mut frame =
            TimelineFrame { generation: gen_now, pts_us: pts, duration_us: frame_us(i + 1, fps) - pts, image };
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

/// Converts a composited frame for the viewer (the frame being presented).
pub fn to_viewer(f: &TimelineFrame) -> ove_gpu::VideoFrame {
    // The colour panel's scopes show the frame the viewer shows.
    crate::scopes::offer(&f.image, f.pts_us);
    ove_gpu::VideoFrame::rgba(f.image.w, f.image.h, f.image.data.clone(), f.pts_us)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gpu_previews_at_1080_lines() {
        assert_eq!(preview_height("gpu"), 1080);
        assert_eq!(preview_height("cpu"), 540);
    }
}
