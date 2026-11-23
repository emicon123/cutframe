use super::*;
use crate::command::test_support::*;
use crate::time::FrameRate;

// ---- Split ----------------------------------------------------------------------------------

#[test]
fn split_cuts_a_selected_clip_and_advances_the_right_source() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.5));
    p.tracks[MAIN].clips[0].audio.as_mut().unwrap().fade_in = t(1.0);
    p.tracks[MAIN].clips[0].audio.as_mut().unwrap().fade_out = t(1.0);
    let after = assert_reversible(&mut SplitClips::new(vec![v], t(1.5)), &p);
    let main = &after.tracks[MAIN];
    assert_eq!(main.clips.len(), 2);
    let (left, right) = (&main.clips[0], &main.clips[1]);
    assert_eq!(left.id, v);
    assert_eq!(
        (left.start, left.duration, left.source_in()),
        (t(0.0), t(1.5), t(0.5))
    );
    assert_eq!(
        (right.start, right.duration, right.source_in()),
        (t(1.5), t(2.5), t(2.0))
    );
    assert_ne!(right.id, v);
    let (la, ra) = (left.audio.as_ref().unwrap(), right.audio.as_ref().unwrap());
    assert_eq!((la.fade_in, la.fade_out), (t(1.0), Time::ZERO));
    assert_eq!((ra.fade_in, ra.fade_out), (Time::ZERO, t(1.0)));
    assert_eq!(crate::timeline::added_clips(&p, &after), vec![right.id]);
}

#[test]
fn split_keeps_image_sources_at_zero() {
    let mut p = empty();
    let i = p.push_clip(MAIN, image(&p, 0.0, 5.0));
    let after = assert_reversible(&mut SplitClips::new(vec![i], t(2.0)), &p);
    assert!(
        after.tracks[MAIN]
            .clips
            .iter()
            .all(|c| c.source_in() == Time::ZERO)
    );
}

#[test]
fn split_right_halves_of_a_link_group_share_a_new_link() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    link(&mut p, &[v, s]);
    let old = clip(&p, v).link;
    let after = assert_reversible(&mut SplitClips::new(vec![v], t(2.0)), &p);
    let (rv, rs) = (&after.tracks[MAIN].clips[1], &after.tracks[AUDIO].clips[1]);
    assert!(rv.link.is_some());
    assert_eq!(rv.link, rs.link);
    assert_ne!(rv.link, old);
    assert_eq!(clip(&after, v).link, old);
    assert_eq!(clip(&after, s).link, old);
}

#[test]
fn split_with_nothing_selected_splits_every_unlocked_track() {
    let (mut p, [a, b, _]) = three_on_main();
    let o = p.push_clip(OVERLAY, color(4.0, 4.0));
    let m = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    p.tracks[AUDIO].locked = true;
    let after = assert_reversible(&mut SplitClips::new(vec![], t(6.0)), &p);
    assert_eq!(after.tracks[MAIN].clips.len(), 4);
    assert_eq!(clip(&after, b).duration, t(1.0));
    assert_eq!(clip(&after, a).duration, t(5.0));
    assert_eq!(clip(&after, o).duration, t(2.0));
    assert_eq!(
        after.tracks[AUDIO], p.tracks[AUDIO],
        "locked tracks are skipped"
    );
    assert_eq!(clip(&after, m).duration, t(4.0));
}

#[test]
fn split_skips_clips_too_close_to_an_edge() {
    let (p, [a, _, _]) = three_on_main();
    let frame = FrameRate::FPS_30.frame_duration();
    assert_eq!(
        try_apply(&p, SplitClips::new(vec![a], t(5.0))),
        Err(CommandError::NothingToSplit)
    );
    assert_eq!(
        try_apply(
            &p,
            SplitClips::new(vec![a], t(5.0) - Time(frame.flicks() / 2))
        ),
        Err(CommandError::NothingToSplit)
    );
    assert!(try_apply(&p, SplitClips::new(vec![a], t(5.0) - frame)).is_ok());
}

#[test]
fn split_of_a_selected_locked_clip_fails() {
    let (mut p, [a, _, _]) = three_on_main();
    p.tracks[MAIN].locked = true;
    assert!(matches!(
        try_apply(&p, SplitClips::new(vec![a], t(2.0))),
        Err(CommandError::TrackLocked(_))
    ));
}

// ---- Delete ---------------------------------------------------------------------------------

#[test]
fn delete_on_an_overlay_leaves_a_gap_and_ripple_closes_it() {
    let (mut p, _) = three_on_main();
    let x = p.push_clip(OVERLAY, color(1.0, 2.0));
    let y = p.push_clip(OVERLAY, color(4.0, 2.0));
    let after = assert_reversible(&mut DeleteClips::new(vec![x], false), &p);
    assert_eq!(starts(&after.tracks[OVERLAY]), vec![t(4.0)]);
    let after = assert_reversible(&mut DeleteClips::new(vec![x], true), &p);
    assert_eq!(ids(&after.tracks[OVERLAY]), vec![y]);
    assert_eq!(starts(&after.tracks[OVERLAY]), vec![t(2.0)]);
    assert_eq!(after.tracks[MAIN], p.tracks[MAIN]);
}

#[test]
fn ripple_on_another_track_moves_linked_members_with_it() {
    let (mut p, _) = three_on_main();
    let x = p.push_clip(OVERLAY, color(1.0, 2.0));
    let y = p.push_clip(OVERLAY, color(4.0, 2.0));
    let s = p.push_clip(AUDIO, sound(&p, 4.0, 1.0, 0.0));
    link(&mut p, &[y, s]);
    let after = assert_reversible(&mut DeleteClips::new(vec![x], true), &p);
    assert_eq!(clip(&after, y).start, t(2.0));
    assert_eq!(clip(&after, s).start, t(2.0));
}

#[test]
fn delete_on_the_magnetic_main_track_closes_the_gap_and_followers_move() {
    let (mut p, [a, b, c]) = three_on_main();
    let inside = p.push_clip(OVERLAY, color(6.0, 2.0)); // entirely inside B
    let partial = p.push_clip(OVERLAY, color(9.0, 1.5)); // starts in B, ends in C
    let later = p.push_clip(OVERLAY, color(12.0, 1.0)); // over C
    let bed = p.push_clip(AUDIO, sound(&p, 0.0, 5.0, 0.0)); // music bed at 0
    let after = assert_reversible(&mut DeleteClips::new(vec![b], false), &p);
    assert_eq!(ids(&after.tracks[MAIN]), vec![a, c]);
    assert_eq!(starts(&after.tracks[MAIN]), vec![t(0.0), t(5.0)]);
    assert!(after.clip(inside).is_none(), "captions inside B go with it");
    assert_eq!(clip(&after, partial).start, t(5.0), "to the removal point");
    assert_eq!(clip(&after, later).start, t(7.0), "keeps its offset into C");
    assert_eq!(clip(&after, bed).start, t(0.0));
}

#[test]
fn delete_leaves_locked_tracks_untouched() {
    let (mut p, [_, b, _]) = three_on_main();
    p.push_clip(AUDIO, sound(&p, 11.0, 1.0, 0.0));
    p.tracks[AUDIO].locked = true;
    let after = assert_reversible(&mut DeleteClips::new(vec![b], true), &p);
    assert_eq!(after.tracks[AUDIO], p.tracks[AUDIO]);
}

#[test]
fn delete_on_a_non_magnetic_main_track_leaves_the_gap_unless_ripple() {
    let (mut p, [a, b, c]) = three_on_main();
    p.magnetic = false;
    let o = p.push_clip(OVERLAY, color(11.0, 1.0));
    let after = assert_reversible(&mut DeleteClips::new(vec![b], false), &p);
    assert_eq!(starts(&after.tracks[MAIN]), vec![t(0.0), t(10.0)]);
    assert_eq!(clip(&after, o).start, t(11.0));
    let after = assert_reversible(&mut DeleteClips::new(vec![b], true), &p);
    assert_eq!(ids(&after.tracks[MAIN]), vec![a, c]);
    assert_eq!(starts(&after.tracks[MAIN]), vec![t(0.0), t(5.0)]);
    assert_eq!(clip(&after, o).start, t(6.0));
}

#[test]
fn delete_removes_linked_members_and_emptied_tracks() {
    let (mut p, [a, _, _]) = three_on_main();
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 5.0, 0.0));
    let v = p.push_clip(OVERLAY, video(&p, 0.0, 1.0, 0.0));
    link(&mut p, &[a, s]);
    let after = assert_reversible(&mut DeleteClips::new(vec![a, v], false), &p);
    assert!(after.clip(s).is_none());
    assert_eq!(after.tracks.len(), 1, "emptied overlay and audio tracks go");
}

#[test]
fn delete_of_a_locked_clip_fails() {
    let (mut p, [a, _, _]) = three_on_main();
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 5.0, 0.0));
    link(&mut p, &[a, s]);
    p.tracks[AUDIO].locked = true;
    assert!(matches!(
        try_apply(&p, DeleteClips::new(vec![a], false)),
        Err(CommandError::TrackLocked(_))
    ));
    assert_eq!(
        try_apply(&p, DeleteClips::new(vec![], false)),
        Err(CommandError::NothingToEdit)
    );
}

#[test]
fn delete_labels() {
    assert_eq!(
        DeleteClips::new(vec![], false).label(),
        HistoryLabel::DeleteClips
    );
    assert_eq!(
        DeleteClips::new(vec![], true).label(),
        HistoryLabel::RippleDeleteClips
    );
    assert_eq!(DeleteClips::cut(vec![]).label(), HistoryLabel::CutClips);
}

// ---- Delete gap -----------------------------------------------------------------------------

#[test]
fn delete_gap_on_the_main_track_ripples_and_followers_move() {
    let mut p = empty();
    p.magnetic = false;
    let a = p.push_clip(MAIN, color(0.0, 2.0));
    let b = p.push_clip(MAIN, color(5.0, 2.0));
    let o = p.push_clip(OVERLAY, color(6.0, 1.0));
    let after = assert_reversible(&mut DeleteGap::new(p.tracks[MAIN].id, t(3.0)), &p);
    assert_eq!(clip(&after, a).start, t(0.0));
    assert_eq!(clip(&after, b).start, t(2.0));
    assert_eq!(clip(&after, o).start, t(3.0));
    // The gap before the first clip.
    let mut q = empty();
    q.magnetic = false;
    let c = q.push_clip(MAIN, color(1.0, 2.0));
    let after = assert_reversible(&mut DeleteGap::new(q.tracks[MAIN].id, t(0.0)), &q);
    assert_eq!(clip(&after, c).start, Time::ZERO);
}

#[test]
fn delete_gap_on_an_overlay_moves_linked_members() {
    let (mut p, _) = three_on_main();
    p.push_clip(OVERLAY, color(0.0, 1.0));
    let y = p.push_clip(OVERLAY, color(3.0, 1.0));
    let s = p.push_clip(AUDIO, sound(&p, 3.0, 1.0, 0.0));
    link(&mut p, &[y, s]);
    let after = assert_reversible(&mut DeleteGap::new(p.tracks[OVERLAY].id, t(2.0)), &p);
    assert_eq!(clip(&after, y).start, t(1.0));
    assert_eq!(clip(&after, s).start, t(1.0));
}

#[test]
fn delete_gap_needs_a_gap_and_an_unlocked_track() {
    let (mut p, _) = three_on_main();
    let main = p.tracks[MAIN].id;
    assert_eq!(
        try_apply(&p, DeleteGap::new(main, t(3.0))),
        Err(CommandError::NoGap)
    );
    assert_eq!(
        try_apply(&p, DeleteGap::new(main, t(20.0))),
        Err(CommandError::NoGap)
    );
    p.push_clip(OVERLAY, color(3.0, 1.0));
    p.tracks[OVERLAY].locked = true;
    let overlay = p.tracks[OVERLAY].id;
    assert_eq!(
        try_apply(&p, DeleteGap::new(overlay, t(1.0))),
        Err(CommandError::TrackLocked(overlay))
    );
}

// ---- Trim to playhead -----------------------------------------------------------------------

#[test]
fn q_and_w_trim_the_main_clip_under_the_playhead() {
    let (mut p, [a, b, c]) = three_on_main();
    let o = p.push_clip(OVERLAY, color(12.0, 1.0));
    // Q: delete left of 7 s in B; B keeps its start, C and the overlay over it move left 2 s.
    let after = assert_reversible(&mut TrimToPlayhead::new(vec![], t(7.0), Side::Left), &p);
    assert_eq!(clip(&after, b).start, t(5.0));
    assert_eq!(clip(&after, b).duration, t(3.0));
    assert_eq!(clip(&after, c).start, t(8.0));
    assert_eq!(clip(&after, o).start, t(10.0));
    assert_eq!(clip(&after, a), clip(&p, a));
    // W: delete right of 7 s.
    let after = assert_reversible(&mut TrimToPlayhead::new(vec![], t(7.0), Side::Right), &p);
    assert_eq!(clip(&after, b).duration, t(2.0));
    assert_eq!(clip(&after, c).start, t(7.0));
}

#[test]
fn trim_to_playhead_on_selected_clips_skips_those_not_crossed() {
    let mut p = empty();
    let a = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.5));
    let x = p.push_clip(OVERLAY, color(1.0, 3.0));
    let after = assert_reversible(&mut TrimToPlayhead::new(vec![x, a], t(2.0), Side::Left), &p);
    // The overlay is trimmed to start at 2 s, then A's left trim moves it with A's content.
    assert_eq!(clip(&after, a).duration, t(2.0));
    assert_eq!(clip(&after, a).source_in(), t(2.5));
    assert_eq!(clip(&after, x).duration, t(2.0));
    assert_eq!(clip(&after, x).start, t(0.0));
    assert_eq!(
        try_apply(&p, TrimToPlayhead::new(vec![x], t(8.0), Side::Left)),
        Err(CommandError::NothingToEdit)
    );
}

// ---- Magnetic -------------------------------------------------------------------------------

#[test]
fn magnetic_on_repacks_and_followers_move() {
    let mut p = empty();
    p.magnetic = false;
    let a = p.push_clip(MAIN, color(1.0, 2.0));
    let b = p.push_clip(MAIN, color(5.0, 2.0));
    let o = p.push_clip(OVERLAY, color(6.0, 1.0));
    let after = assert_reversible(&mut SetMagnetic::new(true), &p);
    assert!(after.magnetic);
    assert_eq!(clip(&after, a).start, t(0.0));
    assert_eq!(clip(&after, b).start, t(2.0));
    assert_eq!(clip(&after, o).start, t(3.0));
    let back = assert_reversible(&mut SetMagnetic::new(false), &after);
    assert!(!back.magnetic);
    assert_eq!(back.tracks, after.tracks);
}

#[test]
fn magnetic_on_fails_when_a_locked_main_track_would_move() {
    let mut p = empty();
    p.magnetic = false;
    p.push_clip(MAIN, color(1.0, 2.0));
    p.tracks[MAIN].locked = true;
    assert!(matches!(
        try_apply(&p, SetMagnetic::new(true)),
        Err(CommandError::TrackLocked(_))
    ));
}
