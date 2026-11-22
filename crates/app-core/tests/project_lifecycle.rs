//! Property tests for the project lifecycle: save → load round trips and project settings changes.

use app_core::command::{CommandError, SetProjectSettings};
use app_core::serialization::{CURRENT_SCHEMA_VERSION, load, to_json};
use app_core::{
    Clip, ClipId, Document, FrameRate, Project, ProjectSettings, Time, Track, TrackKind,
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

/// Clips sorted by start and non-overlapping, from (gap, duration) pairs. Times are arbitrary
/// flicks, not frame-aligned.
fn clips() -> impl Strategy<Value = Vec<Clip>> {
    prop::collection::vec((0i64..1 << 40, 1i64..1 << 40, "\\PC{0,8}"), 0..6).prop_map(|parts| {
        let mut end = 0;
        parts
            .into_iter()
            .map(|(gap, duration, name)| {
                let start = end + gap;
                end = start + duration;
                Clip {
                    id: ClipId::new(),
                    name,
                    start: Time(start),
                    duration: Time(duration),
                }
            })
            .collect()
    })
}

fn track() -> impl Strategy<Value = Track> {
    (
        prop::sample::select(vec![TrackKind::Video, TrackKind::Audio, TrackKind::Text]),
        "\\PC{0,12}",
        any::<bool>(),
        any::<bool>(),
        clips(),
    )
        .prop_map(|(kind, name, muted, hidden, clips)| {
            let mut track = Track::new(kind, name);
            track.muted = muted;
            track.hidden = hidden;
            track.clips = clips;
            track
        })
}

/// A valid project: non-blank name, valid settings, unique ids, well-formed tracks.
fn project() -> impl Strategy<Value = Project> {
    (
        "\\PC{0,16}",
        valid_settings(),
        prop::collection::vec(track(), 0..5),
    )
        .prop_map(|(name, settings, tracks)| {
            let mut project = Project::new(format!("P{name}"));
            project.settings = settings;
            project.tracks = tracks;
            project
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
