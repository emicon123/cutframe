/// Debugging override: `WGPU_FORCE_FALLBACK_ADAPTER=1` (anything but `0`) skips hardware adapters
/// and uses the software rasterizer (WARP on Windows), e.g. to reproduce GPU-less CI locally.
const FORCE_FALLBACK_ENV: &str = "WGPU_FORCE_FALLBACK_ADAPTER";

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
    /// A GPU for offscreen rendering only. Picks the high-performance adapter and falls back to a
    /// software adapter (WARP on Windows, lavapipe/llvmpipe on Linux) so headless CI and GPU-less
    /// machines still work.
    pub fn new() -> Result<Self, GpuError> {
        Self::offscreen(create_instance())
    }

    /// A GPU that can also present to the window behind `target`. Returns the surface for that
    /// window, or `None` (logged) when it cannot be created or no adapter can present to it; the
    /// context then still renders offscreen, like [`Self::new`].
    ///
    /// The surface is created before the adapter is chosen, so the device is created once, on an
    /// adapter that is known to present to the window. Adapters are tried in [`adapter_order`]:
    /// discrete, integrated, other, then the software fallback (WARP).
    ///
    /// # Safety
    /// The handles in `target` must be valid and the window must outlive the returned surface.
    pub unsafe fn with_window(
        target: wgpu::SurfaceTargetUnsafe,
    ) -> Result<(Self, Option<wgpu::Surface<'static>>), GpuError> {
        let instance = create_instance();
        // SAFETY: guaranteed by the caller.
        let surface = match unsafe { instance.create_surface_unsafe(target) } {
            Ok(surface) => surface,
            Err(err) => {
                tracing::error!(%err, "cannot create the preview surface; rendering offscreen only");
                return Ok((Self::offscreen(instance)?, None));
            }
        };

        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let infos: Vec<wgpu::AdapterInfo> = adapters.iter().map(wgpu::Adapter::get_info).collect();
        let types: Vec<wgpu::DeviceType> = infos.iter().map(|info| info.device_type).collect();
        for index in adapter_order(&types, force_fallback()) {
            let (adapter, info) = (&adapters[index], &infos[index]);
            if !adapter.is_surface_supported(&surface) {
                tracing::warn!(
                    adapter = %info.name, backend = ?info.backend, kind = ?info.device_type,
                    "adapter cannot present to the preview window, trying the next one"
                );
                continue;
            }
            match request_device(adapter) {
                Ok((device, queue)) => {
                    tracing::info!(
                        adapter = %info.name, backend = ?info.backend, kind = ?info.device_type,
                        candidates = adapters.len(), "GPU ready, presents to the preview window"
                    );
                    let gpu = Self {
                        instance,
                        adapter: adapter.clone(),
                        device,
                        queue,
                    };
                    return Ok((gpu, Some(surface)));
                }
                Err(err) => tracing::warn!(
                    %err, adapter = %info.name,
                    "cannot create a device on this adapter, trying the next one"
                ),
            }
        }

        tracing::error!(
            candidates = adapters.len(),
            "no adapter can present to the preview window; rendering offscreen only"
        );
        drop(surface);
        Ok((Self::offscreen(instance)?, None))
    }

    fn offscreen(instance: wgpu::Instance) -> Result<Self, GpuError> {
        let request = |force_fallback_adapter, power_preference| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference,
                force_fallback_adapter,
                compatible_surface: None,
                ..Default::default()
            }))
        };
        let adapter = if force_fallback() {
            request(true, wgpu::PowerPreference::LowPower)
        } else {
            request(false, wgpu::PowerPreference::HighPerformance).or_else(|err| {
                tracing::warn!(%err, "no hardware adapter, trying software fallback");
                request(true, wgpu::PowerPreference::LowPower)
            })
        }?;
        let (device, queue) = request_device(&adapter)?;

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

/// Windows uses D3D12 only (ADR-0001). With every backend enabled wgpu prefers Vulkan, and on
/// hybrid-GPU laptops (Optimus) the Vulkan dGPU adapter cannot present to the preview window.
/// `WGPU_BACKEND` still overrides the choice for debugging.
fn create_instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::DX12;
    }
    wgpu::Instance::new(descriptor.with_env())
}

fn request_device(
    adapter: &wgpu::Adapter,
) -> Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("engine"),
        required_features: wgpu::Features::empty(),
        // The Pi's V3D GPU only guarantees downlevel limits; stay within what it offers.
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        memory_hints: wgpu::MemoryHints::Performance,
        ..Default::default()
    }))
}

fn force_fallback() -> bool {
    std::env::var(FORCE_FALLBACK_ENV).is_ok_and(|value| value != "0")
}

/// Indices of `types` in the order adapters are tried for the preview window: high-performance
/// (discrete) first, then integrated, other and virtual GPUs, then the software fallback (WARP).
/// Equal kinds keep the enumeration order (DXGI lists the adapter driving the primary display
/// first). `software_only` keeps only software adapters.
fn adapter_order(types: &[wgpu::DeviceType], software_only: bool) -> Vec<usize> {
    fn rank(kind: wgpu::DeviceType) -> u8 {
        match kind {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            wgpu::DeviceType::Other => 2,
            wgpu::DeviceType::VirtualGpu => 3,
            wgpu::DeviceType::Cpu => 4,
        }
    }
    let mut order: Vec<usize> = (0..types.len())
        .filter(|&index| !software_only || types[index] == wgpu::DeviceType::Cpu)
        .collect();
    // Stable sort: ties keep the enumeration order.
    order.sort_by_key(|&index| rank(types[index]));
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu, Other, VirtualGpu};

    #[test]
    fn hybrid_laptop_tries_discrete_then_integrated_then_warp() {
        // DXGI order on a typical Optimus laptop: iGPU (drives the display), dGPU, WARP.
        assert_eq!(
            adapter_order(&[IntegratedGpu, DiscreteGpu, Cpu], false),
            [1, 0, 2]
        );
    }

    #[test]
    fn software_adapter_is_always_last() {
        assert_eq!(
            adapter_order(&[Cpu, Other, VirtualGpu, IntegratedGpu], false),
            [3, 1, 2, 0]
        );
    }

    #[test]
    fn equal_kinds_keep_enumeration_order() {
        assert_eq!(
            adapter_order(&[DiscreteGpu, Cpu, DiscreteGpu, Cpu], false),
            [0, 2, 1, 3]
        );
    }

    #[test]
    fn software_only_keeps_just_the_fallback() {
        assert_eq!(adapter_order(&[DiscreteGpu, Cpu, IntegratedGpu], true), [1]);
        assert!(adapter_order(&[DiscreteGpu], true).is_empty());
    }

    #[test]
    fn no_adapters_no_candidates() {
        assert!(adapter_order(&[], false).is_empty());
    }
}
