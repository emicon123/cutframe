use super::*;
use crate::command::test_support::*;
use crate::{Document, EditPhase};

/// Main: a video with sound and a colour card; the video's detached-style sound on Audio 1.
fn sample() -> (Project, ClipId, ClipId, ClipId) {
    let mut p = empty();
    let v = p.push_clip(MAIN, video(&p, 0.0, 4.0, 0.0));
    let c = p.push_clip(MAIN, color(4.0, 2.0));
    let s = p.push_clip(AUDIO, sound(&p, 0.0, 3.0, 0.0));
    (p, v, c, s)
}

fn set(clips: Vec<ClipId>, value: PropertyValue) -> SetClipProperty {
    SetClipProperty::new(clips, value)
}

#[test]
fn audio_properties_go_to_clips_that_sound_and_skip_the_others() {
    let (p, v, c, s) = sample();
    let after = assert_reversible(&mut set(vec![v, c, s], PropertyValue::Volume(2.5)), &p);
    assert_eq!(clip(&after, v).audio.as_ref().unwrap().volume.value, 2.5);
    assert_eq!(clip(&after, s).audio.as_ref().unwrap().volume.value, 2.5);
    assert_eq!(clip(&after, c), clip(&p, c));

    let after = assert_reversible(&mut set(vec![s], PropertyValue::PreservePitch(false)), &p);
    assert!(!clip(&after, s).audio.as_ref().unwrap().preserve_pitch);
}

#[test]
fn visual_properties_go_to_clips_that_draw() {
    let (p, v, c, s) = sample();
    let after = assert_reversible(&mut set(vec![v, c, s], PropertyValue::Opacity(0.25)), &p);
    assert_eq!(clip(&after, v).visual.as_ref().unwrap().opacity.value, 0.25);
    assert_eq!(clip(&after, c).visual.as_ref().unwrap().opacity.value, 0.25);
    assert_eq!(clip(&after, s), clip(&p, s));
    let fit = PropertyValue::Fit(crate::props::FitMode::Fill);
    let after = assert_reversible(&mut set(vec![c], fit), &p);
    assert_eq!(
        clip(&after, c).visual.as_ref().unwrap().fit,
        crate::props::FitMode::Fill
    );
}

#[test]
fn no_clip_with_the_property_is_not_applicable() {
    let (p, _, c, s) = sample();
    assert_eq!(
        try_apply(&p, set(vec![c], PropertyValue::FadeIn(t(1.0)))),
        Err(CommandError::PropertyNotApplicable)
    );
    assert_eq!(
        try_apply(&p, set(vec![s], PropertyValue::Rotation(10.0))),
        Err(CommandError::PropertyNotApplicable)
    );
    assert_eq!(
        try_apply(&p, set(vec![], PropertyValue::Rotation(10.0))),
        Err(CommandError::NothingToEdit)
    );
    let missing = ClipId::new();
    assert_eq!(
        try_apply(&p, set(vec![missing], PropertyValue::Rotation(1.0))),
        Err(CommandError::ClipNotFound(missing))
    );
}

#[test]
fn out_of_range_values_are_invalid() {
    let (p, v, c, _) = sample();
    let cases = [
        PropertyValue::Volume(4.5),
        PropertyValue::Volume(f32::NAN),
        PropertyValue::Opacity(1.5),
        PropertyValue::Rotation(f32::INFINITY),
        PropertyValue::Scale(crate::props::Vec2 { x: 0.0, y: 1.0 }),
        PropertyValue::FadeIn(Time(-1)),
        PropertyValue::FadeOut(Time(-1)),
        PropertyValue::Crop(crate::props::CropInsets {
            left: 0.5,
            right: 0.5,
            ..Default::default()
        }),
    ];
    for value in cases {
        assert_eq!(
            try_apply(&p, set(vec![v, c], value)),
            Err(CommandError::InvalidValue),
            "{value:?}"
        );
    }
    assert!(try_apply(&p, set(vec![v], PropertyValue::Volume(4.0))).is_ok());
    assert!(try_apply(&p, set(vec![v], PropertyValue::Volume(0.0))).is_ok());
}

#[test]
fn fades_clamp_to_the_room_the_other_fade_leaves() {
    let (mut p, v, _, s) = sample();
    p.tracks[MAIN].clips[0].audio.as_mut().unwrap().fade_out = t(1.0);
    let after = assert_reversible(&mut set(vec![v, s], PropertyValue::FadeIn(t(10.0))), &p);
    // 4 s clip with a 1 s fade-out; 3 s clip without one.
    assert_eq!(clip(&after, v).audio.as_ref().unwrap().fade_in, t(3.0));
    assert_eq!(clip(&after, s).audio.as_ref().unwrap().fade_in, t(3.0));
    let after = assert_reversible(&mut set(vec![v], PropertyValue::FadeOut(t(10.0))), &after);
    assert_eq!(clip(&after, v).audio.as_ref().unwrap().fade_out, t(1.0));
}

#[test]
fn only_clips_that_have_the_property_must_be_unlocked() {
    let (mut p, v, c, s) = sample();
    p.tracks[AUDIO].locked = true;
    assert_eq!(
        try_apply(&p, set(vec![v, s], PropertyValue::Volume(0.5))),
        Err(CommandError::TrackLocked(p.tracks[AUDIO].id))
    );
    // The sound has no opacity, so its lock does not matter.
    let after = try_apply(&p, set(vec![c, s], PropertyValue::Opacity(0.5))).unwrap();
    assert_eq!(clip(&after, c).visual.as_ref().unwrap().opacity.value, 0.5);
}

#[test]
fn an_unchanged_value_changes_nothing() {
    let (p, v, c, _) = sample();
    assert!(
        !set(vec![v, c], PropertyValue::Opacity(1.0))
            .changes(&p)
            .unwrap()
    );
    assert!(
        set(vec![v, c], PropertyValue::Opacity(0.9))
            .changes(&p)
            .unwrap()
    );
    assert_eq!(
        set(vec![c], PropertyValue::Volume(1.0)).changes(&p),
        Err(CommandError::PropertyNotApplicable)
    );
}

#[test]
fn a_slider_drag_is_one_history_entry() {
    let (p, v, _, _) = sample();
    let mut doc = Document::new(p.clone());
    for volume in [1.5, 2.0, 3.0] {
        doc.edit(
            EditPhase::Update,
            Box::new(set(vec![v], PropertyValue::Volume(volume))),
        )
        .unwrap();
        assert_eq!(
            clip(doc.project(), v).audio.as_ref().unwrap().volume.value,
            volume
        );
    }
    // A rejected update keeps the last valid value.
    assert!(
        doc.edit(
            EditPhase::Update,
            Box::new(set(vec![v], PropertyValue::Volume(9.0)))
        )
        .is_err()
    );
    assert_eq!(
        clip(doc.project(), v).audio.as_ref().unwrap().volume.value,
        3.0
    );
    doc.edit(
        EditPhase::Commit,
        Box::new(set(vec![v], PropertyValue::Volume(2.0))),
    )
    .unwrap();
    assert_eq!(doc.history_len(), 1);
    assert_eq!(doc.undo_label(), Some(HistoryLabel::SetClipProperty));
    assert!(doc.undo());
    assert_eq!(doc.project(), &p);
    // Cancel restores the base.
    doc.edit(
        EditPhase::Update,
        Box::new(set(vec![v], PropertyValue::Volume(0.5))),
    )
    .unwrap();
    assert!(doc.gesture_cancel());
    assert_eq!(doc.project(), &p);
}
