//! The preview thread. It owns the GPU and renders the preview either into a native window
//! (Windows: a child window over the UI's preview area) or into offscreen frames that the UI pulls
//! (Linux development mode, see ADR-0001).
//!
//! All communication goes through a channel, so callers never block on rendering.

use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::render::{
    GpuContext, GpuError, GpuInfo, OFFSCREEN_FORMAT, OffscreenFrame, PatternParams, TestPattern,
    render_offscreen,
};

/// Raw handles of a native window to render into.
struct NativeWindow {
    window: RawWindowHandle,
    display: Option<RawDisplayHandle>,
}

// SAFETY: raw handles are plain identifiers. The caller of `PreviewHandle::attach_window`
// guarantees the window outlives the attachment; the render thread only uses them to create a
// surface, which every wgpu backend allows from any thread.
unsafe impl Send for NativeWindow {}

enum Message {
    Attach {
        window: NativeWindow,
        width: u32,
        height: u32,
    },
    Resize {
        width: u32,
        height: u32,
    },
    SetVisible(bool),
    SetCanvasAspect(f32),
    RenderFrame {
        width: u32,
        height: u32,
        reply: Sender<OffscreenFrame>,
    },
    Shutdown,
}

/// Handle to the preview thread. Dropping it stops the thread.
#[derive(Debug)]
pub struct PreviewHandle {
    tx: Sender<Message>,
    gpu_info: GpuInfo,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Message::Attach { .. } => "Attach",
            Message::Resize { .. } => "Resize",
            Message::SetVisible(_) => "SetVisible",
            Message::SetCanvasAspect(_) => "SetCanvasAspect",
            Message::RenderFrame { .. } => "RenderFrame",
            Message::Shutdown => "Shutdown",
        })
    }
}

impl PreviewHandle {
    /// Starts the preview thread and initializes the GPU on it.
    pub fn spawn(canvas_aspect: f32) -> Result<Self, GpuError> {
        let (tx, rx) = unbounded();
        let (ready_tx, ready_rx) = bounded(1);
        let thread = thread::Builder::new()
            .name("preview-render".into())
            .spawn(move || match GpuContext::new() {
                Ok(gpu) => {
                    let _ = ready_tx.send(Ok(gpu.info()));
                    RenderLoop::new(gpu, canvas_aspect).run(&rx);
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                }
            })
            .expect("failed to spawn the preview thread");
        let gpu_info = ready_rx
            .recv()
            .expect("preview thread exited during GPU initialization")?;
        Ok(Self {
            tx,
            gpu_info,
            thread: Some(thread),
        })
    }

    pub fn gpu_info(&self) -> &GpuInfo {
        &self.gpu_info
    }

    /// Starts rendering continuously into a native window of `width` x `height` physical pixels.
    ///
    /// # Safety
    /// `window` (and `display`, if given) must be valid handles, and the window must stay alive
    /// until this `PreviewHandle` is dropped.
    pub unsafe fn attach_window(
        &self,
        window: RawWindowHandle,
        display: Option<RawDisplayHandle>,
        width: u32,
        height: u32,
    ) {
        self.send(Message::Attach {
            window: NativeWindow { window, display },
            width,
            height,
        });
    }

    /// New size of the attached window, in physical pixels.
    pub fn resize(&self, width: u32, height: u32) {
        self.send(Message::Resize { width, height });
    }

    /// Pauses (hidden) or resumes (visible) rendering into the attached window.
    pub fn set_visible(&self, visible: bool) {
        self.send(Message::SetVisible(visible));
    }

    /// Width / height of the project canvas.
    pub fn set_canvas_aspect(&self, aspect: f32) {
        self.send(Message::SetCanvasAspect(aspect));
    }

    /// Renders one offscreen frame. Returns `None` if the thread does not answer within `timeout`.
    pub fn render_frame(
        &self,
        width: u32,
        height: u32,
        timeout: Duration,
    ) -> Option<OffscreenFrame> {
        let (reply, response) = bounded(1);
        self.send(Message::RenderFrame {
            width,
            height,
            reply,
        });
        response.recv_timeout(timeout).ok()
    }

    fn send(&self, message: Message) {
        if self.tx.send(message).is_err() {
            tracing::error!("preview thread is not running");
        }
    }
}

impl Drop for PreviewHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct WindowTarget {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pattern: TestPattern,
}

struct RenderLoop {
    gpu: GpuContext,
    started: Instant,
    canvas_aspect: f32,
    offscreen_pattern: Option<TestPattern>,
    window: Option<WindowTarget>,
    visible: bool,
}

impl RenderLoop {
    fn new(gpu: GpuContext, canvas_aspect: f32) -> Self {
        Self {
            gpu,
            started: Instant::now(),
            canvas_aspect,
            offscreen_pattern: None,
            window: None,
            visible: true,
        }
    }

    fn run(mut self, rx: &Receiver<Message>) {
        loop {
            // Block while there is nothing to draw; otherwise drain messages and draw a frame.
            if !self.is_drawing() {
                match rx.recv() {
                    Ok(message) => {
                        if !self.handle(message) {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
            for message in rx.try_iter() {
                if !self.handle(message) {
                    return;
                }
            }
            if self.is_drawing() {
                self.draw_window();
            }
        }
    }

    fn is_drawing(&self) -> bool {
        self.visible
            && self
                .window
                .as_ref()
                .is_some_and(|w| w.config.width > 0 && w.config.height > 0)
    }

    /// Returns `false` on shutdown.
    fn handle(&mut self, message: Message) -> bool {
        match message {
            Message::Attach {
                window,
                width,
                height,
            } => self.attach(window, width, height),
            Message::Resize { width, height } => {
                if let Some(target) = &mut self.window {
                    target.config.width = width.max(1);
                    target.config.height = height.max(1);
                    target.surface.configure(&self.gpu.device, &target.config);
                }
            }
            Message::SetVisible(visible) => self.visible = visible,
            Message::SetCanvasAspect(aspect) => self.canvas_aspect = aspect,
            Message::RenderFrame {
                width,
                height,
                reply,
            } => {
                let time = self.elapsed();
                let pattern = self
                    .offscreen_pattern
                    .get_or_insert_with(|| TestPattern::new(&self.gpu, OFFSCREEN_FORMAT));
                let frame =
                    render_offscreen(&self.gpu, pattern, width, height, time, self.canvas_aspect);
                let _ = reply.send(frame);
            }
            Message::Shutdown => return false,
        }
        true
    }

    fn attach(&mut self, window: NativeWindow, width: u32, height: u32) {
        // SAFETY: the caller of `attach_window` guarantees the handles are valid and outlive us.
        let surface = unsafe {
            self.gpu
                .instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: window.display,
                    raw_window_handle: window.window,
                })
        };
        let surface = match surface {
            Ok(surface) => surface,
            Err(err) => {
                tracing::error!(%err, "cannot create preview surface");
                return;
            }
        };
        let Some(mut config) =
            surface.get_default_config(&self.gpu.adapter, width.max(1), height.max(1))
        else {
            tracing::error!("GPU adapter cannot present to the preview window");
            return;
        };
        let capabilities = surface.get_capabilities(&self.gpu.adapter);
        if let Some(srgb) = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
        {
            config.format = srgb;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.desired_maximum_frame_latency = 2;
        surface.configure(&self.gpu.device, &config);
        tracing::info!(width, height, format = ?config.format, "preview window attached");

        let pattern = TestPattern::new(&self.gpu, config.format);
        self.window = Some(WindowTarget {
            surface,
            config,
            pattern,
        });
    }

    fn draw_window(&mut self) {
        let time = self.elapsed();
        let canvas_aspect = self.canvas_aspect;
        let Some(target) = &mut self.window else {
            return;
        };

        let (frame, suboptimal) = match target.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                thread::sleep(Duration::from_millis(16));
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                target.surface.configure(&self.gpu.device, &target.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                tracing::error!("validation error while acquiring the preview frame");
                thread::sleep(Duration::from_millis(100));
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("preview"),
            });
        target.pattern.encode(
            &self.gpu,
            &mut encoder,
            &view,
            PatternParams {
                time_seconds: time,
                viewport_aspect: target.config.width as f32 / target.config.height as f32,
                canvas_aspect,
            },
        );
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu.queue.present(frame);

        if suboptimal {
            target.surface.configure(&self.gpu.device, &target.config);
        }
    }

    fn elapsed(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_frames_on_request() {
        let preview = match PreviewHandle::spawn(16.0 / 9.0) {
            Ok(preview) => preview,
            Err(err) => {
                eprintln!("skipping GPU test: {err}");
                return;
            }
        };
        let frame = preview
            .render_frame(320, 180, Duration::from_secs(30))
            .expect("frame");
        assert_eq!((frame.width, frame.height), (320, 180));
        assert_eq!(frame.rgba.len(), 320 * 180 * 4);
    }
}
