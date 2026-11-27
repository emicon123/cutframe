use super::*;
use crate::media::{MediaAsset, MediaId, samples};
use crate::model::{ClipSource, Rgba8};
use crate::props::AudioProps;

/// A long video for main-track clips and a long song for audio clips.
fn long_media() -> (MediaAsset, MediaAsset) {
    let mut video = samples::video(1);
    video.info.duration = Some(Time(1 << 40));
    let mut song = samples::audio(2);
    song.info.duration = Some(Time(1 << 40));
    (video, song)
}

/// Tracks, top to bottom: "Video 3", "Text 1", "Video 2" (overlays), "Video 1" (main), "Audio 1",
/// "Audio 2", "Audio 3". The main track holds media clips `M0`, `M1`, ... of `durations`,
/// contiguous from 0 (magnetic).
fn project(durations: &[i64]) -> Project {
    let (video, song) = long_media();
    let mut project = Project::new("Timeline");
    project.tracks = vec![
        Track::new(TrackKind::Video, "Video 3"),
        Track::new(TrackKind::Text, "Text 1"),
        Track::new(TrackKind::Video, "Video 2"),
        Track::new(TrackKind::Video, "Video 1"),
        Track::new(TrackKind::Audio, "Audio 1"),
        Track::new(TrackKind::Audio, "Audio 2"),
        Track::new(TrackKind::Audio, "Audio 3"),
    ];
    project.main_track = project.tracks[3].id;
    let mut start = 0;
    for (i, &duration) in durations.iter().enumerate() {
        let clip = media_clip(&format!("M{i}"), video.id, start, duration, true);
        project.tracks[3].clips.push(clip);
        start += duration;
    }
    project.media = vec![video, song];
    project
}

fn media_clip(name: &str, media: MediaId, start: i64, duration: i64, visual: bool) -> Clip {
    let mut clip = Clip::color(name, Time(start), Time(duration), Rgba8::BLACK);
    clip.source = ClipSource::media_at(media, Time::ZERO);
    if !visual {
        clip.visual = None;
        clip.audio = Some(AudioProps::default());
    }
    clip
}

/// Adds a clip that fits `track` (colour card, title or song clip).
fn add(project: &mut Project, track: &str, name: &str, start: i64, duration: i64) -> ClipId {
    let index = project.tracks.iter().position(|t| t.name == track).unwrap();
    let clip = match project.tracks[index].kind {
        TrackKind::Video => Clip::color(name, Time(start), Time(duration), Rgba8::BLACK),
        TrackKind::Text => Clip::text(name, Time(start), Time(duration), name),
        TrackKind::Audio => media_clip(name, project.media[1].id, start, duration, false),
    };
    let id = clip.id;
    place_clip(&mut project.tracks[index], clip).unwrap();
    id
}

fn find<'a>(project: &'a Project, name: &str) -> Option<(&'a Track, &'a Clip)> {
    project
        .tracks
        .iter()
        .find_map(|t| t.clips.iter().find(|c| c.name == name).map(|c| (t, c)))
}

fn start(project: &Project, name: &str) -> i64 {
    find(project, name).expect(name).1.start.flicks()
}

fn track_of(project: &Project, name: &str) -> String {
    find(project, name).expect(name).0.name.clone()
}

fn main_mut(project: &mut Project) -> &mut Track {
    let index = project.main_index().unwrap();
    &mut project.tracks[index]
}

fn main_clip<'a>(project: &'a mut Project, name: &str) -> &'a mut Clip {
    main_mut(project)
        .clips
        .iter_mut()
        .find(|c| c.name == name)
        .unwrap()
}

fn remove_main(project: &mut Project, name: &str) {
    main_mut(project).clips.retain(|c| c.name != name);
}

fn trim_start(project: &mut Project, name: &str, by: i64) {
    let clip = main_clip(project, name);
    if let ClipSource::Media { source_in, .. } = &mut clip.source {
        *source_in = *source_in + Time(by);
    }
    clip.start = clip.start + Time(by);
    clip.duration = clip.duration - Time(by);
}

fn trim_end(project: &mut Project, name: &str, by: i64) {
    let clip = main_clip(project, name);
    clip.duration = clip.duration - Time(by);
}

/// Applies `edit` to the main track, repacks, follows, and checks the result is valid.
fn follow_with(
    before: &Project,
    edit: impl FnOnce(&mut Project),
    delete_inside: bool,
    pool: &mut IdPool<TrackId>,
) -> Result<Project, CommandError> {
    let mut after = before.clone();
    edit(&mut after);
    repack_main(&mut after);
    pool.rewind();
    follow_main(
        before,
        &mut after,
        Follow {
            moved: &[],
            delete_inside,
            track_ids: pool,
        },
    )?;
    assert_eq!(after.validate(), Ok(()), "{after:#?}");
    Ok(after)
}

fn follow(before: &Project, edit: impl FnOnce(&mut Project), delete_inside: bool) -> Project {
    follow_with(before, edit, delete_inside, &mut IdPool::default()).expect("follow")
}

#[test]
fn place_clip_keeps_order_and_rejects_overlaps() {
    let mut track = Track::new(TrackKind::Video, "V");
    let card = |s, d| Clip::color("C", Time(s), Time(d), Rgba8::BLACK);
    assert_eq!(place_clip(&mut track, card(10, 10)), Ok(0));
    assert_eq!(place_clip(&mut track, card(0, 10)), Ok(0));
    assert_eq!(place_clip(&mut track, card(30, 5)), Ok(2));
    assert_eq!(place_clip(&mut track, card(20, 10)), Ok(2));
    let before = track.clone();
    assert_eq!(
        place_clip(&mut track, card(25, 1)),
        Err(CommandError::Overlap)
    );
    assert_eq!(
        place_clip(&mut track, card(34, 10)),
        Err(CommandError::Overlap)
    );
    assert_eq!(track, before);
    let starts: Vec<_> = track.clips.iter().map(|c| c.start.flicks()).collect();
    assert_eq!(starts, [0, 10, 20, 30]);
    assert!(is_free(&track, Time(35), Time(40), &[]));
    let first = track.clips[0].id;
    assert!(is_free(&track, Time(0), Time(10), &[first]));
}

#[test]
fn repack_main_closes_gaps_only_when_magnetic() {
    let mut project = project(&[100, 100, 100]);
    main_mut(&mut project).clips.remove(1);
    let mut off = project.clone();
    off.magnetic = false;
    repack_main(&mut off);
    assert_eq!(start(&off, "M2"), 200);
    repack_main(&mut project);
    assert_eq!(start(&project, "M2"), 100);
    assert_eq!(project.validate(), Ok(()));
}

#[test]
fn normalize_links_clears_groups_of_one() {
    let mut project = project(&[100]);
    let a = add(&mut project, "Video 2", "A", 0, 10);
    let b = add(&mut project, "Text 1", "B", 0, 10);
    let c = add(&mut project, "Video 3", "C", 0, 10);
    let (pair, single) = (LinkId::new(), LinkId::new());
    for clip in project.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
        if clip.id == a || clip.id == b {
            clip.link = Some(pair);
        } else if clip.id == c {
            clip.link = Some(single);
        }
    }
    normalize_links(&mut project);
    assert_eq!(project.clip(a).unwrap().1.link, Some(pair));
    assert_eq!(project.clip(c).unwrap().1.link, None);
    assert_eq!(project.validate(), Ok(()));
}

#[test]
fn id_pool_replays_ids_after_rewind() {
    let mut pool = IdPool::<TrackId>::default();
    let first = [pool.take(), pool.take()];
    pool.rewind();
    assert_eq!([pool.take(), pool.take()], first);
    assert_ne!(pool.take(), first[1]);
}

#[test]
fn anchor_resolution_cases() {
    let mut project = project(&[100, 100]);
    trim_start(&mut project, "M1", 30); // M1 now 130..200 with source_in 30
    main_clip(&mut project, "M1").start = Time(150);
    let main = &project.main().unwrap().clips;
    let (m0, m1) = (main[0].id, main[1].id);
    assert_eq!(anchor_of(&[], Time(5)), None);
    assert_eq!(
        anchor_of(main, Time(0)),
        Some(Anchor::Inside {
            clip: m0,
            content: Time(0)
        })
    );
    assert_eq!(anchor_of(main, Time(120)), Some(Anchor::Gap { next: m1 }));
    assert_eq!(anchor_of(main, Time(100)), Some(Anchor::Gap { next: m1 }));
    assert_eq!(
        anchor_of(main, Time(160)),
        Some(Anchor::Inside {
            clip: m1,
            content: Time(40)
        })
    );
    assert_eq!(anchor_of(main, Time(220)), Some(Anchor::AfterEnd));
}

#[test]
fn followers_keep_their_content_moment_when_the_anchor_is_trimmed() {
    let mut before = project(&[100, 100]);
    add(&mut before, "Video 2", "B-roll", 150, 20);
    add(&mut before, "Text 1", "Early", 110, 10);
    add(&mut before, "Video 3", "Over M0", 20, 10);

    let after = follow(&before, |p| trim_start(p, "M1", 20), false);
    assert_eq!(start(&after, "B-roll"), 130, "same moment of M1's content");
    assert_eq!(
        start(&after, "Early"),
        100,
        "trimmed-away content clamps to the start"
    );
    assert_eq!(start(&after, "Over M0"), 20);

    let after = follow(&before, |p| trim_end(p, "M0", 30), false);
    assert_eq!(start(&after, "B-roll"), 120);
    assert_eq!(start(&after, "Early"), 80);
    assert_eq!(start(&after, "Over M0"), 20);
}

#[test]
fn gap_followers_shift_with_the_next_clip_but_not_before_the_gap() {
    let mut before = project(&[100, 100]);
    before.magnetic = false;
    main_clip(&mut before, "M1").start = Time(200);
    add(&mut before, "Video 2", "Gap", 150, 10);
    add(&mut before, "Video 3", "Near", 110, 10);

    let after = follow(&before, |p| main_clip(p, "M1").start = Time(250), false);
    assert_eq!(start(&after, "Gap"), 200);
    let after = follow(&before, |p| main_clip(p, "M1").start = Time(120), false);
    assert_eq!(start(&after, "Gap"), 100, "never before M0's end");
    assert_eq!(start(&after, "Near"), 100);
}

#[test]
fn followers_after_the_end_keep_their_distance_to_it() {
    let mut before = project(&[100]);
    add(&mut before, "Video 2", "End card", 150, 10);
    let after = follow(&before, |p| trim_end(p, "M0", 40), false);
    assert_eq!(start(&after, "End card"), 110);
    let after = follow(&before, |p| remove_main(p, "M0"), true);
    assert_eq!(start(&after, "End card"), 50);
}

#[test]
fn nothing_follows_an_empty_main_track() {
    let mut before = project(&[]);
    add(&mut before, "Video 2", "Alone", 150, 10);
    let after = follow(&before, |p| p.tracks[0].name = "Renamed".into(), false);
    assert_eq!(start(&after, "Alone"), 150);
}

#[test]
fn the_music_bed_at_zero_stays_put() {
    let mut before = project(&[100, 100, 100, 100]);
    add(&mut before, "Audio 1", "Bed", 0, 350);
    for after in [
        follow(&before, |p| remove_main(p, "M2"), true),
        follow(&before, |p| trim_end(p, "M1", 50), false),
        follow(&before, |p| trim_start(p, "M3", 50), false),
        follow(&before, |p| trim_start(p, "M0", 30), false),
        follow(&before, |p| remove_main(p, "M0"), false),
        follow(&before, |p| remove_main(p, "M0"), true),
    ] {
        assert_eq!(start(&after, "Bed"), 0);
        assert_eq!(track_of(&after, "Bed"), "Audio 1");
    }
}

#[test]
fn ripple_delete_takes_the_followers_inside_and_moves_partial_ones() {
    let mut before = project(&[100, 100, 100]);
    add(&mut before, "Text 1", "Inside", 110, 40);
    add(&mut before, "Text 1", "Partial", 180, 50);
    add(&mut before, "Text 1", "Next", 250, 10);
    add(&mut before, "Video 2", "Edge", 100, 100); // exactly M1's range counts as inside

    let after = follow(&before, |p| remove_main(p, "M1"), true);
    assert!(find(&after, "Inside").is_none());
    assert!(find(&after, "Edge").is_none());
    assert_eq!(start(&after, "Partial"), 100, "to the removal point");
    assert_eq!(start(&after, "Next"), 150);

    // Without `delete_inside` (removing clips of media) nothing is deleted: the followers go to
    // the removal point and the collision is bumped onto a new text track.
    let after = follow(&before, |p| remove_main(p, "M1"), false);
    assert_eq!(after.clips().count(), before.clips().count() - 1);
    assert_eq!(start(&after, "Inside"), 100);
    assert_eq!(start(&after, "Partial"), 100);
    assert_eq!(track_of(&after, "Inside"), "Text 1");
    let bumped = track_of(&after, "Partial");
    assert_ne!(bumped, "Text 1");
    let index = after.tracks.iter().position(|t| t.name == bumped).unwrap();
    assert_eq!(after.tracks[index].kind, TrackKind::Text);
    assert_eq!(
        after.tracks[index + 1].name,
        "Text 1",
        "created directly above"
    );
    assert_eq!(start(&after, "Edge"), 100);
}

#[test]
fn a_split_anchor_resolves_to_the_half_under_the_clip() {
    let mut before = project(&[200]);
    add(&mut before, "Video 2", "B-roll", 150, 10);
    // Split M0 at 100.
    let mut right = main_clip(&mut before, "M0").clone();
    right.id = ClipId::new();
    right.name = "M0b".into();
    right.start = Time(100);
    right.duration = Time(100);
    right.source = ClipSource::media_at(before.media[0].id, Time(100));
    main_clip(&mut before, "M0").duration = Time(100);
    main_mut(&mut before).clips.push(right);
    assert_eq!(before.validate(), Ok(()));

    let after = follow(&before, |p| trim_end(p, "M0", 50), false);
    assert_eq!(start(&after, "B-roll"), 100);
}

#[test]
fn locked_tracks_never_follow() {
    let mut before = project(&[100, 100]);
    add(&mut before, "Audio 1", "Music", 150, 10);
    add(&mut before, "Video 2", "B-roll", 150, 10);
    before.tracks[4].locked = true;
    let after = follow(&before, |p| remove_main(p, "M0"), true);
    assert_eq!(after.tracks[4], before.tracks[4], "bit-identical");
    assert_eq!(start(&after, "B-roll"), 50);
}

#[test]
fn linked_members_of_main_clips_shift_through_the_link() {
    let mut before = project(&[100, 100]);
    let detached = add(&mut before, "Audio 1", "Detached", 100, 100);
    let link = LinkId::new();
    main_clip(&mut before, "M1").link = Some(link);
    before.tracks[4].clips[0].link = Some(link);
    assert_eq!(before.validate(), Ok(()));

    // M0 trimmed by 30: M1 and its audio shift together by -30.
    let after = follow(&before, |p| trim_end(p, "M0", 30), false);
    assert_eq!(start(&after, "Detached"), 70);
    assert_eq!(after.clip(detached).unwrap().1.link, Some(link));

    let mut locked = before.clone();
    locked.tracks[4].locked = true;
    let result = follow_with(
        &locked,
        |p| trim_end(p, "M0", 30),
        false,
        &mut IdPool::default(),
    );
    assert_eq!(result, Err(CommandError::TrackLocked(locked.tracks[4].id)));
    // A delta of zero touches nothing, so the lock does not matter.
    let after = follow(&locked, |p| trim_end(p, "M1", 30), false);
    assert_eq!(start(&after, "Detached"), 100);
}

#[test]
fn link_groups_off_the_main_track_follow_as_one_unit() {
    let mut before = project(&[100, 100]);
    let a = add(&mut before, "Video 2", "G1", 120, 10);
    let b = add(&mut before, "Text 1", "G2", 190, 30); // its own anchor is in M1 too
    let c = add(&mut before, "Audio 1", "G3", 10, 10); // would stay put on its own
    let link = LinkId::new();
    for clip in before.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
        if [a, b, c].contains(&clip.id) {
            clip.link = Some(link);
        }
    }
    // The earliest member (G3, over M0) decides: an end trim of M0 does not move it...
    let after = follow(&before, |p| trim_end(p, "M0", 20), false);
    assert_eq!(
        [
            start(&after, "G1"),
            start(&after, "G2"),
            start(&after, "G3")
        ],
        [120, 190, 10]
    );
    // ...and the whole group moves with it to the removal point when M0 goes.
    let after = follow(&before, |p| remove_main(p, "M0"), true);
    assert_eq!(
        [
            start(&after, "G1"),
            start(&after, "G2"),
            start(&after, "G3")
        ],
        [110, 180, 0],
        "G3 goes to M0's removal point (0), the others keep their offsets"
    );
    let mut later = before.clone();
    later.tracks[4].clips.clear();
    let c = add(&mut later, "Audio 1", "G3", 110, 10);
    for clip in later.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
        if clip.id == c {
            clip.link = Some(link);
        }
    }
    let after = follow(&later, |p| trim_end(p, "M0", 20), false);
    assert_eq!(
        [
            start(&after, "G1"),
            start(&after, "G2"),
            start(&after, "G3")
        ],
        [100, 170, 90]
    );
    // Ripple delete of M1 does not take the group: G2 is not inside M1.
    let after = follow(&later, |p| remove_main(p, "M1"), true);
    assert_eq!(after.clips().count(), later.clips().count() - 1);

    // A member on a locked track keeps the whole group where it is.
    let mut locked = later.clone();
    locked.tracks[4].locked = true;
    let after = follow(&locked, |p| trim_end(p, "M0", 20), false);
    assert_eq!(
        [
            start(&after, "G1"),
            start(&after, "G2"),
            start(&after, "G3")
        ],
        [120, 190, 110]
    );
}

#[test]
fn link_groups_entirely_inside_a_removed_clip_are_deleted() {
    let mut before = project(&[100, 100]);
    let a = add(&mut before, "Video 2", "G1", 110, 10);
    let b = add(&mut before, "Audio 1", "G2", 120, 50);
    let link = LinkId::new();
    for clip in before.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
        if [a, b].contains(&clip.id) {
            clip.link = Some(link);
        }
    }
    let after = follow(&before, |p| remove_main(p, "M1"), true);
    assert!(find(&after, "G1").is_none() && find(&after, "G2").is_none());
    let after = follow(&before, |p| remove_main(p, "M1"), false);
    assert_eq!([start(&after, "G1"), start(&after, "G2")], [100, 110]);
}

#[test]
fn collisions_bump_to_the_nearest_free_track_upward_for_overlays() {
    let mut before = project(&[100, 100, 100]);
    add(&mut before, "Video 2", "X", 100, 50);
    add(&mut before, "Video 2", "Y", 210, 50);
    let after = follow(&before, |p| remove_main(p, "M1"), false);
    assert_eq!([start(&after, "X"), start(&after, "Y")], [100, 110]);
    assert_eq!(track_of(&after, "X"), "Video 2");
    assert_eq!(
        track_of(&after, "Y"),
        "Video 3",
        "upward, skipping the text track"
    );

    // A locked track is skipped: a new track is made directly above "Video 2".
    let mut locked = before.clone();
    locked.tracks[0].locked = true;
    let mut pool = IdPool::default();
    let after = follow_with(&locked, |p| remove_main(p, "M1"), false, &mut pool).unwrap();
    let names: Vec<_> = after.tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Video 3", "Text 1", "Video 4", "Video 2", "Video 1", "Audio 1", "Audio 2", "Audio 3"
        ]
    );
    assert_eq!(track_of(&after, "Y"), "Video 4");
    // Deterministic: the same input (and the same id pool, as on redo) gives the same project.
    let again = follow_with(&locked, |p| remove_main(p, "M1"), false, &mut pool).unwrap();
    assert_eq!(again, after);
}

#[test]
fn bumping_skips_tracks_holding_another_member_of_the_group() {
    let mut before = project(&[100, 100, 100]);
    add(&mut before, "Video 2", "X", 100, 50);
    let y = add(&mut before, "Video 2", "Y", 210, 50);
    let z = add(&mut before, "Video 3", "Z", 210, 50);
    let link = LinkId::new();
    for clip in before.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
        if [y, z].contains(&clip.id) {
            clip.link = Some(link);
        }
    }
    let after = follow(&before, |p| remove_main(p, "M1"), false);
    assert_eq!([start(&after, "Y"), start(&after, "Z")], [110, 110]);
    assert_eq!(track_of(&after, "Z"), "Video 3");
    let bumped = track_of(&after, "Y");
    assert!(
        !["Video 2", "Video 3"].contains(&bumped.as_str()),
        "{bumped}"
    );
}

#[test]
fn audio_bumps_downward_first() {
    let mut before = project(&[100, 100, 100]);
    add(&mut before, "Audio 2", "P", 100, 50);
    add(&mut before, "Audio 2", "Q", 210, 50);
    let after = follow(&before, |p| remove_main(p, "M1"), false);
    assert_eq!(track_of(&after, "Q"), "Audio 3");

    let mut full = before.clone();
    add(&mut full, "Audio 3", "Busy", 100, 100);
    let after = follow(&full, |p| remove_main(p, "M1"), false);
    assert_eq!(track_of(&after, "Q"), "Audio 1", "then upward");
}

#[test]
fn moved_clips_and_new_clips_do_not_follow() {
    let mut before = project(&[100, 100]);
    let moved = add(&mut before, "Video 2", "Moved", 150, 10);
    let mut after = before.clone();
    remove_main(&mut after, "M0");
    repack_main(&mut after);
    add(&mut after, "Video 3", "New", 150, 10);
    follow_main(
        &before,
        &mut after,
        Follow {
            moved: &[moved],
            delete_inside: true,
            track_ids: &mut IdPool::default(),
        },
    )
    .unwrap();
    assert_eq!(start(&after, "Moved"), 150);
    assert_eq!(start(&after, "New"), 150);
}

mod props {
    use proptest::prelude::*;

    use super::*;

    #[derive(Debug, Clone)]
    enum Edit {
        Remove { index: usize, delete_inside: bool },
        Insert { index: usize, duration: i64 },
        TrimStart { index: usize, by: i64 },
        TrimEnd { index: usize, by: i64 },
    }

    fn edit() -> impl Strategy<Value = Edit> {
        prop_oneof![
            (0usize..8, any::<bool>()).prop_map(|(index, delete_inside)| Edit::Remove {
                index,
                delete_inside
            }),
            (0usize..9, 1i64..500).prop_map(|(index, duration)| Edit::Insert { index, duration }),
            (0usize..8, 1i64..500).prop_map(|(index, by)| Edit::TrimStart { index, by }),
            (0usize..8, 1i64..500).prop_map(|(index, by)| Edit::TrimEnd { index, by }),
        ]
    }

    /// (track index, start, duration) of followers, non-overlapping per track.
    fn followers() -> impl Strategy<Value = Vec<(usize, i64, i64)>> {
        let lane = |track: usize| {
            prop::collection::vec((0i64..300, 1i64..300), 0..5).prop_map(move |parts| {
                let mut end = 0;
                parts
                    .into_iter()
                    .map(|(gap, duration)| {
                        let start = end + gap;
                        end = start + duration;
                        (track, start, duration)
                    })
                    .collect::<Vec<_>>()
            })
        };
        (lane(0), lane(1), lane(2), lane(4), lane(5))
            .prop_map(|(a, b, c, d, e)| [a, b, c, d, e].concat())
    }

    proptest! {
        /// A random main-track ripple keeps the project valid, never changes a follower's shape,
        /// loses only the followers a ripple delete takes, leaves locked tracks alone, and keeps
        /// the offset of every unclamped follower inside a surviving anchor.
        #[test]
        fn following_a_main_track_ripple(
            durations in prop::collection::vec(1i64..600, 1..8),
            placed in followers(),
            locks in prop::collection::vec(any::<bool>(), 7),
            edit in edit(),
        ) {
            let mut before = project(&durations);
            for (i, &(track, start, duration)) in placed.iter().enumerate() {
                let name = before.tracks[track].name.clone();
                add(&mut before, &name, &format!("F{i}"), start, duration);
            }
            for (track, locked) in before.tracks.iter_mut().zip(locks) {
                track.locked = locked && track.id != before.main_track;
            }
            prop_assert_eq!(before.validate(), Ok(()));

            let count = durations.len();
            let mut delete_inside = false;
            let mut after = before.clone();
            {
                let main = main_mut(&mut after);
                match edit {
                    Edit::Remove { index, delete_inside: d } => {
                        main.clips.remove(index % count);
                        delete_inside = d;
                    }
                    Edit::Insert { index, duration } => {
                        let clip = media_clip("New", before.media[0].id, 0, duration, true);
                        main.clips.insert(index % (count + 1), clip);
                    }
                    Edit::TrimStart { index, by } => {
                        let clip = &mut main.clips[index % count];
                        let by = by.min(clip.duration.flicks() - 1);
                        if let ClipSource::Media { source_in, .. } = &mut clip.source {
                            *source_in = *source_in + Time(by);
                        }
                        clip.duration = clip.duration - Time(by);
                    }
                    Edit::TrimEnd { index, by } => {
                        let clip = &mut main.clips[index % count];
                        clip.duration = Time((clip.duration.flicks() - by).max(1));
                    }
                }
            }
            repack_main(&mut after);
            let mut pool = IdPool::default();
            follow_main(&before, &mut after, Follow { moved: &[], delete_inside, track_ids: &mut pool })
                .expect("no links, so nothing fails");
            prop_assert_eq!(after.validate(), Ok(()));

            for track in before.tracks.iter().filter(|t| t.locked) {
                prop_assert_eq!(after.track(track.id), Some(track));
            }
            let old_main = &before.main().unwrap().clips;
            let new_main = &after.main().unwrap().clips;
            let mut lost = 0;
            for track in before.tracks.iter().filter(|t| t.id != before.main_track) {
                for clip in &track.clips {
                    let Some((_, moved)) = after.clip(clip.id) else {
                        lost += 1;
                        continue;
                    };
                    prop_assert_eq!(moved.duration, clip.duration);
                    prop_assert_eq!(&moved.source, &clip.source);
                    if track.locked {
                        continue;
                    }
                    if let Some(Anchor::Inside { clip: anchor, content }) = anchor_of(old_main, clip.start)
                        && let Some(new_anchor) = new_main.iter().find(|c| c.id == anchor)
                    {
                        let old_anchor = old_main.iter().find(|c| c.id == anchor).unwrap();
                        let offset = content - new_anchor.source_in();
                        if offset >= Time::ZERO && offset <= new_anchor.duration {
                            prop_assert_eq!(
                                moved.start - new_anchor.start,
                                clip.start - old_anchor.start - (new_anchor.source_in() - old_anchor.source_in())
                            );
                        }
                    }
                }
            }
            if !delete_inside {
                prop_assert_eq!(lost, 0);
            }
        }
    }
}
