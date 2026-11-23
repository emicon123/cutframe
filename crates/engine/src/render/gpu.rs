//! GPU selection and the shared wgpu context.
//!
//! Debugging overrides (environment variables, read when the GPU is created):
//! - `WGPU_FORCE_FALLBACK_ADAPTER=1` forces the software rasterizer (WARP on Windows).
//! - `CUTFRAME_GPU=integrated|discrete` picks which hardware GPU kind is tried first.
//!   `WGPU_FORCE_FALLBACK_ADAPTER` takes precedence over it.

/// Debugging override: `WGPU_FORCE_FALLBACK_ADAPTER=1` (anything but `0`) skips hardware adapters
/// and uses the software rasterizer (WARP on Windows), e.g. to reproduce GPU-less CI locally.
const FORCE_FALLBACK_ENV: &str = "WGPU_FORCE_FALLBACK_ADAPTER";

/// Debugging override: `CUTFRAME_GPU=integrated` tries integrated GPUs before discrete ones (e.g.
/// to measure the preview present rate on a hybrid laptop's iGPU); `CUTFRAME_GPU=discrete` is the
/// default order. Case-insensitive. Unset or empty keeps the default; any other value is ignored
/// with a warning. The preview still falls back to the next adapter when the preferred one cannot
/// present to the window. Offscreen rendering maps `integrated` to the low-power adapter.
const GPU_PREFERENCE_ENV: &str = "CUTFRAME_GPU";

/// Which hardware GPU kind is tried first (see [`GPU_PREFERENCE_ENV`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum GpuPreference {
    #[default]
    Discrete,
    Integrated,
}

impl GpuPreference {
    /// Parses a `CUTFRAME_GPU` value; `None` for an unknown one.
    fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("discrete") {
            Some(Self::Discrete)
        } else if value.eq_ignore_ascii_case("integrated") {
            Some(Self::Integrated)
        } else {
            None
        }
    }

    fn from_env() -> Self {
        match std::env::var(GPU_PREFERENCE_ENV) {
            Ok(value) if value.trim().is_empty() => Self::default(),
            Ok(value) => Self::parse(&value).unwrap_or_else(|| {
                tracing::warn!(
                    value = %value, expected = "integrated | discrete",
                    "unknown {GPU_PREFERENCE_ENV} value, using the default adapter order"
                );
                Self::default()
            }),
            Err(std::env::VarError::NotPresent) => Self::default(),
            Err(err @ std::env::VarError::NotUnicode(_)) => {
                tracing::warn!(
                    %err,
                    "invalid {GPU_PREFERENCE_ENV} value, using the default adapter order"
                );
                Self::default()
            }
        }
    }
}

/// Description of the active GPU, for logs, the status bar and the capability report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuInfo {
    /// Adapter name as the driver reports it.
    pub adapter: String,
    pub backend: String,
    /// True when running on a software rasterizer (WARP, llvmpipe).
    pub software: bool,
    pub device_type: DeviceKind,
    /// Driver description (`AdapterInfo::driver_info`); may be empty.
    pub driver: String,
    /// Whether the preview renders into the native window; `false` = offscreen only.
    pub presents_to_window: bool,
    /// Why this is not the first-choice adapter, if it is not.
    pub fallback: Option<FallbackReason>,
}

/// Kind of GPU adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Discrete,
    Integrated,
    Virtual,
    /// A software rasterizer (WARP on Windows).
    Software,
    Other,
}

impl From<wgpu::DeviceType> for DeviceKind {
    fn from(kind: wgpu::DeviceType) -> Self {
        match kind {
            wgpu::DeviceType::DiscreteGpu => Self::Discrete,
            wgpu::DeviceType::IntegratedGpu => Self::Integrated,
            wgpu::DeviceType::VirtualGpu => Self::Virtual,
            wgpu::DeviceType::Cpu => Self::Software,
            wgpu::DeviceType::Other => Self::Other,
        }
    }
}

/// Why the GPU in use is not the first-choice adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// `WGPU_FORCE_FALLBACK_ADAPTER` forced the software rasterizer.
    ForcedByEnv,
    /// A preferred adapter was skipped because it cannot present to the preview window (or its
    /// device could not be created), or no adapter can present and the preview is offscreen only.
    PreferredCannotPresent,
    /// There is no hardware adapter; the software rasterizer is used.
    NoHardwareAdapter,
    /// The GPU device was lost at runtime and the context was recreated.
    DeviceLost,
}

/// Which adapters a new context may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterMode {
    /// Hardware first, software fallback last (the normal order).
    Auto,
    /// Only the software rasterizer (WARP), e.g. after the hardware device was lost twice.
    SoftwareOnly,
}

/// The wgpu instance, adapter, device and queue shared by all renderers.
#[derive(Debug)]
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Why this is not the first-choice adapter (reported in [`GpuInfo::fallback`]).
    pub fallback: Option<FallbackReason>,
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
        Self::new_with(AdapterMode::Auto)
    }

    /// [`Self::new`] restricted to the adapters `mode` allows.
    pub fn new_with(mode: AdapterMode) -> Result<Self, GpuError> {
        Self::offscreen(create_instance(), mode)
    }

    /// A GPU that can also present to the window behind `target`. Returns the surface for that
    /// window, or `None` (logged) when it cannot be created or no adapter can present to it; the
    /// context then still renders offscreen, like [`Self::new`].
    ///
    /// The surface is created before the adapter is chosen, so the device is created once, on an
    /// adapter that is known to present to the window. Adapters are tried in [`adapter_order`]:
    /// discrete, integrated, other, then the software fallback (WARP); `CUTFRAME_GPU=integrated`
    /// swaps the first two. [`AdapterMode::SoftwareOnly`] tries only the software fallback.
    ///
    /// # Safety
    /// The handles in `target` must be valid and the window must outlive the returned surface.
    pub unsafe fn with_window(
        target: wgpu::SurfaceTargetUnsafe,
        mode: AdapterMode,
    ) -> Result<(Self, Option<wgpu::Surface<'static>>), GpuError> {
        let instance = create_instance();
        let forced = force_fallback();
        // SAFETY: guaranteed by the caller.
        let surface = match unsafe { instance.create_surface_unsafe(target) } {
            Ok(surface) => surface,
            Err(err) => {
                tracing::error!(%err, "cannot create the preview surface; rendering offscreen only");
                return Ok((Self::offscreen_only(instance, mode, forced)?, None));
            }
        };

        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let infos: Vec<wgpu::AdapterInfo> = adapters.iter().map(wgpu::Adapter::get_info).collect();
        let types: Vec<wgpu::DeviceType> = infos.iter().map(|info| info.device_type).collect();
        let preference = GpuPreference::from_env();
        let software_only = forced || mode == AdapterMode::SoftwareOnly;
        let mut skipped = false;
        for index in adapter_order(&types, software_only, preference) {
            let (adapter, info) = (&adapters[index], &infos[index]);
            if !adapter.is_surface_supported(&surface) {
                skipped = true;
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
                        candidates = adapters.len(), ?preference,
                        "GPU ready, presents to the preview window"
                    );
                    let gpu = Self {
                        instance,
                        adapter: adapter.clone(),
                        device,
                        queue,
                        fallback: fallback_reason(&types, types[index], skipped, forced),
                    };
                    return Ok((gpu, Some(surface)));
                }
                Err(err) => {
                    skipped = true;
                    tracing::warn!(
                        %err, adapter = %info.name,
                        "cannot create a device on this adapter, trying the next one"
                    );
                }
            }
        }

        tracing::error!(
            candidates = adapters.len(),
            "no adapter can present to the preview window; rendering offscreen only"
        );
        drop(surface);
        Ok((Self::offscreen_only(instance, mode, forced)?, None))
    }

    /// The offscreen fallback of [`Self::with_window`]: the window cannot be presented to.
    fn offscreen_only(
        instance: wgpu::Instance,
        mode: AdapterMode,
        forced: bool,
    ) -> Result<Self, GpuError> {
        let mut gpu = Self::offscreen(instance, mode)?;
        if !forced {
            gpu.fallback = Some(FallbackReason::PreferredCannotPresent);
        }
        Ok(gpu)
    }

    fn offscreen(instance: wgpu::Instance, mode: AdapterMode) -> Result<Self, GpuError> {
        let request = |force_fallback_adapter, power_preference| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference,
                force_fallback_adapter,
                compatible_surface: None,
                ..Default::default()
            }))
        };
        let forced = force_fallback();
        let adapter = if forced || mode == AdapterMode::SoftwareOnly {
            request(true, wgpu::PowerPreference::LowPower)
        } else {
            let power_preference = match GpuPreference::from_env() {
                GpuPreference::Discrete => wgpu::PowerPreference::HighPerformance,
                GpuPreference::Integrated => wgpu::PowerPreference::LowPower,
            };
            request(false, power_preference).or_else(|err| {
                tracing::warn!(%err, "no hardware adapter, trying software fallback");
                request(true, wgpu::PowerPreference::LowPower)
            })
        }?;
        let (device, queue) = request_device(&adapter)?;

        let info = adapter.get_info();
        tracing::info!(adapter = %info.name, backend = ?info.backend, kind = ?info.device_type, "GPU ready");
        let fallback = if forced {
            Some(FallbackReason::ForcedByEnv)
        } else if info.device_type == wgpu::DeviceType::Cpu && mode == AdapterMode::Auto {
            Some(FallbackReason::NoHardwareAdapter)
        } else {
            None
        };
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            fallback,
        })
    }

    /// The report for this context. `presents_to_window` is `false`: only the owner of the
    /// surface knows whether it presents.
    pub fn info(&self) -> GpuInfo {
        let info = self.adapter.get_info();
        GpuInfo {
            adapter: info.name,
            backend: format!("{:?}", info.backend),
            software: info.device_type == wgpu::DeviceType::Cpu,
            device_type: info.device_type.into(),
            driver: info.driver_info,
            presents_to_window: false,
            fallback: self.fallback,
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

/// Video texture formats requested when the adapter has them, never required (render-graph.md
/// §5.1): `NV12` and `P010` sample hardware-decoded frames in place (zero-copy, D3D12), and
/// `16BIT_NORM` holds 10-bit CPU planes (every D3D12 adapter, WARP included).
const OPTIONAL_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_FORMAT_NV12
    .union(wgpu::Features::TEXTURE_FORMAT_P010)
    .union(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM);

/// The features zero-copy hardware decode needs (render-graph.md §5.4).
const ZERO_COPY_FEATURES: wgpu::Features =
    wgpu::Features::TEXTURE_FORMAT_NV12.union(wgpu::Features::TEXTURE_FORMAT_P010);

impl GpuContext {
    /// Whether decoded D3D12VA frames can be sampled in place: the device is D3D12 and has both
    /// `NV12` and `P010` textures. It says nothing about hardware video decode itself (WARP has
    /// the formats but no video decoder); opening the hardware decoder decides that.
    pub fn supports_zero_copy_video(&self) -> bool {
        self.adapter.get_info().backend == wgpu::Backend::Dx12
            && self.device.features().contains(ZERO_COPY_FEATURES)
    }
}

fn request_device(
    adapter: &wgpu::Adapter,
) -> Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("engine"),
        required_features: adapter.features() & OPTIONAL_FEATURES,
        // The Pi's V3D GPU only guarantees downlevel limits; stay within what it offers.
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        memory_hints: wgpu::MemoryHints::Performance,
        ..Default::default()
    }))
}

/// Why `chosen` (the adapter that presents to the window) is not the first choice. `types` are all
/// enumerated adapters; `skipped` is whether a better-ranked candidate was skipped.
fn fallback_reason(
    types: &[wgpu::DeviceType],
    chosen: wgpu::DeviceType,
    skipped: bool,
    forced: bool,
) -> Option<FallbackReason> {
    if forced {
        Some(FallbackReason::ForcedByEnv)
    } else if chosen == wgpu::DeviceType::Cpu && types.iter().all(|&t| t == wgpu::DeviceType::Cpu) {
        Some(FallbackReason::NoHardwareAdapter)
    } else if skipped {
        Some(FallbackReason::PreferredCannotPresent)
    } else {
        None
    }
}

fn force_fallback() -> bool {
    std::env::var(FORCE_FALLBACK_ENV).is_ok_and(|value| value != "0")
}

/// Indices of `types` in the order adapters are tried for the preview window: high-performance
/// (discrete) first, then integrated, other and virtual GPUs, then the software fallback (WARP).
/// [`GpuPreference::Integrated`] tries integrated GPUs before discrete ones. Equal kinds keep the
/// enumeration order (DXGI lists the adapter driving the primary display first). `software_only`
/// keeps only software adapters (`preference` then has no effect).
fn adapter_order(
    types: &[wgpu::DeviceType],
    software_only: bool,
    preference: GpuPreference,
) -> Vec<usize> {
    let rank = |kind: wgpu::DeviceType| -> u8 {
        match (kind, preference) {
            (wgpu::DeviceType::DiscreteGpu, GpuPreference::Discrete)
            | (wgpu::DeviceType::IntegratedGpu, GpuPreference::Integrated) => 0,
            (wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu, _) => 1,
            (wgpu::DeviceType::Other, _) => 2,
            (wgpu::DeviceType::VirtualGpu, _) => 3,
            (wgpu::DeviceType::Cpu, _) => 4,
        }
    };
    let mut order: Vec<usize> = (0..types.len())
        .filter(|&index| !software_only || types[index] == wgpu::DeviceType::Cpu)
        .collect();
    // Stable sort: ties keep the enumeration order.
    order.sort_by_key(|&index| rank(types[index]));
    order
}

#[cfg(test)]
mod tests {
    use super::GpuPreference::{Discrete, Integrated};
    use super::*;
    use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu, Other, VirtualGpu};

    #[test]
    fn hybrid_laptop_tries_discrete_then_integrated_then_warp() {
        // DXGI order on a typical Optimus laptop: iGPU (drives the display), dGPU, WARP.
        assert_eq!(
            adapter_order(&[IntegratedGpu, DiscreteGpu, Cpu], false, Discrete),
            [1, 0, 2]
        );
    }

    #[test]
    fn software_adapter_is_always_last() {
        assert_eq!(
            adapter_order(&[Cpu, Other, VirtualGpu, IntegratedGpu], false, Discrete),
            [3, 1, 2, 0]
        );
    }

    #[test]
    fn equal_kinds_keep_enumeration_order() {
        assert_eq!(
            adapter_order(&[DiscreteGpu, Cpu, DiscreteGpu, Cpu], false, Discrete),
            [0, 2, 1, 3]
        );
    }

    #[test]
    fn software_only_keeps_just_the_fallback() {
        assert_eq!(
            adapter_order(&[DiscreteGpu, Cpu, IntegratedGpu], true, Discrete),
            [1]
        );
        assert!(adapter_order(&[DiscreteGpu], true, Discrete).is_empty());
    }

    #[test]
    fn no_adapters_no_candidates() {
        assert!(adapter_order(&[], false, Discrete).is_empty());
        assert!(adapter_order(&[], false, Integrated).is_empty());
    }

    #[test]
    fn integrated_preference_tries_integrated_then_discrete_then_warp() {
        assert_eq!(
            adapter_order(&[IntegratedGpu, DiscreteGpu, Cpu], false, Integrated),
            [0, 1, 2]
        );
        assert_eq!(
            adapter_order(&[DiscreteGpu, IntegratedGpu, Cpu], false, Integrated),
            [1, 0, 2]
        );
    }

    #[test]
    fn integrated_preference_keeps_the_rest_of_the_order() {
        assert_eq!(
            adapter_order(
                &[
                    Cpu,
                    VirtualGpu,
                    Other,
                    DiscreteGpu,
                    IntegratedGpu,
                    IntegratedGpu
                ],
                false,
                Integrated
            ),
            [4, 5, 3, 2, 1, 0]
        );
    }

    #[test]
    fn integrated_preference_without_an_igpu_keeps_the_default_order() {
        assert_eq!(
            adapter_order(&[Cpu, Other, DiscreteGpu], false, Integrated),
            [2, 1, 0]
        );
    }

    #[test]
    fn software_only_ignores_the_preference() {
        assert_eq!(
            adapter_order(&[IntegratedGpu, Cpu, DiscreteGpu], true, Integrated),
            [1]
        );
    }

    #[test]
    fn fallback_reason_explains_the_chosen_adapter() {
        use FallbackReason::*;
        // First choice presents: no fallback.
        assert_eq!(
            fallback_reason(
                &[IntegratedGpu, DiscreteGpu, Cpu],
                DiscreteGpu,
                false,
                false
            ),
            None
        );
        // The dGPU could not present, the iGPU does.
        assert_eq!(
            fallback_reason(
                &[IntegratedGpu, DiscreteGpu, Cpu],
                IntegratedGpu,
                true,
                false
            ),
            Some(PreferredCannotPresent)
        );
        // Hardware exists but nothing presents except WARP.
        assert_eq!(
            fallback_reason(&[DiscreteGpu, Cpu], Cpu, true, false),
            Some(PreferredCannotPresent)
        );
        // Only WARP exists (CI runners, VMs).
        assert_eq!(
            fallback_reason(&[Cpu], Cpu, false, false),
            Some(NoHardwareAdapter)
        );
        // The environment override wins over everything.
        assert_eq!(
            fallback_reason(&[DiscreteGpu, Cpu], Cpu, false, true),
            Some(ForcedByEnv)
        );
        assert_eq!(fallback_reason(&[Cpu], Cpu, true, true), Some(ForcedByEnv));
    }

    #[test]
    fn device_types_map_to_kinds() {
        assert_eq!(DeviceKind::from(DiscreteGpu), DeviceKind::Discrete);
        assert_eq!(DeviceKind::from(IntegratedGpu), DeviceKind::Integrated);
        assert_eq!(DeviceKind::from(VirtualGpu), DeviceKind::Virtual);
        assert_eq!(DeviceKind::from(Cpu), DeviceKind::Software);
        assert_eq!(DeviceKind::from(Other), DeviceKind::Other);
    }

    #[test]
    fn parses_the_preference_case_insensitively() {
        assert_eq!(GpuPreference::parse("integrated"), Some(Integrated));
        assert_eq!(GpuPreference::parse("Integrated"), Some(Integrated));
        assert_eq!(GpuPreference::parse(" INTEGRATED "), Some(Integrated));
        assert_eq!(GpuPreference::parse("discrete"), Some(Discrete));
        assert_eq!(GpuPreference::parse("DISCRETE"), Some(Discrete));
        assert_eq!(GpuPreference::parse("igpu"), None);
        assert_eq!(GpuPreference::parse(""), None);
        assert_eq!(GpuPreference::default(), Discrete);
    }
}
