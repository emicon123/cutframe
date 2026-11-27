//! Puts the engine's preview on screen.
//!
//! - Windows: a native child window over the UI's preview rectangle; the engine renders into it
//!   with the GPU, so frames never cross IPC (ADR-0001).
//! - Other platforms (Linux/Pi development): the UI pulls offscreen frames through
//!   `media://localhost/preview/frame` and draws them on a canvas.

#[cfg(windows)]
mod windows;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use app_core::Project;
use arc_swap::ArcSwap;
use engine::playback::{Playback, PlaybackTicks};
use engine::preview::{GpuChanged, MediaUnavailable, PreviewHandle, PreviewSetup};
use engine::render::{GpuError, GpuInfo, OffscreenFrame};
use engine::transport::PlayState;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_specta::Event;

use crate::ipc::events::{AppInfoChanged, PlaybackTick};
use crate::ipc::types::{AppInfo, PreviewMode, PreviewRect};

/// Largest frame served to the UI in `frames` mode.
const MAX_FRAME_SIZE: (u32, u32) = (1920, 1080);

#[derive(Debug)]
pub struct Preview {
    engine: PreviewHandle,
    /// Held while the resulting show/hide is dispatched, so the window and the engine see the
    /// changes in the same order as the state.
    visibility: Mutex<Visibility>,
    #[cfg(windows)]
    child: windows::ChildWindow,
}

impl Preview {
    /// Starts the preview for `main`, showing the project in `snapshot` (`AppState`'s shared
    /// snapshot). Must run on the main thread (window creation). Returns `None` when no GPU is
    /// available; the app keeps working without a preview.
    pub fn create(main: &WebviewWindow, snapshot: Arc<ArcSwap<Project>>) -> Option<Self> {
        let visibility = Mutex::new(Visibility::INITIAL);
        let app = main.app_handle().clone();
        let setup = PreviewSetup {
            on_gpu_changed: report_gpu_changes(app.clone()),
            on_media_unavailable: report_missing_media(app.clone()),
            on_tick: report_ticks(app),
            ..PreviewSetup::with_sound(snapshot)
        };

        #[cfg(windows)]
        {
            // Created hidden, matching `Visibility::INITIAL`. It exists before the engine starts,
            // so the engine picks an adapter that can present to it.
            let child = match windows::ChildWindow::create(main) {
                Ok(child) => child,
                Err(err) => {
                    tracing::error!(%err, "cannot create the preview window");
                    return None;
                }
            };
            // SAFETY: the child window lives as long as the main window, which outlives the
            // engine (the app exits when the main window closes).
            let engine = unsafe {
                PreviewHandle::spawn_with_window(
                    setup,
                    child.raw_window_handle(),
                    Some(windows::raw_display_handle()),
                )
            };
            // If the engine fails, the hidden child window is left behind; it is harmless.
            let engine = start(engine)?;
            Some(Self {
                engine,
                visibility,
                child,
            })
        }

        #[cfg(not(windows))]
        {
            let _ = main;
            let engine = start(PreviewHandle::spawn(setup))?;
            Some(Self { engine, visibility })
        }
    }

    /// Whether the engine renders into the native preview window. False when no GPU adapter can
    /// present to it.
    #[cfg(windows)]
    pub fn has_native_window(&self) -> bool {
        self.engine.has_window()
    }

    /// Frames presented to the native preview window so far.
    #[cfg(windows)]
    pub fn presented_frames(&self) -> u64 {
        self.engine.presented_frames()
    }

    /// Draws and presents once although nothing changed (`--smoke-test`).
    #[cfg(windows)]
    pub fn request_redraw(&self) {
        self.engine.request_redraw();
    }

    /// The platform's mode, or `unavailable` once the engine gave up after GPU losses.
    pub fn mode(&self) -> PreviewMode {
        preview_mode(self.is_available())
    }

    /// False once the engine gave up after repeated GPU losses.
    pub fn is_available(&self) -> bool {
        self.engine.is_available()
    }

    /// The GPU in use now; changes after a runtime device loss.
    pub fn gpu_info(&self) -> Arc<GpuInfo> {
        self.engine.gpu_info()
    }

    /// `AppState` published a new snapshot: the engine redraws.
    pub fn snapshot_changed(&self) {
        self.engine.snapshot_changed();
    }

    /// The transport (play, pause, shuttle, step, seek, loop).
    pub fn playback(&self) -> &Arc<Playback> {
        self.engine.playback()
    }

    /// The shared GPU context for export (ADR-0004 §6); `None` once the preview gave up.
    pub fn gpu(&self) -> Option<Arc<engine::render::GpuContext>> {
        self.engine.gpu()
    }

    /// Moves the preview over `rect`. Shows it only if the UI has requested it; an empty `rect`
    /// hides it. Never shows a preview the UI has hidden (ADR-0005 §3).
    pub fn set_rect(&self, app: &AppHandle, rect: PreviewRect) {
        let mut visibility = self.visibility();
        let change = visibility.set_rect(rect);
        #[cfg(windows)]
        {
            let child = self.child;
            // Move first, then show, so the window never flashes at its old position.
            let result = app.run_on_main_thread(move || {
                child.set_rect(rect);
                if let Some(shown) = change {
                    child.set_visible(shown);
                }
            });
            if let Err(err) = result {
                tracing::error!(%err, "cannot move the preview window");
            }
            self.engine.resize(rect.width, rect.height);
        }
        #[cfg(not(windows))]
        let _ = app;
        if let Some(shown) = change {
            self.engine.set_visible(shown);
        }
    }

    /// The UI's visibility request, e.g. `false` while a dialog overlaps the preview area. The
    /// preview is shown only once the rectangle is non-empty as well.
    pub fn set_visible(&self, app: &AppHandle, visible: bool) {
        let mut visibility = self.visibility();
        let Some(shown) = visibility.request(visible) else {
            return;
        };
        #[cfg(windows)]
        {
            let child = self.child;
            if let Err(err) = app.run_on_main_thread(move || child.set_visible(shown)) {
                tracing::error!(%err, "cannot show or hide the preview window");
            }
        }
        #[cfg(not(windows))]
        let _ = app;
        self.engine.set_visible(shown);
    }

    /// One offscreen frame, clamped to [`MAX_FRAME_SIZE`].
    pub fn render_frame(&self, width: u32, height: u32) -> Option<OffscreenFrame> {
        let width = width.clamp(1, MAX_FRAME_SIZE.0);
        let height = height.clamp(1, MAX_FRAME_SIZE.1);
        self.engine
            .render_frame(width, height, Duration::from_secs(5))
    }

    /// A preview around an engine handle without a window, for tests of `AppState`. Its window
    /// methods must not be called.
    #[cfg(test)]
    pub fn detached(engine: PreviewHandle) -> Self {
        Self {
            engine,
            visibility: Mutex::new(Visibility::INITIAL),
            #[cfg(windows)]
            child: windows::ChildWindow::detached(),
        }
    }

    fn visibility(&self) -> MutexGuard<'_, Visibility> {
        // `Visibility` is two booleans and always consistent, so a poisoned lock is harmless.
        self.visibility
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The two inputs that decide whether the preview is on screen and rendering (ADR-0005 §3): the
/// UI's request (`set_preview_visible`) and a non-empty rectangle (`set_preview_rect`). Both must
/// allow it; a rectangle update never overrides a hide request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Visibility {
    requested: bool,
    has_area: bool,
}

impl Visibility {
    /// Not requested until the UI asks: the preview area requests visible when it mounts and
    /// hidden when it unmounts. Starting as requested would let the native window cover DOM
    /// content drawn before the preview area exists (ADR-0005 §4), e.g. a start screen.
    const INITIAL: Self = Self {
        requested: false,
        has_area: false,
    };

    fn shown(self) -> bool {
        self.requested && self.has_area
    }

    /// Records the UI's request. Returns the new shown state if it changed.
    fn request(&mut self, visible: bool) -> Option<bool> {
        self.update(Self {
            requested: visible,
            ..*self
        })
    }

    /// Records the reported rectangle. Returns the new shown state if it changed.
    fn set_rect(&mut self, rect: PreviewRect) -> Option<bool> {
        self.update(Self {
            has_area: rect.width > 0 && rect.height > 0,
            ..*self
        })
    }

    fn update(&mut self, next: Self) -> Option<bool> {
        let before = self.shown();
        *self = next;
        (next.shown() != before).then_some(next.shown())
    }
}

fn preview_mode(available: bool) -> PreviewMode {
    if !available {
        PreviewMode::Unavailable
    } else if cfg!(windows) {
        PreviewMode::Native
    } else {
        PreviewMode::Frames
    }
}

/// The engine's GPU-changed callback: emits `AppInfoChanged` from the preview thread.
fn report_gpu_changes(app: AppHandle) -> GpuChanged {
    Box::new(move |info, available| {
        let event = AppInfoChanged(AppInfo {
            version: app.package_info().version.to_string(),
            gpu: Some((&info).into()),
            preview_mode: preview_mode(available),
        });
        if let Err(err) = event.emit(&app) {
            tracing::error!(%err, "cannot report the GPU change to the UI");
        }
    })
}

/// The engine's tick callback (render thread): pauses background preview jobs while the
/// transport plays (media-import.md §6.1) and emits `playback-tick`, which never blocks.
fn report_ticks(app: AppHandle) -> PlaybackTicks {
    Box::new(move |status| {
        if let Some(state) = app.try_state::<crate::state::AppState>() {
            state
                .jobs
                .pause_background(status.state != PlayState::Paused);
        }
        if let Err(err) = PlaybackTick::from(status).emit(&app) {
            tracing::warn!(%err, "cannot emit playback-tick");
        }
    })
}

/// The engine's missing-media callback (called from reader threads): marks the media offline
/// (media-import.md §6.6, render-graph.md §10), like a preview generator that finds the file gone.
fn report_missing_media(app: AppHandle) -> MediaUnavailable {
    Arc::new(move |media| {
        let Some(state) = app.try_state::<crate::state::AppState>() else {
            return;
        };
        let snapshot = state.snapshot();
        match snapshot.media_asset(media) {
            Some(asset) => state.report_missing(&app, asset),
            None => tracing::debug!(%media, "missing media is no longer in the project"),
        }
    })
}

/// Logs a failed engine start and syncs a started engine with `Visibility::INITIAL`.
fn start(engine: Result<PreviewHandle, GpuError>) -> Option<PreviewHandle> {
    match engine {
        Ok(engine) => {
            engine.set_visible(Visibility::INITIAL.shown());
            Some(engine)
        }
        Err(err) => {
            tracing::error!(%err, "preview unavailable");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(width: u32, height: u32) -> PreviewRect {
        PreviewRect {
            x: 10,
            y: 20,
            width,
            height,
        }
    }

    #[test]
    fn starts_hidden_until_requested_and_placed() {
        let mut visibility = Visibility::INITIAL;
        assert!(!visibility.shown());
        assert_eq!(visibility.set_rect(rect(640, 360)), None);
        assert!(!visibility.shown());
        assert_eq!(visibility.request(true), Some(true));
        assert!(visibility.shown());
    }

    #[test]
    fn shown_only_when_requested_and_rect_is_non_empty() {
        for requested in [false, true] {
            for (width, height) in [(0, 0), (0, 360), (640, 0), (640, 360)] {
                let mut visibility = Visibility::INITIAL;
                visibility.request(requested);
                visibility.set_rect(rect(width, height));
                assert_eq!(
                    visibility.shown(),
                    requested && width > 0 && height > 0,
                    "requested={requested} rect={width}x{height}"
                );
            }
        }
    }

    #[test]
    fn rect_updates_never_show_a_hidden_preview() {
        let mut visibility = Visibility::INITIAL;
        visibility.request(true);
        assert_eq!(visibility.set_rect(rect(640, 360)), Some(true));
        assert_eq!(visibility.request(false), Some(false));

        // e.g. the window is resized while a dialog has the preview hidden
        for (width, height) in [(800, 450), (0, 0), (1280, 720)] {
            assert_eq!(visibility.set_rect(rect(width, height)), None);
            assert!(!visibility.shown());
        }

        assert_eq!(visibility.request(true), Some(true));
    }

    #[test]
    fn empty_rect_hides_and_a_new_area_shows_again() {
        let mut visibility = Visibility::INITIAL;
        visibility.request(true);
        visibility.set_rect(rect(640, 360));
        assert_eq!(visibility.set_rect(rect(0, 0)), Some(false));
        assert_eq!(visibility.set_rect(rect(320, 180)), Some(true));
    }

    #[test]
    fn repeated_inputs_report_no_change() {
        let mut visibility = Visibility::INITIAL;
        assert_eq!(visibility.request(false), None);
        visibility.request(true);
        assert_eq!(visibility.request(true), None);
        visibility.set_rect(rect(640, 360));
        assert_eq!(visibility.set_rect(rect(700, 400)), None);
    }
}
