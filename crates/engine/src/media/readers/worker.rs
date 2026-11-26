//! One video reader: a decode thread with its own `AVFormatContext` and decoder for one video
//! stream (ADR-0004 amendment), steered by "decode from time `s`" requests (render-graph.md
//! §5.3), and the handle the pool keeps for it.
//!
//! The consumer (render thread) and the reader share a [`FrameQueue`] under one mutex. The
//! consumer sets the target with every lookup; the reader seeks when the queue says so, decodes
//! forward into the queue and wakes the consumer (`FrameReady`, the condition variable for
//! `Exact` lookups). Neither side holds the lock while decoding or converting.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use app_core::Time;
use app_core::media::MediaId;
use rsmpeg::avutil::AVFrame;

use super::decode_path::{DecodePath, PathMemory};
use super::queue::{CATCH_UP_LEAD, Discard, FAR_BEHIND, FrameQueue, Mode, Select};
use super::{EXACT_TIMEOUT, Notifier, placeholder_for};
use crate::media::decode::open_decoder_with_threads;
use crate::media::error::{MediaError, MediaErrorKind};
use crate::media::frame::{CpuFrameConverter, DecodedVideoFrame, FrameData};
use crate::media::reader::VideoReader;
use crate::render::frames::{FetchPolicy, FrameId, FrameLookup, FrameRef, PlaceholderKind};

#[cfg(windows)]
use super::decode_path::HwSlot;
#[cfg(windows)]
use crate::media::d3d12::{self, FrameWrapper, HwDevice};
#[cfg(windows)]
use crate::render::GpuContext;

/// Decode failures within [`FAILURE_WINDOW`] that stop a reader (render-graph.md §10).
const MAX_FAILURES: usize = 3;
const FAILURE_WINDOW: Duration = Duration::from_secs(1);

/// What a reader decodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReaderSource {
    pub media: MediaId,
    /// [`FrameId::source`] of the frames.
    pub source: u64,
    pub path: PathBuf,
    pub stream: u32,
    pub origin: Time,
    /// Duration of one source frame (from the average frame rate; 1/30 s when unknown), for the
    /// catch-up rule.
    pub frame: Time,
}

/// The hardware decoder a reader may open.
#[cfg(windows)]
#[derive(Debug)]
pub(crate) struct HwContext {
    pub device: Arc<HwDevice>,
    pub gpu: Arc<GpuContext>,
    pub slot: HwSlot,
}

/// Everything a reader thread starts with.
#[derive(Debug)]
pub(crate) struct ReaderConfig {
    pub source: ReaderSource,
    /// The path to try first; `Software` unless `hw` is set.
    pub path: DecodePath,
    #[cfg(windows)]
    pub hw: Option<HwContext>,
    pub software_threads: i32,
    pub depth: usize,
    pub byte_cap: usize,
    pub memory: Arc<PathMemory>,
    pub notifier: Arc<Notifier>,
    pub below_normal_priority: bool,
    /// Sleeps this long after every decoded frame: a slow decoder for catch-up tests.
    #[cfg(test)]
    pub decode_delay: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Starting,
    Running,
    Failed(PlaceholderKind, Instant),
}

#[derive(Debug)]
struct State {
    queue: FrameQueue,
    mode: Mode,
    status: Status,
    path: DecodePath,
    /// The sample time of a lookup that did not get its exact frame: the reader wakes the
    /// consumer when a frame for it arrives.
    wanted: Option<Time>,
    /// Which frames the decoder skips (shuttle, catch-up).
    discard: Discard,
    /// After a catch-up jump: lookups before this time keep the reader's target and get its
    /// newest frame.
    hold_until: Option<Time>,
    /// Seeks started (diagnostics, tests).
    seeks: u32,
    /// Catch-up jumps (diagnostics, tests).
    catch_ups: u32,
    shutdown: bool,
    /// Every pts pushed, for tests of progressive scrub.
    #[cfg(test)]
    published: Vec<Time>,
}

impl State {
    /// Whether the consumer waits for what just changed; clears the wait once it is exact.
    fn wake_for(&mut self, pts: Option<Time>) -> bool {
        let Some(wanted) = self.wanted else {
            return false;
        };
        if matches!(self.queue.select(wanted), Select::Exact(_)) {
            self.wanted = None;
            return true;
        }
        pts.is_some_and(|pts| pts <= wanted)
    }
}

#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
    /// Signalled on every change, for both sides.
    cond: Condvar,
    /// Interrupts the reader's current decode (a seek is pending, or shutdown).
    abandon: AtomicBool,
    notifier: Arc<Notifier>,
}

enum Work {
    Seek(Time),
    Decode(Discard),
    Exit,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_work(&self) -> Work {
        let mut state = self.lock();
        loop {
            if state.shutdown {
                return Work::Exit;
            }
            if let Some(t) = state.queue.take_seek() {
                self.abandon.store(false, Ordering::SeqCst);
                state.seeks += 1;
                return Work::Seek(t);
            }
            if state.queue.has_room(state.mode) {
                return Work::Decode(state.discard);
            }
            state = self
                .cond
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn publish(&self, frame: DecodedVideoFrame) {
        let pts = frame.pts;
        let wake = {
            let mut state = self.lock();
            if !state.queue.push(Arc::new(frame)) {
                return;
            }
            #[cfg(test)]
            state.published.push(pts);
            state.wake_for(Some(pts))
        };
        self.cond.notify_all();
        if wake {
            self.notifier.frame_ready();
        }
    }

    /// Records the end of the stream; true when nothing was decoded since the last seek.
    fn end_of_stream(&self) -> bool {
        let (empty, wake) = {
            let mut state = self.lock();
            let empty = state.queue.set_eos();
            (empty, state.wake_for(None))
        };
        self.cond.notify_all();
        if wake {
            self.notifier.frame_ready();
        }
        empty
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.lock());
        self.cond.notify_all();
    }

    fn shutting_down(&self) -> bool {
        self.lock().shutdown
    }

    /// Runs `open` until it is not interrupted by a seek request: the pending seek is taken
    /// once the decoder is open. `None` on shutdown.
    fn open_uninterrupted<T>(
        &self,
        mut open: impl FnMut() -> Result<T, MediaError>,
    ) -> Option<Result<T, MediaError>> {
        loop {
            match open() {
                Err(err) if err.kind == MediaErrorKind::Cancelled => {
                    if self.shutting_down() {
                        return None;
                    }
                    self.abandon.store(false, Ordering::SeqCst);
                }
                other => return Some(other),
            }
        }
    }

    fn fail(&self, kind: PlaceholderKind) {
        let wake = {
            let mut state = self.lock();
            state.status = Status::Failed(kind, Instant::now());
            state.wanted.take().is_some()
        };
        self.cond.notify_all();
        if wake {
            self.notifier.frame_ready();
        }
    }
}

/// The pool's handle of one reader thread. Dropping it stops and joins the thread.
#[derive(Debug)]
pub(crate) struct Reader {
    source: ReaderSource,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    /// The frame this reader last handed out: the stale answer while a seek has nothing yet.
    last_served: Option<FrameRef>,
    pub last_used: Instant,
    /// The pool's frame counter at the last lookup.
    pub used_in_frame: u64,
    /// The sample time the playback scheduler primed this reader for (a clip that starts
    /// soon), until a lookup uses it.
    pub primed: Option<Time>,
    /// The pool's frame counter when it was primed.
    pub primed_in_frame: u64,
}

/// How a lookup steers its reader (playback.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Steering {
    pub mode: Mode,
    /// Shuttle above 2x: decode keyframes only.
    pub keyframes_only: bool,
    /// Playback at 1x or 2x: discard non-reference frames when behind, jump ahead when far
    /// behind.
    pub catch_up: bool,
}

impl Reader {
    pub fn spawn(config: ReaderConfig) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                queue: FrameQueue::new(config.depth, config.byte_cap),
                mode: Mode::Paused,
                status: Status::Starting,
                path: config.path,
                wanted: None,
                discard: Discard::Nothing,
                hold_until: None,
                seeks: 0,
                catch_ups: 0,
                shutdown: false,
                #[cfg(test)]
                published: Vec::new(),
            }),
            cond: Condvar::new(),
            abandon: AtomicBool::new(false),
            notifier: Arc::clone(&config.notifier),
        });
        let source = config.source.clone();
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("video-reader".into())
            .spawn(move || run(&thread_shared, config))?;
        Ok(Self {
            source,
            shared,
            thread: Some(thread),
            last_served: None,
            last_used: Instant::now(),
            used_in_frame: 0,
            primed: None,
            primed_in_frame: 0,
        })
    }

    /// Seeks started so far.
    pub fn seeks(&self) -> u32 {
        self.shared.lock().seeks
    }

    /// Catch-up jumps so far.
    pub fn catch_ups(&self) -> u32 {
        self.shared.lock().catch_ups
    }

    #[cfg(test)]
    pub fn discard(&self) -> Discard {
        self.shared.lock().discard
    }

    /// The playback scheduler: decode towards `t` now (a clip that starts soon), so the frame
    /// is ready when the clip starts.
    pub fn prime(&mut self, t: Time, steering: Steering, frame: u64) {
        let shared = Arc::clone(&self.shared);
        let mut state = shared.lock();
        state.mode = steering.mode;
        state.discard = base_discard(steering, state.discard);
        state.hold_until = None;
        if state.queue.retarget(t) {
            interrupt(&state, &shared.abandon);
        }
        drop(state);
        shared.cond.notify_all();
        self.primed = Some(t);
        self.primed_in_frame = frame;
        self.last_used = Instant::now();
    }

    pub fn serves(&self, media: MediaId, source: u64) -> bool {
        self.source.media == media && self.source.source == source
    }

    pub fn media(&self) -> MediaId {
        self.source.media
    }

    pub fn source(&self) -> u64 {
        self.source.source
    }

    pub fn target(&self) -> Option<Time> {
        self.shared.lock().queue.target()
    }

    /// How far `t` is past what the reader has, when it is reached without a seek.
    pub fn distance(&self, t: Time) -> Option<Time> {
        let state = self.shared.lock();
        match state.status {
            Status::Failed(..) => None,
            _ => state.queue.distance(t),
        }
    }

    /// When the reader failed and why.
    pub fn failed(&self) -> Option<(PlaceholderKind, Instant)> {
        match self.shared.lock().status {
            Status::Failed(kind, at) => Some((kind, at)),
            _ => None,
        }
    }

    pub fn decode_path(&self) -> DecodePath {
        self.shared.lock().path
    }

    fn frame_ref(&self, frame: &Arc<DecodedVideoFrame>) -> FrameRef {
        FrameRef::new(
            FrameId {
                media: self.source.media,
                source: self.source.source,
                pts: Some(frame.pts),
            },
            Arc::clone(frame),
        )
    }

    /// The frame for `t`: retargets the reader and answers by `policy`.
    pub fn lookup(&mut self, t: Time, steering: Steering, policy: FetchPolicy) -> FrameLookup {
        let shared = Arc::clone(&self.shared);
        let mut state = shared.lock();
        self.primed = None;
        state.mode = steering.mode;
        state.discard = base_discard(steering, state.discard);
        let hold = steering.catch_up
            && policy == FetchPolicy::BestAvailable
            && self.catch_up(&mut state, t);
        if !hold && state.queue.retarget(t) {
            interrupt(&state, &shared.abandon);
        }
        shared.cond.notify_all();
        let deadline = Instant::now() + EXACT_TIMEOUT;
        loop {
            if let Status::Failed(kind, _) = state.status {
                state.wanted = None;
                return FrameLookup::Failed(kind);
            }
            let select = state.queue.select(t);
            if let Select::Exact(index) = select {
                state.wanted = None;
                let frame = self.frame_ref(state.queue.frame(index));
                self.last_served = Some(frame.clone());
                return FrameLookup::Exact(frame);
            }
            if policy == FetchPolicy::BestAvailable {
                // Re-arm before the wait is visible to the reader (both under the lock), so
                // the frame it publishes next always sends `FrameReady`.
                shared.notifier.rearm();
                state.wanted = Some(t);
                let stale = match select {
                    Select::Stale(index) => Some(self.frame_ref(state.queue.frame(index))),
                    _ => None,
                };
                drop(state);
                return match stale {
                    Some(frame) => {
                        self.last_served = Some(frame.clone());
                        FrameLookup::Stale(frame)
                    }
                    None => self
                        .last_served
                        .clone()
                        .map_or(FrameLookup::Missing, FrameLookup::Stale),
                };
            }
            state.wanted = Some(t);
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                tracing::warn!(media = %self.source.media, ?t, "timed out waiting for a video frame");
                state.wanted = None;
                return FrameLookup::Failed(PlaceholderKind::DecodeError);
            }
            state = shared
                .cond
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The catch-up rules of playback.md §7 for a lookup at `t` during playback. Returns true
    /// when the reader keeps its target (it jumped ahead of `t`) and the lookup only reads.
    fn catch_up(&self, state: &mut State, t: Time) -> bool {
        if let Some(until) = state.hold_until {
            if t < until && !matches!(state.status, Status::Failed(..)) {
                return true;
            }
            state.hold_until = None;
        }
        match state.queue.behind(t) {
            Some(behind) if behind > FAR_BEHIND => {
                let ahead = t + CATCH_UP_LEAD;
                tracing::debug!(media = %self.source.media, ?behind, "video reader far behind; jumping ahead");
                if state.queue.jump_ahead(ahead) {
                    interrupt(state, &self.shared.abandon);
                }
                state.hold_until = Some(ahead);
                state.catch_ups += 1;
                state.discard = Discard::NonRef;
                true
            }
            Some(behind) if behind > Time(2 * self.source.frame.flicks()) => {
                state.discard = Discard::NonRef;
                false
            }
            Some(behind) if behind <= Time::ZERO && state.discard == Discard::NonRef => {
                state.discard = Discard::Nothing;
                false
            }
            _ => false,
        }
    }

    /// Every pts the reader published since it started (tests).
    #[cfg(test)]
    pub fn published(&self) -> Vec<Time> {
        self.shared.lock().published.clone()
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.shared.update(|state| state.shutdown = true);
        self.shared.abandon.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

/// Decode failures in the last [`FAILURE_WINDOW`].
#[derive(Debug, Default)]
pub(crate) struct Failures(VecDeque<Instant>);

impl Failures {
    /// Records a failure at `now`; true when the reader must stop.
    pub fn record(&mut self, now: Instant) -> bool {
        self.0.push_back(now);
        while self
            .0
            .front()
            .is_some_and(|&at| now.duration_since(at) >= FAILURE_WINDOW)
        {
            self.0.pop_front();
        }
        self.0.len() >= MAX_FAILURES
    }
}

/// Why a decode step failed.
enum Failure {
    /// The reader was interrupted (a newer seek, or shutdown).
    Cancelled,
    /// The hardware path failed: reopen in software.
    Hardware(MediaError),
    Decode(MediaError),
}

/// A reader's open decoder and how it turns `AVFrame`s into [`FrameData`].
struct Decoder<'c> {
    reader: VideoReader<'c>,
    path: DecodePath,
    converter: CpuFrameConverter,
    #[cfg(windows)]
    wrapper: Option<FrameWrapper>,
    /// Held while this decoder uses hardware.
    #[cfg(windows)]
    _slot: Option<HwSlot>,
}

impl<'c> Decoder<'c> {
    /// Opens the configured path; a hardware decoder that cannot open falls back to software.
    fn open(config: &mut ReaderConfig, abandon: &'c AtomicBool) -> Result<Self, MediaError> {
        #[cfg(windows)]
        if let Some(hw) = config.hw.take() {
            let source = &config.source;
            match VideoReader::open_with(
                &source.path,
                source.stream,
                source.origin,
                abandon,
                |stream| d3d12::open_hw_decoder(stream, &hw.device),
            ) {
                Ok(reader) => {
                    let wrapper = (config.path == DecodePath::ZeroCopy)
                        .then(|| FrameWrapper::new(&hw.gpu))
                        .and_then(|w| {
                            w.inspect_err(|err| tracing::warn!(%err, "no zero-copy frame wrapper"))
                                .ok()
                        })
                        .filter(FrameWrapper::zero_copy);
                    let path = if wrapper.is_some() {
                        DecodePath::ZeroCopy
                    } else {
                        DecodePath::Download
                    };
                    return Ok(Self {
                        reader,
                        path,
                        converter: CpuFrameConverter::new(),
                        wrapper,
                        _slot: Some(hw.slot),
                    });
                }
                Err(err) if err.kind == MediaErrorKind::Cancelled => {
                    // Interrupted by a seek request: try the same path again.
                    config.hw = Some(hw);
                    return Err(err);
                }
                Err(err) if placeholder_for(&err) == PlaceholderKind::Offline => return Err(err),
                Err(err) => {
                    if config.memory.fall_back(source.media, DecodePath::Software) {
                        tracing::info!(media = %source.media, %err, "no hardware decoder for this stream; decoding in software");
                    }
                }
            }
        }
        Self::open_software(config, abandon)
    }

    fn open_software(config: &ReaderConfig, abandon: &'c AtomicBool) -> Result<Self, MediaError> {
        let source = &config.source;
        let threads = config.software_threads;
        let reader = VideoReader::open_with(
            &source.path,
            source.stream,
            source.origin,
            abandon,
            |stream| open_decoder_with_threads(stream, threads),
        )?;
        Ok(Self {
            reader,
            path: DecodePath::Software,
            converter: CpuFrameConverter::new(),
            #[cfg(windows)]
            wrapper: None,
            #[cfg(windows)]
            _slot: None,
        })
    }

    fn classify(&self, err: MediaError) -> Failure {
        if err.kind == MediaErrorKind::Cancelled {
            Failure::Cancelled
        } else if self.path == DecodePath::Software {
            Failure::Decode(err)
        } else {
            Failure::Hardware(err)
        }
    }

    /// The next frame, or `None` at the end of the stream.
    fn next(
        &mut self,
        source: &ReaderSource,
        memory: &PathMemory,
    ) -> Result<Option<(Option<Time>, FrameData)>, Failure> {
        match self.reader.next_frame() {
            Ok(None) => Ok(None),
            Ok(Some((pts, frame))) => self
                .frame_data(frame, source, memory)
                .map(|data| Some((pts, data))),
            Err(err) => Err(self.classify(err)),
        }
    }

    #[cfg_attr(not(windows), allow(unused_variables, unused_mut))]
    fn frame_data(
        &mut self,
        mut frame: AVFrame,
        source: &ReaderSource,
        memory: &PathMemory,
    ) -> Result<FrameData, Failure> {
        #[cfg(windows)]
        if d3d12::is_hw_frame(&frame) {
            if let Some(wrapper) = &self.wrapper {
                match wrapper.wrap(frame) {
                    Ok(gpu) => return Ok(FrameData::Gpu(gpu)),
                    Err((back, err)) => {
                        frame = back;
                        self.wrapper = None;
                        self.path = DecodePath::Download;
                        if memory.fall_back(source.media, DecodePath::Download) {
                            tracing::warn!(media = %source.media, %err, "zero-copy wrap failed; downloading hardware frames");
                        }
                    }
                }
            }
            let cpu = d3d12::download(&frame).map_err(Failure::Hardware)?;
            return self
                .converter
                .convert(&cpu)
                .map(FrameData::Cpu)
                .map_err(Failure::Hardware);
        }
        self.converter
            .convert(&frame)
            .map(FrameData::Cpu)
            .map_err(|err| self.classify(err))
    }

    /// Whether the decoder has negotiated hardware surfaces (for the fallback log).
    fn hw_active(&self) -> bool {
        #[cfg(windows)]
        if self.path != DecodePath::Software {
            return d3d12::hw_active(self.reader.decoder());
        }
        false
    }
}

/// Sets a failed status if the thread ends without one (a panic in FFmpeg glue, say), so `Exact`
/// lookups never wait for a dead reader.
struct ExitGuard<'a>(&'a Shared);

impl Drop for ExitGuard<'_> {
    fn drop(&mut self) {
        let state = self.0.lock();
        let unfinished = !state.shutdown && !matches!(state.status, Status::Failed(..));
        drop(state);
        if unfinished {
            self.0.fail(PlaceholderKind::DecodeError);
        }
    }
}

/// Media times strictly increase within a run: frames without a timestamp follow the previous
/// one, and out-of-order timestamps (broken files) are moved just after it.
fn monotonic(pts: Option<Time>, last: Option<Time>) -> Time {
    let next = last.map(|last| last + Time::from_flicks(1));
    match (pts, next) {
        (Some(pts), Some(next)) => pts.max(next),
        (Some(pts), None) => pts,
        (None, next) => next.unwrap_or(Time::ZERO),
    }
}

/// Interrupts the reader's current decode for a pending seek. Not while it opens its file and
/// decoder: the seek is taken after the open anyway, and an open interrupted while FFmpeg
/// probes the stream can fail as "invalid data" instead of "cancelled", which would wrongly
/// send the media to software decode for the rest of the session.
fn interrupt(state: &State, abandon: &AtomicBool) {
    if state.status == Status::Running {
        abandon.store(true, Ordering::SeqCst);
    }
}

/// The discard a lookup or prime asks for: keyframes only for fast shuttle; a catch-up
/// `NonRef` stays until the catch-up rule clears it; otherwise every frame.
fn base_discard(steering: Steering, current: Discard) -> Discard {
    match (steering.keyframes_only, current) {
        (true, _) => Discard::NonKey,
        (false, Discard::NonRef) if steering.catch_up => Discard::NonRef,
        _ => Discard::Nothing,
    }
}

fn skip_frame(discard: Discard) -> rsmpeg::ffi::AVDiscard {
    match discard {
        Discard::Nothing => rsmpeg::ffi::AVDISCARD_DEFAULT,
        Discard::NonRef => rsmpeg::ffi::AVDISCARD_NONREF,
        Discard::NonKey => rsmpeg::ffi::AVDISCARD_NONKEY,
    }
}

fn run(shared: &Shared, mut config: ReaderConfig) {
    let _guard = ExitGuard(shared);
    if config.below_normal_priority {
        super::lower_thread_priority();
    }
    let source = config.source.clone();
    let memory = Arc::clone(&config.memory);
    let opened = shared.open_uninterrupted(|| Decoder::open(&mut config, &shared.abandon));
    let mut decoder = match opened {
        None => return,
        Some(Ok(decoder)) => decoder,
        Some(Err(err)) => {
            let kind = placeholder_for(&err);
            match kind {
                PlaceholderKind::Offline => shared.notifier.media_unavailable(source.media, &err),
                PlaceholderKind::DecodeError => shared.notifier.decode_failed(source.media, &err),
            }
            shared.fail(kind);
            return;
        }
    };
    let path = decoder.path;
    shared.update(|state| {
        state.status = Status::Running;
        state.path = path;
    });
    tracing::debug!(media = %source.media, ?path, "video reader open");

    let mut failures = Failures::default();
    let mut last_pts: Option<Time> = None;
    let mut restarted = false;
    let mut applied = Discard::Nothing;
    loop {
        let failure = match shared.next_work() {
            Work::Exit => return,
            Work::Seek(t) => {
                last_pts = None;
                restarted = false;
                match decoder.reader.seek(t) {
                    Ok(()) => continue,
                    Err(err) => decoder.classify(err),
                }
            }
            Work::Decode(discard) if discard != applied => {
                decoder.reader.set_skip_frame(skip_frame(discard));
                let left_keyframes = applied == Discard::NonKey;
                applied = discard;
                if left_keyframes {
                    // The frames after the last keyframe miss their references: start again
                    // from a keyframe at the target.
                    shared.update(|state| state.queue.force_seek());
                }
                continue;
            }
            Work::Decode(_) => match decoder.next(&source, &memory) {
                Ok(Some((pts, data))) => {
                    let pts = monotonic(pts, last_pts);
                    last_pts = Some(pts);
                    shared.publish(DecodedVideoFrame { pts, data });
                    #[cfg(test)]
                    if !config.decode_delay.is_zero() {
                        std::thread::sleep(config.decode_delay);
                    }
                    continue;
                }
                Ok(None) => {
                    if !shared.end_of_stream() {
                        continue;
                    }
                    // The seek found no frame (an imprecise index): read from the start once.
                    if restarted {
                        Failure::Decode(MediaError::new(
                            MediaErrorKind::Damaged,
                            "the video stream has no frames",
                        ))
                    } else {
                        restarted = true;
                        last_pts = None;
                        match decoder.reader.rewind() {
                            Ok(()) => {
                                shared.update(|state| state.queue.restart());
                                continue;
                            }
                            Err(err) => decoder.classify(err),
                        }
                    }
                }
                Err(failure) => failure,
            },
        };
        match failure {
            Failure::Cancelled => {}
            Failure::Hardware(err) => {
                if memory.fall_back(source.media, DecodePath::Software) {
                    tracing::warn!(
                        media = %source.media, %err, hw_active = decoder.hw_active(),
                        "hardware decode failed; reopening in software"
                    );
                }
                drop(decoder);
                let opened =
                    shared.open_uninterrupted(|| Decoder::open_software(&config, &shared.abandon));
                decoder = match opened {
                    None => return,
                    Some(Ok(decoder)) => decoder,
                    Some(Err(err)) => {
                        let kind = placeholder_for(&err);
                        if kind == PlaceholderKind::Offline {
                            shared.notifier.media_unavailable(source.media, &err);
                        }
                        shared.fail(kind);
                        return;
                    }
                };
                last_pts = None;
                applied = Discard::Nothing;
                shared.update(|state| {
                    state.path = DecodePath::Software;
                    state.queue.force_seek();
                });
            }
            Failure::Decode(err) => {
                tracing::debug!(media = %source.media, %err, "video decode error");
                if failures.record(Instant::now()) {
                    shared.notifier.decode_failed(source.media, &err);
                    shared.fail(PlaceholderKind::DecodeError);
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_failures_within_one_second_stop_the_reader() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut f = Failures::default();
        assert!(!f.record(at(0)));
        assert!(!f.record(at(500)));
        assert!(f.record(at(999)), "three within 1 s");

        let mut f = Failures::default();
        assert!(!f.record(at(0)));
        assert!(!f.record(at(600)));
        assert!(!f.record(at(1000)), "the first one has aged out");
        assert!(f.record(at(1100)));
    }

    #[test]
    fn timestamps_increase_strictly() {
        let t = Time::from_flicks;
        assert_eq!(monotonic(Some(t(10)), None), t(10));
        assert_eq!(monotonic(None, None), t(0));
        assert_eq!(monotonic(None, Some(t(10))), t(11));
        assert_eq!(monotonic(Some(t(5)), Some(t(10))), t(11));
        assert_eq!(monotonic(Some(t(20)), Some(t(10))), t(20));
    }
}
