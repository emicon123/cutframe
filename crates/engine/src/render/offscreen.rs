use super::{GpuContext, OFFSCREEN_FORMAT, PatternParams, TestPattern};

/// A rendered frame read back to the CPU: tightly packed RGBA8 rows, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffscreenFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Renders one frame into a texture and reads it back.
///
/// Used by the development preview on Linux, golden-frame tests and the smoke test. Export will
/// use the same render path with an encoder instead of a CPU read-back.
pub fn render_offscreen(
    gpu: &GpuContext,
    pattern: &TestPattern,
    width: u32,
    height: u32,
    time_seconds: f32,
    canvas_aspect: f32,
) -> OffscreenFrame {
    let width = width.max(1);
    let height = height.max(1);
    let device = &gpu.device;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen.target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OFFSCREEN_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Rows in a texture-to-buffer copy must be aligned to 256 bytes.
    let unpadded_row = width * 4;
    let padded_row = unpadded_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("offscreen.readback"),
        size: u64::from(padded_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("offscreen"),
    });
    pattern.encode(
        gpu,
        &mut encoder,
        &view,
        PatternParams {
            time_seconds,
            viewport_aspect: width as f32 / height as f32,
            canvas_aspect,
        },
    );
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let submission = gpu.queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| {
        if let Err(err) = result {
            tracing::error!(%err, "offscreen read-back failed");
        }
    });
    if let Err(err) = device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    }) {
        tracing::error!(%err, "GPU poll failed during read-back");
    }

    let mapped = slice
        .get_mapped_range()
        .expect("read-back buffer is mapped after a successful poll");
    let mut rgba = Vec::with_capacity((unpadded_row * height) as usize);
    for row in mapped.chunks_exact(padded_row as usize) {
        rgba.extend_from_slice(&row[..unpadded_row as usize]);
    }
    drop(mapped);
    buffer.unmap();

    OffscreenFrame {
        width,
        height,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu() -> Option<GpuContext> {
        match GpuContext::new() {
            Ok(gpu) => Some(gpu),
            Err(err) => {
                // CI images always ship a software adapter; locally this only skips.
                eprintln!("skipping GPU test: {err}");
                None
            }
        }
    }

    #[test]
    fn renders_letterboxed_test_pattern() {
        let Some(gpu) = gpu() else { return };
        let pattern = TestPattern::new(&gpu, OFFSCREEN_FORMAT);
        // A square viewport with a 16:9 canvas: top and bottom rows are letterbox bars.
        let frame = render_offscreen(&gpu, &pattern, 64, 64, 0.0, 16.0 / 9.0);
        assert_eq!(frame.rgba.len(), 64 * 64 * 4);

        let pixel = |x: usize, y: usize| &frame.rgba[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        let bar = pixel(10, 0);
        // Linear 0.035 is ~53 after sRGB encoding.
        assert!(
            bar.iter().take(3).all(|&c| c < 64),
            "letterbox should be near-black, got {bar:?}"
        );
        let content = pixel(10, 32);
        assert!(
            content.iter().take(3).any(|&c| c > 100),
            "canvas should be coloured, got {content:?}"
        );
        assert_eq!(content[3], 255);
    }

    #[test]
    fn handles_widths_that_need_row_padding() {
        let Some(gpu) = gpu() else { return };
        let pattern = TestPattern::new(&gpu, OFFSCREEN_FORMAT);
        let frame = render_offscreen(&gpu, &pattern, 33, 7, 1.5, 16.0 / 9.0);
        assert_eq!(
            (frame.width, frame.height, frame.rgba.len()),
            (33, 7, 33 * 7 * 4)
        );
    }
}
