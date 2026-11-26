//! The audio callback never allocates (playback.md §12, CLAUDE.md rule 4): `OutputCallback`
//! runs 10,000 device buffers under a global allocator that counts allocations of the calling
//! thread, including seeks (stale blocks dropped), pauses, underruns, loop wraps, the end,
//! grains, and 1, 2 and 6 device channels in f32, i16, i32 and u16.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use app_core::Time;
use engine::audio::{AudioBlock, AudioShared, BLOCK_FRAMES, OutputCallback, ring, sample_offset};
use engine::playback::clock::ClockCell;
use engine::transport::TransportShared;

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<u64> = const { Cell::new(0) };
}

fn note() {
    // `try_with`: the allocator also runs while thread locals are torn down.
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            let _ = COUNT.try_with(|count| count.set(count.get() + 1));
        }
    });
}

// SAFETY: forwards every call to the system allocator unchanged; only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: the caller's contract is passed on unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: as above.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        // SAFETY: as above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` and returns the allocations it made on this thread.
fn allocations(f: impl FnOnce()) -> u64 {
    COUNT.with(|c| c.set(0));
    COUNTING.with(|c| c.set(true));
    f();
    COUNTING.with(|c| c.set(false));
    COUNT.with(|c| c.get())
}

const SR: u32 = 48_000;

fn block(session: u32, seek_gen: u32, epoch: u32, index: i64) -> AudioBlock {
    let time = sample_offset(index * BLOCK_FRAMES as i64, SR);
    let mut samples = [0.0; 2 * BLOCK_FRAMES];
    for (i, s) in samples.iter_mut().enumerate() {
        *s = ((i % 50) as f32 - 25.0) / 25.0;
    }
    AudioBlock {
        session,
        seek_gen,
        epoch,
        time,
        end: time + sample_offset(BLOCK_FRAMES as i64, SR),
        frames: BLOCK_FRAMES as u16,
        samples,
        ..AudioBlock::EMPTY
    }
}

/// Every allocation (ring, callback, buffers) happens before counting starts.
fn exercise(channels: usize, calls: usize) -> u64 {
    let (mut producer, consumer) = ring();
    let transport = Arc::new(TransportShared::new());
    let cell = Arc::new(ClockCell::new());
    let shared = Arc::new(AudioShared::new());
    let mut callback = OutputCallback::new(
        consumer,
        Arc::clone(&transport),
        Arc::clone(&cell),
        Arc::clone(&shared),
        channels,
        SR,
    );
    let mut out_f32 = vec![0.0f32; 441 * channels];
    let mut out_i16 = vec![0i16; 480 * channels];
    let mut out_i32 = vec![0i32; 512 * channels];
    let mut out_u16 = vec![0u16; 100 * channels];
    let mut session = cell.begin_session();
    let mut seek_gen = transport.seek(Time::ZERO);
    let mut epoch = 1;
    let mut index = 0;
    transport.set_started(true);
    // What the `cpal` stream passes: 20 ms from the callback to the DAC.
    let info = cpal::OutputCallbackInfo::new(cpal::OutputStreamTimestamp {
        callback: cpal::StreamInstant::new(5, 0),
        playback: cpal::StreamInstant::new(5, 20_000_000),
    });

    let count = allocations(|| {
        for call in 0..calls {
            match call % 100 {
                // A seek: blocks already in the ring become stale.
                10 => seek_gen = transport.seek(Time::ZERO),
                // A pause, then grains while paused.
                30 => {
                    transport.set_started(false);
                    session = cell.begin_session();
                }
                31..=34 => {
                    let mut grain = block(session, seek_gen, 0, index);
                    grain.grain = true;
                    let _ = producer.push(grain);
                }
                // Play again.
                40 => {
                    session = cell.begin_session();
                    transport.set_started(true);
                }
                // A loop wrap.
                60 => {
                    epoch += 1;
                    index = 0;
                }
                // The end: a short last block, then an end marker.
                80 => {
                    let mut last = block(session, seek_gen, epoch, index);
                    last.frames = 100;
                    last.last = true;
                    let _ = producer.push(last);
                    let mut marker = block(session, seek_gen, epoch, index + 1);
                    marker.frames = 0;
                    marker.last = true;
                    let _ = producer.push(marker);
                }
                85 => epoch += 1,
                _ => {}
            }
            // Underruns: every 7th buffer gets no new audio.
            if call % 7 != 0 && !(30..40).contains(&(call % 100)) {
                for _ in 0..2 {
                    let _ = producer.push(block(session, seek_gen, epoch, index));
                    index += 1;
                }
            }
            let now = call as u64 * 10_000_000;
            match call % 4 {
                // `process` as the stream calls it: real time, load statistics.
                0 => callback.process(&mut out_f32, &info),
                1 => callback.process_at(&mut out_i16, 20_000_000, now),
                2 => callback.process_at(&mut out_i32, 20_000_000, now),
                _ => callback.process_at(&mut out_u16, 20_000_000, now),
            }
        }
    });
    assert!(shared.underruns() > 0, "underruns were exercised");
    assert!(cell.load().is_some(), "the clock was published");
    assert!(shared.load().calls > 0, "`process` measured itself");
    count
}

#[test]
fn the_callback_never_allocates() {
    for channels in [1, 2, 6] {
        assert_eq!(exercise(channels, 10_000), 0, "{channels} channels");
    }
}

#[test]
fn the_counting_allocator_counts() {
    let count = allocations(|| {
        let v = vec![1u8; 16];
        std::hint::black_box(v);
    });
    assert_eq!(count, 2, "one allocation and one free");
}
