use super::*;
use crate::command::{Batch, RemoveMedia};
use crate::media::samples;
use crate::model::LinkId;
use crate::{Document, MediaAsset};

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

/// A new project (main "Video 1", "Audio 1") with a 5 s video with audio, a 3 s song, a photo
/// and a silent video.
fn project() -> Project {
    let mut project = Project::new("Clips");
    let mut silent = samples::video(4);
    silent.info.audio.clear();
    project.media = vec![
        samples::video(1),
        samples::audio(2),
        samples::image(3),
        silent,
    ];
    project
}

fn media(project: &Project, index: usize) -> NewClipSource {
    NewClipSource::Media {
        media: project.media[index].id,
    }
}

const GRAY: NewClipSource = NewClipSource::Color {
    color: Rgba8::MIGRATED_GRAY,
};

fn add(
    project: &Project,
    sources: Vec<NewClipSource>,
    placement: Placement,
) -> (Project, Vec<ClipId>) {
    let mut command = AddClips::new(sources, placement);
    let ids = command.clip_ids().to_vec();
    (assert_reversible(&mut command, project), ids)
}

fn try_add(
    project: &Project,
    sources: Vec<NewClipSource>,
    placement: Placement,
) -> Result<(), CommandError> {
    let mut doc = Document::new(project.clone());
    let result = doc.execute(Box::new(AddClips::new(sources, placement)));
    if result.is_err() {
        assert_eq!(*doc.project(), *project, "a failed add changes nothing");
    }
    result
}

fn starts(track: &Track) -> Vec<i64> {
    track.clips.iter().map(|c| c.start.flicks()).collect()
}

const SECOND: i64 = crate::time::FLICKS_PER_SECOND;

#[test]
fn append_fills_the_main_track_and_routes_audio_only_media() {
    let project = project();
    let sources = vec![
        media(&project, 0),
        media(&project, 2),
        GRAY,
        media(&project, 1),
        media(&project, 3),
    ];
    let (after, ids) = add(&project, sources, Placement::Append);
    assert_eq!(ids.len(), 5);
    let main = after.main().unwrap();
    assert_eq!(starts(main), [0, 5 * SECOND, 10 * SECOND, 15 * SECOND]);
    assert_eq!(main.clips[0].id, ids[0]);
    assert_eq!(main.clips[0].name, "clip-1");
    assert!(
        main.clips[0].audio.is_some(),
        "video with audio keeps its sound"
    );
    assert!(main.clips[1].audio.is_none(), "images are silent");
    assert_eq!(main.clips[1].duration, DEFAULT_IMAGE_DURATION);
    assert_eq!(main.clips[2].duration, DEFAULT_COLOR_DURATION);
    assert_eq!(main.clips[2].name, "#808080ff");
    assert!(main.clips[3].audio.is_none(), "silent video");
    let audio = &after.tracks[1];
    assert_eq!(audio.clips.len(), 1);
    assert_eq!(audio.clips[0].id, ids[3]);
    assert_eq!(audio.clips[0].duration.flicks(), 3 * SECOND);
    assert!(audio.clips[0].visual.is_none() && audio.clips[0].audio.is_some());
    assert_eq!(after.media_users(project.media[0].id), 1);
}

#[test]
fn append_creates_an_audio_track_when_none_is_free() {
    let mut project = project();
    project.tracks[1].locked = true;
    let (after, ids) = add(&project, vec![media(&project, 1)], Placement::Append);
    assert_eq!(after.tracks.len(), 3);
    assert_eq!(
        after.tracks[1].clips[0].id, ids[0],
        "new track at the top of the audio section"
    );
    assert_eq!(after.tracks[1].kind, TrackKind::Audio);
    assert!(after.tracks[2].locked);
}

#[test]
fn insert_goes_to_the_nearest_boundary_and_the_followers_move() {
    let project = project();
    let (mut project, _) = add(
        &project,
        vec![media(&project, 0), media(&project, 0)],
        Placement::Append,
    );
    // B-roll over the second clip, music from 0.
    let (with_broll, _) = add(
        &project,
        vec![GRAY],
        Placement::Drop {
            track: TrackTarget::New { index: 0 },
            start: Time(6 * SECOND),
        },
    );
    project = with_broll;
    let (with_music, _) = add(
        &project,
        vec![media(&project, 1)],
        Placement::Drop {
            track: TrackTarget::Existing {
                track: project.tracks[2].id,
            },
            start: Time::ZERO,
        },
    );
    project = with_music;

    // 6.5 s is nearest to the boundary at 5 s.
    let (after, ids) = add(
        &project,
        vec![GRAY],
        Placement::Insert {
            at: Time(6 * SECOND + SECOND / 2),
        },
    );
    let main = after.main().unwrap();
    assert_eq!(main.clips[1].id, ids[0]);
    assert_eq!(starts(main), [0, 5 * SECOND, 10 * SECOND]);
    assert_eq!(
        after.tracks[0].clips[0].start.flicks(),
        11 * SECOND,
        "B-roll follows its clip"
    );
    assert_eq!(
        after.tracks[2].clips[0].start,
        Time::ZERO,
        "the music bed stays"
    );

    // Ties go to the earlier boundary; a drop on the magnetic main track is an insert.
    let (after, _) = add(
        &project,
        vec![GRAY],
        Placement::Drop {
            track: TrackTarget::Existing {
                track: project.main_track,
            },
            start: Time(SECOND / 2 * 5),
        },
    );
    assert_eq!(
        after.main().unwrap().clips[0].source,
        ClipSource::Color {
            color: Rgba8::MIGRATED_GRAY
        }
    );
}

#[test]
fn insert_on_a_non_magnetic_main_track_ripples_the_later_clips() {
    let mut project = project();
    project.magnetic = false;
    let (project, _) = add(
        &project,
        vec![GRAY],
        Placement::Drop {
            track: TrackTarget::Existing {
                track: project.main_track,
            },
            start: Time(10 * SECOND),
        },
    );
    let (after, _) = add(
        &project,
        vec![GRAY],
        Placement::Insert {
            at: Time(9 * SECOND),
        },
    );
    assert_eq!(starts(after.main().unwrap()), [10 * SECOND, 15 * SECOND]);
    // A drop on the non-magnetic main track stays where it is dropped, and overlaps fail.
    assert_eq!(
        try_add(
            &project,
            vec![GRAY],
            Placement::Drop {
                track: TrackTarget::Existing {
                    track: project.main_track
                },
                start: Time(12 * SECOND)
            },
        ),
        Err(CommandError::Overlap)
    );
}

#[test]
fn drops_on_overlay_and_audio_tracks() {
    let project = project();
    let (after, ids) = add(
        &project,
        vec![media(&project, 2), GRAY],
        Placement::Drop {
            track: TrackTarget::New { index: 7 },
            start: Time(SECOND),
        },
    );
    assert_eq!(after.tracks.len(), 3);
    assert_eq!(
        after.tracks[0].kind,
        TrackKind::Video,
        "clamped into the overlay section"
    );
    assert_eq!(after.main_index(), Some(1));
    assert_eq!(starts(&after.tracks[0]), [SECOND, 6 * SECOND]);
    assert_eq!(after.tracks[0].clips[1].id, ids[1]);

    // Overlap on the overlay.
    assert_eq!(
        try_add(
            &after,
            vec![GRAY],
            Placement::Drop {
                track: TrackTarget::Existing {
                    track: after.tracks[0].id
                },
                start: Time(2 * SECOND)
            },
        ),
        Err(CommandError::Overlap)
    );
    // Audio-only media onto a new track at index 0 lands in the audio section.
    let (after, _) = add(
        &project,
        vec![media(&project, 1)],
        Placement::Drop {
            track: TrackTarget::New { index: 0 },
            start: Time::ZERO,
        },
    );
    let names: Vec<_> = after.tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Video 1", "Audio 2", "Audio 1"]);
}

#[test]
fn incompatible_locked_and_unknown_targets_fail() {
    let mut project = project();
    let audio_track = project.tracks[1].id;
    let unknown = TrackId::new();
    let drop = |track| Placement::Drop {
        track: TrackTarget::Existing { track },
        start: Time::ZERO,
    };
    assert_eq!(
        try_add(&project, vec![media(&project, 0)], drop(audio_track)),
        Err(CommandError::TrackKindMismatch)
    );
    assert_eq!(
        try_add(&project, vec![media(&project, 1)], drop(project.main_track)),
        Err(CommandError::TrackKindMismatch)
    );
    assert_eq!(
        try_add(
            &project,
            vec![media(&project, 1)],
            Placement::Insert { at: Time::ZERO }
        ),
        Err(CommandError::TrackKindMismatch)
    );
    assert_eq!(
        try_add(&project, vec![GRAY], drop(unknown)),
        Err(CommandError::TrackNotFound(unknown))
    );
    let missing = MediaId::new();
    assert_eq!(
        try_add(
            &project,
            vec![NewClipSource::Media { media: missing }],
            Placement::Append
        ),
        Err(CommandError::MediaNotFound(missing))
    );
    assert_eq!(
        try_add(&project, vec![], Placement::Append),
        Err(CommandError::NothingToEdit)
    );
    let mut with_text = project.clone();
    crate::command::AddTrack::new(TrackKind::Text, Some(0))
        .apply(&mut with_text)
        .unwrap();
    let text_track = with_text.tracks[0].id;
    assert_eq!(
        try_add(&with_text, vec![GRAY], drop(text_track)),
        Err(CommandError::TrackKindMismatch)
    );

    project.tracks[0].locked = true;
    let main = project.main_track;
    assert_eq!(
        try_add(&project, vec![GRAY], Placement::Append),
        Err(CommandError::TrackLocked(main))
    );
    assert_eq!(
        try_add(&project, vec![GRAY], Placement::Insert { at: Time::ZERO }),
        Err(CommandError::TrackLocked(main))
    );
}

#[test]
fn times_past_the_supported_range_are_rejected() {
    let mut project = project();
    project.magnetic = false;
    let far = Time(crate::time::MAX_IPC_FLICKS - SECOND);
    let result = try_add(
        &project,
        vec![GRAY],
        Placement::Drop {
            track: TrackTarget::Existing {
                track: project.main_track,
            },
            start: far,
        },
    );
    assert!(
        matches!(result, Err(CommandError::InvalidResult(_))),
        "{result:?}"
    );
}

#[test]
fn redo_recreates_the_same_clip_and_track_ids() {
    let project = project();
    let mut doc = Document::new(project.clone());
    doc.execute(Box::new(AddClips::new(
        vec![media(&project, 1)],
        Placement::Drop {
            track: TrackTarget::New { index: 9 },
            start: Time::ZERO,
        },
    )))
    .unwrap();
    let after = doc.snapshot();
    assert!(doc.undo());
    assert_eq!(*doc.project(), project);
    assert!(doc.redo());
    assert_eq!(doc.snapshot(), after);
}

fn with_clips() -> Project {
    let project = project();
    let video = media(&project, 0);
    let (project, _) = add(&project, vec![GRAY, video, GRAY, video], Placement::Append);
    // A caption over the first video clip, a photo over the last card.
    let (project, _) = add(
        &project,
        vec![GRAY],
        Placement::Drop {
            track: TrackTarget::New { index: 0 },
            start: Time(6 * SECOND),
        },
    );
    let (mut project, _) = add(
        &project,
        vec![media(&project, 2)],
        Placement::Drop {
            track: TrackTarget::New { index: 0 },
            start: Time(11 * SECOND),
        },
    );
    project.background = CanvasBackground::Image {
        media: project.media[2].id,
    };
    project
}

#[test]
fn remove_clips_of_media_repacks_moves_followers_and_removes_emptied_tracks() {
    let project = with_clips();
    let video = project.media[0].id;
    let mut command = RemoveClipsOfMedia::new(video);
    let after = assert_reversible(&mut command, &project);
    assert_eq!(after.media_users(video), 0);
    let main = after.main().unwrap();
    assert_eq!(starts(main), [0, 5 * SECOND]);
    assert_eq!(
        after.clips().count(),
        project.clips().count() - 2,
        "followers are never deleted"
    );
    // The caption over the removed clip went to its removal point (5 s), the photo over the
    // last card follows it (11 s - 5 s).
    let overlay_starts: Vec<i64> = after.tracks[..after.main_index().unwrap()]
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.start.flicks()))
        .collect();
    assert_eq!(overlay_starts, [6 * SECOND, 5 * SECOND]);
    assert_eq!(
        after.background,
        CanvasBackground::Image {
            media: project.media[2].id
        }
    );

    // Removing the photo clears the background and removes the emptied overlay.
    let photo = project.media[2].id;
    let after = assert_reversible(&mut RemoveClipsOfMedia::new(photo), &project);
    assert_eq!(after.background, CanvasBackground::default());
    assert_eq!(after.tracks.len(), project.tracks.len() - 1);
}

#[test]
fn remove_media_batch_removes_clips_and_media_in_one_step() {
    let project = with_clips();
    let ids: Vec<MediaId> = vec![project.media[0].id, project.media[2].id];
    let mut doc = Document::new(project.clone());
    assert_eq!(
        doc.execute(Box::new(RemoveMedia::new(ids.clone()))),
        Err(CommandError::MediaInUse(ids[0])),
        "RemoveMedia alone still refuses"
    );
    let mut children: Vec<Box<dyn Command>> = ids
        .iter()
        .map(|&id| Box::new(RemoveClipsOfMedia::new(id)) as Box<dyn Command>)
        .collect();
    children.push(Box::new(RemoveMedia::new(ids.clone())));
    doc.execute(Box::new(Batch::new(HistoryLabel::RemoveMedia, children)))
        .unwrap();
    let after = doc.snapshot();
    assert_eq!(after.validate(), Ok(()));
    assert!(
        after
            .media
            .iter()
            .all(|m: &MediaAsset| !ids.contains(&m.id))
    );
    assert_eq!(doc.undo_label(), Some(HistoryLabel::RemoveMedia));
    assert!(doc.undo());
    assert_eq!(*doc.project(), project);
}

#[test]
fn remove_clips_of_media_fails_on_locked_tracks_and_normalises_links() {
    let mut project = with_clips();
    let video = project.media[0].id;
    let main = project.main_index().unwrap();
    project.tracks[main].locked = true;
    let mut doc = Document::new(project.clone());
    assert_eq!(
        doc.execute(Box::new(RemoveClipsOfMedia::new(video))),
        Err(CommandError::TrackLocked(project.main_track))
    );

    // A link between a removed clip and a surviving one is cleared.
    let mut project = with_clips();
    let video = project.media[0].id;
    let link = LinkId::new();
    let main = project.main_index().unwrap();
    project.tracks[main].clips[1].link = Some(link);
    project.tracks[0].clips[0].link = Some(link);
    assert_eq!(project.validate(), Ok(()));
    let after = assert_reversible(&mut RemoveClipsOfMedia::new(video), &project);
    assert!(after.clips().all(|c| c.link.is_none()));

    // Nothing uses the song: no change.
    let song = project.media[1].id;
    let after = assert_reversible(&mut RemoveClipsOfMedia::new(song), &project);
    assert_eq!(after, project);
}

mod props {
    use proptest::prelude::*;

    use super::*;

    fn source() -> impl Strategy<Value = usize> {
        0usize..5 // media 0..4, or 4 = a colour card
    }

    fn placement() -> impl Strategy<Value = (u8, i64, usize)> {
        (0u8..4, 0i64..30 * SECOND, 0usize..6)
    }

    proptest! {
        /// Random adds (append, insert, drops on existing and new tracks) keep the project valid,
        /// add exactly their clips when they succeed, change nothing when they fail, and undoing
        /// everything restores the start.
        #[test]
        fn random_adds_keep_the_project_valid(
            steps in prop::collection::vec((prop::collection::vec(source(), 1..4), placement()), 0..12),
            magnetic in any::<bool>(),
        ) {
            let mut start = project();
            start.magnetic = magnetic;
            let mut doc = Document::new(start.clone());
            for (picks, (kind, at, track)) in steps {
                let before = doc.snapshot();
                let sources: Vec<_> = picks
                    .iter()
                    .map(|&p| if p == 4 { GRAY } else { media(&before, p) })
                    .collect();
                let placement = match kind {
                    0 => Placement::Append,
                    1 => Placement::Insert { at: Time(at) },
                    2 => Placement::Drop {
                        track: TrackTarget::Existing { track: before.tracks[track % before.tracks.len()].id },
                        start: Time(at),
                    },
                    _ => Placement::Drop { track: TrackTarget::New { index: track as u32 }, start: Time(at) },
                };
                let command = AddClips::new(sources.clone(), placement);
                let ids = command.clip_ids().to_vec();
                match doc.execute(Box::new(command)) {
                    Ok(()) => {
                        let after = doc.project();
                        prop_assert_eq!(after.validate(), Ok(()));
                        prop_assert_eq!(after.clips().count(), before.clips().count() + sources.len());
                        for id in ids {
                            prop_assert!(after.clip(id).is_some());
                        }
                    }
                    Err(_) => prop_assert_eq!(&doc.snapshot(), &before),
                }
            }
            while doc.undo() {}
            prop_assert_eq!(doc.project(), &start);
        }
    }
}

#[test]
fn images_take_the_given_duration_and_media_clips_start_at_normal_speed() {
    let p = project();
    let mut command = AddClips::new(vec![media(&p, 2), media(&p, 0)], Placement::Append)
        .with_image_duration(Time::from_seconds(2));
    let ids = command.clip_ids().to_vec();
    let after = assert_reversible(&mut command, &p);
    let (_, image) = after.clip(ids[0]).unwrap();
    assert_eq!(image.duration, Time::from_seconds(2));
    let (_, video) = after.clip(ids[1]).unwrap();
    assert_eq!(video.start, Time::from_seconds(2));
    assert_eq!(
        video.source,
        ClipSource::media_at(p.media[0].id, Time::ZERO)
    );
    assert!(video.audio.as_ref().unwrap().preserve_pitch);
    let mut zero =
        AddClips::new(vec![media(&p, 2)], Placement::Append).with_image_duration(Time::ZERO);
    let mut edited = p.clone();
    assert_eq!(zero.apply(&mut edited), Err(CommandError::InvalidValue));
    // Without a duration, images get the default.
    let (after, ids) = add(&p, vec![media(&p, 2)], Placement::Append);
    assert_eq!(
        after.clip(ids[0]).unwrap().1.duration,
        DEFAULT_IMAGE_DURATION
    );
}
