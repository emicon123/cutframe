//! Preview quality, zoom and pan (render-graph.md §8, task 0340): how large the preview
//! composites ([`preview_render_size`]) and where the present pass puts the canvas in the window
//! ([`present_rect`]). Pure, so both are unit-tested without a GPU. Export never uses them.

use super::plan::render_size;
use super::present::letterbox;

/// The preview's resolution scale: the render size is divided by 1, 2 or 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewQuality {
    #[default]
    Full,
    Half,
    Quarter,
}

impl PreviewQuality {
    /// The divisor of the render size.
    pub fn divisor(self) -> u32 {
        match self {
            Self::Full => 1,
            Self::Half => 2,
            Self::Quarter => 4,
        }
    }
}

/// Smallest zoom percentage (`PreviewZoom::Percent`).
pub const MIN_ZOOM_PERCENT: u16 = 25;
/// Largest zoom percentage.
pub const MAX_ZOOM_PERCENT: u16 = 400;
/// How far the pan may move the view centre from the canvas centre, in canvas sizes: at most
/// to the canvas edge.
pub const MAX_PAN: f32 = 0.5;

/// The preview's zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewZoom {
    /// The whole canvas fitted into the window, letterboxed. Pan is ignored.
    #[default]
    Fit,
    /// Canvas pixels at this percentage of window (physical) pixels: 100 shows one canvas pixel
    /// per screen pixel. [`MIN_ZOOM_PERCENT`]..=[`MAX_ZOOM_PERCENT`].
    Percent(u16),
}

/// Zoom and pan of the preview.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PreviewView {
    pub zoom: PreviewZoom,
    /// The canvas point at the window centre, relative to the canvas centre, in canvas widths
    /// and heights: (0, 0) centred, (0.5, 0) the right edge in the middle of the window.
    /// -[`MAX_PAN`]..=[`MAX_PAN`] per axis; used with `Percent` only.
    pub pan: (f32, f32),
}

impl PreviewView {
    /// The whole canvas, letterboxed (the default).
    pub const FIT: Self = Self {
        zoom: PreviewZoom::Fit,
        pan: (0.0, 0.0),
    };

    /// The view with its percentage clamped to the zoom range and its pan to
    /// -[`MAX_PAN`]..=[`MAX_PAN`] (non-finite pan values become 0). `Fit` has no pan.
    pub fn clamped(self) -> Self {
        let pan = |v: f32| {
            if v.is_finite() {
                v.clamp(-MAX_PAN, MAX_PAN)
            } else {
                0.0
            }
        };
        match self.zoom {
            PreviewZoom::Fit => Self::FIT,
            PreviewZoom::Percent(percent) => Self {
                zoom: PreviewZoom::Percent(percent.clamp(MIN_ZOOM_PERCENT, MAX_ZOOM_PERCENT)),
                pan: (pan(self.pan.0), pan(self.pan.1)),
            },
        }
    }
}

/// The canvas size on screen at `percent` %, in window pixels (at least 1 x 1).
fn zoomed(canvas: (u32, u32), percent: u16) -> (u64, u64) {
    let scale = |side: u32| (u64::from(side.max(1)) * u64::from(percent) / 100).max(1);
    (scale(canvas.0), scale(canvas.1))
}

/// The preview's render size (render-graph.md §8): `min(canvas, fit x zoom) x quality`, rounded
/// to even. `Fit` fits the canvas into `window` (the preview rectangle, physical pixels); a
/// percentage scales the canvas itself (the part outside the window is composited too, so pans
/// need no new composite).
pub fn preview_render_size(
    canvas: (u32, u32),
    window: (u32, u32),
    quality: PreviewQuality,
    view: PreviewView,
) -> (u32, u32) {
    let bounds = match view.clamped().zoom {
        PreviewZoom::Fit => window,
        PreviewZoom::Percent(percent) => {
            let (w, h) = zoomed(canvas, percent);
            (
                u32::try_from(w).unwrap_or(u32::MAX),
                u32::try_from(h).unwrap_or(u32::MAX),
            )
        }
    };
    render_size(canvas, bounds, quality.divisor())
}

/// Where the present pass draws the canvas in a `target`-sized window, as (x, y, width, height)
/// in window pixels: letterboxed for `Fit`; for a percentage, `canvas` (project pixels) scaled
/// by it with the panned point at the window centre, on whole pixels. Window pixels outside the
/// rectangle are black; the rectangle may extend past the window.
pub fn present_rect(canvas: (u32, u32), target: (u32, u32), view: PreviewView) -> [f32; 4] {
    let view = view.clamped();
    let PreviewZoom::Percent(percent) = view.zoom else {
        return letterbox(canvas, target);
    };
    let (w, h) = zoomed(canvas, percent);
    let (w, h) = (w as f64, h as f64);
    let (tw, th) = (f64::from(target.0.max(1)), f64::from(target.1.max(1)));
    let x = (tw / 2.0 - (0.5 + f64::from(view.pan.0)) * w).round();
    let y = (th / 2.0 - (0.5 + f64::from(view.pan.1)) * h).round();
    [x as f32, y as f32, w as f32, h as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_HD: (u32, u32) = (1920, 1080);

    fn percent(percent: u16, pan: (f32, f32)) -> PreviewView {
        PreviewView {
            zoom: PreviewZoom::Percent(percent),
            pan,
        }
    }

    #[test]
    fn views_are_clamped() {
        assert_eq!(percent(10, (0.0, 0.0)).clamped(), percent(25, (0.0, 0.0)));
        assert_eq!(percent(0, (0.0, 0.0)).clamped(), percent(25, (0.0, 0.0)));
        assert_eq!(
            percent(1000, (0.7, -2.0)).clamped(),
            percent(400, (0.5, -0.5))
        );
        assert_eq!(
            percent(150, (f32::NAN, f32::INFINITY)).clamped(),
            percent(150, (0.0, 0.0))
        );
        let fit = PreviewView {
            zoom: PreviewZoom::Fit,
            pan: (0.3, 0.3),
        };
        assert_eq!(fit.clamped(), PreviewView::FIT, "fit has no pan");
    }

    #[test]
    fn render_size_is_min_of_canvas_and_fit_times_zoom_times_quality() {
        let window = (1280, 720);
        let size = |quality, view| preview_render_size(FULL_HD, window, quality, view);
        let fit = PreviewView::FIT;
        assert_eq!(size(PreviewQuality::Full, fit), (1280, 720));
        assert_eq!(size(PreviewQuality::Half, fit), (640, 360));
        assert_eq!(size(PreviewQuality::Quarter, fit), (320, 180));
        // 50 %: half the canvas, whatever the window.
        let half = percent(50, (0.0, 0.0));
        assert_eq!(size(PreviewQuality::Full, half), (960, 540));
        assert_eq!(size(PreviewQuality::Quarter, half), (240, 134));
        // 200 %: never above the canvas.
        let double = percent(200, (0.25, 0.0));
        assert_eq!(size(PreviewQuality::Full, double), FULL_HD);
        assert_eq!(size(PreviewQuality::Half, double), (960, 540));
        assert_eq!(size(PreviewQuality::Quarter, double), (480, 270));
        // Out-of-range zoom is clamped first: 10 % renders like 25 %.
        assert_eq!(
            size(PreviewQuality::Full, percent(10, (0.0, 0.0))),
            (480, 270)
        );
        // A 4K canvas in a full-screen 1080p window stays at the window size at Fit.
        assert_eq!(
            preview_render_size((3840, 2160), FULL_HD, PreviewQuality::Full, fit),
            FULL_HD
        );
    }

    #[test]
    fn present_rect_maps_the_visible_canvas_region() {
        let target = (1280, 720);
        // Fit: the letterbox.
        assert_eq!(
            present_rect(FULL_HD, target, PreviewView::FIT),
            [0.0, 0.0, 1280.0, 720.0]
        );
        // 100 %, centred: the middle 1280 x 720 canvas pixels.
        assert_eq!(
            present_rect(FULL_HD, target, percent(100, (0.0, 0.0))),
            [-320.0, -180.0, 1920.0, 1080.0]
        );
        // 200 % panned to the right edge: the canvas' right edge at the window centre, black
        // to its right.
        assert_eq!(
            present_rect(FULL_HD, target, percent(200, (0.5, 0.0))),
            [-3200.0, -720.0, 3840.0, 2160.0]
        );
        // 25 %: smaller than the window, centred (black around it).
        assert_eq!(
            present_rect(FULL_HD, target, percent(25, (0.0, 0.0))),
            [400.0, 225.0, 480.0, 270.0]
        );
        // The pan is clamped: 0.9 acts like 0.5.
        assert_eq!(
            present_rect(FULL_HD, target, percent(200, (0.9, -0.9))),
            present_rect(FULL_HD, target, percent(200, (0.5, -0.5)))
        );
    }
}

/// The present pass with zoom, pan and quality on WARP (render-graph.md §12): a canvas of four
/// coloured quadrants, composited at the preview's render size and presented into a 64 x 64
/// window. Asserts exact colours away from the quadrant edges (bilinear filtering blurs only
/// those) and black outside the canvas.
#[cfg(test)]
mod warp_tests {
    use app_core::props::CropInsets;
    use app_core::{ClipId, Rgba8, Time};

    use super::*;
    use crate::media::readers::{ReaderOptions, ReaderPool};
    use crate::render::present::PresentPass;
    use crate::render::{
        AdapterMode, Affine2, BackgroundPlan, Compositor, GpuContext, LayerContent, LayerPlan,
        OUTPUT_FORMAT, Readback, RenderPlan, create_canvas,
    };

    const CANVAS: (u32, u32) = (64, 64);
    const WINDOW: (u32, u32) = (64, 64);
    const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 255, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    const WHITE: [u8; 3] = [255, 255, 255];
    const BLACK: [u8; 3] = [0, 0, 0];

    fn quadrant(id: u128, x: f32, y: f32, [r, g, b]: [u8; 3]) -> LayerPlan {
        LayerPlan {
            clip: ClipId(uuid::Uuid::from_u128(id)),
            content: LayerContent::Color(Rgba8 { r, g, b, a: 255 }),
            transform: Affine2::rect(x, y, 32.0, 32.0),
            crop: CropInsets::default(),
            opacity: 1.0,
        }
    }

    /// The window's RGBA pixels for `quality` and `view`.
    fn present(gpu: &GpuContext, quality: PreviewQuality, view: PreviewView) -> Vec<u8> {
        let size = preview_render_size(CANVAS, WINDOW, quality, view);
        let plan = RenderPlan {
            frame: 0,
            time: Time::ZERO,
            canvas: CANVAS,
            render_size: size,
            background: BackgroundPlan::Color(Rgba8::BLACK),
            layers: vec![
                quadrant(1, 0.0, 0.0, RED),
                quadrant(2, 32.0, 0.0, GREEN),
                quadrant(3, 0.0, 32.0, BLUE),
                quadrant(4, 32.0, 32.0, WHITE),
            ],
        };
        let (_texture, canvas) = create_canvas(&gpu.device, size);
        let mut compositor = Compositor::new(gpu);
        let mut frames = ReaderPool::new(None, ReaderOptions::default(), |_| {});
        let mut output = PresentPass::new(&gpu.device, OUTPUT_FORMAT);
        let mut readback = Readback::new(&gpu.device, WINDOW);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        assert!(
            compositor
                .render(gpu, &mut encoder, &plan, &mut frames, &canvas)
                .drawn
        );
        output.encode_rect(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &canvas,
            &readback.view,
            present_rect(CANVAS, WINDOW, view),
        );
        readback.encode_copy(&mut encoder);
        let submission = gpu.queue.submit([encoder.finish()]);
        compositor.submitted(gpu);
        readback.read(gpu, submission).expect("read-back").to_vec()
    }

    fn assert_pixels(rgba: &[u8], expected: &[((u32, u32), [u8; 3])], case: &str) {
        for &((x, y), colour) in expected {
            let at = ((y * WINDOW.0 + x) * 4) as usize;
            let actual = &rgba[at..at + 3];
            let close = actual.iter().zip(colour).all(|(a, e)| a.abs_diff(e) <= 1);
            assert!(
                close,
                "{case}: ({x}, {y}) is {actual:?}, expected {colour:?}"
            );
        }
    }

    #[test]
    fn zoom_pan_and_quality_map_the_visible_canvas_region() {
        let gpu = GpuContext::new_with(AdapterMode::SoftwareOnly)
            .expect("WARP (the D3D12 software adapter) ships with Windows");
        let zoom = |percent, pan| PreviewView {
            zoom: PreviewZoom::Percent(percent),
            pan,
        };

        let fit = present(&gpu, PreviewQuality::Full, PreviewView::FIT);
        assert_pixels(
            &fit,
            &[
                ((16, 16), RED),
                ((48, 16), GREEN),
                ((16, 48), BLUE),
                ((48, 48), WHITE),
            ],
            "fit",
        );

        // 200 % panned to the right edge: the left half shows the canvas' right quarter
        // (green over white), the right half is past the canvas (black).
        let edge = present(&gpu, PreviewQuality::Full, zoom(200, (0.5, 0.0)));
        assert_pixels(
            &edge,
            &[
                ((16, 16), GREEN),
                ((16, 48), WHITE),
                ((48, 16), BLACK),
                ((48, 48), BLACK),
            ],
            "200 % at the right edge",
        );

        // 200 % at quarter quality (a 16 x 16 composite), centred on the white quadrant.
        let quarter = present(&gpu, PreviewQuality::Quarter, zoom(200, (0.25, 0.25)));
        for (x, y) in [(8, 8), (32, 32), (56, 56), (8, 56)] {
            assert_pixels(&quarter, &[((x, y), WHITE)], "200 % quarter quality");
        }

        // 25 % at half quality: a 16 x 16 picture in the middle, black around it.
        let small = present(&gpu, PreviewQuality::Half, zoom(25, (0.0, 0.0)));
        assert_pixels(
            &small,
            &[
                ((4, 4), BLACK),
                ((60, 60), BLACK),
                ((26, 26), RED),
                ((37, 26), GREEN),
                ((26, 37), BLUE),
                ((37, 37), WHITE),
            ],
            "25 % half quality",
        );
    }
}
