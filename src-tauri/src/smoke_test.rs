//! `--smoke-test`: starts the app, renders through the real preview path and exits with a status
//! code. CI runs it against the installed build on WARP (no GPU on the runners).
//!
//! 1. One offscreen frame (all platforms).
//! 2. Windows only: shows the native preview child window through the same calls the UI's
//!    `set_preview_rect` / `set_preview_visible` make, and waits until the engine has presented
//!    [`FRAMES_TO_PRESENT`] frames into it (ADR-0005 §7). The preview renders on change, so the
//!    test asks for a redraw every [`REDRAW_INTERVAL`] until then. Afterwards it logs how many
//!    frames the idle preview presented in [`IDLE_CHECK`] (expected: 0; logged, not checked).
//!    Linux uses the `frames` mode, which step 1 already covers.
//!
//! Exit codes (see [`Outcome`]):
//!
//! | code | meaning |
//! |---|---|
//! | 0 | passed |
//! | 1 | no offscreen frame rendered |
//! | 2 | no GPU adapter: the preview is unavailable |
//! | 3 | Windows: the preview window has no surface (no adapter can present to it) |
//! | 4 | Windows: fewer than [`FRAMES_TO_PRESENT`] frames presented within [`PRESENT_TIMEOUT`] |
//! | 5 | the smoke test did not finish within [`OVERALL_TIMEOUT`] (hang) |
//! | 6 | release build refused to start: the loaded FFmpeg is not the LGPL build (any run, not only the smoke test; see `ffmpeg_check.rs`) |

use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::preview::Preview;
use crate::state::AppState;

/// Command-line flag that runs the smoke test instead of a normal session.
pub const FLAG: &str = "--smoke-test";

/// Frames the engine must present into the native preview window.
#[cfg_attr(not(windows), allow(dead_code))]
const FRAMES_TO_PRESENT: u64 = 30;
/// Time allowed for presenting [`FRAMES_TO_PRESENT`] frames once the window is shown.
#[cfg_attr(not(windows), allow(dead_code))]
const PRESENT_TIMEOUT: Duration = Duration::from_secs(20);
/// How often the smoke test asks the idle preview for a redraw while counting presents.
#[cfg_attr(not(windows), allow(dead_code))]
const REDRAW_INTERVAL: Duration = Duration::from_millis(10);
/// How long the smoke test watches the idle preview after the presents.
#[cfg_attr(not(windows), allow(dead_code))]
const IDLE_CHECK: Duration = Duration::from_millis(500);
/// Hard limit for the whole run, from process start (GPU and webview start-up included).
const OVERALL_TIMEOUT: Duration = Duration::from_secs(90);

/// Process exit code of the smoke test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum Outcome {
    Passed = 0,
    NoOffscreenFrame = 1,
    NoGpu = 2,
    #[cfg_attr(not(windows), allow(dead_code))]
    NoPreviewSurface = 3,
    #[cfg_attr(not(windows), allow(dead_code))]
    TooFewFramesPresented = 4,
    Timeout = 5,
}

/// Kills the process with [`Outcome::Timeout`] if the smoke test hangs anywhere, including
/// before the app's setup finishes. Call once at process start.
pub fn start_watchdog() {
    thread::Builder::new()
        .name("smoke-test-watchdog".into())
        .spawn(|| {
            thread::sleep(OVERALL_TIMEOUT);
            tracing::error!(timeout = ?OVERALL_TIMEOUT, "smoke test timed out");
            std::process::exit(Outcome::Timeout as i32);
        })
        .expect("failed to spawn the smoke-test watchdog");
}

/// Runs the checks on a background thread and exits the app with the [`Outcome`] code. Call
/// after `AppState` is managed.
pub fn run(app: AppHandle) {
    thread::spawn(move || {
        let outcome = check(&app);
        // Give the log a moment to flush before exiting.
        thread::sleep(Duration::from_millis(200));
        app.exit(outcome as i32);
    });
}

fn check(app: &AppHandle) -> Outcome {
    let state = app.state::<AppState>();
    let Some(preview) = &state.preview else {
        tracing::error!("smoke test failed: no GPU");
        return Outcome::NoGpu;
    };
    let gpu = preview.gpu_info();

    match preview.render_frame(320, 180) {
        Some(frame) if frame.rgba.len() == 320 * 180 * 4 => {}
        _ => {
            tracing::error!(adapter = %gpu.adapter, "smoke test failed: no offscreen frame rendered");
            return Outcome::NoOffscreenFrame;
        }
    }

    let (presented, idle_presents) = match present(app, preview) {
        Ok(counts) => counts,
        Err(outcome) => return outcome,
    };
    tracing::info!(
        adapter = %gpu.adapter,
        backend = %gpu.backend,
        software = gpu.software,
        mode = ?preview.mode(),
        presented,
        idle_presents,
        "smoke test passed"
    );
    Outcome::Passed
}

/// Shows the native preview window, requests redraws until enough frames are presented, then
/// watches the idle preview. Returns the presented frames and the presents while idle.
#[cfg(windows)]
fn present(app: &AppHandle, preview: &Preview) -> Result<(u64, u64), Outcome> {
    use std::time::Instant;

    use crate::ipc::types::PreviewRect;

    let gpu = preview.gpu_info();
    if !preview.has_native_window() {
        tracing::error!(
            adapter = %gpu.adapter,
            "smoke test failed: no adapter can present to the preview window"
        );
        return Err(Outcome::NoPreviewSurface);
    }

    let before = preview.presented_frames();
    preview.set_rect(
        app,
        PreviewRect {
            x: 0,
            y: 0,
            width: 320,
            height: 180,
        },
    );
    preview.set_visible(app, true);

    let deadline = Instant::now() + PRESENT_TIMEOUT;
    loop {
        let presented = preview.presented_frames() - before;
        if presented >= FRAMES_TO_PRESENT {
            return Ok((presented, idle_presents(preview)));
        }
        if Instant::now() >= deadline {
            tracing::error!(
                adapter = %gpu.adapter,
                presented,
                expected = FRAMES_TO_PRESENT,
                timeout = ?PRESENT_TIMEOUT,
                "smoke test failed: too few frames presented to the preview window"
            );
            return Err(Outcome::TooFewFramesPresented);
        }
        // Render on change: nothing else changes, so ask for the next frame.
        preview.request_redraw();
        thread::sleep(REDRAW_INTERVAL);
    }
}

/// Presents of the preview within [`IDLE_CHECK`] after the last redraw request settled. Render on
/// change (render-graph.md §7) makes it 0 unless something else (e.g. the UI) changed an input.
#[cfg(windows)]
fn idle_presents(preview: &Preview) -> u64 {
    // Let the redraws already queued finish.
    thread::sleep(Duration::from_millis(100));
    let before = preview.presented_frames();
    thread::sleep(IDLE_CHECK);
    let idle = preview.presented_frames() - before;
    if idle > 0 {
        tracing::warn!(idle, "the idle preview kept presenting");
    }
    idle
}

/// Linux and other development platforms use the `frames` mode: nothing is presented natively.
#[cfg(not(windows))]
fn present(_app: &AppHandle, _preview: &Preview) -> Result<(u64, u64), Outcome> {
    Ok((0, 0))
}
