//! Which decoder a reader opens (render-graph.md §5.4): hardware or software, zero-copy or
//! download, the `CUTFRAME_HWDECODE` switch, the hardware reader limit and the per-media memory
//! of failed hardware paths.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use app_core::media::MediaId;

/// `CUTFRAME_HWDECODE=off` (or `0`, `false`, any case) forces software decode, for debugging and
/// for the "every fixture renders without hardware decode" check.
pub(crate) const HWDECODE_ENV: &str = "CUTFRAME_HWDECODE";

/// How a stream's frames reach the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecodePath {
    /// D3D12VA surfaces sampled in place (`FrameData::Gpu`).
    ZeroCopy,
    /// D3D12VA decode with a download to CPU planes (NV12 or P010).
    Download,
    /// Software decode.
    Software,
}

/// Whether a `CUTFRAME_HWDECODE` value allows hardware decode.
pub(crate) fn hw_decode_allowed(value: Option<&str>) -> bool {
    !value.is_some_and(|value| {
        let value = value.trim();
        ["off", "0", "false"]
            .iter()
            .any(|off| value.eq_ignore_ascii_case(off))
    })
}

/// [`hw_decode_allowed`] for the process environment.
pub(crate) fn hw_decode_allowed_by_env() -> bool {
    hw_decode_allowed(std::env::var(HWDECODE_ENV).ok().as_deref())
}

/// Decoder threads of a software reader: `clamp(cores / 2, 2, 8)`.
pub(crate) fn software_threads(cores: usize) -> i32 {
    (cores / 2).clamp(2, 8) as i32
}

/// [`software_threads`] for this machine.
pub(crate) fn software_threads_here() -> i32 {
    software_threads(std::thread::available_parallelism().map_or(4, |n| n.get()))
}

/// What a new reader can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PathInputs {
    /// `CUTFRAME_HWDECODE` allows hardware decode.
    pub hw_allowed: bool,
    /// A D3D12VA device exists on the render device (not WARP, not another backend).
    pub hw_device: bool,
    /// The device samples NV12 and P010 textures (`GpuContext::supports_zero_copy_video`).
    pub zero_copy: bool,
    /// The stream's coded picture size.
    pub extent: (u32, u32),
    /// Smallest width and height the hardware decoder is opened for.
    pub min_extent: u32,
    /// What failed for this media earlier in the session.
    pub remembered: Option<DecodePath>,
}

/// The path a new reader tries first. A hardware path still needs a free hardware slot
/// ([`HwSlots`]); without one the reader decodes in software.
pub(crate) fn initial_path(inputs: PathInputs) -> DecodePath {
    let small = inputs.extent.0 < inputs.min_extent || inputs.extent.1 < inputs.min_extent;
    if !inputs.hw_allowed
        || !inputs.hw_device
        || small
        || inputs.remembered == Some(DecodePath::Software)
    {
        return DecodePath::Software;
    }
    if inputs.zero_copy && inputs.remembered != Some(DecodePath::Download) {
        DecodePath::ZeroCopy
    } else {
        DecodePath::Download
    }
}

/// Counts the readers that hold a hardware decoder (at most 4 per pool, playback.md §7).
#[derive(Debug)]
pub(crate) struct HwSlots {
    used: AtomicUsize,
    max: usize,
}

/// One hardware decoder; frees its slot when dropped.
#[derive(Debug)]
pub(crate) struct HwSlot(Arc<HwSlots>);

impl HwSlots {
    pub fn new(max: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            max,
        })
    }

    /// A slot, or `None` when `max` readers already decode in hardware.
    pub fn try_acquire(self: &Arc<Self>) -> Option<HwSlot> {
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < self.max).then_some(used + 1)
            })
            .ok()
            .map(|_| HwSlot(Arc::clone(self)))
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
}

impl Drop for HwSlot {
    fn drop(&mut self) {
        self.0.used.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Hardware paths that failed per media, remembered for the session (render-graph.md §10), so a
/// new reader of that media starts on the path that works.
#[derive(Debug, Default)]
pub(crate) struct PathMemory(Mutex<HashMap<MediaId, DecodePath>>);

impl PathMemory {
    pub fn get(&self, media: MediaId) -> Option<DecodePath> {
        self.0.lock().ok()?.get(&media).copied()
    }

    /// Records that `media` falls back to `path`. True the first time (log it then): a path
    /// only ever moves down, from zero-copy to download to software.
    pub fn fall_back(&self, media: MediaId, path: DecodePath) -> bool {
        let Ok(mut map) = self.0.lock() else {
            return false;
        };
        let rank = |p: DecodePath| match p {
            DecodePath::ZeroCopy => 0,
            DecodePath::Download => 1,
            DecodePath::Software => 2,
        };
        match map.get(&media) {
            Some(&old) if rank(old) >= rank(path) => false,
            _ => {
                map.insert(media, path);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hwdecode_env_values() {
        assert!(hw_decode_allowed(None));
        assert!(hw_decode_allowed(Some("")));
        assert!(hw_decode_allowed(Some("on")));
        assert!(hw_decode_allowed(Some("auto")));
        for off in ["off", "OFF", " Off ", "0", "false", "False"] {
            assert!(!hw_decode_allowed(Some(off)), "{off:?}");
        }
    }

    #[test]
    fn software_threads_are_half_the_cores_within_2_to_8() {
        assert_eq!(software_threads(1), 2);
        assert_eq!(software_threads(4), 2);
        assert_eq!(software_threads(10), 5);
        assert_eq!(software_threads(20), 8);
        assert_eq!(software_threads(64), 8);
    }

    fn inputs() -> PathInputs {
        PathInputs {
            hw_allowed: true,
            hw_device: true,
            zero_copy: true,
            extent: (3840, 2160),
            min_extent: 256,
            remembered: None,
        }
    }

    #[test]
    fn path_choice() {
        assert_eq!(initial_path(inputs()), DecodePath::ZeroCopy);
        let p = |change: fn(&mut PathInputs)| {
            let mut i = inputs();
            change(&mut i);
            initial_path(i)
        };
        assert_eq!(p(|i| i.hw_allowed = false), DecodePath::Software);
        assert_eq!(p(|i| i.hw_device = false), DecodePath::Software, "WARP");
        assert_eq!(p(|i| i.extent = (255, 2160)), DecodePath::Software);
        assert_eq!(p(|i| i.extent = (64, 64)), DecodePath::Software);
        assert_eq!(p(|i| i.zero_copy = false), DecodePath::Download);
        let remembered = |path| {
            let mut i = inputs();
            i.remembered = Some(path);
            initial_path(i)
        };
        assert_eq!(remembered(DecodePath::Download), DecodePath::Download);
        assert_eq!(remembered(DecodePath::Software), DecodePath::Software);
        assert_eq!(remembered(DecodePath::ZeroCopy), DecodePath::ZeroCopy);
    }

    #[test]
    fn hardware_slots_are_limited_and_freed_on_drop() {
        let slots = HwSlots::new(2);
        let a = slots.try_acquire().expect("first");
        let b = slots.try_acquire().expect("second");
        assert!(slots.try_acquire().is_none(), "limit reached");
        assert_eq!(slots.used(), 2);
        drop(a);
        assert_eq!(slots.used(), 1);
        let _c = slots.try_acquire().expect("a freed slot");
        drop(b);
        assert_eq!(slots.used(), 1);
        assert!(HwSlots::new(0).try_acquire().is_none());
    }

    #[test]
    fn remembered_paths_only_move_down() {
        let memory = PathMemory::default();
        let media = MediaId::new();
        assert_eq!(memory.get(media), None);
        assert!(memory.fall_back(media, DecodePath::Download));
        assert!(
            !memory.fall_back(media, DecodePath::Download),
            "logged once"
        );
        assert!(!memory.fall_back(media, DecodePath::ZeroCopy));
        assert_eq!(memory.get(media), Some(DecodePath::Download));
        assert!(memory.fall_back(media, DecodePath::Software));
        assert!(!memory.fall_back(media, DecodePath::Download));
        assert_eq!(memory.get(media), Some(DecodePath::Software));
    }
}
