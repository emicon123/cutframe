//! A plain Win32 child window that the engine renders the preview into.
//!
//! It is a sibling of the WebView2 controller inside the main window and sits on top of it in
//! z-order, so it covers the preview rectangle reported by the UI. The DOM cannot draw over it.

use std::num::NonZeroIsize;

use raw_window_handle::{
    HasWindowHandle, RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use tauri::WebviewWindow;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HTTRANSPARENT, HWND_TOP, RegisterClassW, SW_HIDE, SW_SHOWNA,
    SWP_NOACTIVATE, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WM_ERASEBKGND, WM_NCHITTEST,
    WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS,
};
use windows::core::w;

use crate::ipc::types::PreviewRect;

/// The child window's HWND, stored as an integer so it can be copied to the main thread.
#[derive(Debug, Clone, Copy)]
pub struct ChildWindow {
    hwnd: NonZeroIsize,
}

#[derive(Debug, thiserror::Error)]
pub enum ChildWindowError {
    #[error("main window has no Win32 handle: {0}")]
    Handle(String),
    #[error("Win32 error: {0}")]
    Win32(#[from] windows::core::Error),
}

pub fn raw_display_handle() -> RawDisplayHandle {
    RawDisplayHandle::Windows(WindowsDisplayHandle::new())
}

impl ChildWindow {
    /// Creates a hidden child window of `main`. Must run on the main thread.
    pub fn create(main: &WebviewWindow) -> Result<Self, ChildWindowError> {
        let parent = match main
            .window_handle()
            .map_err(|e| ChildWindowError::Handle(e.to_string()))?
            .as_raw()
        {
            RawWindowHandle::Win32(handle) => HWND(handle.hwnd.get() as *mut _),
            other => {
                return Err(ChildWindowError::Handle(format!(
                    "unexpected handle {other:?}"
                )));
            }
        };

        let class = w!("CutframePreview");
        // SAFETY: plain Win32 calls with valid arguments; `wndproc` matches the WNDPROC signature.
        let hwnd = unsafe {
            let instance = GetModuleHandleW(None)?;
            let class_def = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: class,
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                ..Default::default()
            };
            // Returns 0 if the class already exists, which is fine.
            RegisterClassW(&class_def);
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                w!("Preview"),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                Some(parent),
                None,
                Some(instance.into()),
                None,
            )?
        };
        let hwnd = NonZeroIsize::new(hwnd.0 as isize)
            .ok_or_else(|| ChildWindowError::Handle("null HWND".into()))?;
        Ok(Self { hwnd })
    }

    pub fn raw_window_handle(&self) -> RawWindowHandle {
        RawWindowHandle::Win32(Win32WindowHandle::new(self.hwnd))
    }

    /// Moves the window over `rect` and keeps it on top. Never changes its visibility: only
    /// [`Self::set_visible`] does, so a resize cannot show a preview the UI has hidden
    /// (ADR-0005 §3). Call on the main thread.
    pub fn set_rect(self, rect: PreviewRect) {
        let width = i32::try_from(rect.width).unwrap_or(i32::MAX);
        let height = i32::try_from(rect.height).unwrap_or(i32::MAX);
        // SAFETY: `hwnd` is a live child window created by `create`.
        let result = unsafe {
            SetWindowPos(
                self.hwnd(),
                Some(HWND_TOP),
                rect.x,
                rect.y,
                width,
                height,
                SWP_NOACTIVATE,
            )
        };
        if let Err(err) = result {
            tracing::error!(%err, "SetWindowPos failed for the preview window");
        }
    }

    /// Call on the main thread.
    pub fn set_visible(self, visible: bool) {
        // SAFETY: `hwnd` is a live child window created by `create`.
        unsafe {
            let _ = ShowWindow(self.hwnd(), if visible { SW_SHOWNA } else { SW_HIDE });
        }
    }

    fn hwnd(self) -> HWND {
        HWND(self.hwnd.get() as *mut _)
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // The GPU paints every pixel; skip GDI background erasing to avoid flicker.
        WM_ERASEBKGND => LRESULT(1),
        // Stage 0: let mouse input fall through to the parent. Preview interaction comes later.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        // SAFETY: forwarding the unmodified message to the default window procedure.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
