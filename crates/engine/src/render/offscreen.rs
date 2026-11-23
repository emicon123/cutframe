//! 8-bit output targets read back to the CPU: the offline renderer's `Rgba8` output (golden
//! tests, stills) and the preview's offscreen frames (legacy Linux `frames` mode, the smoke
//! test's offscreen step).

use super::GpuContext;

/// Format of 8-bit output read back to the CPU: R'G'B'A written as they are (render-graph.md
/// §5.1: display-referred working values, so never an `*Srgb` format).
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A rendered frame read back to the CPU: tightly packed RGBA8 rows, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffscreenFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Why a read-back failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadbackError {
    #[error("waiting for the GPU failed: {0}")]
    Poll(#[from] wgpu::PollError),
    #[error("mapping the read-back buffer failed: {0}")]
    Map(#[from] wgpu::BufferAsyncError),
    #[error("the read-back buffer was not mapped")]
    NotMapped,
    #[error("reading the mapped buffer failed: {0}")]
    Range(#[from] wgpu::MapRangeError),
}

/// An [`OUTPUT_FORMAT`] render target of one size, its read-back buffer and the tight CPU copy.
/// Reused for every frame of that size (review F48).
#[derive(Debug)]
pub(crate) struct Readback {
    pub size: (u32, u32),
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    buffer: wgpu::Buffer,
    padded_row: u32,
    rgba: Vec<u8>,
}

impl Readback {
    pub fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let (width, height) = (size.0.max(1), size.1.max(1));
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("readback.target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Rows in a texture-to-buffer copy must be aligned to 256 bytes.
        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback.buffer"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            size: (width, height),
            texture,
            view,
            buffer,
            padded_row,
            rgba: Vec::with_capacity(width as usize * height as usize * 4),
        }
    }

    /// Records the copy of the target into the read-back buffer (after the output pass).
    pub fn encode_copy(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(self.size.1),
                },
            },
            wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Waits for `submission` (which contains [`Self::encode_copy`]) and returns the tight
    /// RGBA8 rows, `size.0 * 4` bytes each.
    pub fn read(
        &mut self,
        gpu: &GpuContext,
        submission: wgpu::SubmissionIndex,
    ) -> Result<&[u8], ReadbackError> {
        let slice = self.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        gpu.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        rx.try_recv().map_err(|_| ReadbackError::NotMapped)??;
        {
            let mapped = slice.get_mapped_range()?;
            let row = self.size.0 as usize * 4;
            self.rgba.clear();
            for padded in mapped.chunks_exact(self.padded_row as usize) {
                self.rgba.extend_from_slice(&padded[..row]);
            }
        }
        self.buffer.unmap();
        Ok(&self.rgba)
    }
}
