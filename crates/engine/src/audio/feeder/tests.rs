//! Clock and feeder simulation (playback.md §12): the block maker, the ring and the callback
//! driven by a fake device in virtual time, with latency, callback jitter and late feeder
//! wake-ups. No device, no thread, no file.

use app_core::{FLICKS_PER_SECOND, Project, Time};
use arc_swap::ArcSwap;

use super::*;
use crate::audio::mix::tests::{Constant, audio_asset, audio_clip};
use crate::audio::mix::{PcmProvider, Voice};
use crate::playback::clock::{AudioClock, ClockCell, ClockReading};

const SR: u32 = 48_000;
const MS: u64 = 1_000_000;
const SECOND: i64 = FLICKS_PER_SECOND;
/// One device sample at 48 kHz.
const SAMPLE: i64 = 14_700;

/// A project with one audio clip of `seconds` from time 0.
fn project(seconds: i64) -> Project {
    let asset = audio_asset(seconds + 1);
    let clip = audio_clip(
        asset.id,
        Time::ZERO,
        Time::from_seconds(seconds),
        Time::ZERO,
    );
    let mut project = Project::new("sim");
    project.media.push(asset);
    project.tracks[1].clips.push(clip);
    project
}

/// A fake 48 kHz device: callbacks of `buffer` frames, each called `latency` (plus up to 3 ms
/// of jitter) before its first frame reaches the DAC; the feeder wakes every 2 ms, every 50th
/// time 16 ms late.
struct Sim {
    transport: Arc<TransportShared>,
    cell: Arc<ClockCell>,
    shared: Arc<AudioShared>,
    clock: AudioClock,
    producer: BlockProducer,
    callback: OutputCallback,
    maker: BlockMaker,
    epochs: u32,
    pushed: usize,
    project: Project,
    looping: bool,
    now: u64,
    next_callback: u64,
    next_feed: u64,
    feeds: u64,
    callbacks: u64,
    played: u64,
    dac0: u64,
    buffer: usize,
    latency: u64,
    seed: u64,
    out: Vec<f32>,
    /// `(epoch, time, end, frames, last)` of every block pushed.
    blocks: Vec<(u32, Time, Time, u16, bool)>,
    feeding: bool,
}

impl Sim {
    fn new(project: Project, from: Time, rate: i8, looping: bool) -> Self {
        let (producer, consumer) = ring();
        let transport = Arc::new(TransportShared::new());
        let cell = Arc::new(ClockCell::new());
        let shared = Arc::new(AudioShared::new());
        let seek_gen = transport.seek(from);
        transport.set_looping(looping);
        let session = cell.begin_session();
        let callback = OutputCallback::new(
            consumer,
            Arc::clone(&transport),
            Arc::clone(&cell),
            Arc::clone(&shared),
            2,
            SR,
        );
        let clock = AudioClock::new(Arc::clone(&cell), Arc::clone(&transport));
        let buffer = 480;
        let latency = 15 * MS;
        let dac0 = 1_000 * MS;
        Self {
            transport,
            cell,
            shared,
            clock,
            producer,
            callback,
            maker: BlockMaker::new(session, seek_gen, 1, rate, from, SR),
            epochs: 1,
            pushed: 0,
            project,
            looping,
            now: dac0 - latency - 50 * MS,
            next_callback: dac0 - latency,
            next_feed: dac0 - latency - 50 * MS,
            feeds: 0,
            callbacks: 0,
            played: 0,
            dac0,
            buffer,
            latency,
            seed: 0x2026_1126,
            out: vec![0.0; 2 * buffer],
            blocks: Vec::new(),
            feeding: true,
        }
    }

    fn dac(&self, frame: u64) -> u64 {
        self.dac0 + frame * 1_000_000_000 / u64::from(SR)
    }

    fn jitter(&mut self) -> u64 {
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.seed >> 33) % (3 * MS)
    }

    fn ready(&self) -> bool {
        self.pushed >= TARGET_FILL_BLOCKS || self.blocks.last().is_some_and(|b| b.4)
    }

    fn feed(&mut self, sources: &mut dyn PcmProvider) {
        let looping = self.looping;
        while RING_BLOCKS - self.producer.slots() < TARGET_FILL_BLOCKS {
            let Some(block) = self
                .maker
                .next(&self.project, looping, sources, &mut self.epochs)
            else {
                break;
            };
            self.blocks
                .push((block.epoch, block.time, block.end, block.frames, block.last));
            self.producer.push(block).unwrap();
            self.pushed += 1;
        }
    }

    /// Runs until virtual time `until`.
    fn run_until(&mut self, until: u64, sources: &mut dyn PcmProvider) {
        loop {
            let next = self.next_callback.min(self.next_feed);
            if next > until {
                break;
            }
            self.now = next;
            if next == self.next_feed {
                if self.feeding {
                    self.feed(sources);
                }
                self.feeds += 1;
                let late = if self.feeds.is_multiple_of(50) {
                    16 * MS
                } else {
                    0
                };
                self.next_feed += 2 * MS + late;
            } else {
                let latency = self.dac(self.played) - self.now;
                let mut out = std::mem::take(&mut self.out);
                self.callback.process_at(&mut out, latency, self.now);
                self.out = out;
                self.played += self.buffer as u64;
                self.callbacks += 1;
                let jitter = self.jitter();
                self.next_callback = self.dac(self.played) - self.latency - jitter;
            }
        }
        self.now = until;
    }

    fn reading(&self, at: u64) -> Option<ClockReading> {
        self.clock.reading_at(at)
    }

    /// When the first sample of the current run reached (or reaches) the DAC.
    fn run_at(&self) -> u64 {
        self.cell.load().expect("a clock sample").run_at
    }
}

fn flicks_between(from: u64, to: u64) -> i64 {
    ((i128::from(to) - i128::from(from)) * i128::from(SECOND) / 1_000_000_000) as i64
}

#[test]
fn preroll_starts_the_clock_with_the_first_sample() {
    let mut sources = Constant::default();
    let mut sim = Sim::new(project(10), Time::from_seconds(2), 1, false);
    let start = sim.now;
    sim.run_until(start + 40 * MS, &mut sources);
    assert!(sim.ready(), "the ring reached its target");
    assert_eq!(sim.reading(sim.now), None, "not started");
    assert!(
        sim.cell.load().is_none(),
        "nothing published during preroll"
    );

    // The render thread ends preroll.
    sim.transport.set_started(true);
    let started_at = sim.now;
    sim.run_until(started_at + 60 * MS, &mut sources);
    let first = sim.run_at();
    assert!(first > started_at);
    assert_eq!(
        sim.reading(first - 1),
        None,
        "silence until the first sample is heard"
    );
    let at_first = sim.reading(first).unwrap();
    assert_eq!(at_first.time, Time::from_seconds(2));
    let later = sim.reading(first + 10 * MS).unwrap();
    assert_eq!(
        later.time,
        Time::from_seconds(2) + Time(flicks_between(0, 10 * MS))
    );
    assert_eq!(sim.shared.underruns(), 0);
}

#[test]
fn loops_wrap_at_the_end_block_with_the_next_epoch() {
    let mut sources = Constant::default();
    let mut sim = Sim::new(project(1), Time(SECOND / 2), 1, true);
    sim.transport.set_started(true);
    // Read the clock as the render thread does: at the (virtual) present.
    let mut probes = Vec::new();
    let mut at = sim.dac0;
    while at < sim.dac0 + 3_100 * MS {
        sim.run_until(at, &mut sources);
        probes.push((at, sim.reading(at)));
        at += 7 * MS + 333;
    }
    // The blocks: every epoch ends exactly at 1 s, the next begins at 0 with epoch + 1.
    let mut wraps = 0;
    for pair in sim.blocks.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a.0 != b.0 {
            wraps += 1;
            assert_eq!(a.2, Time::from_seconds(1), "the last block ends at the end");
            assert_eq!(b.1, Time::ZERO);
            assert_eq!(b.0, a.0 + 1);
        } else {
            assert_eq!(a.2, b.1, "blocks are contiguous");
        }
        assert!(!a.4, "never `last` while looping");
    }
    assert!(wraps >= 3, "{wraps} wraps");
    // From 0.5 s the first loop is 93.75 blocks, each later one (1 s) 187.5 blocks: the last
    // blocks have 192, then 128 frames.
    let short: Vec<u16> = sim
        .blocks
        .iter()
        .map(|b| b.3)
        .filter(|&f| f != 256)
        .collect();
    assert_eq!(short[0], 192);
    assert!(short[1..].iter().all(|&f| f == 128), "{short:?}");

    // The clock follows the loop within one sample. Started before the first callback, so
    // device frame 0 (at `dac0`) is the first sample.
    let first = sim.dac0;
    let base = probes[0].1.expect("audible at the first sample");
    assert_eq!(base.time, Time(SECOND / 2));
    for &(at, reading) in &probes {
        let total = SECOND / 2 + flicks_between(first, at);
        let expected = Time(total % SECOND);
        let loops = (total / SECOND) as u32;
        let near_wrap = expected.flicks() < SAMPLE || SECOND - expected.flicks() < SAMPLE;
        let r = reading.expect("a reading during playback");
        if !near_wrap {
            assert_eq!(r.epoch, base.epoch + loops, "at {at}");
            assert!(
                (r.time - expected).flicks().abs() <= SAMPLE,
                "at {at}: {:?} vs {expected:?}",
                r.time
            );
        }
    }
    assert_eq!(sim.shared.underruns(), 0);
}

#[test]
fn the_end_stops_at_the_duration() {
    let mut sources = Constant::default();
    let mut sim = Sim::new(project(1), Time(SECOND * 9 / 10), 1, false);
    sim.transport.set_started(true);
    let start = sim.now;
    sim.run_until(start + 400 * MS, &mut sources);
    let last = *sim.blocks.last().unwrap();
    assert!(last.4, "the last block is marked");
    assert_eq!(last.2, Time::from_seconds(1));
    assert!(
        sim.maker
            .next(&sim.project, false, &mut sources, &mut sim.epochs)
            .is_none()
    );
    let first = sim.dac0;
    let before = sim.reading(first + 50 * MS).unwrap();
    assert!(!before.ended);
    let end = sim.reading(sim.now).unwrap();
    assert!(end.ended);
    assert_eq!(end.time, Time::from_seconds(1));
    assert_eq!(sim.shared.underruns(), 0, "the end is no underrun");
}

#[test]
fn the_end_on_a_block_boundary_and_play_at_the_end() {
    let mut sources = Constant::default();
    // 1 s minus exactly 2 blocks: the end falls on a block boundary.
    let from = Time::from_seconds(1) - crate::audio::sample_offset(512, SR);
    let mut sim = Sim::new(project(1), from, 1, false);
    sim.transport.set_started(true);
    let start = sim.now;
    sim.run_until(start + 200 * MS, &mut sources);
    let frames: Vec<u16> = sim.blocks.iter().map(|b| b.3).collect();
    assert_eq!(frames, [256, 256]);
    assert!(sim.blocks[1].4);
    let end = sim.reading(sim.now).unwrap();
    assert!(end.ended);
    assert_eq!(end.time, Time::from_seconds(1));

    // Play exactly at the end: only the end marker, heard (and ended) at the DAC.
    let mut sim = Sim::new(project(1), Time::from_seconds(1), 1, false);
    sim.transport.set_started(true);
    let start = sim.now;
    sim.run_until(start + 200 * MS, &mut sources);
    let frames: Vec<u16> = sim.blocks.iter().map(|b| b.3).collect();
    assert_eq!(frames, [0]);
    assert_eq!(sim.reading(sim.dac0 - 1), None);
    let end = sim.reading(sim.dac0).unwrap();
    assert!(end.ended);
    assert_eq!(end.time, Time::from_seconds(1));
}

#[test]
fn reverse_and_fast_rates_move_the_clock_and_stop_at_zero() {
    let mut sources = Constant::default();
    let mut sim = Sim::new(project(10), Time(SECOND / 2), -1, false);
    sim.transport.set_started(true);
    let start = sim.now;
    sim.run_until(start + 300 * MS, &mut sources);
    let r = sim.reading(sim.dac0 + 100 * MS).unwrap();
    assert_eq!(r.time, Time(SECOND / 2 - flicks_between(0, 100 * MS)));
    assert_eq!(r.rate, -1);
    assert!(sources.reads.is_empty(), "shuttle audio is muted");
    sim.run_until(start + 800 * MS, &mut sources);
    let end = sim.reading(sim.now).unwrap();
    assert!(end.ended);
    assert_eq!(end.time, Time::ZERO);

    let mut sim = Sim::new(project(10), Time::ZERO, 4, false);
    sim.transport.set_started(true);
    let start = sim.now;
    sim.run_until(start + 300 * MS, &mut sources);
    let r = sim.reading(sim.dac0 + 50 * MS).unwrap();
    assert_eq!(r.time, Time(4 * flicks_between(0, 50 * MS)));
}

#[test]
fn a_rate_change_continues_after_the_queued_audio() {
    let mut sources = Constant::default();
    let mut sim = Sim::new(project(60), Time::ZERO, 1, false);
    sim.transport.set_started(true);
    let start = sim.now;
    sim.run_until(start + 200 * MS, &mut sources);
    let switch = sim.maker.position();
    sim.epochs += 1;
    let epoch = sim.epochs;
    sim.maker.set_rate(2, epoch);
    sim.run_until(start + 600 * MS, &mut sources);
    // Audible `switch` happens at the DAC time of that sample; from there at 2x.
    let switch_at = sim.dac0
        + u64::try_from(i128::from(switch.flicks()) * 1_000_000_000 / i128::from(SECOND)).unwrap();
    let before = sim.reading(switch_at - 10 * MS).unwrap();
    assert_eq!(before.rate, 1);
    assert!(
        (before.time - (switch - Time(flicks_between(0, 10 * MS))))
            .flicks()
            .abs()
            <= SAMPLE
    );
    let after = sim.reading(switch_at + 100 * MS).unwrap();
    assert_eq!((after.rate, after.epoch), (2, epoch));
    let expected = switch + Time(2 * flicks_between(0, 100 * MS));
    assert!(
        (after.time - expected).flicks().abs() <= 2 * SAMPLE,
        "{after:?} vs {expected:?}"
    );
}

#[test]
fn an_underrun_freezes_the_clock_and_resumes_without_a_jump() {
    let mut sources = Constant::default();
    let mut sim = Sim::new(project(60), Time::ZERO, 1, false);
    sim.transport.set_started(true);
    let start = sim.now;
    sim.run_until(start + 300 * MS, &mut sources);
    sim.feeding = false;
    sim.run_until(start + 600 * MS, &mut sources);
    assert!(sim.shared.underruns() > 0);
    let frozen = sim.reading(sim.now).unwrap();
    assert_eq!(frozen, sim.reading(sim.now + 50 * MS).unwrap(), "frozen");
    assert!(!frozen.ended);
    sim.feeding = true;
    let resumed_at = sim.now;
    sim.run_until(resumed_at + 300 * MS, &mut sources);
    // From the resume on the clock never goes back and moves at 1x again.
    let mut last = frozen.time;
    let mut at = resumed_at;
    while at < sim.now - 100 * MS {
        let t = sim.reading(at).unwrap().time;
        assert!(t >= last, "the clock went back at {at}");
        last = t;
        at += MS;
    }
    let a = sim.reading(sim.now - 100 * MS).unwrap().time;
    let b = sim.reading(sim.now - 50 * MS).unwrap().time;
    assert_eq!(b - a, Time(flicks_between(0, 50 * MS)));
}

/// Checks that every read continues exactly where the previous one ended, as a 44.1 kHz source
/// sees it.
#[derive(Debug, Default)]
struct ContinuousSource {
    next: Option<Time>,
    frames: u64,
    worst_gap: i64,
}

impl PcmProvider for ContinuousSource {
    fn read(&mut self, voice: &Voice<'_>, out: &mut [f32]) -> bool {
        if let Some(next) = self.next {
            let gap = (voice.source_time - next).flicks().abs();
            self.worst_gap = self.worst_gap.max(gap);
        }
        let frames = out.len() / 2;
        self.frames += frames as u64;
        self.next =
            Some(voice.source_time + crate::audio::sample_offset(frames as i64, voice.sample_rate));
        out.fill(0.25);
        true
    }
}

#[test]
fn ten_simulated_minutes_end_within_one_sample() {
    let mut sources = ContinuousSource::default();
    let mut sim = Sim::new(project(700), Time::ZERO, 1, false);
    sim.transport.set_started(true);
    let start = sim.now;
    let minutes = 10;
    sim.run_until(start + minutes * 60 * 1_000 * MS + 100 * MS, &mut sources);
    assert_eq!(sim.shared.underruns(), 0);
    assert_eq!(sources.worst_gap, 0, "the mixer's source time never drifts");
    let first = sim.dac0;
    let at = first + minutes * 60 * 1_000 * MS;
    let r = sim.reading(at).unwrap();
    let expected = Time::from_seconds(minutes as i64 * 60);
    assert!(
        (r.time - expected).flicks().abs() <= SAMPLE,
        "clock {:?} vs {expected:?}",
        r.time
    );
    // The mixer's time: block i starts exactly at i x 256 samples.
    let blocks = sim.blocks.len() as i64;
    assert_eq!(
        sim.blocks.last().unwrap().1,
        crate::audio::sample_offset((blocks - 1) * 256, SR)
    );
    // The source position equals the timeline position; at 44.1 kHz (16,000 flicks per
    // sample) the source and the 48 kHz mix stay within one source sample.
    let source_end = sources.next.unwrap();
    assert_eq!(source_end, sim.maker.position());
    assert!(sources.frames > 48_000 * 600);
}

#[test]
fn grains_are_80_ms_with_5_ms_fades() {
    let project = project(10);
    let mut sources = Constant::default();
    let blocks = make_grain(&project, Time::from_seconds(1), 3, 9, SR, &mut sources);
    assert_eq!(blocks.len(), 15, "3840 frames");
    assert!(
        blocks
            .iter()
            .all(|b| b.grain && b.session == 9 && b.seek_gen == 3)
    );
    assert_eq!(blocks[0].time, Time::from_seconds(1));
    assert_eq!(
        blocks[1].time,
        Time::from_seconds(1) + crate::audio::sample_offset(256, SR)
    );
    let pcm: Vec<f32> = blocks
        .iter()
        .flat_map(|b| b.samples[..2 * usize::from(b.frames)].iter().copied())
        .collect();
    assert_eq!(pcm.len(), 2 * 3840);
    assert_eq!(pcm[0], 0.0);
    assert!((pcm[2 * 120] - 0.5).abs() < 1e-6, "half way up the fade");
    assert_eq!(pcm[2 * 240], 1.0);
    assert_eq!(pcm[2 * 3839], 0.0);
}

#[test]
fn grains_are_throttled_and_skip_tiny_moves() {
    let frame = app_core::FrameRate::FPS_30.frame_duration();
    let mut grains = Grains::default();
    let t0 = Instant::now();
    grains.pending = Some((Time::from_seconds(1), 1, 1));
    assert!(grains.take(t0, frame).is_some());
    grains.pending = Some((Time::from_seconds(2), 2, 2));
    assert!(
        grains.take(t0 + Duration::from_millis(10), frame).is_none(),
        "40 ms apart"
    );
    assert!(grains.pending.is_some(), "kept for later");
    assert_eq!(
        grains.take(t0 + Duration::from_millis(41), frame),
        Some((Time::from_seconds(2), 2, 2))
    );
    // Less than one frame from the last grain: none.
    grains.pending = Some((Time::from_seconds(2) + Time(frame.flicks() - 1), 3, 3));
    assert!(
        grains
            .take(t0 + Duration::from_millis(100), frame)
            .is_none()
    );
    assert!(grains.pending.is_none());
}

#[test]
fn the_engine_plays_on_the_null_output_without_a_device() {
    let transport = Arc::new(TransportShared::new());
    let mut project = project(1);
    // The media file does not exist: the voice is silent, timing is unaffected.
    project.media[0].path = app_core::media::MediaPath::absolute("C:/no/such/file.wav");
    let snapshot = Arc::new(ArcSwap::from_pointee(project));
    let lost = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&lost);
    let engine = AudioEngine::with_options(
        Arc::clone(&transport),
        snapshot,
        move |_| flag.store(true, Ordering::Relaxed),
        AudioOptions { use_device: false },
    )
    .unwrap();
    let clock = engine.clock();
    let seek_gen = transport.seek(Time(SECOND / 2));
    transport.set_state(crate::transport::PlayState::Prerolling);
    engine.play(Time(SECOND / 2), 1, seek_gen);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !engine.ring_ready() {
        assert!(Instant::now() < deadline, "preroll never finished");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(clock.now(), None);
    transport.set_started(true);
    let mut reading = None;
    while reading.is_none_or(|r: ClockReading| !r.ended) {
        assert!(
            Instant::now() < deadline,
            "playback never ended: {reading:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
        reading = clock.now().or(reading);
    }
    let end = reading.unwrap();
    assert_eq!(end.time, Time::from_seconds(1));
    assert_eq!(end.seek_gen, seek_gen);
    let output = engine.output().unwrap();
    assert!(!output.device);
    engine.pause();
    assert_eq!(clock.now(), None, "paused");
    assert!(!lost.load(Ordering::Relaxed));
}
