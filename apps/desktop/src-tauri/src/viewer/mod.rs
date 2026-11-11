//! The native viewer: a GPU surface aligned with the viewer rectangle of the UI (design D2/D3).
//!
//! Layering candidates live behind [`NativeLayer`]:
//! - candidate 1 "below": a native surface under a transparent web view (macOS: CAMetalLayer view
//!   below the WKWebView; Windows: child HWND at the bottom of the z-order under WebView2),
//! - candidate 2 "above": the same native surface placed above the web view,
//! - candidate 3 "stream": no native surface; frames are rendered offscreen and streamed to a
//!   canvas in the UI. Used on Linux (development only) and as the fallback everywhere.
//!
//! Rendering runs on one "viewer" thread that owns the GPU objects; platform view/window calls
//! run on the main thread.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use specta::Type;

use ove_gpu::{GpuContext, GpuOptions, LogicalRect, PresentOutcome, VideoFrame, ViewerRenderer, ViewerSurface};

#[cfg(target_os = "macos")]
pub mod macos;
pub mod stream;
#[cfg(windows)]
pub mod windows;

/// The viewer element's rectangle as measured by the UI, in CSS pixels relative to the web view,
/// plus the device pixel ratio. `visible: false` (or a zero size) hides the surface.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ViewerRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale_factor: f64,
    pub visible: bool,
}

impl ViewerRect {
    pub fn hidden() -> Self {
        ViewerRect { x: 0.0, y: 0.0, width: 0.0, height: 0.0, scale_factor: 1.0, visible: false }
    }

    pub fn is_shown(&self) -> bool {
        self.visible && self.width >= 1.0 && self.height >= 1.0
    }

    pub fn physical(&self) -> ove_gpu::PhysicalRect {
        let scale = if self.scale_factor.is_finite() && self.scale_factor > 0.0 { self.scale_factor } else { 1.0 };
        LogicalRect { x: self.x, y: self.y, width: self.width, height: self.height }.to_physical(scale)
    }
}

/// Which presentation path is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ViewerBackend {
    /// Candidate 1: native surface below a transparent web view.
    NativeBelow,
    /// Candidate 2: native surface above the web view.
    NativeAbove,
    /// Candidate 3: offscreen render streamed to a canvas.
    Stream,
}

/// Native view/window hosting the GPU surface. Methods are called on the main thread.
pub trait NativeLayer: Send {
    fn backend(&self) -> ViewerBackend;
    /// Positions and shows/hides the layer to match the viewer rectangle.
    fn apply_rect(&self, rect: &ViewerRect);
    /// The raw surface target for wgpu.
    ///
    /// # Safety
    /// The returned target is valid while `self` is alive.
    unsafe fn surface_target(&self) -> ove_gpu::wgpu::SurfaceTargetUnsafe;
}

/// Messages to the viewer thread.
pub enum ViewerMsg {
    Frame(VideoFrame),
    Rect(ViewerRect),
    /// Use a native layer's surface (the layer itself stays with the main thread).
    Attach {
        target: SendTarget,
        backend: ViewerBackend,
    },
    /// Diagnostics/test hook: destroy the GPU device as a driver reset would.
    LoseDevice,
    /// Forces a panic inside the viewer thread (crash-containment test hook).
    Panic,
    Quit,
}

/// A raw surface target moved to the viewer thread.
pub struct SendTarget(pub ove_gpu::wgpu::SurfaceTargetUnsafe);

// SAFETY: the target is only used to create the surface on the viewer thread; the native layer it
// points to is owned by the app state and outlives the viewer thread.
unsafe impl Send for SendTarget {}

/// Counters shown by the diagnostics overlay.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ViewerStats {
    pub backend: Option<ViewerBackend>,
    pub gpu_adapter: Option<String>,
    pub presented_frames: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub pixel_format: Option<String>,
    pub copies_last_frame: u32,
    pub last_present_ms: f64,
    pub device_recoveries: u32,
    pub physical_width: u32,
    pub physical_height: u32,
    pub image_x: i32,
    pub image_y: i32,
    pub image_width: u32,
    pub image_height: u32,
    /// Time from the last seek/step request to the frame being presented.
    pub last_seek_ms: Option<f64>,
}

/// Callbacks from the viewer thread to the app.
pub struct ViewerHooks {
    /// A streamed frame is ready (candidate 3).
    pub on_stream_frame: Box<dyn Fn(u64, u32, u32) + Send>,
    /// A contained failure to report to the UI.
    pub on_error: Box<dyn Fn(crate::error::AppError) + Send>,
    /// The presentation path changed.
    pub on_backend: Box<dyn Fn(ViewerBackend) + Send>,
}

struct ViewerThread {
    renderer: Option<ViewerRenderer>,
    seek_started: Arc<Mutex<Option<Instant>>>,
    surface: Option<ViewerSurface>,
    rect: ViewerRect,
    stream: Arc<stream::StreamSink>,
    stats: Arc<Mutex<ViewerStats>>,
    hooks: ViewerHooks,
    /// CPU fallback keeps the last frame when there is no GPU.
    last_frame: Option<VideoFrame>,
}

/// Starts the viewer thread. GPU initialisation happens on that thread so app start is not
/// delayed by adapter enumeration.
pub fn spawn(
    rx: Receiver<ViewerMsg>,
    stream: Arc<stream::StreamSink>,
    stats: Arc<Mutex<ViewerStats>>,
    seek_started: Arc<Mutex<Option<Instant>>>,
    hooks: ViewerHooks,
    gpu_ready: Sender<String>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new().name("viewer".into()).spawn(move || {
        let renderer = match GpuContext::new(GpuOptions { allow_software: true }) {
            Ok(ctx) => {
                let summary = ctx.summary().to_string();
                let _ = gpu_ready.send(summary.clone());
                if let Ok(mut s) = stats.lock() {
                    s.gpu_adapter = Some(summary);
                }
                Some(ViewerRenderer::new(ctx))
            }
            Err(e) => {
                tracing::error!("GPU unavailable, viewer uses the CPU fallback: {e}");
                let _ = gpu_ready.send(format!("none ({e})"));
                None
            }
        };
        if let Ok(mut s) = stats.lock() {
            s.backend = Some(ViewerBackend::Stream);
        }
        let mut t = ViewerThread {
            renderer,
            seek_started,
            surface: None,
            rect: ViewerRect::hidden(),
            stream,
            stats,
            hooks,
            last_frame: None,
        };
        while let Ok(msg) = rx.recv() {
            if matches!(msg, ViewerMsg::Quit) {
                break;
            }
            // A panic while handling one message is contained: logged (with backtrace by the
            // panic hook), reported to the UI, and the thread keeps serving.
            if let Err(e) = crate::worker::contain("the viewer", || t.handle(msg)) {
                (t.hooks.on_error)(e);
            }
        }
    })
}

impl ViewerThread {
    fn handle(&mut self, msg: ViewerMsg) {
        match msg {
            ViewerMsg::Frame(frame) => {
                if let Some(r) = self.renderer.as_mut()
                    && let Err(e) = r.recover_if_lost(None).and_then(|_| r.upload(&frame))
                {
                    tracing::warn!("frame upload failed: {e}");
                    (self.hooks.on_error)(e.into());
                    return;
                }
                if let Ok(mut s) = self.stats.lock() {
                    s.frame_width = frame.width;
                    s.frame_height = frame.height;
                    s.pixel_format = Some(format!("{:?}", frame.format));
                    s.copies_last_frame = 1;
                }
                self.last_frame = Some(frame);
                self.present();
            }
            ViewerMsg::Rect(rect) => {
                self.rect = rect;
                self.resize_surface();
                self.present();
            }
            ViewerMsg::Attach { target, backend } => self.attach(target, backend),
            ViewerMsg::LoseDevice => {
                if let Some(r) = &self.renderer {
                    r.context().simulate_device_loss();
                }
                self.present();
            }
            ViewerMsg::Panic => panic!("forced viewer panic (diagnostics)"),
            ViewerMsg::Quit => {}
        }
    }

    fn attach(&mut self, target: SendTarget, backend: ViewerBackend) {
        let Some(renderer) = &self.renderer else {
            tracing::warn!("no GPU: native layer not used, staying on the stream presenter");
            (self.hooks.on_backend)(ViewerBackend::Stream);
            return;
        };
        // SAFETY: the native layer behind `target` is owned by the app state for the app's life.
        match unsafe { ViewerSurface::from_raw(renderer, target.0) } {
            Ok(surface) => {
                tracing::info!(?backend, "native viewer surface attached");
                if let Ok(mut s) = self.stats.lock() {
                    s.backend = Some(backend);
                }
                self.surface = Some(surface);
                (self.hooks.on_backend)(backend);
                self.resize_surface();
                self.present();
            }
            Err(e) => {
                tracing::error!("native viewer surface failed, using the stream presenter: {e}");
                (self.hooks.on_backend)(ViewerBackend::Stream);
            }
        }
    }

    fn resize_surface(&mut self) {
        let phys = self.rect.physical();
        let (w, h) = if self.rect.is_shown() { (phys.width, phys.height) } else { (0, 0) };
        if let Ok(mut s) = self.stats.lock() {
            s.physical_width = w;
            s.physical_height = h;
        }
        if let (Some(surface), Some(renderer)) = (self.surface.as_mut(), self.renderer.as_ref()) {
            surface.resize(renderer, w, h);
        }
    }

    fn present(&mut self) {
        let started = Instant::now();
        let shown = self.rect.is_shown();
        let mut info = None;
        let mut recovered = false;
        let mut published = None;
        if let (Some(surface), Some(renderer)) = (self.surface.as_mut(), self.renderer.as_mut()) {
            for _ in 0..3 {
                match surface.present(renderer) {
                    Ok(PresentOutcome::Presented(i)) => {
                        info = Some(i);
                        break;
                    }
                    Ok(PresentOutcome::Skipped) => break,
                    Ok(PresentOutcome::Reconfigured) => continue,
                    Ok(PresentOutcome::Recovered) => {
                        recovered = true;
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!("present failed: {e}");
                        break;
                    }
                }
            }
        } else if shown {
            let phys = self.rect.physical();
            // Cap the streamed size; the canvas scales the rest (dev path only).
            let (w, h) = stream::cap_size(phys.width, phys.height);
            if let Some(renderer) = self.renderer.as_mut() {
                match renderer.recover_if_lost(None) {
                    Ok(true) => recovered = true,
                    Ok(false) => {}
                    Err(e) => tracing::warn!("GPU recovery failed: {e}"),
                }
                match renderer.render_to_rgba(w, h) {
                    Ok((rgba, i)) => {
                        info = Some(i);
                        let seq = self.stream.publish(w, h, rgba);
                        published = Some((seq, w, h));
                    }
                    Err(e) => tracing::warn!("stream render failed: {e}"),
                }
            } else if let Some(frame) = &self.last_frame {
                let rgba = ove_gpu::cpu::compose_letterboxed(frame, w, h, [0, 0, 0, 255]);
                let seq = self.stream.publish(w, h, rgba);
                published = Some((seq, w, h));
            }
        }
        if let Ok(mut s) = self.stats.lock() {
            if let Some(i) = info {
                s.presented_frames += 1;
                s.image_x = i.image.x;
                s.image_y = i.image.y;
                s.image_width = i.image.width;
                s.image_height = i.image.height;
            }
            if recovered {
                s.device_recoveries += 1;
            }
            if info.is_some()
                && let Some(t) = self.seek_started.lock().ok().and_then(|mut g| g.take())
            {
                s.last_seek_ms = Some(t.elapsed().as_secs_f64() * 1000.0);
            }
            s.last_present_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        // Notify after the stats are updated so listeners see consistent diagnostics.
        if let Some((seq, w, h)) = published {
            (self.hooks.on_stream_frame)(seq, w, h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_visibility() {
        let r = ViewerRect { x: 0.0, y: 0.0, width: 100.0, height: 50.0, scale_factor: 2.0, visible: true };
        assert!(r.is_shown());
        assert_eq!(r.physical().width, 200);
        assert!(!ViewerRect { visible: false, ..r }.is_shown());
        assert!(!ViewerRect { width: 0.0, ..r }.is_shown());
        assert!(!ViewerRect::hidden().is_shown());
        // A bogus scale factor never yields a zero or negative surface.
        assert_eq!(ViewerRect { scale_factor: f64::NAN, ..r }.physical().width, 100);
    }

    type Harness = (
        Sender<ViewerMsg>,
        Arc<stream::StreamSink>,
        Arc<Mutex<ViewerStats>>,
        Receiver<(u64, u32, u32)>,
        Receiver<crate::error::AppError>,
    );

    fn start() -> Harness {
        let (tx, rx) = std::sync::mpsc::channel();
        let sink = Arc::new(stream::StreamSink::default());
        let stats = Arc::new(Mutex::new(ViewerStats::default()));
        let (ftx, frx) = std::sync::mpsc::channel();
        let (etx, erx) = std::sync::mpsc::channel();
        let ftx = Mutex::new(ftx);
        let etx = Mutex::new(etx);
        let (gtx, grx) = std::sync::mpsc::channel();
        spawn(
            rx,
            sink.clone(),
            stats.clone(),
            Arc::new(Mutex::new(None)),
            ViewerHooks {
                on_stream_frame: Box::new(move |a, b, c| {
                    let _ = ftx.lock().unwrap().send((a, b, c));
                }),
                on_error: Box::new(move |e| {
                    let _ = etx.lock().unwrap().send(e);
                }),
                on_backend: Box::new(|_| {}),
            },
            gtx,
        )
        .unwrap();
        let _ = grx.recv();
        (tx, sink, stats, frx, erx)
    }

    #[test]
    fn stream_presenter_follows_rect_and_survives_panics() {
        let (tx, sink, stats, frames, errors) = start();
        let rect = ViewerRect { x: 10.0, y: 20.0, width: 320.0, height: 180.0, scale_factor: 2.0, visible: true };
        tx.send(ViewerMsg::Rect(rect)).unwrap();
        tx.send(ViewerMsg::Frame(crate::media::test_pattern_frame(5, 100, 30.0))).unwrap();
        let wait = std::time::Duration::from_secs(20);
        let (_, w, h) = frames.recv_timeout(wait).unwrap();
        assert_eq!((w, h), (640, 360), "physical size of the viewer rect");
        let (_, w, h) = frames.recv_timeout(wait).unwrap();
        assert_eq!((w, h), (640, 360));
        let latest = sink.latest().unwrap();
        assert_eq!(latest.rgba.len(), 640 * 360 * 4);

        tx.send(ViewerMsg::Panic).unwrap();
        let err = errors.recv_timeout(wait).unwrap();
        assert_eq!(err.code, crate::error::ErrorCode::WorkerPanic);

        // Still serving after the panic; device loss recovers and keeps the frame.
        tx.send(ViewerMsg::LoseDevice).unwrap();
        frames.recv_timeout(wait).unwrap();
        let s = stats.lock().unwrap().clone();
        if s.gpu_adapter.is_some() {
            assert_eq!(s.device_recoveries, 1);
        }
        assert_eq!((s.image_width, s.image_height), (640, 360));

        // Hidden viewer: nothing is streamed.
        tx.send(ViewerMsg::Rect(ViewerRect { visible: false, ..rect })).unwrap();
        tx.send(ViewerMsg::Quit).unwrap();
        assert!(frames.recv_timeout(std::time::Duration::from_millis(500)).is_err());
    }
}
