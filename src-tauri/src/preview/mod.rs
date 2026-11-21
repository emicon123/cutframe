//! Puts the engine's preview on screen.
//!
//! - Windows: a native child window over the UI's preview rectangle; the engine renders into it
//!   with the GPU, so frames never cross IPC (ADR-0001).
//! - Other platforms (Linux/Pi development): the UI pulls offscreen frames through
//!   `media://localhost/preview/frame` and draws them on a canvas.

#[cfg(windows)]
mod windows;

use std::time::Duration;

use app_core::ProjectSettings;
use engine::preview::PreviewHandle;
use engine::render::{GpuInfo, OffscreenFrame};
use tauri::{AppHandle, WebviewWindow};

use crate::ipc::types::{PreviewMode, PreviewRect};

/// Largest frame served to the UI in `frames` mode.
const MAX_FRAME_SIZE: (u32, u32) = (1920, 1080);

#[derive(Debug)]
pub struct Preview {
    engine: PreviewHandle,
    #[cfg(windows)]
    child: windows::ChildWindow,
}

impl Preview {
    /// Starts the preview for `main`. Must run on the main thread (window creation).
    /// Returns `None` when no GPU is available; the app keeps working without a preview.
    pub fn create(main: &WebviewWindow, settings: &ProjectSettings) -> Option<Self> {
        let engine = match PreviewHandle::spawn(aspect(settings)) {
            Ok(engine) => engine,
            Err(err) => {
                tracing::error!(%err, "preview unavailable");
                return None;
            }
        };

        #[cfg(windows)]
        {
            let child = match windows::ChildWindow::create(main) {
                Ok(child) => child,
                Err(err) => {
                    tracing::error!(%err, "cannot create the preview window");
                    return None;
                }
            };
            // SAFETY: the child window lives as long as the main window, which outlives the
            // engine (the app exits when the main window closes).
            unsafe {
                engine.attach_window(
                    child.raw_window_handle(),
                    Some(windows::raw_display_handle()),
                    1,
                    1,
                )
            };
            engine.set_visible(false);
            Some(Self { engine, child })
        }

        #[cfg(not(windows))]
        {
            let _ = main;
            Some(Self { engine })
        }
    }

    pub fn mode(&self) -> PreviewMode {
        if cfg!(windows) {
            PreviewMode::Native
        } else {
            PreviewMode::Frames
        }
    }

    pub fn gpu_info(&self) -> &GpuInfo {
        self.engine.gpu_info()
    }

    pub fn set_canvas_aspect(&self, settings: &ProjectSettings) {
        self.engine.set_canvas_aspect(aspect(settings));
    }

    pub fn set_rect(&self, app: &AppHandle, rect: PreviewRect) {
        #[cfg(windows)]
        {
            let child = self.child;
            if let Err(err) = app.run_on_main_thread(move || child.set_rect(rect)) {
                tracing::error!(%err, "cannot move the preview window");
            }
            self.engine.resize(rect.width, rect.height);
            self.engine.set_visible(rect.width > 0 && rect.height > 0);
        }
        #[cfg(not(windows))]
        let _ = (app, rect);
    }

    pub fn set_visible(&self, app: &AppHandle, visible: bool) {
        #[cfg(windows)]
        {
            let child = self.child;
            if let Err(err) = app.run_on_main_thread(move || child.set_visible(visible)) {
                tracing::error!(%err, "cannot show or hide the preview window");
            }
        }
        #[cfg(not(windows))]
        let _ = app;
        self.engine.set_visible(visible);
    }

    /// One offscreen frame, clamped to [`MAX_FRAME_SIZE`].
    pub fn render_frame(&self, width: u32, height: u32) -> Option<OffscreenFrame> {
        let width = width.clamp(1, MAX_FRAME_SIZE.0);
        let height = height.clamp(1, MAX_FRAME_SIZE.1);
        self.engine
            .render_frame(width, height, Duration::from_secs(5))
    }
}

fn aspect(settings: &ProjectSettings) -> f32 {
    settings.width.max(1) as f32 / settings.height.max(1) as f32
}
