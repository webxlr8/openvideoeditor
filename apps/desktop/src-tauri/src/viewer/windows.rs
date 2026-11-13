//! Windows candidates 1 and 2: a child HWND sized to the viewer rectangle that hosts the D3D12
//! swap chain. Candidate 1 puts it at the bottom of the z-order under a WebView2 with a
//! transparent background; candidate 2 puts it on top. Mouse input passes through to the web
//! view either way (`HTTRANSPARENT`).
//!
//! Not the default (see `requested_backend`): measured on Windows Server 2025 with WebView2 156, candidate 1 shows
//! the desktop behind the app window where the picture should be: WebView2's transparent background does not reveal
//! a sibling child window below it. Candidate 2 is unmeasured. The DirectComposition-visual variant of candidate 1 is
//! not implemented yet. Selected only with `OVE_VIEWER_LAYERING=below|above`.

use std::num::NonZeroIsize;
use std::sync::Once;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HTTRANSPARENT, HWND_BOTTOM, HWND_TOP, RegisterClassW, SWP_HIDEWINDOW,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, WINDOW_EX_STYLE, WM_NCHITTEST, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS,
};
use windows::core::w;

use super::{NativeLayer, ViewerBackend, ViewerRect};

/// Window procedure of the viewer surface class: transparent to hit tests, default handling otherwise.
///
/// # Safety
///
/// Called by Windows only, with the arguments of a message sent to a window of this class.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCHITTEST {
        return LRESULT(HTTRANSPARENT as isize);
    }
    // SAFETY: the arguments are exactly those Windows passed to this window procedure, forwarded unchanged.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub struct ChildSurfaceWindow {
    hwnd: NonZeroIsize,
    hinstance: isize,
    below: bool,
}

// SAFETY: the struct holds only plain handle values (no pointers it dereferences). The window calls that need
// the UI thread (`apply_rect`) are made there by the caller; the viewer thread only builds a raw window handle for
// wgpu, which supports creating a D3D12 swap chain for an HWND from another thread.
unsafe impl Send for ChildSurfaceWindow {}

impl ChildSurfaceWindow {
    /// Creates the child window inside `parent` (the main window's HWND). UI thread only.
    pub fn new(parent: isize, below: bool) -> Result<Self, String> {
        static REGISTER: Once = Once::new();
        // SAFETY: the class name and window title are static wide strings; `wndproc` matches the WNDPROC
        // signature; the class is registered once per process before any window of it is created; `parent` is the
        // main window's live HWND, passed by the caller on the UI thread.
        unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            REGISTER.call_once(|| {
                let class = WNDCLASSW {
                    lpfnWndProc: Some(wndproc),
                    hInstance: hinstance.into(),
                    lpszClassName: w!("OveViewerSurface"),
                    hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                    ..Default::default()
                };
                RegisterClassW(&class);
            });
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("OveViewerSurface"),
                None,
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                Some(HWND(parent as *mut _)),
                None,
                Some(hinstance.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            let hwnd = NonZeroIsize::new(hwnd.0 as isize).ok_or("CreateWindowExW returned a null window")?;
            Ok(ChildSurfaceWindow { hwnd, hinstance: hinstance.0 as isize, below })
        }
    }
}

impl NativeLayer for ChildSurfaceWindow {
    fn backend(&self) -> ViewerBackend {
        if self.below { ViewerBackend::NativeBelow } else { ViewerBackend::NativeAbove }
    }

    fn apply_rect(&self, rect: &ViewerRect) {
        let p = rect.physical();
        let order = if self.below { HWND_BOTTOM } else { HWND_TOP };
        let show = if rect.is_shown() { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
        // SAFETY: `self.hwnd` is the child window this struct created, destroyed only with its parent (which
        // outlives the layer); `SetWindowPos` takes plain values otherwise.
        let _ = unsafe {
            SetWindowPos(
                HWND(self.hwnd.get() as *mut _),
                Some(order),
                p.x,
                p.y,
                p.width.max(1) as i32,
                p.height.max(1) as i32,
                SWP_NOACTIVATE | show,
            )
        };
    }

    /// See [`NativeLayer::surface_target`].
    unsafe fn surface_target(&self) -> ove_gpu::wgpu::SurfaceTargetUnsafe {
        let mut handle = raw_window_handle::Win32WindowHandle::new(self.hwnd);
        handle.hinstance = NonZeroIsize::new(self.hinstance);
        ove_gpu::wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(raw_window_handle::RawDisplayHandle::Windows(
                raw_window_handle::WindowsDisplayHandle::new(),
            )),
            raw_window_handle: raw_window_handle::RawWindowHandle::Win32(handle),
        }
    }
}

// No Drop: DestroyWindow must run on the UI thread that created the window; the child is
// destroyed together with its parent window.
