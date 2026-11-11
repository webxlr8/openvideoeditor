//! macOS candidate 1: an NSView backed by a CAMetalLayer, inserted into the web view's parent
//! directly below the (transparent) WKWebView, so web menus and dialogs draw over the video.
//!
//! UNVERIFIED on hardware: compiled for aarch64-apple-darwin only; placement scenarios still need
//! the screenshot tests on a Mac (tasks 5.2).

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::CAMetalLayer;

use super::{NativeLayer, ViewerBackend, ViewerRect};

pub struct MetalLayerView {
    view: Retained<NSView>,
    layer: Retained<CAMetalLayer>,
    webview: Retained<NSView>,
    backend: ViewerBackend,
}

// SAFETY: the AppKit objects are only touched from the main thread (`apply_rect` is dispatched
// there by the caller); the viewer thread only uses the CAMetalLayer pointer through wgpu, which
// is supported off the main thread for drawable acquisition.
unsafe impl Send for MetalLayerView {}

impl MetalLayerView {
    /// Creates the layer view next to the WKWebView `webview` (from `with_webview`).
    ///
    /// # Safety
    /// `webview` must be a valid WKWebView pointer; must be called on the main thread.
    pub unsafe fn new(webview: *mut c_void, below: bool) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
        // SAFETY: caller guarantees a valid NSView subclass (WKWebView) pointer.
        let webview: Retained<NSView> = unsafe { Retained::retain(webview.cast::<NSView>()) }.ok_or("null web view")?;
        // SAFETY: plain property read on the main thread.
        let parent = unsafe { webview.superview() }.ok_or("web view has no parent view")?;
        let view =
            NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0)));
        let layer = CAMetalLayer::new();
        layer.setOpaque(true);
        view.setWantsLayer(true);
        view.setLayer(Some(&layer));
        view.setHidden(true);
        let order = if below { NSWindowOrderingMode::Below } else { NSWindowOrderingMode::Above };
        parent.addSubview_positioned_relativeTo(&view, order, Some(&webview));
        Ok(MetalLayerView {
            view,
            layer,
            webview,
            backend: if below { ViewerBackend::NativeBelow } else { ViewerBackend::NativeAbove },
        })
    }
}

impl NativeLayer for MetalLayerView {
    fn backend(&self) -> ViewerBackend {
        self.backend
    }

    fn apply_rect(&self, rect: &ViewerRect) {
        if !rect.is_shown() {
            self.view.setHidden(true);
            return;
        }
        // The parent view is not flipped: convert the top-left CSS rectangle to bottom-left.
        let height = self.webview.frame().size.height;
        let frame =
            NSRect::new(NSPoint::new(rect.x, height - rect.y - rect.height), NSSize::new(rect.width, rect.height));
        self.view.setFrame(frame);
        self.layer.setContentsScale(rect.scale_factor);
        self.view.setHidden(false);
    }

    unsafe fn surface_target(&self) -> ove_gpu::wgpu::SurfaceTargetUnsafe {
        ove_gpu::wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(Retained::as_ptr(&self.layer) as *mut c_void)
    }
}
