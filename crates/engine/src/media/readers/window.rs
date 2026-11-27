//! The GOP-reverse queue of a reader that serves a reversed clip (`clip-speed-audio.md` §5.2).
//! Pure logic, like [`super::queue::FrameQueue`], under the reader's lock.
//!
//! A decoder only runs forward, so a reverse reader decodes **windows**: for a sample time `s`
//! that no window covers it seeks to the keyframe at or before `s` and decodes forward into a
//! CPU ring until a frame lies past `s`. The ring keeps at most [`REVERSE_WINDOW_BYTES`] of
//! frames (the newest, when a long 4K GOP does not fit). Lookups then go downwards through the
//! window with the usual rule (the last frame with `pts <= s`). While the window is in use the
//! reader prepares the next earlier window ("below": it ends where the current one starts), so
//! playback crosses a GOP boundary without a stall. At most two windows exist, so a reverse
//! reader holds at most 2 x [`REVERSE_WINDOW_BYTES`].

use std::collections::VecDeque;
use std::sync::Arc;

use app_core::Time;

use super::queue::{Mode, Select};
use crate::media::frame::DecodedVideoFrame;

/// CPU memory one window of a reverse reader may hold (the forward queue's cap; 1080p NV12 is
/// 3.1 MB a frame, 4K P010 about 25 MB).
pub(crate) const REVERSE_WINDOW_BYTES: usize = super::queue::CPU_QUEUE_BYTES;

/// How far a reverse seek backs off when it landed on a keyframe at or after the window's end;
/// doubled on each retry. Container indexes (MP4 with B-frames) seek by decode timestamps, which
/// can lie a few frames before the presentation time, so "the keyframe at or before `t`" may be
/// the keyframe just after `t`.
const SEEK_BACKOFF: Time = super::queue::SEEK_BACKOFF;

/// The seek for the next earlier window starts this far before the window's end (the first
/// frame of the current window, usually a keyframe), so a seek by decode time does not land on
/// that keyframe and need a second try.
const BELOW_SEEK_LEAD: Time = Time::from_flicks(app_core::FLICKS_PER_SECOND / 4);

/// Frames decoded forward from one seek.
#[derive(Debug)]
struct Window {
    /// Exclusive end. While decoding: the frames wanted are those with `pts < end`. Once
    /// complete: the pts of the first frame after the window (`Time::MAX` at the end of the
    /// stream), so the window answers every time below it exactly.
    end: Time,
    /// Ascending pts, all below `end`.
    frames: VecDeque<Arc<DecodedVideoFrame>>,
    /// CPU bytes of `frames`.
    bytes: usize,
    complete: bool,
    /// Nothing in the stream lies before the first frame: it is the frame for any earlier time.
    bottom: bool,
    /// Where the next seek goes when it is not `end` minus one flick: after a seek that landed
    /// at or past `end`, further back.
    seek_to: Option<Time>,
    /// The first frame of the last seek, when it lay at or past `end`.
    landed: Option<Time>,
    /// Seeks retried further back so far.
    retries: u32,
}

impl Window {
    fn new(end: Time) -> Self {
        Self {
            end,
            frames: VecDeque::new(),
            bytes: 0,
            complete: false,
            bottom: false,
            seek_to: None,
            landed: None,
            retries: 0,
        }
    }

    /// The time the reader seeks to: it lands on the keyframe at or before it.
    fn seek_point(&self) -> Time {
        let last = self.end - Time::from_flicks(1);
        self.seek_to.map_or(last, |to| to.min(last))
    }

    fn low(&self) -> Option<Time> {
        self.frames.front().map(|f| f.pts)
    }

    /// Whether the window has the frame for `t`, or will have it once decoded (a window whose
    /// seek has not produced a frame yet covers everything below its end).
    fn covers(&self, t: Time) -> bool {
        if t >= self.end {
            return false;
        }
        match self.low() {
            None => !self.complete,
            Some(low) => t >= low || self.bottom,
        }
    }
}

/// Which window the reader decodes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Current,
    Below,
}

#[derive(Debug)]
pub(crate) struct WindowQueue {
    byte_cap: usize,
    /// The consumer's latest sample time.
    target: Option<Time>,
    /// The window lookups are served from.
    current: Option<Window>,
    /// The next earlier window, prepared while `current` is in use.
    below: Option<Window>,
    /// The window being decoded, until it is complete.
    filling: Option<Slot>,
    /// The reader has to seek for the window being filled and has not started yet. Frames
    /// pushed meanwhile belong to the old position and are dropped.
    seek_pending: bool,
}

impl WindowQueue {
    pub fn new(byte_cap: usize) -> Self {
        Self {
            byte_cap: byte_cap.max(1),
            target: None,
            current: None,
            below: None,
            filling: None,
            seek_pending: false,
        }
    }

    pub fn target(&self) -> Option<Time> {
        self.target
    }

    pub fn frame(&self, index: usize) -> &Arc<DecodedVideoFrame> {
        &self.current.as_ref().expect("a current window").frames[index]
    }

    fn window(&mut self, slot: Slot) -> Option<&mut Window> {
        match slot {
            Slot::Current => self.current.as_mut(),
            Slot::Below => self.below.as_mut(),
        }
    }

    /// CPU bytes held by both windows (tests of the memory bound).
    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.current.as_ref().map_or(0, |w| w.bytes) + self.below.as_ref().map_or(0, |w| w.bytes)
    }

    /// Pts of the current window (tests).
    #[cfg(test)]
    pub fn pts(&self) -> Vec<i64> {
        self.current
            .as_ref()
            .map(|w| w.frames.iter().map(|f| f.pts.flicks()).collect())
            .unwrap_or_default()
    }

    /// Starts a window for `t`: the reader seeks to the keyframe at or before `t` and decodes
    /// until a frame lies past it.
    fn start(&mut self, t: Time) {
        self.current = Some(Window::new(t + Time::from_flicks(1)));
        self.below = None;
        self.filling = Some(Slot::Current);
        self.seek_pending = true;
    }

    /// Sets the consumer's sample time. Returns true when the reader must seek (the caller then
    /// interrupts its current decode).
    pub fn retarget(&mut self, t: Time) -> bool {
        if self.target == Some(t) {
            return self.seek_pending;
        }
        self.target = Some(t);
        if self.current.as_ref().is_some_and(|w| w.covers(t)) {
            if self.filling == Some(Slot::Current) {
                self.lower_end(t);
            }
            return self.seek_pending;
        }
        let below_current = self
            .current
            .as_ref()
            .and_then(Window::low)
            .is_some_and(|low| t < low);
        if below_current && self.below.as_ref().is_some_and(|w| w.covers(t)) {
            // Playback went down into the prepared window; the old one is done with.
            self.current = self.below.take();
            if self.filling == Some(Slot::Below) {
                self.filling = Some(Slot::Current);
            }
            return self.seek_pending;
        }
        self.start(t);
        true
    }

    /// The consumer moved down inside the window being decoded: frames above `t` are not
    /// needed any more (lookups only go down from here), so the window ends after `t`. Frames
    /// already decoded past `t` complete it at once.
    fn lower_end(&mut self, t: Time) {
        let end = t + Time::from_flicks(1);
        let Some(window) = self.current.as_mut() else {
            return;
        };
        if window.complete || end >= window.end {
            return;
        }
        window.end = end;
        let mut after = None;
        while window.frames.len() > 1 && window.frames.back().is_some_and(|f| f.pts >= end) {
            if let Some(frame) = window.frames.pop_back() {
                window.bytes -= frame.data.cpu_bytes();
                after = Some(frame.pts);
            }
        }
        if let Some(after) = after {
            window.end = after;
            window.complete = true;
            self.filling = None;
        }
    }

    /// Makes the reader start the window of the current target again (after it reopened its
    /// decoder, or left keyframes-only decoding).
    pub fn force_seek(&mut self) {
        if let Some(t) = self.target {
            self.start(t);
        }
    }

    /// The seek the reader has to start, if any: the pending one, or (while playing) the next
    /// earlier window once the current one is complete. Returns the time to seek to; the reader
    /// lands on the keyframe at or before it.
    pub fn take_seek(&mut self, mode: Mode) -> Option<Time> {
        if self.seek_pending {
            self.seek_pending = false;
            let slot = self.filling?;
            let window = self.window(slot)?;
            window.frames.clear();
            window.bytes = 0;
            return Some(window.seek_point());
        }
        if self.filling.is_some() || self.below.is_some() || mode != Mode::Playing {
            return None;
        }
        let current = self.current.as_ref()?;
        if !current.complete || current.bottom {
            return None;
        }
        let low = current.low()?;
        // `low` is usually a keyframe, which a seek by decode time would land on: start the
        // seek a little earlier.
        let mut below = Window::new(low);
        below.seek_to = Some(low - BELOW_SEEK_LEAD);
        self.below = Some(below);
        self.filling = Some(Slot::Below);
        self.below.as_ref().map(Window::seek_point)
    }

    /// The reader read the stream again from its start (a seek that found no frame).
    pub fn restart(&mut self) {
        if let Some(window) = self.filling.and_then(|slot| self.window(slot)) {
            window.frames.clear();
            window.bytes = 0;
        }
    }

    /// Adds a decoded frame (pts strictly increasing since the last seek). Returns false when
    /// the frame was dropped because a seek is pending or nothing is being decoded.
    pub fn push(&mut self, frame: Arc<DecodedVideoFrame>) -> bool {
        if self.seek_pending {
            return false;
        }
        let byte_cap = self.byte_cap;
        let Some(slot) = self.filling else {
            return false;
        };
        let Some(window) = self.window(slot) else {
            return false;
        };
        let pts = frame.pts;
        if pts >= window.end {
            if window.frames.is_empty() && window.landed != Some(pts) {
                // The seek landed past the window: try again further back. Landing on the same
                // frame twice means nothing lies before it.
                window.landed = Some(pts);
                let back = Time::from_flicks(SEEK_BACKOFF.flicks() << window.retries.min(8));
                window.seek_to = Some(window.seek_point() - back);
                window.retries += 1;
                self.seek_pending = true;
                return false;
            }
            if window.frames.is_empty() {
                // No frame before the end: the stream starts here.
                window.bytes = frame.data.cpu_bytes();
                window.frames.push_back(frame);
                window.bottom = true;
                window.end = pts + Time::from_flicks(1);
            } else {
                window.end = pts;
            }
            window.complete = true;
            self.filling = None;
            return true;
        }
        window.bytes += frame.data.cpu_bytes();
        window.frames.push_back(frame);
        // The ring keeps the newest frames: the next earlier window decodes the rest again.
        while window.bytes > byte_cap && window.frames.len() > 1 {
            if let Some(old) = window.frames.pop_front() {
                window.bytes -= old.data.cpu_bytes();
            }
        }
        true
    }

    /// The decoder drained. Returns true when no frame was decoded since the last seek (the
    /// reader then reads from the start, or gives up).
    pub fn set_eos(&mut self) -> bool {
        if self.seek_pending {
            return false;
        }
        let Some(slot) = self.filling else {
            return false;
        };
        let Some(window) = self.window(slot) else {
            return false;
        };
        if window.frames.is_empty() {
            return true;
        }
        window.end = Time::from_flicks(i64::MAX);
        window.complete = true;
        self.filling = None;
        false
    }

    /// Whether the reader should decode another frame.
    pub fn has_room(&self) -> bool {
        !self.seek_pending && self.filling.is_some()
    }

    /// The frame for `t` (sample and hold) from the current window.
    pub fn select(&self, t: Time) -> Select {
        let Some(window) = &self.current else {
            return Select::Missing;
        };
        let at_or_before = window.frames.partition_point(|f| f.pts <= t);
        match at_or_before.checked_sub(1) {
            None if window.frames.is_empty() => Select::Missing,
            None if window.bottom => Select::Exact(0),
            None => Select::Stale(0),
            Some(index)
                if index + 1 < window.frames.len() || (window.complete && t < window.end) =>
            {
                Select::Exact(index)
            }
            Some(index) => Select::Stale(index),
        }
    }

    /// `Some(0)` when a window covers `t` without a new seek (for picking the best of several
    /// readers of one media).
    pub fn distance(&self, t: Time) -> Option<Time> {
        if self.seek_pending && self.current.as_ref().is_none_or(|w| !w.covers(t)) {
            return None;
        }
        let covered = self.current.as_ref().is_some_and(|w| w.covers(t))
            || self.below.as_ref().is_some_and(|w| w.covers(t));
        covered.then_some(Time::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::frame::{CpuFrame, FrameData, PlaneBuf, PlaneLayout, SampleDepth};

    const F: i64 = app_core::FLICKS_PER_SECOND / 30;

    fn t(flicks: i64) -> Time {
        Time::from_flicks(flicks)
    }

    fn frame(n: i64, bytes: usize) -> Arc<DecodedVideoFrame> {
        Arc::new(DecodedVideoFrame {
            pts: t(n * F),
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

    /// A stream with a keyframe every `gop` frames and `len` frames: what the decoder returns
    /// after a seek to `to` (from the keyframe at or before it to the end).
    fn decode_from(to: Time, gop: i64, len: i64) -> impl Iterator<Item = i64> {
        let n = to.flicks().div_euclid(F).clamp(0, len - 1);
        (n - n % gop)..len
    }

    /// Runs the reader side until it has nothing to do: seeks and decodes `gop`-frame GOPs.
    fn run(q: &mut WindowQueue, mode: Mode, gop: i64, len: i64, bytes: usize) -> u32 {
        let mut seeks = 0;
        while let Some(to) = q.take_seek(mode) {
            seeks += 1;
            let mut frames = decode_from(to, gop, len);
            loop {
                if !q.has_room() {
                    break;
                }
                match frames.next() {
                    Some(n) => {
                        q.push(frame(n, bytes));
                    }
                    None => {
                        q.set_eos();
                        break;
                    }
                }
            }
        }
        seeks
    }

    fn exact_frame(q: &WindowQueue, at: Time) -> i64 {
        match q.select(at) {
            Select::Exact(index) => q.frame(index).pts.flicks() / F,
            other => panic!("not exact at {at:?}: {other:?}"),
        }
    }

    #[test]
    fn a_window_ends_after_the_target_and_starts_at_its_keyframe() {
        let mut q = WindowQueue::new(REVERSE_WINDOW_BYTES);
        assert!(q.retarget(t(25 * F + F / 2)), "a new window seeks");
        assert_eq!(q.take_seek(Mode::Paused), Some(t(25 * F + F / 2)));
        for n in decode_from(t(25 * F), 10, 60) {
            if !q.has_room() {
                break;
            }
            q.push(frame(n, 4));
        }
        assert_eq!(q.pts(), (20..=25).map(|n| n * F).collect::<Vec<_>>());
        assert_eq!(exact_frame(&q, t(25 * F + F / 2)), 25);
        // Down through the window without a seek.
        for n in (20..25).rev() {
            assert!(!q.retarget(t(n * F + F / 2)));
            assert_eq!(exact_frame(&q, t(n * F + F / 2)), n);
        }
        // Paused: nothing below is prepared.
        assert_eq!(q.take_seek(Mode::Paused), None);
    }

    #[test]
    fn playing_prepares_the_next_earlier_window() {
        let mut q = WindowQueue::new(REVERSE_WINDOW_BYTES);
        q.retarget(t(25 * F));
        assert_eq!(run(&mut q, Mode::Playing, 10, 60, 4), 2, "current + below");
        assert_eq!(q.take_seek(Mode::Playing), None, "two windows at most");
        // Crossing the GOP boundary at 20 F: the prepared window takes over without a seek.
        assert!(!q.retarget(t(19 * F)));
        assert_eq!(exact_frame(&q, t(19 * F)), 19);
        assert_eq!(q.pts(), (10..20).map(|n| n * F).collect::<Vec<_>>());
        assert_eq!(
            run(&mut q, Mode::Playing, 10, 60, 4),
            1,
            "the next one below"
        );
        for n in (0..19).rev() {
            assert!(!q.retarget(t(n * F)), "frame {n}");
            assert_eq!(exact_frame(&q, t(n * F)), n);
            run(&mut q, Mode::Playing, 10, 60, 4);
        }
        // Below the first frame the reader looked once whether the stream starts earlier: it
        // does not, so frame 0 answers every earlier time.
        assert!(!q.retarget(t(-F)));
        assert_eq!(exact_frame(&q, t(-F)), 0, "the stream's first frame");
        assert_eq!(q.take_seek(Mode::Playing), None, "nothing below the start");
    }

    #[test]
    fn the_byte_cap_keeps_the_newest_frames_and_the_next_window_decodes_the_rest() {
        // A 10-frame GOP, room for 3 frames per window.
        let mut q = WindowQueue::new(12);
        q.retarget(t(9 * F));
        let mut seeks = run(&mut q, Mode::Playing, 10, 60, 4);
        assert_eq!(q.pts(), vec![7 * F, 8 * F, 9 * F]);
        assert!(q.bytes() <= 2 * 12, "at most two windows");
        let mut shown = Vec::new();
        for n in (0..=9).rev() {
            let at = t(n * F + F / 2);
            assert!(!q.retarget(at), "frame {n} needs no seek of its own");
            shown.push(exact_frame(&q, at));
            seeks += run(&mut q, Mode::Playing, 10, 60, 4);
            assert!(q.bytes() <= 2 * 12);
        }
        assert_eq!(shown, (0..=9).rev().collect::<Vec<_>>());
        // Windows 7..=9, 4..=6, 1..=3, 0, and the look below the start (two seeks that both
        // land on frame 0).
        assert_eq!(seeks, 6);
    }

    #[test]
    fn moving_down_while_a_window_decodes_ends_it_early() {
        let mut q = WindowQueue::new(REVERSE_WINDOW_BYTES);
        q.retarget(t(29 * F));
        q.take_seek(Mode::Playing);
        for n in 20..=24 {
            q.push(frame(n, 4));
        }
        // Playback moved on to 22 while the reader decoded up to 24: the window is complete.
        assert!(!q.retarget(t(22 * F)));
        assert!(!q.has_room());
        assert_eq!(exact_frame(&q, t(22 * F)), 22);
        assert_eq!(q.pts(), vec![20 * F, 21 * F, 22 * F]);
    }

    #[test]
    fn a_jump_or_a_move_up_starts_a_new_window() {
        let mut q = WindowQueue::new(REVERSE_WINDOW_BYTES);
        q.retarget(t(25 * F));
        run(&mut q, Mode::Paused, 10, 60, 4);
        assert!(q.retarget(t(26 * F)), "above the window");
        assert!(q.retarget(t(5 * F)), "far below, nothing prepared");
        assert_eq!(run(&mut q, Mode::Paused, 10, 60, 4), 1, "one seek, to 5 F");
        assert_eq!(exact_frame(&q, t(5 * F)), 5);
    }

    #[test]
    fn frames_of_an_abandoned_position_are_dropped() {
        let mut q = WindowQueue::new(REVERSE_WINDOW_BYTES);
        q.retarget(t(25 * F));
        q.take_seek(Mode::Paused);
        q.push(frame(20, 4));
        assert!(q.retarget(t(45 * F)));
        assert!(!q.push(frame(21, 4)), "a seek is pending");
        assert_eq!(q.take_seek(Mode::Paused), Some(t(45 * F)));
        assert!(q.pts().is_empty());
        assert_eq!(q.select(t(45 * F)), Select::Missing);
    }

    #[test]
    fn a_seek_that_lands_past_the_window_backs_off() {
        let mut q = WindowQueue::new(REVERSE_WINDOW_BYTES);
        q.retarget(t(56 * F));
        assert_eq!(q.take_seek(Mode::Paused), Some(t(56 * F)));
        // The container index found the keyframe at 57 (its decode time is before 56).
        assert!(!q.push(frame(57, 4)));
        let back = q.take_seek(Mode::Paused).expect("a second seek");
        assert_eq!(back, t(56 * F) - SEEK_BACKOFF);
        for n in decode_from(back, 10, 60) {
            if !q.has_room() {
                break;
            }
            q.push(frame(n, 4));
        }
        assert_eq!(exact_frame(&q, t(56 * F)), 56);
    }

    #[test]
    fn the_last_frame_of_the_stream_holds() {
        let mut q = WindowQueue::new(REVERSE_WINDOW_BYTES);
        q.retarget(t(100 * F));
        run(&mut q, Mode::Paused, 10, 60, 4);
        assert_eq!(exact_frame(&q, t(100 * F)), 59);
        assert!(!q.retarget(t(80 * F)), "still the same window");
        assert_eq!(exact_frame(&q, t(80 * F)), 59);
    }
}
