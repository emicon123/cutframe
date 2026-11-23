//! How the compositor gets decoded frames (render-graph.md §4.1): the fetch policy, the lookup
//! result and the placeholder kinds. The readers implement [`FrameProvider`]; `build_plan` and the
//! compositor only use these types.

use std::sync::Arc;

use app_core::Time;
use app_core::media::MediaId;

#[cfg(windows)]
use crate::media::GpuFrame;
use crate::media::{CpuFrame, DecodedVideoFrame, FrameData};

/// What a layer shows instead of its media (render-graph.md §4.3, §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaceholderKind {
    /// The media file is missing or unreadable.
    Offline,
    /// The file opened but its decoder failed.
    DecodeError,
}

/// What happens when the exact frame is not decoded yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchPolicy {
    /// Preview: never wait; the exact frame if ready, else the nearest decoded frame of that
    /// reader (`Stale`), else `Missing`.
    BestAvailable,
    /// Export, golden tests, `--smoke-test`: block until the exact frame is decoded or the source
    /// fails. Deterministic.
    Exact,
}

/// Identity of one source frame: equal ids mean equal pixels. The compositor keys its source
/// texture cache with it (render-graph.md §6) and compares it to skip redundant composites (§7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameId {
    pub media: MediaId,
    /// Which file, stream and (for images) decode size of the media the frame came from. It
    /// changes when the media is relinked or an image is decoded again at another size.
    pub source: u64,
    /// The frame's media time; `None` for images.
    pub pts: Option<Time>,
}

/// A decoded frame handed to the compositor. Cheap to clone (reference counted).
///
/// **Release contract.** The reader keeps its own reference only while the frame is in its
/// queue. A `FrameRef` keeps the pixels alive, and for [`FrameData::Gpu`] it keeps the decoder
/// surface out of FFmpeg's pool. The compositor therefore:
/// 1. converts a GPU frame right away (render-graph.md §6: the source cache stores the converted
///    texture, never the decoder surface);
/// 2. records `GpuFrame::record_release` into the same command encoder after the frame's last
///    use;
/// 3. keeps a clone of the `FrameRef` until that submission has completed
///    (`Queue::on_submitted_work_done`, or a later `Device::poll` that has seen it), then drops
///    it. Only then may FFmpeg reuse the surface.
///
/// CPU frames have no such rule: they can be dropped as soon as they are uploaded.
#[derive(Debug, Clone)]
pub struct FrameRef {
    id: FrameId,
    frame: Arc<DecodedVideoFrame>,
}

impl FrameRef {
    pub(crate) fn new(id: FrameId, frame: Arc<DecodedVideoFrame>) -> Self {
        Self { id, frame }
    }

    pub fn id(&self) -> FrameId {
        self.id
    }

    /// The frame's media time; `None` for images.
    pub fn pts(&self) -> Option<Time> {
        self.id.pts
    }

    pub fn data(&self) -> &FrameData {
        &self.frame.data
    }

    /// The picture size inside the planes (CPU) or the surface (GPU, whose `coded` size can be
    /// larger). Stored orientation: the convert pass applies `ConvertParams::rotation` and
    /// `display_size`.
    pub fn visible(&self) -> (u32, u32) {
        self.frame.data.visible()
    }

    /// The planes of a CPU frame.
    pub fn cpu(&self) -> Option<&CpuFrame> {
        match &self.frame.data {
            FrameData::Cpu(frame) => Some(frame),
            #[cfg(windows)]
            FrameData::Gpu(_) => None,
        }
    }

    /// The surface of a zero-copy hardware frame.
    #[cfg(windows)]
    pub fn gpu(&self) -> Option<&GpuFrame> {
        match &self.frame.data {
            FrameData::Gpu(frame) => Some(frame),
            FrameData::Cpu(_) => None,
        }
    }
}

/// The answer of a [`FrameProvider`].
#[derive(Debug, Clone)]
pub enum FrameLookup {
    /// The frame for the requested time: the last frame with `pts <= sample_time` (or the first
    /// frame of a stream that starts later; the last frame after the end of the stream).
    Exact(FrameRef),
    /// Not decoded yet (only with [`FetchPolicy::BestAvailable`]): the nearest frame the reader
    /// has, e.g. the keyframe while a seek decodes forward (progressive scrub).
    Stale(FrameRef),
    /// Nothing decoded yet (only with [`FetchPolicy::BestAvailable`]).
    Missing,
    /// The media is offline or its decoder failed: draw the placeholder.
    Failed(PlaceholderKind),
}

impl FrameLookup {
    /// The frame of an `Exact` or `Stale` answer.
    pub fn frame(&self) -> Option<&FrameRef> {
        match self {
            Self::Exact(frame) | Self::Stale(frame) => Some(frame),
            Self::Missing | Self::Failed(_) => None,
        }
    }
}

/// Where the compositor gets decoded frames (render-graph.md §4.1). Implemented by
/// `engine::media::readers::ReaderPool`; tests can implement it with fixed frames.
pub trait FrameProvider {
    /// Called once before the lookups of one composited frame, so the provider knows which
    /// readers that frame already uses (two layers of one media at different times get two
    /// readers). The default does nothing.
    fn begin_frame(&mut self) {}

    /// The frame for a video layer: the last decoded frame with `pts <= sample_time` (sample and
    /// hold, media-import.md §5). `policy` decides what happens when it is not decoded yet.
    fn video(&mut self, media: MediaId, sample_time: Time, policy: FetchPolicy) -> FrameLookup;

    /// The decoded picture of an image layer (decoded once, CPU, packed RGBA with straight
    /// alpha, in stored orientation).
    fn image(&mut self, media: MediaId, policy: FetchPolicy) -> FrameLookup;
}
