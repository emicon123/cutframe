use super::*;
use crate::command::test_support::*;
use crate::command::{DeleteClips, MoveClips, TrackTarget};
use crate::props::AudioProps;

// ---- Link / unlink --------------------------------------------------------------------------

#[test]
fn link_joins_clips_on_different_tracks() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let o = p.push_clip(OVERLAY, color(1.0, 1.0));
    let after = assert_reversible(&mut LinkClips::new(vec![v, o]), &p);
    assert!(clip(&after, v).link.is_some());
    assert_eq!(clip(&after, v).link, clip(&after, o).link);
}

#[test]
fn linked_clips_then_move_together() {
    let mut p = empty();
    p.magnetic = false;
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    let linked = try_apply(&p, LinkClips::new(vec![v, s])).unwrap();
    let target = TrackTarget::Existing {
        track: linked.main_track,
    };
    let moved = try_apply(&linked, MoveClips::new(vec![v], v, t(2.0), target, true)).unwrap();
    assert_eq!(clip(&moved, s).start, t(2.0));
}

#[test]
fn link_replaces_previous_links_and_clears_left_singles() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    let o = p.push_clip(OVERLAY, color(0.0, 1.0));
    link(&mut p, &[v, s]);
    let old = clip(&p, v).link;
    let after = assert_reversible(&mut LinkClips::new(vec![v, o]), &p);
    assert_ne!(clip(&after, v).link, old);
    assert_eq!(clip(&after, v).link, clip(&after, o).link);
    assert_eq!(clip(&after, s).link, None, "a group of one loses its link");
}

#[test]
fn link_needs_two_clips_on_different_tracks() {
    let mut p = empty();
    let a = p.push_clip(MAIN, color(0.0, 1.0));
    let b = p.push_clip(MAIN, color(1.0, 1.0));
    assert_eq!(
        try_apply(&p, LinkClips::new(vec![a])),
        Err(CommandError::InvalidLink)
    );
    assert_eq!(
        try_apply(&p, LinkClips::new(vec![a, a])),
        Err(CommandError::InvalidLink)
    );
    assert_eq!(
        try_apply(&p, LinkClips::new(vec![a, b])),
        Err(CommandError::InvalidLink)
    );
    let missing = ClipId::new();
    assert_eq!(
        try_apply(&p, LinkClips::new(vec![a, missing])),
        Err(CommandError::ClipNotFound(missing))
    );
}

#[test]
fn link_and_unlink_reject_locked_clips() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    p.tracks[AUDIO].locked = true;
    let audio = p.tracks[AUDIO].id;
    assert_eq!(
        try_apply(&p, LinkClips::new(vec![v, s])),
        Err(CommandError::TrackLocked(audio))
    );
    link(&mut p, &[v, s]);
    assert_eq!(
        try_apply(&p, UnlinkClips::new(vec![s])),
        Err(CommandError::TrackLocked(audio))
    );
}

#[test]
fn unlink_clears_the_listed_clips_and_normalises_groups() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 4.0, 0.0));
    let o = p.push_clip(OVERLAY, color(0.0, 1.0));
    link(&mut p, &[v, s, o]);
    let after = assert_reversible(&mut UnlinkClips::new(vec![o]), &p);
    assert_eq!(clip(&after, o).link, None);
    assert!(clip(&after, v).link.is_some());
    assert_eq!(clip(&after, v).link, clip(&after, s).link);

    let after = assert_reversible(&mut UnlinkClips::new(vec![v]), &p);
    assert_eq!(clip(&after, v).link, None);
    assert_eq!(clip(&after, s).link, clip(&after, o).link);

    let after = assert_reversible(&mut UnlinkClips::new(vec![v, o]), &p);
    assert!(after.clips().all(|c| c.link.is_none()));
}

#[test]
fn unlink_of_unlinked_clips_is_nothing_to_edit() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    assert_eq!(
        try_apply(&p, UnlinkClips::new(vec![v])),
        Err(CommandError::NothingToEdit)
    );
}

// ---- Detach audio ---------------------------------------------------------------------------

#[test]
fn detach_moves_the_sound_to_a_linked_audio_clip() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.5));
    p.tracks[MAIN].clips[0].audio.as_mut().unwrap().fade_in = t(1.0);
    let after = assert_reversible(&mut DetachAudio::new(vec![v]), &p);
    let video = clip(&after, v);
    assert_eq!(video.audio, None);
    assert!(video.visual.is_some());
    let added = crate::timeline::added_clips(&p, &after);
    assert_eq!(added.len(), 1);
    let sound = clip(&after, added[0]);
    assert_eq!(track_of(&after, sound.id), AUDIO);
    assert_eq!(
        (sound.start, sound.duration, sound.source_in()),
        (t(0.0), t(4.0), t(0.5))
    );
    assert_eq!(sound.source.media(), video.source.media());
    assert_eq!(sound.visual, None);
    assert_eq!(sound.audio.as_ref().map(|a| a.fade_in), Some(t(1.0)));
    assert!(video.link.is_some());
    assert_eq!(video.link, sound.link);
}

#[test]
fn detached_clips_delete_together() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let detached = try_apply(&p, DetachAudio::new(vec![v])).unwrap();
    let deleted = try_apply(&detached, DeleteClips::new(vec![v], false)).unwrap();
    assert_eq!(deleted.clips().count(), 0);
}

#[test]
fn detach_skips_busy_and_locked_audio_tracks_and_creates_one_at_the_top() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    p.push_clip(AUDIO, sound(&p, 1.0, 1.0, 0.0));
    let mut locked = Track::new(TrackKind::Audio, "Audio 2");
    locked.locked = true;
    p.tracks.push(locked);
    let after = assert_reversible(&mut DetachAudio::new(vec![v]), &p);
    assert_eq!(after.tracks.len(), p.tracks.len() + 1);
    let new = &after.tracks[MAIN + 1];
    assert_eq!(new.kind, TrackKind::Audio);
    assert_eq!(new.name, "Audio 3");
    assert_eq!(new.clips.len(), 1);
    assert_eq!(
        after.tracks[AUDIO + 1].clips.len(),
        1,
        "the busy track is unchanged"
    );

    // A free unlocked track further down is used instead of a new one.
    let mut free = p.clone();
    free.tracks.push(Track::new(TrackKind::Audio, "Audio 4"));
    let after = assert_reversible(&mut DetachAudio::new(vec![v]), &free);
    assert_eq!(after.tracks.len(), free.tracks.len());
    assert_eq!(after.tracks.last().unwrap().clips.len(), 1);
}

#[test]
fn detach_keeps_an_existing_group_and_avoids_its_tracks() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    // The group already has a sound on the audio track (a manual link), at another time.
    let s = p.push_clip(AUDIO, sound(&p, 4.0, 1.0, 0.0));
    let o = p.push_clip(OVERLAY, color(0.0, 1.0));
    link(&mut p, &[v, s, o]);
    let group = clip(&p, v).link;
    let after = assert_reversible(&mut DetachAudio::new(vec![v]), &p);
    let added = crate::timeline::added_clips(&p, &after);
    let new = clip(&after, added[0]);
    assert_eq!(new.link, group);
    assert_ne!(track_of(&after, new.id), track_of(&after, s));
}

#[test]
fn detach_several_clips_and_skip_the_ones_without_sound() {
    let mut p = empty();
    let a = p.push_clip(MAIN, video(&p, 0.0, 2.0, 0.0));
    let b = p.push_clip(MAIN, video(&p, 2.0, 2.0, 1.0));
    let i = p.push_clip(MAIN, image(&p, 4.0, 1.0));
    let c = p.push_clip(MAIN, color(5.0, 1.0));
    let s = p.push_clip(AUDIO, sound(&p, 6.0, 1.0, 0.0));
    let after = assert_reversible(&mut DetachAudio::new(vec![a, b, i, c, s]), &p);
    let added = crate::timeline::added_clips(&p, &after);
    assert_eq!(added.len(), 2);
    assert!(added.iter().all(|id| track_of(&after, *id) == AUDIO));
    assert_ne!(clip(&after, a).link, clip(&after, b).link);

    assert_eq!(
        try_apply(&p, DetachAudio::new(vec![i, c, s])),
        Err(CommandError::NothingToEdit)
    );
    let detached = try_apply(&p, DetachAudio::new(vec![a])).unwrap();
    assert_eq!(
        try_apply(&detached, DetachAudio::new(vec![a])),
        Err(CommandError::NothingToEdit),
        "a clip detaches once"
    );
}

#[test]
fn detach_rejects_a_clip_on_a_locked_track() {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    p.tracks[MAIN].locked = true;
    assert_eq!(
        try_apply(&p, DetachAudio::new(vec![v])),
        Err(CommandError::TrackLocked(p.tracks[MAIN].id))
    );
}

#[test]
fn detach_on_an_overlay_works_like_on_the_main_track() {
    let mut p = empty();
    let v = p.push_clip(OVERLAY, video(&p, 1.0, 2.0, 0.0));
    let after = assert_reversible(&mut DetachAudio::new(vec![v]), &p);
    assert_eq!(after.tracks[AUDIO].clips.len(), 1);
    assert_eq!(
        after.tracks[AUDIO].clips[0].audio,
        Some(AudioProps::default())
    );
}
