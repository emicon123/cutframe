//! The audio path of playback and export (playback.md §6).
//!
//! - [`ring`]: fixed-size [`AudioBlock`]s in one wait-free `rtrb` ring from the feeder to the
//!   callback.
//! - [`callback`]: [`OutputCallback`], the real-time device callback: copy, channel and sample
//!   format conversion, clock samples. No allocation, lock, I/O or logging (CLAUDE.md rule 4).
//! - [`feeder`]: the `audio-feeder` thread and its handle [`AudioEngine`]: the `cpal` stream,
//!   play, pause, seek, loop, shuttle rates, scrub grains, the null output without a device.
//! - [`mix`]: [`mix_block`], the mixer shared by playback (device rate) and export (project
//!   rate), over a [`PcmProvider`].
//! - [`reader`]: [`AudioReader`] (one FFmpeg demuxer, decoder and resampler per voice) and
//!   [`AudioSources`], the [`PcmProvider`] over them. Usable on any thread.
//!
//! Timing is integer flicks: sample `k` of a block is `k x FLICKS_PER_SECOND / sample_rate`
//! flicks after its start, exact for every rate WASAPI uses ([`sample_offset`]).

pub mod callback;
pub mod feeder;
pub mod mix;
pub mod reader;
pub mod ring;

use app_core::{FLICKS_PER_SECOND, Time};

pub use callback::{CallbackLoad, OutputCallback, OutputSample};
pub use feeder::{AudioEngine, AudioEvent, OutputInfo};
pub use mix::{PcmProvider, Voice, mix_block};
pub use reader::{AudioReader, AudioSources};
pub use ring::{
    AudioBlock, AudioShared, BLOCK_FRAMES, BlockConsumer, BlockProducer, RING_BLOCKS,
    TARGET_FILL_BLOCKS, ring,
};

/// The timeline offset of sample `samples` at `sample_rate`: `samples x FLICKS_PER_SECOND /
/// sample_rate`. Exact (an integer number of flicks per sample) for 8, 11.025, 16, 22.05, 32,
/// 44.1, 48, 88.2, 96, 176.4 and 192 kHz; other rates round to the nearest flick in `i128`
/// (playback.md §6.2). Negative `samples` count backwards.
pub fn sample_offset(samples: i64, sample_rate: u32) -> Time {
    let rate = i64::from(sample_rate.max(1));
    if FLICKS_PER_SECOND % rate == 0 {
        return Time(samples.saturating_mul(FLICKS_PER_SECOND / rate));
    }
    let numerator = i128::from(samples) * i128::from(FLICKS_PER_SECOND);
    let rate = i128::from(rate);
    let rounded = if numerator >= 0 {
        (numerator + rate / 2) / rate
    } else {
        (numerator - rate / 2) / rate
    };
    Time(i64::try_from(rounded).unwrap_or(if rounded < 0 { i64::MIN } else { i64::MAX }))
}

/// How many samples at `sample_rate`, `0, 1, 2, ...`, start before `span`: the smallest `k`
/// with `sample_offset(k) >= span` (0 for an empty or negative span).
pub fn samples_before(span: Time, sample_rate: u32) -> i64 {
    if span <= Time::ZERO {
        return 0;
    }
    let rate = i128::from(sample_rate.max(1));
    let fps = i128::from(FLICKS_PER_SECOND);
    // ceil(span x rate / fps), then corrected for the rounding of `sample_offset`.
    let mut k =
        i64::try_from((i128::from(span.flicks()) * rate + fps - 1) / fps).unwrap_or(i64::MAX);
    while k > 0 && sample_offset(k - 1, sample_rate) >= span {
        k -= 1;
    }
    while sample_offset(k, sample_rate) < span {
        k += 1;
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_offsets_are_exact_for_common_rates() {
        assert_eq!(sample_offset(1, 48_000), Time(14_700));
        assert_eq!(sample_offset(1, 44_100), Time(16_000));
        assert_eq!(sample_offset(1, 96_000), Time(7_350));
        assert_eq!(sample_offset(-256, 48_000), Time(-256 * 14_700));
        assert_eq!(sample_offset(48_000, 48_000), Time::from_seconds(1));
        // 384 kHz does not divide the flick rate: rounded, 1837.5 -> 1838.
        assert_eq!(sample_offset(1, 384_000), Time(1_838));
        assert_eq!(sample_offset(384_000, 384_000), Time::from_seconds(1));
    }

    #[test]
    fn samples_before_counts_sample_starts() {
        assert_eq!(samples_before(Time::ZERO, 48_000), 0);
        assert_eq!(samples_before(Time(1), 48_000), 1);
        assert_eq!(samples_before(Time(14_700), 48_000), 1);
        assert_eq!(samples_before(Time(14_701), 48_000), 2);
        assert_eq!(samples_before(Time::from_seconds(1), 44_100), 44_100);
        for span in [1, 1_837, 1_838, 1_839, 1_000_000] {
            let k = samples_before(Time(span), 384_000);
            assert!(sample_offset(k, 384_000) >= Time(span));
            assert!(k == 0 || sample_offset(k - 1, 384_000) < Time(span));
        }
    }
}
