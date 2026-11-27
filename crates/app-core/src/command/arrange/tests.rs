use proptest::prelude::*;

use super::*;
use crate::Document;
use crate::EditPhase;
use crate::media::samples;
use crate::model::{Rgba8, TrackKind};
use crate::props::{AudioProps, VisualProps};
use crate::time::FLICKS_PER_SECOND;

const S: i64 = FLICKS_PER_SECOND;

fn t(seconds: f64) -> Time {
    Time((seconds * S as f64).round() as i64)
}

/// Apply → revert gives the original, re-apply gives the same result. Returns that result.
fn assert_reversible(command: &mut dyn Command, project: &Project) -> Project {
    let mut edited = project.clone();
    command.apply(&mut edited).expect("apply");
    assert_eq!(edited.validate(), Ok(()));
    let after = edited.clone();
    command.revert(&mut edited);
    assert_eq!(&edited, project, "revert restores the original");
    command.apply(&mut edited).expect("re-apply");
    assert_eq!(edited, after, "re-apply reproduces the edit");
    after
}

/// Runs `command` through a document and checks that a failure changes nothing.
fn try_apply(project: &Project, command: impl Command + 'static) -> Result<Project, CommandError> {
    let mut doc = Document::new(project.clone());
    let result = doc.execute(Box::new(command));
    if result.is_err() {
        assert_eq!(doc.project(), project, "a failed edit changes nothing");
    }
    result.map(|()| doc.project().clone())
}

/// Tracks: 0 overlay "Video 2", 1 main "Video 1", 2 "Audio 1". Media: 5 s video with audio, 3 s
/// song, a photo. No clips.
fn empty() -> Project {
    let mut project = Project::new("Arrange");
    project.media = vec![samples::video(1), samples::audio(2), samples::image(3)];
    project
        .tracks
        .insert(0, Track::new(TrackKind::Video, "Video 2"));
    project
}

const OVERLAY: usize = 0;
const MAIN: usize = 1;
const AUDIO: usize = 2;

fn color(start: f64, duration: f64) -> Clip {
    Clip::color("card", t(start), t(duration), Rgba8::MIGRATED_GRAY)
}

/// A clip of the 5 s video (with its sound) on a video track.
fn video(project: &Project, start: f64, duration: f64, source_in: f64) -> Clip {
    Clip {
        id: ClipId::new(),
        name: "clip".into(),
        start: t(start),
        duration: t(duration),
        source: ClipSource::media_at(project.media[0].id, t(source_in)),
        link: None,
        visual: Some(VisualProps::default()),
        audio: Some(AudioProps::default()),
    }
}

/// The video's sound on an audio track (as a detached clip would be).
fn sound(project: &Project, start: f64, duration: f64, source_in: f64) -> Clip {
    Clip {
        visual: None,
        ..video(project, start, duration, source_in)
    }
}

fn image(project: &Project, start: f64, duration: f64) -> Clip {
    Clip {
        source: ClipSource::media_at(project.media[2].id, Time::ZERO),
        audio: None,
        ..video(project, start, duration, 0.0)
    }
}

trait PushClip {
    fn push_clip(&mut self, track: usize, clip: Clip) -> ClipId;
}

impl PushClip for Project {
    fn push_clip(&mut self, track: usize, clip: Clip) -> ClipId {
        let id = clip.id;
        place_clip(&mut self.tracks[track], clip).expect("free");
        id
    }
}

fn link(project: &mut Project, ids: &[ClipId]) {
    let link = LinkId::new();
    for track in &mut project.tracks {
        for clip in &mut track.clips {
            if ids.contains(&clip.id) {
                clip.link = Some(link);
            }
        }
    }
}

/// Main track: A, B, C, 5 s colour cards each.
fn three_on_main() -> (Project, [ClipId; 3]) {
    let mut p = empty();
    let a = p.push_clip(MAIN, color(0.0, 5.0));
    let b = p.push_clip(MAIN, color(5.0, 5.0));
    let c = p.push_clip(MAIN, color(10.0, 5.0));
    assert_eq!(p.validate(), Ok(()));
    (p, [a, b, c])
}

fn ids(track: &Track) -> Vec<ClipId> {
    track.clips.iter().map(|c| c.id).collect()
}

fn starts(track: &Track) -> Vec<Time> {
    track.clips.iter().map(|c| c.start).collect()
}

fn clip(project: &Project, id: ClipId) -> &Clip {
    project.clip(id).expect("clip exists").1
}

fn track_of(project: &Project, id: ClipId) -> usize {
    project.clip(id).expect("clip exists").0
}

fn existing(project: &Project, index: usize) -> TrackTarget {
    TrackTarget::Existing {
        track: project.tracks[index].id,
    }
}

fn mv(clips: &[ClipId], anchor: ClipId, start: Time, track: TrackTarget) -> MoveClips {
    MoveClips::new(clips.to_vec(), anchor, start, track, true)
}

// ---- MoveClips ----------------------------------------------------------------------------

#[test]
fn moving_on_the_magnetic_main_track_reorders_by_midpoint() {
    let (p, [a, b, c]) = three_on_main();
    let main = existing(&p, MAIN);
    // A's start at 7 s is before B's midpoint (7.5 s): no change.
    let after = assert_reversible(&mut mv(&[a], a, t(7.0), main), &p);
    assert_eq!(ids(&after.tracks[MAIN]), [a, b, c]);
    // At 8 s it passes B's midpoint and lands between B and C.
    let after = assert_reversible(&mut mv(&[a], a, t(8.0), main), &p);
    assert_eq!(ids(&after.tracks[MAIN]), [b, a, c]);
    assert_eq!(starts(&after.tracks[MAIN]), [t(0.0), t(5.0), t(10.0)]);
    // Far right or negative: last or first.
    let after = assert_reversible(&mut mv(&[a], a, t(99.0), main), &p);
    assert_eq!(ids(&after.tracks[MAIN]), [b, c, a]);
    let after = assert_reversible(&mut mv(&[c], c, t(-3.0), main), &p);
    assert_eq!(ids(&after.tracks[MAIN]), [c, a, b]);
}

#[test]
fn several_main_clips_move_as_a_block() {
    let (p, [a, b, c]) = three_on_main();
    let after = assert_reversible(&mut mv(&[a, c], a, t(8.0), existing(&p, MAIN)), &p);
    assert_eq!(ids(&after.tracks[MAIN]), [b, a, c]);
}

#[test]
fn a_move_into_an_occupied_spot_is_rejected() {
    let mut p = empty();
    let x = p.push_clip(OVERLAY, color(0.0, 5.0));
    let y = p.push_clip(OVERLAY, color(6.0, 5.0));
    let overlay = existing(&p, OVERLAY);
    assert_eq!(
        try_apply(&p, mv(&[y], y, t(3.0), overlay)).unwrap_err(),
        CommandError::Overlap
    );
    assert_eq!(
        try_apply(&p, mv(&[x], x, t(-1.0), overlay)).unwrap_err(),
        CommandError::Overlap,
        "nothing starts before 0"
    );
    let after = assert_reversible(&mut mv(&[y], y, t(5.0), overlay), &p);
    assert_eq!(clip(&after, y).start, t(5.0), "touching is fine");
    // Both together keep their offset.
    let after = assert_reversible(&mut mv(&[x, y], y, t(8.0), overlay), &p);
    assert_eq!(starts(&after.tracks[OVERLAY]), [t(2.0), t(8.0)]);
}

#[test]
fn a_non_magnetic_main_track_keeps_gaps_and_rejects_overlaps() {
    let (mut p, [a, b, _]) = three_on_main();
    p.magnetic = false;
    let main = existing(&p, MAIN);
    assert_eq!(
        try_apply(&p, mv(&[a], a, t(2.0), main)).unwrap_err(),
        CommandError::Overlap
    );
    let mut q = p.clone();
    q.tracks[MAIN].clips.pop();
    let after = assert_reversible(&mut mv(&[b], b, t(12.0), main), &q);
    assert_eq!(starts(&after.tracks[MAIN]), [t(0.0), t(12.0)]);
}

#[test]
fn vertical_moves_between_overlays_new_tracks_and_the_main_track() {
    let (mut p, [a, b, c]) = three_on_main();
    let x = p.push_clip(OVERLAY, color(1.0, 2.0));

    // Overlay → main: inserted at the midpoint index, the main track repacks.
    let after = assert_reversible(&mut mv(&[x], x, t(5.0), existing(&p, MAIN)), &p);
    let main = after.main().unwrap();
    assert_eq!(ids(main), [a, x, b, c]);
    assert_eq!(starts(main), [t(0.0), t(5.0), t(7.0), t(12.0)]);
    assert_eq!(
        after.tracks.len(),
        2,
        "the emptied overlay is removed on commit"
    );

    // Main → a new overlay above everything: the main track closes the gap.
    let mut command = mv(&[b], b, t(6.0), TrackTarget::New { index: 0 });
    let after = assert_reversible(&mut command, &p);
    assert_eq!(after.tracks.len(), 4);
    assert_eq!(ids(&after.tracks[0]), [b]);
    assert_eq!(clip(&after, b).start, t(6.0));
    assert_eq!(starts(after.main().unwrap()), [t(0.0), t(5.0)]);

    // A video clip cannot go to an audio track.
    assert_eq!(
        try_apply(&p, mv(&[x], x, t(1.0), existing(&p, AUDIO))).unwrap_err(),
        CommandError::TrackKindMismatch
    );
    // A new track for a video clip is clamped into the overlay section.
    let after = assert_reversible(&mut mv(&[x], x, t(1.0), TrackTarget::New { index: 9 }), &p);
    assert_eq!(
        after.tracks[0].clips[0].id, x,
        "the emptied overlay is gone"
    );
    assert_eq!(after.tracks[1].id, after.main_track);
}

#[test]
fn clips_on_several_tracks_move_in_time_only() {
    let mut p = empty();
    let x = p.push_clip(OVERLAY, color(1.0, 2.0));
    let m = p.push_clip(MAIN, color(0.0, 5.0));
    let _ = m;
    p.magnetic = false;
    let y = p.push_clip(OVERLAY + 1, color(6.0, 1.0));
    let after = assert_reversible(
        &mut mv(&[x, y], x, t(2.0), TrackTarget::New { index: 0 }),
        &p,
    );
    assert_eq!(after.tracks.len(), 3, "no track created");
    assert_eq!(clip(&after, x).start, t(2.0));
    assert_eq!(clip(&after, y).start, t(7.0));
    assert_eq!(track_of(&after, y), MAIN);
}

#[test]
fn linked_clips_follow_in_time_on_their_own_tracks() {
    let mut p = empty();
    p.magnetic = false;
    let v = p.push_clip(MAIN, video(&p, 0.0, 5.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 5.0, 0.0));
    link(&mut p, &[v, s]);

    // Only the video is listed; the sound comes with it.
    let after = assert_reversible(&mut mv(&[v], v, t(2.0), existing(&p, MAIN)), &p);
    assert_eq!(clip(&after, v).start, t(2.0));
    assert_eq!(clip(&after, s).start, t(2.0));
    assert_eq!(track_of(&after, s), AUDIO);

    // Dragging the video to the overlay moves only the video vertically.
    let after = assert_reversible(&mut mv(&[v, s], v, t(3.0), existing(&p, OVERLAY)), &p);
    assert_eq!(track_of(&after, v), OVERLAY);
    assert_eq!(track_of(&after, s), AUDIO);
    assert_eq!(clip(&after, s).start, t(3.0));

    // A member running into a clip on its own track fails the whole move.
    let mut q = p.clone();
    q.push_clip(AUDIO, sound(&q, 6.0, 1.0, 0.0));
    assert_eq!(
        try_apply(&q, mv(&[v], v, t(4.0), existing(&q, MAIN))).unwrap_err(),
        CommandError::Overlap
    );
}

#[test]
fn linked_members_of_a_reordered_main_clip_shift_by_its_actual_change() {
    let (mut p, [a, b, c]) = three_on_main();
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 3.0, 0.0));
    link(&mut p, &[a, s]);
    assert_eq!(p.validate(), Ok(()));
    // Dragging the sound by 8 s moves A to the slot after B (start 5 s): the sound goes with A.
    let after = assert_reversible(&mut mv(&[s], s, t(8.0), existing(&p, AUDIO)), &p);
    assert_eq!(ids(after.main().unwrap()), [b, a, c]);
    assert_eq!(clip(&after, a).start, t(5.0));
    assert_eq!(clip(&after, s).start, t(5.0));
}

#[test]
fn followers_move_with_their_main_clip_and_locked_tracks_stay() {
    let (mut p, [a, b, c]) = three_on_main();
    // B-roll 1 s into A, music from 0 on a locked track, an end card after the main track.
    let broll = p.push_clip(OVERLAY, color(1.0, 2.0));
    let card = p.push_clip(OVERLAY, color(16.0, 2.0));
    let music = p.push_clip(
        AUDIO,
        Clip {
            visual: None,
            audio: Some(AudioProps::default()),
            source: ClipSource::media_at(p.media[1].id, Time::ZERO),
            ..video(&p, 0.0, 3.0, 0.0)
        },
    );
    p.tracks[AUDIO].locked = true;
    assert_eq!(p.validate(), Ok(()));

    let after = assert_reversible(&mut mv(&[a], a, t(99.0), existing(&p, MAIN)), &p);
    assert_eq!(ids(after.main().unwrap()), [b, c, a]);
    assert_eq!(
        clip(&after, broll).start,
        t(11.0),
        "B-roll stays 1 s into A"
    );
    assert_eq!(
        clip(&after, card).start,
        t(16.0),
        "the main track's end did not change"
    );
    assert_eq!(clip(&after, music).start, Time::ZERO);
    assert_eq!(
        after.tracks[AUDIO], p.tracks[AUDIO],
        "locked track is untouched"
    );

    // Moving A off the main track: B-roll anchored inside A moves with... A's removal point.
    let after = assert_reversible(&mut mv(&[a], a, t(20.0), existing(&p, OVERLAY)), &p);
    assert_eq!(starts(after.main().unwrap()), [t(0.0), t(5.0)]);
    assert_eq!(
        clip(&after, broll).start,
        Time::ZERO,
        "to the removal point"
    );
    assert_eq!(
        clip(&after, card).start,
        t(11.0),
        "end card keeps its distance to the end"
    );
}

#[test]
fn locked_tracks_reject_moves() {
    let (mut p, [a, ..]) = three_on_main();
    let x = p.push_clip(OVERLAY, color(1.0, 2.0));
    let mut locked = p.clone();
    locked.tracks[OVERLAY].locked = true;
    let overlay_id = locked.tracks[OVERLAY].id;
    assert_eq!(
        try_apply(&locked, mv(&[x], x, t(3.0), existing(&locked, OVERLAY))).unwrap_err(),
        CommandError::TrackLocked(overlay_id)
    );
    assert_eq!(
        try_apply(&locked, mv(&[a], a, t(3.0), existing(&locked, OVERLAY))).unwrap_err(),
        CommandError::TrackLocked(overlay_id),
        "a locked target"
    );
    let missing = ClipId::new();
    assert_eq!(
        try_apply(&p, mv(&[missing], a, t(3.0), existing(&p, MAIN))).unwrap_err(),
        CommandError::ClipNotFound(missing)
    );
}

// ---- TrimClip -----------------------------------------------------------------------------

fn trim(clip: ClipId, edge: Edge, time: Time) -> TrimClip {
    TrimClip::new(clip, edge, time, true)
}

#[test]
fn trims_are_clamped_to_the_source_and_one_frame() {
    let mut p = empty();
    let v = p.push_clip(OVERLAY, video(&p, 2.0, 3.0, 1.0));
    let frame = p.settings.frame_rate.frame_duration();

    let after = assert_reversible(&mut trim(v, Edge::End, t(99.0)), &p);
    assert_eq!(clip(&after, v).duration, t(4.0), "source end is 5 s");
    let after = assert_reversible(&mut trim(v, Edge::End, t(0.0)), &p);
    assert_eq!(clip(&after, v).duration, frame);

    let after = assert_reversible(&mut trim(v, Edge::Start, t(2.5)), &p);
    let c = clip(&after, v);
    assert_eq!(
        (c.start, c.duration, c.source_in()),
        (t(2.5), t(2.5), t(1.5))
    );
    let after = assert_reversible(&mut trim(v, Edge::Start, t(0.0)), &p);
    let c = clip(&after, v);
    assert_eq!(
        (c.start, c.source_in()),
        (t(1.0), Time::ZERO),
        "source_in stops at 0"
    );
    let after = assert_reversible(&mut trim(v, Edge::Start, t(99.0)), &p);
    let c = clip(&after, v);
    assert_eq!((c.end(), c.duration), (t(5.0), frame));
}

#[test]
fn images_and_colour_cards_have_no_source_limit() {
    let mut p = empty();
    let i = p.push_clip(OVERLAY, image(&p, 2.0, 3.0));
    let k = p.push_clip(OVERLAY, color(10.0, 1.0));
    let after = assert_reversible(&mut trim(i, Edge::Start, t(0.0)), &p);
    let c = clip(&after, i);
    assert_eq!(
        (c.start, c.duration, c.source_in()),
        (Time::ZERO, t(5.0), Time::ZERO)
    );
    let after = assert_reversible(&mut trim(k, Edge::End, t(60.0)), &p);
    assert_eq!(clip(&after, k).end(), t(60.0));
}

#[test]
fn trims_stop_at_neighbours() {
    let mut p = empty();
    let x = p.push_clip(OVERLAY, color(0.0, 2.0));
    let y = p.push_clip(OVERLAY, color(3.0, 5.0));
    let after = assert_reversible(&mut trim(x, Edge::End, t(5.0)), &p);
    assert_eq!(clip(&after, x).end(), t(3.0));
    let after = assert_reversible(&mut trim(y, Edge::Start, t(1.0)), &p);
    assert_eq!(clip(&after, y).start, t(2.0));
}

#[test]
fn trims_on_the_magnetic_main_track_ripple_and_followers_move() {
    let mut p = empty();
    let a = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let b = p.push_clip(MAIN, color(4.0, 5.0));
    // B-roll 1 s into B, a follower 3 s into A.
    let over_b = p.push_clip(OVERLAY, color(5.0, 1.0));
    let over_a = p.push_clip(OVERLAY, color(3.0, 1.0));

    // End trim: B ripples, the neighbour does not limit it.
    let after = assert_reversible(&mut trim(a, Edge::End, t(9.0)), &p);
    assert_eq!(clip(&after, a).duration, t(5.0), "source end");
    assert_eq!(clip(&after, b).start, t(5.0));
    assert_eq!(
        clip(&after, over_b).start,
        t(6.0),
        "B-roll stays 1 s into B"
    );
    assert_eq!(clip(&after, over_a).start, t(3.0));

    // Start trim: A keeps its start, its content shifts, B ripples left.
    let after = assert_reversible(&mut trim(a, Edge::Start, t(1.0)), &p);
    let c = clip(&after, a);
    assert_eq!(
        (c.start, c.duration, c.source_in()),
        (Time::ZERO, t(3.0), t(1.0))
    );
    assert_eq!(clip(&after, b).start, t(3.0));
    assert_eq!(clip(&after, over_b).start, t(4.0));
    assert_eq!(
        clip(&after, over_a).start,
        t(2.0),
        "the follower stays on the same moment of A's content"
    );
}

#[test]
fn linked_members_trim_together_clamped_to_the_tightest() {
    let mut p = empty();
    p.magnetic = false;
    let v = p.push_clip(MAIN, video(&p, 0.0, 3.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 3.0, 0.0));
    let blocker = p.push_clip(AUDIO, sound(&p, 4.0, 1.0, 0.0));
    link(&mut p, &[v, s]);
    let _ = blocker;
    let after = assert_reversible(&mut trim(v, Edge::End, t(5.0)), &p);
    assert_eq!(
        clip(&after, v).end(),
        t(4.0),
        "the sound's neighbour limits the group"
    );
    assert_eq!(clip(&after, s).end(), t(4.0));

    // A member whose edge is elsewhere is not trimmed.
    let mut q = p.clone();
    let shorter = &mut q.tracks[AUDIO].clips[0];
    shorter.duration = t(2.0);
    let after = assert_reversible(&mut trim(v, Edge::End, t(3.5)), &q);
    assert_eq!(clip(&after, v).end(), t(3.5));
    assert_eq!(clip(&after, s).end(), t(2.0));

    // A locked member fails the trim.
    let mut locked = p.clone();
    locked.tracks[AUDIO].locked = true;
    assert_eq!(
        try_apply(&locked, trim(v, Edge::End, t(3.5))).unwrap_err(),
        CommandError::TrackLocked(locked.tracks[AUDIO].id)
    );
}

#[test]
fn linked_members_of_a_rippling_start_trim_keep_their_start() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    link(&mut p, &[v, s]);
    let after = assert_reversible(&mut trim(s, Edge::Start, t(1.0)), &p);
    for id in [v, s] {
        let c = clip(&after, id);
        assert_eq!(
            (c.start, c.duration, c.source_in()),
            (Time::ZERO, t(3.0), t(1.0))
        );
    }
}

#[test]
fn trims_shorten_fades_to_fit() {
    let mut p = empty();
    let mut c = sound(&p, 0.0, 4.0, 0.0);
    let audio = c.audio.as_mut().unwrap();
    audio.fade_in = t(2.0);
    audio.fade_out = t(2.0);
    let s = p.push_clip(AUDIO, c);
    let after = assert_reversible(&mut trim(s, Edge::End, t(3.0)), &p);
    let audio = clip(&after, s).audio.as_ref().unwrap();
    assert_eq!((audio.fade_in, audio.fade_out), (t(2.0), t(1.0)));
}

// ---- Gestures (ADR-0006) ------------------------------------------------------------------

/// A drag as the IPC handler runs it: updates keep emptied tracks, the commit removes them.
fn drag(doc: &mut Document, make: impl Fn(bool) -> Box<dyn Command>, updates: usize) {
    for _ in 0..updates {
        doc.edit(EditPhase::Update, make(false)).expect("update");
    }
    doc.edit(EditPhase::Commit, make(true)).expect("commit");
}

#[test]
fn a_drag_of_any_length_is_one_history_entry() {
    let (mut p, [a, b, c]) = three_on_main();
    let x = p.push_clip(OVERLAY, color(1.0, 2.0));
    let mut doc = Document::new(p.clone());
    for step in 0..100 {
        let start = t(f64::from(step) * 0.1);
        doc.edit(
            EditPhase::Update,
            Box::new(MoveClips::new(vec![a], a, start, existing(&p, MAIN), false)),
        )
        .unwrap();
    }
    doc.edit(
        EditPhase::Commit,
        Box::new(MoveClips::new(vec![a], a, t(8.0), existing(&p, MAIN), true)),
    )
    .unwrap();
    assert_eq!(doc.history_len(), 1);
    assert_eq!(doc.undo_label(), Some(HistoryLabel::MoveClips));
    assert_eq!(ids(&doc.project().tracks[MAIN]), [b, a, c]);
    assert!(doc.undo());
    assert_eq!(doc.project(), &p);

    drag(
        &mut doc,
        |last| Box::new(TrimClip::new(x, Edge::End, t(4.0), last)),
        50,
    );
    assert_eq!(doc.history_len(), 1, "the redo entry was replaced");
    assert_eq!(doc.undo_label(), Some(HistoryLabel::TrimClip));
    assert_eq!(clip(doc.project(), x).end(), t(4.0));
}

#[test]
fn an_update_into_an_overlap_keeps_the_last_valid_state_and_cancel_restores() {
    let mut p = empty();
    let x = p.push_clip(OVERLAY, color(0.0, 2.0));
    p.push_clip(OVERLAY, color(5.0, 2.0));
    let overlay = existing(&p, OVERLAY);
    let mut doc = Document::new(p.clone());
    doc.edit(
        EditPhase::Update,
        Box::new(MoveClips::new(vec![x], x, t(2.0), overlay, false)),
    )
    .unwrap();
    let valid = doc.project().clone();
    assert_eq!(
        doc.edit(
            EditPhase::Update,
            Box::new(MoveClips::new(vec![x], x, t(4.0), overlay, false))
        ),
        Err(CommandError::Overlap)
    );
    assert_eq!(doc.project(), &valid);
    assert!(doc.has_gesture());
    assert!(doc.gesture_cancel());
    assert_eq!(doc.project(), &p);
    assert_eq!(doc.history_len(), 0);
}

#[test]
fn emptied_tracks_are_removed_on_commit_only() {
    let (mut p, _) = three_on_main();
    let x = p.push_clip(OVERLAY, color(1.0, 2.0));
    let mut doc = Document::new(p.clone());
    let to_new = |last| -> Box<dyn Command> {
        Box::new(MoveClips::new(
            vec![x],
            x,
            t(1.0),
            TrackTarget::New { index: 0 },
            last,
        ))
    };
    doc.edit(EditPhase::Update, to_new(false)).unwrap();
    assert_eq!(
        doc.project().tracks.len(),
        4,
        "the emptied overlay stays during the drag"
    );
    assert!(doc.project().tracks[1].clips.is_empty());
    doc.edit(EditPhase::Commit, to_new(true)).unwrap();
    assert_eq!(doc.project().tracks.len(), 3);
    assert_eq!(doc.project().tracks[0].clips[0].id, x);
    assert!(doc.undo());
    assert_eq!(doc.project(), &p);
}

// ---- Properties -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Op {
    Move {
        clip: usize,
        start_ms: i64,
        track: usize,
        updates: usize,
        cancel: bool,
    },
    Trim {
        clip: usize,
        end: bool,
        time_ms: i64,
        updates: usize,
        cancel: bool,
    },
    Undo,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..32usize, -2_000..40_000i64, 0..5usize, 0..4usize, prop::bool::weighted(0.2))
            .prop_map(|(clip, start_ms, track, updates, cancel)| Op::Move { clip, start_ms, track, updates, cancel }),
        4 => (0..32usize, prop::bool::ANY, -2_000..40_000i64, 0..4usize, prop::bool::weighted(0.2))
            .prop_map(|(clip, end, time_ms, updates, cancel)| Op::Trim { clip, end, time_ms, updates, cancel }),
        1 => Just(Op::Undo),
    ]
}

/// A project with video and colour clips on the main track, overlays (two linked to main clips
/// through sound on the audio track) and some sound.
fn sample(magnetic: bool, lengths: &[u8]) -> Project {
    let mut p = empty();
    p.magnetic = magnetic;
    let mut at = 0.0;
    for (i, &len) in lengths.iter().enumerate() {
        let len = 0.5 + f64::from(len % 8) * 0.5;
        let gap = if magnetic {
            0.0
        } else {
            f64::from(len as u8 % 2)
        };
        at += gap;
        if i % 2 == 0 {
            let v = p.push_clip(MAIN, video(&p, at, len.min(4.0), 0.5));
            if i % 4 == 0 {
                let s = p.push_clip(AUDIO, sound(&p, at, len.min(4.0), 0.5));
                link(&mut p, &[v, s]);
            }
            at += len.min(4.0);
        } else {
            p.push_clip(MAIN, color(at, len));
            p.push_clip(OVERLAY, color(at + 0.25, 0.25));
            at += len;
        }
    }
    assert_eq!(p.validate(), Ok(()));
    p
}

fn clip_ids(project: &Project) -> Vec<ClipId> {
    let mut ids: Vec<ClipId> = project.clips().map(|c| c.id).collect();
    ids.sort();
    ids
}

/// Builds one gesture step: (is the commit, jitter in ms).
type MakeEdit = Box<dyn Fn(bool, i64) -> Box<dyn Command>>;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// Random drags never break an invariant or lose a clip, each committed drag is one history
    /// entry, trims stay inside the source, and undoing everything restores the start.
    #[test]
    fn random_drags_keep_the_project_valid(
        magnetic in prop::bool::ANY,
        lengths in prop::collection::vec(0u8..16, 2..8),
        ops in prop::collection::vec(op(), 1..24),
    ) {
        let start = sample(magnetic, &lengths);
        let mut doc = Document::new(start.clone());
        let all = clip_ids(&start);
        let mut entries = 0usize;
        for op in ops {
            let project = doc.project().clone();
            let pick = |i: usize| {
                let clips: Vec<ClipId> = project.clips().map(|c| c.id).collect();
                clips[i % clips.len()]
            };
            let track_at = |i: usize| -> TrackTarget {
                match project.tracks.get(i) {
                    Some(track) => TrackTarget::Existing { track: track.id },
                    None => TrackTarget::New { index: 0 },
                }
            };
            let (make, updates, cancel): (MakeEdit, usize, bool) = match op {
                Op::Move { clip, start_ms, track, updates, cancel } => {
                    let id = pick(clip);
                    let target = track_at(track);
                    (Box::new(move |last, jitter| Box::new(MoveClips::new(vec![id], id, Time((start_ms + jitter) * S / 1000), target, last))), updates, cancel)
                }
                Op::Trim { clip, end, time_ms, updates, cancel } => {
                    let id = pick(clip);
                    let edge = if end { Edge::End } else { Edge::Start };
                    (Box::new(move |last, jitter| Box::new(TrimClip::new(id, edge, Time((time_ms + jitter) * S / 1000), last))), updates, cancel)
                }
                Op::Undo => {
                    if doc.undo() {
                        entries -= 1;
                    }
                    continue;
                }
            };
            for step in 0..updates {
                let before = doc.project().clone();
                if doc.edit(EditPhase::Update, make(false, step as i64 * 100)).is_err() {
                    prop_assert_eq!(doc.project(), &before, "a failed update keeps the draft");
                }
                prop_assert_eq!(doc.project().validate(), Ok(()));
            }
            if cancel {
                doc.gesture_cancel();
                prop_assert_eq!(doc.project(), &project, "cancel restores the base");
                continue;
            }
            match doc.edit(EditPhase::Commit, make(true, 0)) {
                Ok(()) => entries += 1,
                Err(_) => {
                    doc.gesture_cancel();
                    prop_assert_eq!(doc.project(), &project);
                }
            }
            prop_assert!(!doc.has_gesture());
            prop_assert_eq!(doc.history_cursor(), entries, "one entry per committed drag");
            let now = doc.project();
            prop_assert_eq!(now.validate(), Ok(()));
            prop_assert_eq!(clip_ids(now), all.clone(), "no clip is lost or added");
            for c in now.clips() {
                if let Some(end) = c.source_end() {
                    prop_assert!(c.source_in() >= Time::ZERO && end <= t(5.0));
                }
            }
        }
        while doc.undo() {}
        prop_assert_eq!(doc.project(), &start);
    }
}

/// `clip` at `percent` speed, reversed or not.
fn retimed(mut clip: Clip, percent: u16, reversed: bool) -> Clip {
    if let ClipSource::Media { speed, reverse, .. } = &mut clip.source {
        *speed = crate::time::Speed::new(percent).unwrap();
        *reverse = reversed;
    }
    clip
}

#[test]
fn trims_of_sped_up_and_reversed_clips_clamp_to_the_media_and_keep_the_content() {
    let mut p = empty();
    // 1 s at 2x reversed: source 3 s down to 1 s of the 5 s video.
    let a = p.push_clip(OVERLAY, retimed(video(&p, 1.0, 1.0, 1.0), 200, true));
    let shown = clip(&p, a).source_time(t(1.5));

    let after = assert_reversible(&mut TrimClip::new(a, Edge::End, t(10.0), true), &p);
    let c = clip(&after, a);
    // The end reaches source 0: 1.5 s at 2x.
    assert_eq!((c.start, c.duration), (t(1.0), t(1.5)));
    assert_eq!((c.source_in(), c.source_end()), (Time::ZERO, Some(t(3.0))));
    assert_eq!(c.source_time(t(1.5)), shown);

    let after = assert_reversible(&mut TrimClip::new(a, Edge::Start, Time::ZERO, true), &p);
    let c = clip(&after, a);
    // The start reaches the media's end: 1 s more at 2x is 2 s of source.
    assert_eq!((c.start, c.duration), (Time::ZERO, t(2.0)));
    assert_eq!((c.source_in(), c.source_end()), (t(1.0), Some(t(5.0))));
    assert_eq!(c.source_time(t(1.5)), shown);

    let mut p = empty();
    let b = p.push_clip(OVERLAY, retimed(video(&p, 1.0, 1.0, 1.0), 200, false));
    let after = assert_reversible(&mut TrimClip::new(b, Edge::Start, Time::ZERO, true), &p);
    let c = clip(&after, b);
    // Forward, the start reaches source 0 after 0.5 s.
    assert_eq!((c.start, c.source_in()), (t(0.5), Time::ZERO));
    let after = assert_reversible(&mut TrimClip::new(b, Edge::End, t(10.0), true), &p);
    assert_eq!(clip(&after, b).source_end(), Some(t(5.0)));
    assert_eq!(clip(&after, b).end(), t(3.0));
}
