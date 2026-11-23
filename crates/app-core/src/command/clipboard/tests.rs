use super::*;
use crate::command::test_support::*;
use crate::timeline::added_clips;

#[test]
fn copy_takes_linked_members_and_relative_offsets() {
    let (mut p, [_, b, _]) = three_on_main();
    let s = p.push_clip(AUDIO, sound(&p, 6.0, 1.0, 0.0));
    let o = p.push_clip(OVERLAY, color(7.0, 1.0));
    link(&mut p, &[o, s]);
    let content = ClipboardContent::copy(&p, &[b, o]).unwrap();
    let items: Vec<(ClipId, bool, Time)> = content
        .items
        .iter()
        .map(|i| (i.clip.id, i.on_main, i.offset))
        .collect();
    assert_eq!(
        items,
        vec![(o, false, t(2.0)), (b, true, t(0.0)), (s, false, t(1.0))]
    );
    assert_eq!(
        ClipboardContent::copy(&p, &[]),
        Err(CommandError::NothingToEdit)
    );
}

#[test]
fn paste_inserts_main_clips_at_the_nearest_boundary_and_keeps_groups_in_sync() {
    let (mut p, [a, b, c]) = three_on_main();
    let o = p.push_clip(OVERLAY, color(1.0, 1.0));
    let follower = p.push_clip(OVERLAY, color(11.0, 1.0));
    let content = ClipboardContent::copy(&p, &[a, o]).unwrap();
    let mut paste = PasteClips::new(content, t(6.0));
    let new = paste.clip_ids().to_vec();
    let after = assert_reversible(&mut paste, &p);
    let mut added = added_clips(&p, &after);
    added.sort();
    let mut expected = new.clone();
    expected.sort();
    assert_eq!(added, expected);
    // Nearest boundary to 6 s is 5 s: A's copy goes between A and B.
    let main = &after.tracks[MAIN];
    assert_eq!(main.clips[1].start, t(5.0));
    assert_eq!(ids(main)[0], a);
    assert_eq!(&ids(main)[2..], &[b, c]);
    assert_eq!(clip(&after, c).start, t(15.0));
    assert_eq!(
        clip(&after, follower).start,
        t(16.0),
        "followers ripple with C"
    );
    // The overlay copy keeps its 1 s offset from the main copy.
    let pasted_overlay = new
        .iter()
        .find(|id| track_of(&after, **id) == OVERLAY)
        .unwrap();
    assert_eq!(clip(&after, *pasted_overlay).start, t(6.0));
}

#[test]
fn paste_falls_back_to_new_tracks_and_regroups_links() {
    let (mut p, _) = three_on_main();
    let o = p.push_clip(OVERLAY, color(1.0, 2.0));
    let s = p.push_clip(AUDIO, sound(&p, 1.0, 2.0, 0.0));
    link(&mut p, &[o, s]);
    let content = ClipboardContent::copy(&p, &[o]).unwrap();
    // Pasting over the originals: both ranges are taken, so both go to new tracks.
    let mut paste = PasteClips::new(content.clone(), t(2.0));
    let after = assert_reversible(&mut paste, &p);
    assert_eq!(after.tracks.len(), p.tracks.len() + 2);
    let new = paste.clip_ids();
    let links: Vec<_> = new.iter().map(|id| clip(&after, *id).link).collect();
    assert!(links[0].is_some());
    assert_eq!(links[0], links[1]);
    assert_ne!(links[0], clip(&p, o).link);
    // The new video track is directly above the main track, the audio one directly below.
    let main = after.main_index().unwrap();
    let tracks: Vec<usize> = new.iter().map(|id| track_of(&after, *id)).collect();
    assert!(tracks.contains(&(main - 1)) && tracks.contains(&(main + 1)));
    // A free spot on the original tracks is used.
    let after = assert_reversible(&mut PasteClips::new(content, t(5.0)), &p);
    assert_eq!(after.tracks.len(), p.tracks.len());
}

#[test]
fn paste_never_targets_a_locked_track() {
    let (mut p, [a, _, _]) = three_on_main();
    let content = ClipboardContent::copy(&p, &[a]).unwrap();
    p.tracks[MAIN].locked = true;
    let main = p.tracks[MAIN].clone();
    let after = assert_reversible(&mut PasteClips::new(content, t(5.0)), &p);
    assert_eq!(after.main().unwrap(), &main);
    assert_eq!(after.tracks.len(), p.tracks.len() + 1);
}

#[test]
fn paste_on_a_non_magnetic_main_track_uses_the_time_if_free() {
    let (mut p, [a, _, _]) = three_on_main();
    p.magnetic = false;
    let content = ClipboardContent::copy(&p, &[a]).unwrap();
    let mut paste = PasteClips::new(content, t(20.0));
    let after = assert_reversible(&mut paste, &p);
    assert_eq!(clip(&after, paste.clip_ids()[0]).start, t(20.0));
    assert_eq!(track_of(&after, paste.clip_ids()[0]), MAIN);
}

#[test]
fn paste_fails_for_removed_media() {
    let mut p = empty();
    let v = p.push_clip(OVERLAY, video(&p, 0.0, 1.0, 0.0));
    let content = ClipboardContent::copy(&p, &[v]).unwrap();
    let media = p.media[0].id;
    p.tracks[OVERLAY].clips.clear();
    p.media.remove(0);
    assert_eq!(
        try_apply(&p, PasteClips::new(content, t(0.0))),
        Err(CommandError::MediaNotFound(media))
    );
}

#[test]
fn duplicate_pastes_after_the_selection() {
    let (p, [a, b, c]) = three_on_main();
    let mut dup = PasteClips::duplicate(&p, &[a]).unwrap();
    assert_eq!(dup.label(), HistoryLabel::DuplicateClips);
    let after = assert_reversible(&mut dup, &p);
    let main = &after.tracks[MAIN];
    assert_eq!(ids(main), vec![a, dup.clip_ids()[0], b, c]);
    assert_eq!(starts(main), vec![t(0.0), t(5.0), t(10.0), t(15.0)]);

    let mut q = empty();
    let o = q.push_clip(OVERLAY, color(1.0, 2.0));
    let mut dup = PasteClips::duplicate(&q, &[o]).unwrap();
    let after = assert_reversible(&mut dup, &q);
    assert_eq!(starts(&after.tracks[OVERLAY]), vec![t(1.0), t(3.0)]);
}
