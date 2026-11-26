//! A reader's decode-ahead queue and its rules (render-graph.md §5.3, §6; playback.md §7). Pure
//! logic: the reader thread pushes decoded frames, the consumer retargets and selects; both under
//! the reader's lock.

use std::collections::VecDeque;
use std::sync::Arc;

use app_core::Time;

use crate::media::frame::DecodedVideoFrame;

/// Decoding forward is cheaper than a seek in long-GOP phone footage: a target up to this far
/// past the last decoded frame is reached without seeking.
pub(crate) const NO_SEEK_WINDOW: Time = Time::from_seconds(1);

/// CPU memory one reader's queue may hold (4K P010 is about 25 MB a frame).
pub(crate) const CPU_QUEUE_BYTES: usize = 160 * 1024 * 1024;

/// Frames past the target a paused reader decodes (for the Right key), then it stops.
pub(crate) const PAUSED_AHEAD: usize = 2;

/// Catch-up (playback.md §7): a reader whose newest frame is more than this far behind the
/// sample time seeks ahead of it.
pub(crate) const FAR_BEHIND: Time = Time(app_core::FLICKS_PER_SECOND / 2);

/// How far past the sample time a catch-up seek lands.
pub(crate) const CATCH_UP_LEAD: Time = Time(app_core::FLICKS_PER_SECOND / 4);

/// Which frames the decoder skips (`AVCodecContext::skip_frame`, playback.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Discard {
    /// Decode every frame.
    Nothing,
    /// Skip frames no other frame references (catch-up).
    NonRef,
    /// Keyframes only (shuttle above 2x).
    NonKey,
}

/// How far a reader decodes ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The exact frame plus [`PAUSED_AHEAD`].
    Paused,
    /// Until the queue is full.
    Playing,
}

/// What the queue holds for a sample time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Select {
    /// Index of the frame for the time: the last one with `pts <= t` and a later frame (or the
    /// end of the stream) behind it, or the first frame of a stream that starts after `t`.
    Exact(usize),
    /// Index of the nearest frame; the exact one is still being decoded.
    Stale(usize),
    Missing,
}

#[derive(Debug)]
pub(crate) struct FrameQueue {
    frames: VecDeque<Arc<DecodedVideoFrame>>,
    /// CPU bytes of `frames`.
    bytes: usize,
    depth: usize,
    byte_cap: usize,
    /// The consumer's latest sample time.
    target: Option<Time>,
    /// The reader has to seek to `target` and has not started yet. Frames pushed meanwhile
    /// belong to the old position and are dropped.
    seek_pending: bool,
    /// The target of the seek the reader runs, until its first frame arrives.
    seeking: Option<Time>,
    /// The first frame after a seek when it came after the seek target: nothing in the stream
    /// lies before it, so it is the frame for any earlier time.
    stream_start: Option<Time>,
    /// The last decoded frame since the last seek.
    last_pts: Option<Time>,
    /// The decoder has drained: nothing follows the last frame.
    eos: bool,
    /// The target of the last seek (or catch-up jump) until a frame at or past it arrives:
    /// the reader is still decoding forward to it, so it is not behind playback.
    approaching: Option<Time>,
}

impl FrameQueue {
    pub fn new(depth: usize, byte_cap: usize) -> Self {
        Self {
            frames: VecDeque::with_capacity(depth),
            bytes: 0,
            depth: depth.max(2),
            byte_cap,
            target: None,
            seek_pending: false,
            seeking: None,
            stream_start: None,
            last_pts: None,
            eos: false,
            approaching: None,
        }
    }

    pub fn target(&self) -> Option<Time> {
        self.target
    }

    pub fn frame(&self, index: usize) -> &Arc<DecodedVideoFrame> {
        &self.frames[index]
    }

    #[cfg(test)]
    pub fn pts(&self) -> Vec<i64> {
        self.frames.iter().map(|f| f.pts.flicks()).collect()
    }

    /// Whether `t` is reached from what the reader has (or is about to decode) without a seek.
    fn reachable(&self, t: Time) -> bool {
        let Some(front) = self.frames.front().map(|f| f.pts) else {
            // Nothing decoded since the last seek: decode on if that seek lands close before `t`.
            return self
                .seeking
                .is_some_and(|s| t >= s && t <= s + NO_SEEK_WINDOW);
        };
        if t < front {
            return self.stream_start == Some(front);
        }
        if self.eos {
            return true;
        }
        t <= self.last_pts.unwrap_or(front) + NO_SEEK_WINDOW
    }

    /// The distance from what the reader has to `t` when `t` is reachable without a seek (for
    /// picking the best of several readers of one media).
    pub fn distance(&self, t: Time) -> Option<Time> {
        if self.seek_pending || !self.reachable(t) {
            return None;
        }
        let from = self
            .frames
            .front()
            .map(|f| f.pts)
            .or(self.seeking)
            .unwrap_or(t);
        Some(Time::from_flicks((t - from).flicks().max(0)))
    }

    /// Sets the consumer's sample time. Returns true when the reader must seek (the caller then
    /// interrupts the reader's current decode). A newer target replaces a pending one
    /// (coalescing); frames before `t` are dropped except the last one (sample and hold).
    pub fn retarget(&mut self, t: Time) -> bool {
        if self.target == Some(t) {
            return self.seek_pending;
        }
        self.target = Some(t);
        if !self.seek_pending && self.reachable(t) {
            self.drop_before(t);
            return false;
        }
        self.clear();
        self.seek_pending = true;
        true
    }

    /// Makes the reader seek to the current target again (after it reopened its decoder).
    pub fn force_seek(&mut self) {
        self.clear();
        self.seek_pending = self.target.is_some();
    }

    /// The pending seek, taken by the reader thread when it starts it.
    pub fn take_seek(&mut self) -> Option<Time> {
        if !self.seek_pending {
            return None;
        }
        self.seek_pending = false;
        self.clear();
        self.seeking = self.target;
        self.approaching = self.target;
        self.target
    }

    /// The reader read the stream again from its start (a seek that found no frame).
    pub fn restart(&mut self) {
        self.clear();
        self.seeking = Some(Time::from_flicks(i64::MIN));
    }

    fn clear(&mut self) {
        self.frames.clear();
        self.bytes = 0;
        self.seeking = None;
        self.stream_start = None;
        self.last_pts = None;
        self.eos = false;
        self.approaching = None;
    }

    fn drop_before(&mut self, t: Time) {
        while self.frames.len() >= 2 && self.frames[1].pts <= t {
            if let Some(old) = self.frames.pop_front() {
                self.bytes -= old.data.cpu_bytes();
            }
        }
    }

    /// Adds a decoded frame (pts strictly increasing since the last seek). Frames up to the
    /// target replace each other: only the last one is kept. Returns false when the frame was
    /// dropped because a seek is pending.
    pub fn push(&mut self, frame: Arc<DecodedVideoFrame>) -> bool {
        if self.seek_pending {
            return false;
        }
        let pts = frame.pts;
        if let Some(seek) = self.seeking.take()
            && pts > seek
        {
            self.stream_start = Some(pts);
        }
        self.last_pts = Some(pts);
        if self.approaching.is_some_and(|a| pts >= a) {
            self.approaching = None;
        }
        if self.target.is_some_and(|t| pts <= t) {
            self.frames.clear();
            self.bytes = 0;
        }
        self.bytes += frame.data.cpu_bytes();
        self.frames.push_back(frame);
        true
    }

    /// The decoder drained. Returns true when no frame was decoded since the last seek (the
    /// reader then reads from the start, or gives up).
    pub fn set_eos(&mut self) -> bool {
        if self.seek_pending {
            return false;
        }
        self.eos = true;
        self.approaching = None;
        self.frames.is_empty()
    }

    /// How far the newest decoded frame is behind `t` (negative: ahead), for catch-up. `None`
    /// while a seek is pending or running, while the reader decodes forward to the target of
    /// its last seek, at the end of the stream and before the first frame.
    pub fn behind(&self, t: Time) -> Option<Time> {
        if self.seek_pending || self.seeking.is_some() || self.approaching.is_some() || self.eos {
            return None;
        }
        self.last_pts.map(|newest| t - newest)
    }

    /// Catch-up: moves the target to `t` (ahead of the sample time); the reader decodes
    /// forward to it, or seeks when it is out of reach. Returns true when a seek is needed.
    pub fn jump_ahead(&mut self, t: Time) -> bool {
        let seek = self.retarget(t);
        if !seek {
            self.approaching = Some(t);
        }
        seek
    }

    /// Whether the reader should decode another frame.
    pub fn has_room(&self, mode: Mode) -> bool {
        if self.seek_pending || self.eos || self.target.is_none() {
            return false;
        }
        let len = self.frames.len();
        // The frame at the target and one after it (which makes it exact) always fit.
        if len < 2 {
            return true;
        }
        if len >= self.depth || self.bytes >= self.byte_cap {
            return false;
        }
        match mode {
            Mode::Playing => true,
            Mode::Paused => self.ahead() < PAUSED_AHEAD,
        }
    }

    /// Frames after the target.
    fn ahead(&self) -> usize {
        let t = self.target.unwrap_or(Time::from_flicks(i64::MAX));
        self.frames.len() - self.frames.partition_point(|f| f.pts <= t)
    }

    /// The frame for `t` (sample and hold).
    pub fn select(&self, t: Time) -> Select {
        let at_or_before = self.frames.partition_point(|f| f.pts <= t);
        let Some(index) = at_or_before.checked_sub(1) else {
            return match self.frames.front() {
                None => Select::Missing,
                Some(first) if self.stream_start == Some(first.pts) => Select::Exact(0),
                Some(_) => Select::Stale(0),
            };
        };
        if index + 1 < self.frames.len() || self.eos {
            Select::Exact(index)
        } else {
            Select::Stale(index)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::frame::{CpuFrame, FrameData, PlaneBuf, PlaneLayout, SampleDepth};

    fn frame(pts: i64, bytes: usize) -> Arc<DecodedVideoFrame> {
        Arc::new(DecodedVideoFrame {
            pts: Time::from_flicks(pts),
            data: FrameData::Cpu(CpuFrame {
                layout: PlaneLayout::Packed,
                depth: SampleDepth::U8,
                visible: (1, 1),
                planes: [
                    Some(PlaneBuf {
                        stride: 4,
                        bytes: vec![0; bytes],
                    }),
                    None,
                    None,
                ],
            }),
        })
    }

    const S: i64 = app_core::FLICKS_PER_SECOND;
    const F: i64 = S / 30;

    fn t(flicks: i64) -> Time {
        Time::from_flicks(flicks)
    }

    /// A queue that has sought to `target` and decoded frames at `pts`.
    fn decoded(target: i64, pts: &[i64]) -> FrameQueue {
        let mut q = FrameQueue::new(6, CPU_QUEUE_BYTES);
        assert!(q.retarget(t(target)), "a new reader seeks");
        assert_eq!(q.take_seek(), Some(t(target)));
        for &p in pts {
            assert!(q.push(frame(p, 4)));
        }
        q
    }

    #[test]
    fn frames_before_the_target_are_dropped_except_the_last() {
        // Seek to 10 F from a keyframe at 0: frames 0..=10 replace each other.
        let mut q = decoded(10 * F, &[]);
        for n in 0..=10 {
            assert!(q.push(frame(n * F, 4)));
            assert_eq!(
                q.pts(),
                vec![n * F],
                "progressive scrub: frame {n} is published"
            );
            assert_eq!(q.select(t(10 * F)), Select::Stale(0));
        }
        q.push(frame(11 * F, 4));
        assert_eq!(q.pts(), vec![10 * F, 11 * F]);
        assert_eq!(q.select(t(10 * F)), Select::Exact(0));
    }

    #[test]
    fn sample_and_hold_picks_the_last_frame_at_or_before_t() {
        let q = decoded(0, &[0, F, 3 * F, 4 * F]);
        assert_eq!(q.select(t(0)), Select::Exact(0));
        assert_eq!(q.select(t(F + 1)), Select::Exact(1));
        assert_eq!(q.select(t(3 * F - 1)), Select::Exact(1), "VFR gap holds");
        assert_eq!(q.select(t(3 * F)), Select::Exact(2));
        // The last frame is exact only once something follows it.
        assert_eq!(q.select(t(10 * F)), Select::Stale(3));
    }

    #[test]
    fn the_last_frame_holds_at_the_end_of_the_stream() {
        let mut q = decoded(0, &[0, F]);
        assert_eq!(q.select(t(5 * F)), Select::Stale(1));
        assert!(!q.set_eos());
        assert_eq!(q.select(t(5 * F)), Select::Exact(1));
        assert!(!q.has_room(Mode::Playing), "nothing more to decode");
        // Any later target is reached without a seek.
        assert!(!q.retarget(t(100 * S)));
        assert_eq!(q.select(t(100 * S)), Select::Exact(0));
        assert_eq!(q.pts(), vec![F]);
    }

    #[test]
    fn a_stream_that_starts_after_the_target_shows_its_first_frame() {
        let mut q = decoded(0, &[2 * F, 3 * F]);
        assert_eq!(q.select(t(0)), Select::Exact(0));
        assert_eq!(q.select(t(F)), Select::Exact(0));
        // Earlier still: nothing exists before the first frame, no seek.
        assert!(!q.retarget(t(-S)));
        assert_eq!(q.select(t(-S)), Select::Exact(0));
        // A keyframe before the target is not the start of the stream.
        let q = decoded(10 * F, &[5 * F]);
        assert_eq!(q.select(t(4 * F)), Select::Stale(0));
    }

    #[test]
    fn no_seek_within_one_second_after_the_last_decoded_frame() {
        let mut q = decoded(0, &[0, F, 2 * F]);
        assert!(!q.retarget(t(F)), "inside the queue");
        assert_eq!(q.pts(), vec![F, 2 * F]);
        assert!(!q.retarget(t(2 * F + S)), "exactly 1 s past the last frame");
        assert_eq!(
            q.pts(),
            vec![2 * F],
            "the last frame at or before the target stays"
        );
        assert!(q.retarget(t(2 * F + S + 1)), "past the window");
        assert_eq!(q.pts(), Vec::<i64>::new());
        assert_eq!(q.select(t(2 * F + S + 1)), Select::Missing);

        let mut q = decoded(0, &[0, F, 2 * F]);
        assert!(q.retarget(t(-1)), "backwards");
        let mut q = decoded(10 * F, &[10 * F, 11 * F]);
        assert!(
            q.retarget(t(9 * F)),
            "before the first frame of a mid-stream seek"
        );
    }

    #[test]
    fn a_running_seek_serves_targets_shortly_after_it() {
        let mut q = decoded(10 * F, &[]);
        assert_eq!(q.distance(t(12 * F)), Some(t(2 * F)));
        assert!(!q.retarget(t(12 * F)), "the seek to 10 F reaches 12 F");
        assert!(q.retarget(t(9 * F)), "before the running seek's target");
        assert_eq!(q.distance(t(9 * F)), None, "a seek is pending");
    }

    #[test]
    fn newer_requests_replace_a_pending_seek() {
        let mut q = decoded(0, &[0, F]);
        assert!(q.retarget(t(10 * S)));
        assert!(
            q.retarget(t(20 * S)),
            "still pending: the caller keeps the reader interrupted"
        );
        assert!(
            !q.push(frame(2 * F, 4)),
            "a frame of the old position is dropped"
        );
        assert_eq!(
            q.take_seek(),
            Some(t(20 * S)),
            "only the newest target is sought"
        );
        assert_eq!(q.take_seek(), None);
        assert!(!q.retarget(t(20 * S)), "same target, no new seek");
    }

    #[test]
    fn paused_readers_stop_two_frames_ahead() {
        let mut q = decoded(F, &[0]);
        assert!(q.has_room(Mode::Paused));
        q.push(frame(F, 4));
        assert!(q.has_room(Mode::Paused));
        q.push(frame(2 * F, 4));
        assert!(q.has_room(Mode::Paused), "exact + 1");
        q.push(frame(3 * F, 4));
        assert!(!q.has_room(Mode::Paused), "exact + 2");
        assert!(q.has_room(Mode::Playing));
        for n in 4..=6 {
            q.push(frame(n * F, 4));
        }
        assert_eq!(q.pts().len(), 6);
        assert!(!q.has_room(Mode::Playing), "queue depth");
        // Playback moves on: room again.
        assert!(!q.retarget(t(3 * F)));
        assert!(q.has_room(Mode::Playing));
    }

    #[test]
    fn cpu_frames_are_capped_by_bytes_but_two_always_fit() {
        let mut q = FrameQueue::new(6, 100);
        q.retarget(t(0));
        q.take_seek();
        q.push(frame(0, 80));
        assert!(q.has_room(Mode::Playing));
        q.push(frame(F, 80));
        assert!(!q.has_room(Mode::Playing), "over the byte cap");
        assert_eq!(q.select(t(0)), Select::Exact(0));
        // Dropping consumed frames frees their bytes.
        assert!(!q.retarget(t(F)));
        assert!(q.has_room(Mode::Playing));
        q.push(frame(2 * F, 10));
        assert!(q.has_room(Mode::Playing), "90 bytes");
        q.push(frame(3 * F, 10));
        assert!(!q.has_room(Mode::Playing), "100 bytes");
    }

    #[test]
    fn nothing_decodes_without_a_target_or_with_a_pending_seek() {
        let mut q = FrameQueue::new(6, CPU_QUEUE_BYTES);
        assert!(!q.has_room(Mode::Playing));
        assert!(q.retarget(t(0)));
        assert!(!q.has_room(Mode::Playing), "seek first");
        q.take_seek();
        assert!(q.has_room(Mode::Playing));
    }

    #[test]
    fn behind_is_measured_only_once_the_seek_target_was_reached() {
        let mut q = decoded(10 * F, &[]);
        assert_eq!(q.behind(t(20 * F)), None, "seeking");
        q.push(frame(0, 4));
        assert_eq!(
            q.behind(t(20 * F)),
            None,
            "decoding forward to the seek target"
        );
        for n in 1..=10 {
            q.push(frame(n * F, 4));
        }
        assert_eq!(q.behind(t(20 * F)), Some(t(10 * F)));
        assert_eq!(q.behind(t(9 * F)), Some(t(-F)), "ahead");

        // A jump ahead within reach decodes forward without a seek; until it gets there the
        // reader is not behind.
        assert!(!q.jump_ahead(t(15 * F)));
        assert_eq!(q.behind(t(30 * F)), None);
        for n in 11..=15 {
            q.push(frame(n * F, 4));
        }
        assert_eq!(q.behind(t(16 * F)), Some(t(F)));
        assert!(q.jump_ahead(t(15 * F + 2 * S)), "out of reach: a seek");
    }

    #[test]
    fn end_of_stream_right_after_a_seek_reports_an_empty_queue() {
        let mut q = decoded(100 * S, &[]);
        assert!(q.set_eos());
        q.restart();
        assert!(q.has_room(Mode::Paused));
        q.push(frame(0, 4));
        assert_eq!(q.select(t(100 * S)), Select::Stale(0));
        q.set_eos();
        assert_eq!(q.select(t(100 * S)), Select::Exact(0));
    }
}
