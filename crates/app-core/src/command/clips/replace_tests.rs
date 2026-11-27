use super::*;
use crate::command::test_support::*;
use crate::media::samples;

/// `empty()` plus a silent 5 s video (index 3) and a 2 s video with sound (index 4).
fn with_more_media() -> Project {
    let mut p = empty();
    let mut silent = samples::video(4);
    silent.info.audio.clear();
    let mut short = samples::video(5);
    short.info.duration = Some(t(2.0));
    p.media.extend([silent, short]);
    p
}

fn media_of(p: &Project, id: ClipId) -> Option<MediaId> {
    clip(p, id).source.media()
}

#[test]
fn replace_keeps_place_link_and_props() {
    let mut p = with_more_media();
    p.magnetic = false;
    let mut v = video(&p, 1.0, 3.0, 1.5);
    v.visual.as_mut().unwrap().opacity.value = 0.5;
    v.audio.as_mut().unwrap().volume.value = 2.0;
    let v = p.push_clip(MAIN, v);
    let s = p.push_clip(AUDIO, sound(&p, 1.0, 3.0, 1.5));
    link(&mut p, &[v, s]);
    p.media.push(samples::video(6));
    let new = p.media[5].id;
    let after = assert_reversible(&mut ReplaceClipMedia::new(v, new), &p);
    let c = clip(&after, v);
    let old = clip(&p, v);
    assert_eq!(media_of(&after, v), Some(new));
    assert_eq!(
        (c.start, c.duration, c.source_in(), c.link),
        (old.start, old.duration, old.source_in(), old.link)
    );
    assert_eq!(c.visual, old.visual);
    assert_eq!(c.audio, old.audio);
    assert_eq!(c.name, "clip-6");
    assert_eq!(clip(&after, s), clip(&p, s), "linked members are unchanged");
}

#[test]
fn replace_resets_source_in_when_the_media_is_too_short_from_there() {
    let mut p = with_more_media();
    let v = p.push_clip(MAIN, video(&p, 0.0, 2.0, 2.0));
    let short = p.media[4].id;
    let after = assert_reversible(&mut ReplaceClipMedia::new(v, short), &p);
    assert_eq!(clip(&after, v).source_in(), Time::ZERO);
    assert_eq!(clip(&after, v).duration, t(2.0));
}

#[test]
fn replace_fails_when_the_media_is_shorter_than_the_clip() {
    let mut p = with_more_media();
    let v = p.push_clip(MAIN, video(&p, 0.0, 3.0, 0.0));
    let short = p.media[4].id;
    assert_eq!(
        try_apply(&p, ReplaceClipMedia::new(v, short)),
        Err(CommandError::MediaTooShort)
    );
}

#[test]
fn replace_with_an_image_or_from_a_colour_card() {
    let mut p = with_more_media();
    let v = p.push_clip(MAIN, video(&p, 0.0, 3.0, 1.0));
    let c = p.push_clip(MAIN, color(3.0, 7.0));
    let photo = p.media[2].id;
    let after = assert_reversible(&mut ReplaceClipMedia::new(v, photo), &p);
    let replaced = clip(&after, v);
    assert_eq!(replaced.source_in(), Time::ZERO);
    assert_eq!(replaced.audio, None, "an image has no sound");

    let after = assert_reversible(&mut ReplaceClipMedia::new(c, photo), &p);
    assert_eq!(media_of(&after, c), Some(photo));
    assert_eq!(
        clip(&after, c).duration,
        t(7.0),
        "images have no length limit"
    );
    let video = p.media[0].id;
    assert_eq!(
        try_apply(&p, ReplaceClipMedia::new(c, video)),
        Err(CommandError::MediaTooShort)
    );
}

#[test]
fn replace_sets_audio_from_the_new_media() {
    let mut p = with_more_media();
    let c = p.push_clip(MAIN, color(0.0, 2.0));
    let silent = p.media[3].id;
    let with_sound = p.media[0].id;
    // A card gets default sound from media with sound.
    let after = assert_reversible(&mut ReplaceClipMedia::new(c, with_sound), &p);
    assert_eq!(clip(&after, c).audio, Some(AudioProps::default()));
    // Silent media drops the sound.
    let v = clip(&after, c).id;
    let after = assert_reversible(&mut ReplaceClipMedia::new(v, silent), &after);
    assert_eq!(clip(&after, v).audio, None);
    // And silent → with sound gets it back (the sound was not detached).
    let after = assert_reversible(&mut ReplaceClipMedia::new(v, with_sound), &after);
    assert_eq!(clip(&after, v).audio, Some(AudioProps::default()));
}

#[test]
fn replace_keeps_detached_audio_detached() {
    let mut p = with_more_media();
    let mut v = video(&p, 0.0, 2.0, 0.0);
    v.audio = None;
    let v = p.push_clip(MAIN, v);
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 2.0, 0.0));
    link(&mut p, &[v, s]);
    p.media.push(samples::video(6));
    let new = p.media[5].id;
    let after = assert_reversible(&mut ReplaceClipMedia::new(v, new), &p);
    assert_eq!(clip(&after, v).audio, None);
}

#[test]
fn replace_checks_the_track_kind() {
    let mut p = with_more_media();
    let v = p.push_clip(MAIN, video(&p, 0.0, 2.0, 0.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 2.0, 0.0));
    let (song, photo, silent) = (p.media[1].id, p.media[2].id, p.media[3].id);
    assert_eq!(
        try_apply(&p, ReplaceClipMedia::new(v, song)),
        Err(CommandError::TrackKindMismatch),
        "audio-only media cannot go on a video track"
    );
    for media in [photo, silent] {
        assert_eq!(
            try_apply(&p, ReplaceClipMedia::new(s, media)),
            Err(CommandError::TrackKindMismatch),
            "an audio track needs media with sound"
        );
    }
    // An audio clip takes a song, keeping its sound props.
    let after = assert_reversible(&mut ReplaceClipMedia::new(s, song), &p);
    assert_eq!(media_of(&after, s), Some(song));
    assert_eq!(clip(&after, s).audio, clip(&p, s).audio);

    p.tracks.insert(0, Track::new(TrackKind::Text, "Text 1"));
    let caption = p.push_clip(0, Clip::text("t", t(0.0), t(1.0), "Hi"));
    assert_eq!(
        try_apply(&p, ReplaceClipMedia::new(caption, photo)),
        Err(CommandError::TrackKindMismatch)
    );
}

#[test]
fn replace_rejects_locked_tracks_and_unknown_ids() {
    let mut p = with_more_media();
    let v = p.push_clip(MAIN, video(&p, 0.0, 2.0, 0.0));
    let new = p.media[3].id;
    let missing_media = MediaId::new();
    assert_eq!(
        try_apply(&p, ReplaceClipMedia::new(v, missing_media)),
        Err(CommandError::MediaNotFound(missing_media))
    );
    let missing = ClipId::new();
    assert_eq!(
        try_apply(&p, ReplaceClipMedia::new(missing, new)),
        Err(CommandError::ClipNotFound(missing))
    );
    p.tracks[MAIN].locked = true;
    assert_eq!(
        try_apply(&p, ReplaceClipMedia::new(v, new)),
        Err(CommandError::TrackLocked(p.tracks[MAIN].id))
    );
}

#[test]
fn replace_keeps_speed_and_direction_and_checks_the_used_range() {
    let mut p = with_more_media();
    // 2 s at 2x uses 4 s of the 5 s video.
    let v = p.push_clip(MAIN, with_speed(video(&p, 0.0, 2.0, 1.0), 200, true));
    let silent = p.media[3].id;
    let after = assert_reversible(&mut ReplaceClipMedia::new(v, silent), &p);
    let c = clip(&after, v);
    assert_eq!(
        (c.speed(), c.reversed(), c.source_in()),
        (speed(200), true, t(1.0))
    );
    // The 2 s video cannot hold 4 s of source.
    let short = p.media[4].id;
    assert_eq!(
        try_apply(&p, ReplaceClipMedia::new(v, short)),
        Err(CommandError::MediaTooShort)
    );
    // An image plays at 1x.
    let photo = p.media[2].id;
    let after = assert_reversible(&mut ReplaceClipMedia::new(v, photo), &p);
    let c = clip(&after, v);
    assert_eq!(
        (c.speed(), c.reversed(), c.duration),
        (speed(100), false, t(2.0))
    );
}

#[test]
fn replace_of_a_freeze_frame_needs_a_video() {
    let mut p = with_more_media();
    let freeze = Clip {
        source: ClipSource::Freeze {
            media: p.media[0].id,
            source_time: t(3.0),
        },
        audio: None,
        ..color(0.0, 2.0)
    };
    let f = p.push_clip(MAIN, freeze);
    let short = p.media[4].id;
    let after = assert_reversible(&mut ReplaceClipMedia::new(f, short), &p);
    assert_eq!(
        clip(&after, f).source,
        ClipSource::Freeze {
            media: short,
            source_time: Time::ZERO
        },
        "3 s is past the end of a 2 s video"
    );
    assert_eq!(clip(&after, f).audio, None, "a freeze stays silent");
    let silent = p.media[3].id;
    let after = assert_reversible(&mut ReplaceClipMedia::new(f, silent), &p);
    assert_eq!(
        clip(&after, f).source,
        ClipSource::Freeze {
            media: silent,
            source_time: t(3.0)
        }
    );
    let photo = p.media[2].id;
    assert_eq!(
        try_apply(&p, ReplaceClipMedia::new(f, photo)),
        Err(CommandError::MediaKindMismatch {
            id: photo,
            expected: MediaKind::Video,
            got: MediaKind::Image
        })
    );
}
