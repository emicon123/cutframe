//! The output pass of the canvas (render-graph.md §4.2 step 4, §8): a bilinear scale of the
//! `Rgba16Float` canvas into an 8-bit target with a black letterbox. The preview presents into
//! its `Bgra8Unorm` surface with it, and the offline output pass converts the canvas 1:1 into
//! `Rgba8Unorm` R'G'B' with it (`OfflineRenderer`). Engine overlays (task 0360) are drawn after
//! it, in the preview only.

use bytemuck::{Pod, Zeroable};

const SHADER: &str = include_str!("shaders/present.wgsl");

/// The shader's `Params`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    rect: [f32; 4],
}

/// The pipeline for one target format. The uniform buffer is written with
/// `Queue::write_buffer`, so submit each encoder before encoding the next output with the same
/// pass.
#[derive(Debug)]
pub(crate) struct PresentPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    /// The bind group of the last canvas view, rebuilt when the canvas changes.
    bound: Option<(wgpu::TextureView, wgpu::BindGroup)>,
}

impl PresentPass {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("present"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present.layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<Uniforms>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present.pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("present.sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("present.uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            bound: None,
        }
    }

    /// Records the scale of `canvas` (`canvas_size` pixels) into `target` (`target_size`
    /// pixels, this pass's format): fitted and centred, black around it.
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        canvas: &wgpu::TextureView,
        canvas_size: (u32, u32),
        target: &wgpu::TextureView,
        target_size: (u32, u32),
    ) {
        let rect = letterbox(canvas_size, target_size);
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&Uniforms { rect }));
        if self.bound.as_ref().is_none_or(|(view, _)| view != canvas) {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("present.bind_group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(canvas),
                    },
                ],
            });
            self.bound = Some((canvas.clone(), bind_group));
        }
        let Some((_, bind_group)) = &self.bound else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("present.pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The rectangle (x, y, width, height in target pixels) of a `canvas`-shaped picture fitted
/// into `target` and centred, on whole pixels so the letterbox edges are sharp.
pub(crate) fn letterbox(canvas: (u32, u32), target: (u32, u32)) -> [f32; 4] {
    let (cw, ch) = (u64::from(canvas.0.max(1)), u64::from(canvas.1.max(1)));
    let (tw, th) = (u64::from(target.0.max(1)), u64::from(target.1.max(1)));
    // The tighter axis fills the target; integer arithmetic keeps it exact for equal aspects.
    let (w, h) = if tw * ch <= th * cw {
        (tw, (tw * ch).div_ceil(cw).min(th))
    } else {
        ((th * cw).div_ceil(ch).min(tw), th)
    };
    let (x, y) = ((tw - w) / 2, (th - h) / 2);
    [x as f32, y as f32, w as f32, h as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_fits_and_centres_on_whole_pixels() {
        assert_eq!(
            letterbox((1920, 1080), (1280, 720)),
            [0.0, 0.0, 1280.0, 720.0]
        );
        assert_eq!(
            letterbox((1920, 1080), (1280, 1000)),
            [0.0, 140.0, 1280.0, 720.0]
        );
        assert_eq!(
            letterbox((1080, 1920), (1280, 720)),
            [437.0, 0.0, 405.0, 720.0]
        );
        assert_eq!(letterbox((64, 36), (64, 36)), [0.0, 0.0, 64.0, 36.0]);
        assert_eq!(letterbox((1920, 1080), (0, 0)), [0.0, 0.0, 1.0, 1.0]);
    }
}
