use super::*;
use crate::Document;
use crate::command::test_support::*;

fn frame(p: &Project) -> Time {
    p.settings.frame_rate.frame_duration()
}

fn freeze_at(clip: Option<ClipId>, at: Time) -> InsertFreezeFrame {
    InsertFreezeFrame::new(clip, at, t(1.0))
}

#[test]
fn a_freeze_splits_the_main_clip_and_ripples_the_rest() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.5));
    let c = p.push_clip(MAIN, color(4.0, 2.0));
    // Over source second 3.5, in what becomes the right half.
    let o = p.push_clip(OVERLAY, color(3.0, 0.5));
    let mut command = freeze_at(None, t(1.5) + Time(1_000));
    let id = command.clip_id();
    let after = assert_reversible(&mut command, &p);
    let main = &after.tracks[MAIN];
    assert_eq!(main.clips.len(), 4);
    let (left, freeze, right) = (&main.clips[0], &main.clips[1], &main.clips[2]);
    assert_eq!((left.id, left.duration), (v, t(1.5)));
    assert_eq!(freeze.id, id);
    assert_eq!((freeze.start, freeze.duration), (t(1.5), t(1.0)));
    let expected = clip(&p, v).frame_sample_time(t(1.5), frame(&p));
    assert_eq!(
        freeze.source,
        ClipSource::Freeze {
            media: p.media[0].id,
            source_time: expected.unwrap(),
        }
    );
    assert_eq!(expected, Some(t(2.0) + Time(frame(&p).flicks() / 2)));
    assert_eq!((freeze.audio.as_ref(), freeze.link), (None, None));
    assert_eq!(freeze.visual, clip(&p, v).visual);
    assert_eq!(freeze.name, clip(&p, v).name);
    assert_eq!((right.start, right.source_in()), (t(2.5), t(2.0)));
    assert_eq!(clip(&after, c).start, t(5.0));
    assert_eq!(
        clip(&after, o).start,
        t(4.0),
        "the overlay keeps its content"
    );
}

#[test]
fn a_freeze_at_the_first_frame_goes_before_the_clip_and_at_the_last_after_it() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.5));
    let after = assert_reversible(&mut freeze_at(Some(v), Time(5)), &p);
    let main = &after.tracks[MAIN];
    assert_eq!(main.clips.len(), 2);
    assert!(matches!(main.clips[0].source, ClipSource::Freeze { .. }));
    assert_eq!(main.clips[0].start, t(0.0));
    assert_eq!(
        (clip(&after, v).start, clip(&after, v).duration),
        (t(1.0), t(4.0))
    );

    // A clip that ends inside its last frame: the freeze goes after it.
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.5));
    p.tracks[MAIN].clips[0].duration = t(4.0) - Time(1_000);
    let c = p.push_clip(MAIN, color(4.0, 1.0));
    p.tracks[MAIN].clips[1].start = t(4.0) - Time(1_000);
    let end = clip(&p, v).end();
    let after = assert_reversible(&mut freeze_at(None, end - Time(1)), &p);
    let main = &after.tracks[MAIN];
    assert_eq!(main.clips.len(), 3);
    assert_eq!(main.clips[0].id, v);
    assert_eq!(main.clips[1].start, end);
    assert_eq!(clip(&after, c).start, end + t(1.0));
}

#[test]
fn detached_audio_right_half_shifts_and_leaves_silence_under_the_freeze() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    link(&mut p, &[v, s]);
    let after = assert_reversible(&mut freeze_at(Some(v), t(2.0)), &p);
    let audio = &after.tracks[AUDIO];
    assert_eq!(audio.clips.len(), 2);
    assert_eq!(
        (audio.clips[0].start, audio.clips[0].end()),
        (t(0.0), t(2.0))
    );
    assert_eq!(
        (audio.clips[1].start, audio.clips[1].end()),
        (t(3.0), t(5.0))
    );
    let main = &after.tracks[MAIN];
    assert_eq!(main.clips[2].start, t(3.0));
    assert_eq!(
        main.clips[2].link, audio.clips[1].link,
        "the right halves stay linked"
    );
}

#[test]
fn a_non_magnetic_main_track_ripples_its_later_clips() {
    let mut p = empty();
    p.magnetic = false;
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let c = p.push_clip(MAIN, color(6.0, 2.0));
    let after = assert_reversible(&mut freeze_at(Some(v), t(2.0)), &p);
    assert_eq!(clip(&after, c).start, t(7.0));
}

#[test]
fn a_freeze_on_an_overlay_track_ripples_that_track() {
    let mut p = empty();
    p.push_clip(MAIN, color(0.0, 10.0));
    let a = p.push_clip(OVERLAY, video(&p, 0.0, 2.0, 0.0));
    let b = p.push_clip(OVERLAY, color(2.0, 1.0));
    let after = assert_reversible(&mut freeze_at(Some(a), t(1.0)), &p);
    let overlay = &after.tracks[OVERLAY];
    assert_eq!(overlay.clips.len(), 4);
    assert_eq!(starts(overlay), vec![t(0.0), t(1.0), t(2.0), t(3.0)]);
    assert_eq!(clip(&after, b).start, t(3.0));
    assert_eq!(after.tracks[MAIN], p.tracks[MAIN]);

    // A shifted linked member that runs into a clip on its own track fails.
    let mut p = empty();
    p.push_clip(MAIN, color(0.0, 10.0));
    let a = p.push_clip(OVERLAY, video(&p, 0.0, 2.0, 0.0));
    let b = p.push_clip(OVERLAY, color(2.0, 1.0));
    let sb = p.push_clip(AUDIO, sound(&p, 2.0, 1.0, 0.0));
    link(&mut p, &[b, sb]);
    p.push_clip(AUDIO, sound(&p, 3.5, 1.0, 0.0));
    assert_eq!(
        try_apply(&p, freeze_at(Some(a), t(1.0))),
        Err(CommandError::Overlap)
    );
}

#[test]
fn only_video_media_clips_freeze() {
    let mut p = empty();
    let i = p.push_clip(MAIN, image(&p, 0.0, 2.0));
    let c = p.push_clip(MAIN, color(2.0, 2.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 2.0, 0.0));
    for (clip, at) in [(Some(i), 1.0), (Some(c), 3.0), (None, 3.0), (Some(s), 1.0)] {
        assert_eq!(
            try_apply(&p, freeze_at(clip, t(at))),
            Err(CommandError::PropertyNotApplicable),
            "{clip:?}"
        );
    }
    assert_eq!(
        try_apply(&p, freeze_at(None, t(9.0))),
        Err(CommandError::NothingToEdit)
    );
    assert_eq!(
        try_apply(&p, freeze_at(Some(i), t(3.0))),
        Err(CommandError::NothingToEdit),
        "the clip is not under the playhead"
    );
    let v = p.push_clip(OVERLAY, video(&p, 0.0, 2.0, 0.0));
    assert_eq!(
        try_apply(&p, InsertFreezeFrame::new(Some(v), t(1.0), Time::ZERO)),
        Err(CommandError::InvalidValue)
    );
    p.tracks[OVERLAY].locked = true;
    assert_eq!(
        try_apply(&p, freeze_at(Some(v), t(1.0))),
        Err(CommandError::TrackLocked(p.tracks[OVERLAY].id))
    );
}

#[test]
fn a_freeze_of_a_fast_reversed_clip_shows_the_frame_the_preview_shows() {
    let mut p = empty();
    let v = p.push_clip(MAIN, with_speed(video(&p, 0.0, 2.0, 0.5), 200, true));
    let at = t(0.7);
    let rate = p.settings.frame_rate;
    let t_n = Time::from_frames(at.to_frames(rate), rate);
    let shown = clip(&p, v).frame_sample_time(t_n, frame(&p)).unwrap();
    let after = try_apply(&p, freeze_at(None, at)).unwrap();
    let freeze = &after.tracks[MAIN].clips[1];
    assert_eq!(freeze.source_time(freeze.start), Some(shown));
    // Both halves keep speed and direction, and their content stays in place.
    let right = &after.tracks[MAIN].clips[2];
    assert_eq!((right.speed(), right.reversed()), (speed(200), true));
    let probe = t_n + t(0.5);
    assert_eq!(
        right.source_time(probe + t(1.0)),
        clip(&p, v).source_time(probe)
    );
}

#[test]
fn one_undo_restores_exactly_and_redo_recreates_the_same_clips() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    link(&mut p, &[v, s]);
    let mut doc = Document::new(p.clone());
    doc.execute(Box::new(freeze_at(None, t(2.0)))).unwrap();
    let done = doc.project().clone();
    assert_eq!(doc.undo_label(), Some(HistoryLabel::InsertFreezeFrame));
    assert!(doc.undo());
    assert_eq!(doc.project(), &p);
    assert!(doc.redo());
    assert_eq!(doc.project(), &done);
}
