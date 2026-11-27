//! The source mapping (design `clip-speed-audio.md` §3.2, §9).

use proptest::prelude::*;

use super::*;

fn media_clip(start: i64, duration: i64, source_in: i64, percent: u16, reverse: bool) -> Clip {
    Clip {
        source: ClipSource::Media {
            media: MediaId(uuid::Uuid::nil()),
            source_in: Time(source_in),
            speed: Speed::new(percent).expect("valid speed"),
            reverse,
        },
        ..Clip::color("c", Time(start), Time(duration), Rgba8::BLACK)
    }
}

#[test]
fn normal_speed_equals_the_plain_mapping() {
    let clip = media_clip(1_000, 500, 7_000, 100, false);
    assert_eq!(clip.source_end(), Some(Time(7_500)));
    for t in [1_000, 1_001, 1_250, 1_499] {
        assert_eq!(clip.source_time(Time(t)), Some(Time(7_000 + t - 1_000)));
    }
    assert_eq!(clip.source_time(Time(999)), None);
    assert_eq!(clip.source_time(Time(1_500)), None);
    assert_eq!(clip.sub_source_in(Time(100), Time(400)), Time(7_100));
}

#[test]
fn speed_scales_the_used_range() {
    let double = media_clip(0, 1_000, 100, 200, false);
    assert_eq!(double.source_end(), Some(Time(2_100)));
    assert_eq!(double.source_time(Time(10)), Some(Time(120)));
    let half = media_clip(0, 1_001, 0, 50, false);
    assert_eq!(half.source_end(), Some(Time(501)), "ceil(1001 / 2)");
    assert_eq!(half.source_time(Time(3)), Some(Time(1)));
}

#[test]
fn reverse_starts_at_the_end_of_the_range() {
    let clip = media_clip(10, 1_000, 500, 100, true);
    assert_eq!(clip.source_end(), Some(Time(1_500)));
    assert_eq!(clip.source_time(Time(10)), Some(Time(1_499)));
    assert_eq!(clip.source_time(Time(1_009)), Some(Time(500)));
    let fast = media_clip(0, 333, 0, 300, true);
    assert_eq!(fast.source_end(), Some(Time(999)));
    assert!(fast.source_time(Time(332)) >= Some(Time(0)));
    assert_eq!(fast.source_time(Time(0)), Some(Time(998)));
}

#[test]
fn frame_sample_time_is_the_frame_centre() {
    let frame = FrameRate::FPS_30.frame_duration();
    let half = Time(frame.flicks() / 2);
    let clip = media_clip(0, frame.flicks() * 10, 12_345, 100, false);
    for n in 0..10 {
        let t_n = Time(frame.flicks() * n);
        assert_eq!(
            clip.frame_sample_time(t_n, frame),
            Some(clip.source_time(t_n).unwrap() + half)
        );
    }
    assert_eq!(
        clip.frame_sample_time(Time(frame.flicks() * 10), frame),
        None
    );
    // A clip ending mid-frame samples its last instant, never past its range.
    let short = media_clip(0, frame.flicks() / 4, 0, 100, false);
    assert_eq!(
        short.frame_sample_time(Time::ZERO, frame),
        Some(Time(frame.flicks() / 4 - 1))
    );
}

#[test]
fn freeze_holds_one_time() {
    let clip = Clip {
        source: ClipSource::Freeze {
            media: MediaId(uuid::Uuid::nil()),
            source_time: Time(42),
        },
        ..Clip::color("f", Time(100), Time(50), Rgba8::BLACK)
    };
    assert_eq!(clip.source_time(Time(100)), Some(Time(42)));
    assert_eq!(clip.source_time(Time(149)), Some(Time(42)));
    assert_eq!(clip.source_time(Time(150)), None);
    assert_eq!(clip.frame_sample_time(Time(120), Time(30)), Some(Time(42)));
    assert_eq!(clip.source_end(), None);
    assert_eq!(clip.source_in(), Time::ZERO);
    assert_eq!(clip.source.media(), Some(MediaId(uuid::Uuid::nil())));
    assert_eq!(clip.content_at(Time(120)), Time(20));
    assert_eq!(clip.offset_of_content(Time(20)), Time(20));
}

#[test]
fn extend_room_stops_at_the_media_limits() {
    let len = Time(10_000);
    // Forward 2x: 1 000..3 000 of 10 000 used.
    let clip = media_clip(0, 1_000, 1_000, 200, false);
    assert_eq!(clip.extend_room(len), (Time(500), Time(3_500)));
    // Reversed, the start edge reaches the top of the media, the end edge its start.
    let reversed = media_clip(0, 1_000, 1_000, 200, true);
    assert_eq!(reversed.extend_room(len), (Time(3_500), Time(500)));
    let card = Clip::color("c", Time(0), Time(10), Rgba8::BLACK);
    assert_eq!(card.extend_room(len), (Time(i64::MAX), Time(i64::MAX)));
}

/// A media clip and the length of its media.
fn any_clip() -> impl Strategy<Value = (Clip, Time)> {
    (
        0..1_000_000i64,
        1..1_000_000i64,
        0..1_000_000i64,
        Speed::MIN..=Speed::MAX,
        any::<bool>(),
        0..1_000_000i64,
    )
        .prop_map(|(start, duration, source_in, percent, reverse, slack)| {
            let clip = media_clip(start, duration, source_in, percent, reverse);
            let len = clip.source_end().unwrap() + Time(slack);
            (clip, len)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every instant of the clip maps inside the used range, forward non-decreasing and
    /// reversed non-increasing, and the content round-trips for speeds of 1x and above.
    #[test]
    fn source_time_stays_in_range_and_is_monotonic(
        (clip, _) in any_clip(),
        offsets in prop::collection::vec(0.0..1.0f64, 2..16),
    ) {
        let (a, end) = (clip.source_in(), clip.source_end().unwrap());
        let mut ts: Vec<Time> = offsets
            .iter()
            .map(|f| clip.start + Time((f * clip.duration.flicks() as f64) as i64))
            .chain([clip.start, clip.end() - Time(1)])
            .collect();
        ts.sort();
        let mapped: Vec<Time> = ts.iter().map(|&t| clip.source_time(t).unwrap()).collect();
        for &s in &mapped {
            prop_assert!(a <= s && s < end, "{s:?} outside {a:?}..{end:?}");
        }
        let ordered = mapped.windows(2).all(|w| if clip.reversed() { w[0] >= w[1] } else { w[0] <= w[1] });
        prop_assert!(ordered);
        for &t in &ts {
            prop_assert_eq!(clip.content_at(t), clip.source_time(t).unwrap());
            if clip.speed() >= Speed::NORMAL {
                prop_assert_eq!(clip.start + clip.offset_of_content(clip.content_at(t)), t);
            } else {
                let back = clip.start + clip.offset_of_content(clip.content_at(t));
                prop_assert!(back <= t && clip.content_at(back) == clip.content_at(t));
            }
        }
    }

    /// A split at any interior point keeps every content moment within one flick, and both
    /// halves stay inside the original range.
    #[test]
    fn split_keeps_content_in_place((clip, _) in any_clip(), at in 0.0..1.0f64, probe in 0.0..1.0f64) {
        prop_assume!(clip.duration > Time(1));
        let offset = Time(1 + (at * (clip.duration.flicks() - 1) as f64) as i64).min(clip.duration - Time(1));
        let rest = clip.duration - offset;
        let mut left = clip.clone();
        let mut right = clip.clone();
        let (left_in, right_in) = (clip.sub_source_in(Time::ZERO, offset), clip.sub_source_in(offset, rest));
        left.duration = offset;
        right.start = clip.start + offset;
        right.duration = rest;
        for (half, source_in) in [(&mut left, left_in), (&mut right, right_in)] {
            if let ClipSource::Media { source_in: s, .. } = &mut half.source {
                *s = source_in;
            }
        }
        for half in [&left, &right] {
            prop_assert!(half.source_in() >= clip.source_in());
            prop_assert!(half.source_end() <= clip.source_end());
            let t = half.start + Time((probe * (half.duration.flicks() - 1) as f64) as i64);
            let (old, new) = (clip.source_time(t).unwrap(), half.source_time(t).unwrap());
            prop_assert!((old - new).flicks().abs() <= 1, "{old:?} vs {new:?}");
        }
    }

    /// Extending an edge by its room keeps the used range inside the media; one flick more
    /// leaves it (the room is exact).
    #[test]
    fn extend_room_is_exact((clip, len) in any_clip(), start_edge in any::<bool>()) {
        let (start_room, end_room) = clip.extend_room(len);
        let within = |extra: Time| {
            let (offset, duration) = if start_edge {
                (-extra, clip.duration + extra)
            } else {
                (Time::ZERO, clip.duration + extra)
            };
            let mut grown = clip.clone();
            grown.duration = duration;
            if let ClipSource::Media { source_in, .. } = &mut grown.source {
                *source_in = clip.sub_source_in(offset, duration);
            }
            grown.source_in() >= Time::ZERO && grown.source_end().unwrap() <= len
        };
        let room = if start_edge { start_room } else { end_room };
        prop_assert!(within(room));
        prop_assert!(!within(room + Time(1)));
    }
}
