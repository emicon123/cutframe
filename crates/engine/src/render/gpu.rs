/// Human-readable description of the active GPU, for logs and the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuInfo {
    pub adapter: String,
    pub backend: String,
    /// True when running on a software rasterizer (WARP, llvmpipe).
    pub software: bool,
}

/// The wgpu instance, adapter, device and queue shared by all renderers.
#[derive(Debug)]
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("no GPU adapter available (hardware or software fallback): {0}")]
    NoAdapter(#[from] wgpu::RequestAdapterError),
    #[error("failed to create GPU device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("failed to create surface: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
}

impl GpuContext {
    /// Picks the high-performance adapter. Falls back to a software adapter (WARP on Windows,
    /// lavapipe/llvmpipe on Linux) so headless CI and GPU-less machines still work.
    pub fn new() -> Result<Self, GpuError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .or_else(|err| {
            tracing::warn!(%err, "no hardware adapter, trying software fallback");
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: true,
                compatible_surface: None,
                ..Default::default()
            }))
        })?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("engine"),
                required_features: wgpu::Features::empty(),
                // The Pi's V3D GPU only guarantees downlevel limits; stay within what it offers.
                required_limits:
                    wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            }))?;

        let info = adapter.get_info();
        tracing::info!(adapter = %info.name, backend = ?info.backend, kind = ?info.device_type, "GPU ready");
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    pub fn info(&self) -> GpuInfo {
        let info = self.adapter.get_info();
        GpuInfo {
            adapter: info.name,
            backend: format!("{:?}", info.backend),
            software: info.device_type == wgpu::DeviceType::Cpu,
        }
    }
}
