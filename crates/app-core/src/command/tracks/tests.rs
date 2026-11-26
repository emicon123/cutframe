use super::*;
use crate::command::test_support::*;
use crate::command::{DeleteClips, SplitClips};
use crate::model::{Track, TrackKind};

#[test]
fn set_track_flag_sets_each_flag_and_reverts() {
    let p = empty();
    let id = p.tracks[AUDIO].id;
    for flag in [TrackFlag::Muted, TrackFlag::Hidden, TrackFlag::Locked] {
        let mut command = SetTrackFlag::new(id, flag, true);
        assert_eq!(command.changes(&p), Ok(true));
        let after = assert_reversible(&mut command, &p);
        let track = after.track(id).unwrap();
        let (muted, hidden, locked) = (track.muted, track.hidden, track.locked);
        assert_eq!(
            (muted, hidden, locked),
            (
                flag == TrackFlag::Muted,
                flag == TrackFlag::Hidden,
                flag == TrackFlag::Locked
            )
        );
        assert_eq!(SetTrackFlag::new(id, flag, true).changes(&after), Ok(false));
    }
}

#[test]
fn set_track_flag_labels_name_the_flag() {
    let id = TrackId::new();
    let label = |flag| SetTrackFlag::new(id, flag, true).label();
    assert_eq!(label(TrackFlag::Muted), HistoryLabel::SetTrackMuted);
    assert_eq!(label(TrackFlag::Hidden), HistoryLabel::SetTrackHidden);
    assert_eq!(label(TrackFlag::Locked), HistoryLabel::SetTrackLocked);
}

#[test]
fn set_track_flag_rejects_unknown_tracks() {
    let p = empty();
    let missing = TrackId::new();
    let command = SetTrackFlag::new(missing, TrackFlag::Muted, true);
    assert_eq!(
        command.changes(&p),
        Err(CommandError::TrackNotFound(missing))
    );
    assert_eq!(
        try_apply(&p, command),
        Err(CommandError::TrackNotFound(missing))
    );
}

#[test]
fn a_track_locked_by_command_rejects_edits_until_unlocked() {
    let mut p = empty();
    let o = p.push_clip(OVERLAY, color(0.0, 1.0));
    let id = p.tracks[OVERLAY].id;
    let locked = try_apply(&p, SetTrackFlag::new(id, TrackFlag::Locked, true)).unwrap();
    assert_eq!(
        try_apply(&locked, DeleteClips::new(vec![o], false)),
        Err(CommandError::TrackLocked(id))
    );
    let unlocked = try_apply(&locked, SetTrackFlag::new(id, TrackFlag::Locked, false)).unwrap();
    assert!(try_apply(&unlocked, DeleteClips::new(vec![o], false)).is_ok());
}

#[test]
fn muted_and_hidden_tracks_still_edit() {
    let mut p = empty();
    let o = p.push_clip(OVERLAY, color(0.0, 2.0));
    p.tracks[OVERLAY].muted = true;
    p.tracks[OVERLAY].hidden = true;
    assert!(try_apply(&p, SplitClips::new(vec![o], t(1.0))).is_ok());
}

/// Tracks: overlays V3, V2 (index 0, 1), main (2), audio A1, A2, A3 (3, 4, 5).
fn sections() -> Project {
    let mut p = empty();
    p.tracks.insert(0, Track::new(TrackKind::Text, "Text 1"));
    p.tracks.push(Track::new(TrackKind::Audio, "Audio 2"));
    p.tracks.push(Track::new(TrackKind::Audio, "Audio 3"));
    assert_eq!(p.main_index(), Some(2));
    assert_eq!(p.validate(), Ok(()));
    p
}

fn names(p: &Project) -> Vec<&str> {
    p.tracks.iter().map(|t| t.name.as_str()).collect()
}

#[test]
fn move_track_reorders_within_the_overlay_section() {
    let mut p = sections();
    p.push_clip(1, color(0.0, 1.0));
    let id = p.tracks[1].id;
    let after = assert_reversible(&mut MoveTrack::new(id, 0), &p);
    assert_eq!(
        names(&after),
        [
            "Video 2", "Text 1", "Video 1", "Audio 1", "Audio 2", "Audio 3"
        ]
    );
    assert_eq!(after.tracks[0].clips, p.tracks[1].clips);
    // Past the main track it stays directly above it.
    let top = p.tracks[0].id;
    let after = assert_reversible(&mut MoveTrack::new(top, 5), &p);
    assert_eq!(
        names(&after),
        [
            "Video 2", "Text 1", "Video 1", "Audio 1", "Audio 2", "Audio 3"
        ]
    );
}

#[test]
fn move_track_reorders_within_the_audio_section() {
    let p = sections();
    let last = p.tracks[5].id;
    let after = assert_reversible(&mut MoveTrack::new(last, 0), &p);
    assert_eq!(
        names(&after),
        [
            "Text 1", "Video 2", "Video 1", "Audio 3", "Audio 1", "Audio 2"
        ]
    );
    let first = p.tracks[3].id;
    let after = assert_reversible(&mut MoveTrack::new(first, 4), &p);
    assert_eq!(
        names(&after),
        [
            "Text 1", "Video 2", "Video 1", "Audio 2", "Audio 1", "Audio 3"
        ]
    );
    let after = assert_reversible(&mut MoveTrack::new(first, 99), &p);
    assert_eq!(
        names(&after),
        [
            "Text 1", "Video 2", "Video 1", "Audio 2", "Audio 3", "Audio 1"
        ]
    );
}

#[test]
fn move_track_to_its_own_place_changes_nothing() {
    let p = sections();
    let id = p.tracks[4].id;
    assert_eq!(MoveTrack::new(id, 4).changes(&p), Ok(false));
    assert_eq!(MoveTrack::new(id, 5).changes(&p), Ok(true));
    // Clamped back to where it is.
    let top = p.tracks[0].id;
    assert_eq!(MoveTrack::new(top, 0).changes(&p), Ok(false));
}

#[test]
fn locked_tracks_still_reorder() {
    let mut p = sections();
    p.tracks[3].locked = true;
    let id = p.tracks[3].id;
    let after = assert_reversible(&mut MoveTrack::new(id, 5), &p);
    assert_eq!(after.tracks[5].id, id);
}

#[test]
fn the_main_track_does_not_move() {
    let p = sections();
    assert_eq!(
        try_apply(&p, MoveTrack::new(p.main_track, 0)),
        Err(CommandError::MainTrack)
    );
    let missing = TrackId::new();
    assert_eq!(
        try_apply(&p, MoveTrack::new(missing, 0)),
        Err(CommandError::TrackNotFound(missing))
    );
}
