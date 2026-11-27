use proptest::prelude::*;

use super::*;
use crate::command::test_support::*;
use crate::{Document, EditPhase};

fn set_speed(clips: Vec<ClipId>, percent: u16) -> SetClipSpeed {
    SetClipSpeed::new(clips, speed(percent))
}

#[test]
fn speed_keeps_the_start_and_the_source_range_and_ripples_the_main_track() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.5));
    let c = p.push_clip(MAIN, color(4.0, 2.0));
    // Over the card, 1 s into it.
    let o = p.push_clip(OVERLAY, color(5.0, 0.5));
    let after = assert_reversible(&mut set_speed(vec![v], 200), &p);
    let fast = clip(&after, v);
    assert_eq!((fast.start, fast.duration), (t(0.0), t(2.0)));
    assert_eq!(
        (fast.source_in(), fast.source_end()),
        (t(0.5), Some(t(4.5)))
    );
    assert_eq!(fast.speed(), speed(200));
    assert_eq!(clip(&after, c).start, t(2.0));
    assert_eq!(
        clip(&after, o).start,
        t(3.0),
        "the overlay follows its card"
    );

    let after = assert_reversible(&mut set_speed(vec![v], 50), &p);
    assert_eq!(clip(&after, v).duration, t(8.0));
    assert_eq!(clip(&after, c).start, t(8.0));
}

#[test]
fn followers_keep_their_content_moment_of_a_sped_up_clip() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    // Over source second 2 of the video.
    let o = p.push_clip(OVERLAY, color(2.0, 0.5));
    let after = assert_reversible(&mut set_speed(vec![v], 200), &p);
    assert_eq!(clip(&after, o).start, t(1.0));
    let after = assert_reversible(&mut set_speed(vec![v], 400), &p);
    assert_eq!(clip(&after, o).start, t(0.5));
}

#[test]
fn a_linked_pair_changes_together() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    link(&mut p, &[v, s]);
    let after = assert_reversible(&mut set_speed(vec![v], 200), &p);
    for id in [v, s] {
        let c = clip(&after, id);
        assert_eq!((c.duration, c.speed()), (t(2.0), speed(200)));
    }
}

#[test]
fn a_speed_that_runs_into_a_neighbour_off_the_magnetic_track_fails() {
    let mut p = empty();
    p.push_clip(MAIN, color(0.0, 8.0));
    let a = p.push_clip(OVERLAY, video(&p, 0.0, 2.0, 0.0));
    p.push_clip(OVERLAY, color(2.0, 2.0));
    assert_eq!(
        try_apply(&p, set_speed(vec![a], 50)),
        Err(CommandError::Overlap)
    );
    // Speeding up leaves room.
    assert!(try_apply(&p, set_speed(vec![a], 200)).is_ok());

    let mut p = empty();
    p.magnetic = false;
    let v = p.push_clip(MAIN, video(&p, 0.0, 2.0, 0.0));
    p.push_clip(MAIN, color(3.0, 2.0));
    assert_eq!(
        try_apply(&p, set_speed(vec![v], 50)),
        Err(CommandError::Overlap)
    );
    assert!(
        try_apply(&p, set_speed(vec![v], 80)).is_ok(),
        "2.5 s still fits"
    );
}

#[test]
fn a_clip_shorter_than_one_frame_is_invalid() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 0.5, 0.0));
    // 0.5 s at 100x is 5 ms, under a 30 fps frame.
    assert_eq!(
        try_apply(&p, set_speed(vec![v], 10_000)),
        Err(CommandError::InvalidValue)
    );
    assert!(
        try_apply(&p, set_speed(vec![v], 1_000)).is_ok(),
        "50 ms is enough"
    );
}

#[test]
fn only_video_and_audio_media_change_speed() {
    let mut p = empty();
    let i = p.push_clip(MAIN, image(&p, 0.0, 2.0));
    let v = p.push_clip(MAIN, video(&p, 2.0, 2.0, 0.0));
    let c = p.push_clip(MAIN, color(4.0, 1.0));
    let after = assert_reversible(&mut set_speed(vec![i, v, c], 200), &p);
    assert_eq!(clip(&after, i), clip(&p, i));
    assert_eq!(clip(&after, v).duration, t(1.0));
    assert_eq!(
        try_apply(&p, set_speed(vec![i, c], 200)),
        Err(CommandError::PropertyNotApplicable)
    );
    assert_eq!(
        try_apply(&p, set_speed(vec![], 200)),
        Err(CommandError::NothingToEdit)
    );
}

#[test]
fn a_reversed_clip_keeps_the_top_of_its_range() {
    let mut p = empty();
    let v = p.push_clip(MAIN, with_speed(video(&p, 0.0, 3.0, 1.0), 100, true));
    let first = clip(&p, v).source_time(Time::ZERO);
    let after = assert_reversible(&mut set_speed(vec![v], 300), &p);
    let fast = clip(&after, v);
    assert_eq!(fast.duration, t(1.0));
    assert_eq!(fast.source_end(), Some(t(4.0)));
    assert_eq!(fast.source_time(Time::ZERO), first, "the first frame stays");
}

#[test]
fn fades_shrink_with_the_clip() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let audio = p.tracks[MAIN].clips[0].audio.as_mut().unwrap();
    audio.fade_in = t(3.0);
    audio.fade_out = t(1.0);
    let after = assert_reversible(&mut set_speed(vec![v], 200), &p);
    let audio = clip(&after, v).audio.as_ref().unwrap();
    assert_eq!((audio.fade_in, audio.fade_out), (t(2.0), Time::ZERO));
}

#[test]
fn locked_targets_fail() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    link(&mut p, &[v, s]);
    p.tracks[AUDIO].locked = true;
    assert_eq!(
        try_apply(&p, set_speed(vec![v], 200)),
        Err(CommandError::TrackLocked(p.tracks[AUDIO].id))
    );
    assert_eq!(
        try_apply(&p, SetClipReverse::new(vec![v], true)),
        Err(CommandError::TrackLocked(p.tracks[AUDIO].id))
    );
}

#[test]
fn a_speed_drag_is_one_history_entry_and_changes_report_no_op() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let c = p.push_clip(MAIN, color(4.0, 1.0));
    assert!(!set_speed(vec![v], 100).changes(&p).unwrap());
    assert!(set_speed(vec![v], 150).changes(&p).unwrap());
    let mut doc = Document::new(p.clone());
    for percent in [150, 200, 400] {
        doc.edit(EditPhase::Update, Box::new(set_speed(vec![v], percent)))
            .unwrap();
        // Every update starts from the base, so nothing accumulates.
        assert_eq!(
            clip(doc.project(), v).duration,
            speed(percent).timeline_floor(t(4.0))
        );
        assert_eq!(clip(doc.project(), c).start, clip(doc.project(), v).end());
    }
    doc.edit(EditPhase::Commit, Box::new(set_speed(vec![v], 200)))
        .unwrap();
    assert_eq!(doc.history_len(), 1);
    assert_eq!(doc.undo_label(), Some(HistoryLabel::SetClipSpeed));
    assert!(doc.undo());
    assert_eq!(doc.project(), &p);
}

#[test]
fn reverse_moves_nothing_and_keeps_link_groups_in_step() {
    let mut p = empty();
    let v = p.push_clip(MAIN, with_speed(video(&p, 0.0, 2.0, 0.5), 200, false));
    let s = p.push_clip(AUDIO, with_speed(sound(&p, 0.0, 2.0, 0.5), 200, false));
    link(&mut p, &[v, s]);
    let o = p.push_clip(OVERLAY, color(1.0, 0.5));
    let after = assert_reversible(&mut SetClipReverse::new(vec![s], true), &p);
    for id in [v, s] {
        let (old, new) = (clip(&p, id), clip(&after, id));
        assert!(new.reversed());
        assert_eq!(
            (new.start, new.duration, new.source_in(), new.source_end()),
            (old.start, old.duration, old.source_in(), old.source_end())
        );
    }
    assert_eq!(clip(&after, o), clip(&p, o));
    assert!(!SetClipReverse::new(vec![v], true).changes(&after).unwrap());
    assert!(SetClipReverse::new(vec![v], false).changes(&after).unwrap());
    let i = p.push_clip(OVERLAY, image(&p, 3.0, 1.0));
    assert_eq!(
        try_apply(&p, SetClipReverse::new(vec![i], true)),
        Err(CommandError::PropertyNotApplicable)
    );
}

proptest! {
    /// 1x -> s -> 1x shortens the duration by at most `p / 100` flicks (not at all below 1x),
    /// never lengthens it, and never widens the used source range.
    #[test]
    fn a_speed_round_trip_keeps_the_duration(
        duration_ms in 40i64..4_000,
        percent in Speed::MIN..=Speed::MAX,
        reverse in any::<bool>(),
    ) {
        let mut p = empty();
        let d = Time(duration_ms * S / 1000);
        let v = p.push_clip(MAIN, with_speed(video(&p, 0.0, 0.1, 0.25), 100, reverse));
        p.tracks[MAIN].clips[0].duration = d;
        let Ok(fast) = try_apply(&p, set_speed(vec![v], percent)) else {
            // Only a clip under one frame at that speed fails.
            prop_assert!(speed(percent).timeline_floor(d) < p.settings.frame_rate.frame_duration());
            return Ok(());
        };
        let back = try_apply(&fast, set_speed(vec![v], 100)).unwrap();
        let lost = (d - clip(&back, v).duration).flicks();
        prop_assert!((0..=i64::from(percent) / 100).contains(&lost), "lost {}", lost);
        let range = |p: &Project| (clip(p, v).source_in(), clip(p, v).source_end().unwrap());
        let ((a0, e0), (a1, e1)) = (range(&p), range(&back));
        prop_assert!(a0 <= a1 && e1 <= e0, "the used range never grows");
    }
}

#[test]
fn the_sound_of_a_later_main_clip_moves_with_it() {
    let mut p = empty();
    let a = p.push_clip(MAIN, video(&p, 0.0, 2.0, 0.0));
    let b = p.push_clip(MAIN, video(&p, 2.0, 2.0, 0.0));
    let sa = p.push_clip(AUDIO, sound(&p, 0.0, 2.0, 0.0));
    let sb = p.push_clip(AUDIO, sound(&p, 2.0, 2.0, 0.0));
    link(&mut p, &[a, sa]);
    link(&mut p, &[b, sb]);
    let after = assert_reversible(&mut set_speed(vec![a, b], 200), &p);
    assert_eq!(clip(&after, b).start, t(1.0));
    assert_eq!(clip(&after, sb).start, t(1.0), "in sync with its video");
    assert_eq!(clip(&after, sb).duration, t(1.0));
}
