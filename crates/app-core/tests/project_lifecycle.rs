//! Property tests for the project lifecycle: save → load round trips and project settings changes.

use app_core::command::{CommandError, SetProjectSettings};
use app_core::media::samples;
use app_core::serialization::{CURRENT_SCHEMA_VERSION, load, to_json};
use app_core::{
    AudioProps, Clip, ClipSource, CropInsets, Document, FitMode, FrameRate, MediaAsset, MediaId,
    Project, ProjectSettings, Rgba8, Time, Track, TrackKind, Vec2, VisualProps,
};
use proptest::prelude::*;

/// Frame rates the settings dialog offers, plus range edges.
const RATES: [(u32, u32); 10] = [
    (1, 1),
    (24000, 1001),
    (24, 1),
    (25, 1),
    (30000, 1001),
    (30, 1),
    (50, 1),
    (60000, 1001),
    (60, 1),
    (240, 1),
];

fn valid_settings() -> impl Strategy<Value = ProjectSettings> {
    (
        8u32..=4096,
        8u32..=4096,
        prop::sample::select(RATES.to_vec()),
        prop::sample::select(vec![44_100u32, 48_000]),
    )
        .prop_map(|(w, h, (num, den), sample_rate)| ProjectSettings {
            width: w * 2,
            height: h * 2,
            frame_rate: FrameRate::new(num, den).expect("aligned"),
            sample_rate,
        })
}

/// Settings that may break any range: odd or out-of-range sizes, 250 fps, unsupported sample rates.
fn any_settings() -> impl Strategy<Value = ProjectSettings> {
    prop_oneof![
        3 => valid_settings(),
        1 => (
            0u32..9000,
            0u32..9000,
            prop::sample::select(vec![(30, 1), (250, 1), (1, 2)]),
            prop::sample::select(vec![22_050u32, 48_000, 96_000]),
        )
            .prop_map(|(width, height, (num, den), sample_rate)| ProjectSettings {
                width,
                height,
                frame_rate: FrameRate::new(num, den).expect("aligned"),
                sample_rate,
            }),
    ]
}

/// (start, duration, name) of clips sorted by start and non-overlapping, from (gap, duration)
/// pairs. Times are arbitrary flicks, not frame-aligned. `contiguous` drops the gaps (a magnetic
/// main track).
fn spans(contiguous: bool) -> impl Strategy<Value = Vec<(Time, Time, String)>> {
    prop::collection::vec((0i64..1 << 40, 1i64..1 << 40, "\\PC{0,8}"), 0..6).prop_map(
        move |parts| {
            let mut end = 0;
            parts
                .into_iter()
                .map(|(gap, duration, name)| {
                    let start = if contiguous { end } else { end + gap };
                    end = start + duration;
                    (Time(start), Time(duration), name)
                })
                .collect()
        },
    )
}

/// Visual props with arbitrary in-range values.
fn visual() -> impl Strategy<Value = VisualProps> {
    (
        -2.0f32..2.0,
        0.01f32..10.0,
        -720.0f32..720.0,
        0.0f32..=1.0,
        0.0f32..0.49,
        any::<bool>(),
    )
        .prop_map(|(position, scale, rotation, opacity, crop, flip)| {
            let mut props = VisualProps::default();
            props.position.value = Vec2 {
                x: position,
                y: -position,
            };
            props.scale.value = Vec2 { x: scale, y: scale };
            props.rotation.value = rotation;
            props.opacity.value = opacity;
            props.crop.value = CropInsets {
                left: crop,
                top: crop,
                right: crop,
                bottom: 0.0,
            };
            props.flip.horizontal = flip;
            props.fit = if flip { FitMode::Fill } else { FitMode::Fit };
            props
        })
}

/// A long audio asset that every generated audio clip uses.
fn song() -> MediaAsset {
    let mut song = samples::audio(7);
    song.info.duration = Some(Time(1 << 46));
    song
}

fn track(kind: TrackKind, contiguous: bool, song: MediaId) -> impl Strategy<Value = Track> {
    (
        "\\PC{0,12}",
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        spans(contiguous),
        visual(),
        0.0f32..=4.0,
    )
        .prop_map(
            move |(name, muted, hidden, locked, spans, visual, volume)| {
                let mut track = Track::new(kind, name);
                track.muted = muted;
                track.hidden = hidden;
                track.locked = locked;
                track.clips = spans
                    .into_iter()
                    .map(|(start, duration, name)| match kind {
                        TrackKind::Video => Clip {
                            visual: Some(visual.clone()),
                            ..Clip::color(
                                name,
                                start,
                                duration,
                                Rgba8 {
                                    r: 1,
                                    g: 2,
                                    b: 3,
                                    a: 4,
                                },
                            )
                        },
                        TrackKind::Text => Clip::text(name.clone(), start, duration, name),
                        TrackKind::Audio => {
                            let mut audio = AudioProps::default();
                            audio.volume.value = volume;
                            Clip {
                                source: ClipSource::Media {
                                    media: song,
                                    source_in: start,
                                },
                                visual: None,
                                audio: Some(audio),
                                ..Clip::color(name, start, duration, Rgba8::BLACK)
                            }
                        }
                    })
                    .collect();
                track
            },
        )
}

/// A valid project: non-blank name, valid settings, unique ids, overlay tracks, a main track and
/// audio tracks with clips that fit them.
fn project() -> impl Strategy<Value = Project> {
    let song = song();
    let id = song.id;
    let overlay = prop::sample::select(vec![TrackKind::Video, TrackKind::Text])
        .prop_flat_map(move |kind| track(kind, false, id));
    (
        "\\PC{0,16}",
        valid_settings(),
        any::<bool>(),
        prop::collection::vec(overlay, 0..3),
        prop::collection::vec(track(TrackKind::Audio, false, id), 0..3),
    )
        .prop_flat_map(move |(name, settings, magnetic, overlays, audio)| {
            let song = song.clone();
            track(TrackKind::Video, magnetic, id).prop_map(move |main| {
                let mut project = Project::new(format!("P{name}"));
                project.settings = settings;
                project.magnetic = magnetic;
                project.main_track = main.id;
                project.tracks = overlays
                    .iter()
                    .cloned()
                    .chain(std::iter::once(main))
                    .chain(audio.iter().cloned())
                    .collect();
                project.media = vec![song.clone()];
                project
            })
        })
}

#[derive(Debug, Clone)]
enum Op {
    Set(ProjectSettings),
    Undo,
    Redo,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => any_settings().prop_map(Op::Set),
        1 => Just(Op::Undo),
        1 => Just(Op::Redo),
    ]
}

proptest! {
    /// save → load restores the identical project, and generated projects are valid.
    #[test]
    fn save_then_load_restores_the_identical_project(project in project()) {
        prop_assert_eq!(project.validate(), Ok(()));
        let loaded = load(&to_json(&project)).expect("load");
        prop_assert_eq!(loaded.source_version, CURRENT_SCHEMA_VERSION);
        prop_assert_eq!(loaded.project, project);
    }

    /// Settings changes with undo/redo follow a reference model and never touch clip times.
    #[test]
    fn settings_changes_leave_clip_times_bit_exact(
        project in project(),
        ops in prop::collection::vec(op(), 0..30),
    ) {
        let tracks = project.tracks.clone();
        let mut doc = Document::new(project.clone());
        let mut history = vec![project.settings];
        let mut cursor = 0;

        for op in ops {
            match op {
                Op::Set(settings) => {
                    match doc.execute(Box::new(SetProjectSettings::new(settings))) {
                        Ok(()) => {
                            history.truncate(cursor + 1);
                            history.push(settings);
                            cursor += 1;
                        }
                        Err(err) => {
                            prop_assert!(settings.validate().is_err());
                            prop_assert!(matches!(err, CommandError::InvalidSettings(_)));
                        }
                    }
                }
                Op::Undo => {
                    prop_assert_eq!(doc.undo(), cursor > 0);
                    cursor = cursor.saturating_sub(1);
                }
                Op::Redo => {
                    let redone = doc.redo();
                    prop_assert_eq!(redone, cursor + 1 < history.len());
                    if redone {
                        cursor += 1;
                    }
                }
            }
            prop_assert_eq!(doc.project().settings, history[cursor]);
            prop_assert_eq!(&doc.project().tracks, &tracks);
            prop_assert_eq!(doc.project().validate(), Ok(()));
        }

        while doc.undo() {}
        prop_assert_eq!(&*doc.snapshot(), &project);
    }
}
