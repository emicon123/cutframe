//! Property tests over random edit sequences (design `timeline-editing.md` §11, task 0280 gate):
//! no command leaves an invalid project (overlaps, gaps on a magnetic main track, broken links),
//! undo and redo round-trip, and ripples on the main track keep followers on their anchors.

use proptest::prelude::*;

use super::test_support::*;
use super::*;
use crate::model::{Clip, ClipId, Project, TrackKind};
use crate::time::Time;
use crate::timeline::{Anchor, anchor_of};
use crate::{Document, EditPhase};

#[derive(Debug, Clone)]
enum Op {
    Move {
        clip: usize,
        ms: i64,
        track: usize,
    },
    Trim {
        clip: usize,
        end: bool,
        ms: i64,
    },
    Split {
        clip: Option<usize>,
        ms: i64,
    },
    Delete {
        clip: usize,
        ripple: bool,
    },
    DeleteGap {
        track: usize,
        ms: i64,
    },
    TrimToPlayhead {
        clip: Option<usize>,
        ms: i64,
        right: bool,
    },
    Magnetic(bool),
    Paste {
        clip: usize,
        ms: i64,
    },
    Duplicate {
        clip: usize,
    },
    Lock {
        track: usize,
    },
    Undo,
    Redo,
}

fn op() -> impl Strategy<Value = Op> {
    let ms = -1_000..40_000i64;
    // Times near the clips of the sample, where splits, gaps and playhead trims find something.
    let near = 0..12_000i64;
    prop_oneof![
        3 => (0..64usize, ms.clone(), 0..6usize).prop_map(|(clip, ms, track)| Op::Move { clip, ms, track }),
        3 => (0..64usize, any::<bool>(), ms.clone()).prop_map(|(clip, end, ms)| Op::Trim { clip, end, ms }),
        3 => (prop::option::of(0..64usize), near.clone()).prop_map(|(clip, ms)| Op::Split { clip, ms }),
        3 => (0..64usize, any::<bool>()).prop_map(|(clip, ripple)| Op::Delete { clip, ripple }),
        2 => (0..6usize, near.clone()).prop_map(|(track, ms)| Op::DeleteGap { track, ms }),
        2 => (prop::option::of(0..64usize), near, any::<bool>())
            .prop_map(|(clip, ms, right)| Op::TrimToPlayhead { clip, ms, right }),
        1 => any::<bool>().prop_map(Op::Magnetic),
        2 => (0..64usize, ms).prop_map(|(clip, ms)| Op::Paste { clip, ms }),
        2 => (0..64usize).prop_map(|clip| Op::Duplicate { clip }),
        1 => (0..6usize).prop_map(|track| Op::Lock { track }),
        1 => Just(Op::Undo),
        1 => Just(Op::Redo),
    ]
}

fn ms(ms: i64) -> Time {
    Time(ms * S / 1000)
}

/// Video and colour clips on the main track, overlays over them (one pair linked to sound), a
/// music bed and loose sound on the audio track.
fn sample(magnetic: bool, lengths: &[u8]) -> Project {
    let mut p = empty();
    p.magnetic = magnetic;
    let mut at = 0.0;
    for (i, &len) in lengths.iter().enumerate() {
        let len = 0.5 + f64::from(len % 8) * 0.5;
        if !magnetic {
            at += f64::from(len as u8 % 2);
        }
        if i % 2 == 0 {
            let len = len.min(4.0);
            let v = p.push_clip(MAIN, video(&p, at, len, 0.5));
            if i % 4 == 0 {
                let s = p.push_clip(AUDIO, sound(&p, at, len, 0.5));
                link(&mut p, &[v, s]);
            }
            at += len;
        } else {
            p.push_clip(MAIN, color(at, len));
            p.push_clip(OVERLAY, color(at + 0.25, 0.25));
            at += len;
        }
    }
    assert_eq!(p.validate(), Ok(()));
    p
}

fn pick(project: &Project, i: usize) -> Option<ClipId> {
    let clips: Vec<ClipId> = project.clips().map(|c| c.id).collect();
    (!clips.is_empty()).then(|| clips[i % clips.len()])
}

/// The command for `op` on `project`, if it builds one.
fn command(
    project: &Project,
    op: &Op,
    clipboard: &mut Option<ClipboardContent>,
) -> Option<Box<dyn Command>> {
    let track = |i: usize| {
        project
            .tracks
            .get(i % project.tracks.len().max(1))
            .map(|t| t.id)
    };
    Some(match *op {
        Op::Move {
            clip,
            ms: m,
            track: tr,
        } => {
            let id = pick(project, clip)?;
            let target = match project.tracks.get(tr) {
                Some(t) => TrackTarget::Existing { track: t.id },
                None => TrackTarget::New { index: 0 },
            };
            Box::new(MoveClips::new(vec![id], id, ms(m), target, true))
        }
        Op::Trim { clip, end, ms: m } => {
            let edge = if end { Edge::End } else { Edge::Start };
            Box::new(TrimClip::new(pick(project, clip)?, edge, ms(m), true))
        }
        Op::Split { clip, ms: m } => {
            let clips = match clip {
                Some(c) => vec![pick(project, c)?],
                None => vec![],
            };
            Box::new(SplitClips::new(clips, ms(m)))
        }
        Op::Delete { clip, ripple } => {
            let id = pick(project, clip)?;
            if ripple && clip % 3 == 0 {
                *clipboard = ClipboardContent::copy(project, &[id]).ok();
                Box::new(DeleteClips::cut(vec![id]))
            } else {
                Box::new(DeleteClips::new(vec![id], ripple))
            }
        }
        Op::DeleteGap { track: tr, ms: m } => Box::new(DeleteGap::new(track(tr)?, ms(m))),
        Op::TrimToPlayhead { clip, ms: m, right } => {
            let clips = match clip {
                Some(c) => vec![pick(project, c)?],
                None => vec![],
            };
            let side = if right { Side::Right } else { Side::Left };
            Box::new(TrimToPlayhead::new(clips, ms(m), side))
        }
        Op::Magnetic(on) => Box::new(SetMagnetic::new(on)),
        Op::Paste { clip, ms: m } => {
            if clipboard.is_none() || clip % 2 == 0 {
                *clipboard = ClipboardContent::copy(project, &[pick(project, clip)?]).ok();
            }
            Box::new(PasteClips::new(clipboard.clone()?, ms(m.max(0))))
        }
        Op::Duplicate { clip } => {
            Box::new(PasteClips::duplicate(project, &[pick(project, clip)?]).ok()?)
        }
        Op::Lock { .. } | Op::Undo | Op::Redo => return None,
    })
}

fn check_invariants(project: &Project) -> Result<(), TestCaseError> {
    prop_assert_eq!(project.validate(), Ok(()));
    for c in project.clips() {
        if let Some(end) = c.source_end() {
            prop_assert!(c.source_in() >= Time::ZERO && end <= t(5.0));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Random sequences of every timeline command keep the project valid, a failed command
    /// changes nothing, and undoing everything gives the start while redoing everything gives
    /// the end.
    #[test]
    fn random_edit_sequences_keep_the_project_valid_and_undo_round_trips(
        magnetic in any::<bool>(),
        lengths in prop::collection::vec(0u8..16, 2..8),
        ops in prop::collection::vec(op(), 1..32),
    ) {
        let start = sample(magnetic, &lengths);
        let mut doc = Document::new(start.clone());
        let mut clipboard = None;
        for op in &ops {
            match op {
                Op::Undo => { doc.undo(); }
                Op::Redo => { doc.redo(); }
                Op::Lock { track } => {
                    // Locking is not a command yet (task 0290): change the base and restart the
                    // history from there.
                    let mut project = doc.project().clone();
                    let n = project.tracks.len();
                    let track = &mut project.tracks[track % n];
                    track.locked = !track.locked;
                    doc = Document::new(project);
                }
                _ => {
                    let before = doc.project().clone();
                    let Some(command) = command(&before, op, &mut clipboard) else { continue };
                    let gesture = matches!(op, Op::Move { .. } | Op::Trim { .. });
                    let result = if gesture {
                        doc.edit(EditPhase::Commit, command)
                    } else {
                        doc.execute(command)
                    };
                    if result.is_err() {
                        prop_assert_eq!(doc.project(), &before, "a failed edit changes nothing");
                    }
                }
            }
            check_invariants(doc.project())?;
        }
        let base = doc.history_cursor();
        let end = doc.project().clone();
        while doc.undo() {}
        check_invariants(doc.project())?;
        while doc.redo() {}
        prop_assert_eq!(doc.history_cursor(), doc.history_len());
        if base == doc.history_len() {
            prop_assert_eq!(doc.project(), &end, "redo of everything gives the end state");
        }
    }

    /// Undo of a whole sequence without locks gives the exact start state.
    #[test]
    fn undoing_a_whole_sequence_restores_the_start(
        magnetic in any::<bool>(),
        lengths in prop::collection::vec(0u8..16, 2..8),
        ops in prop::collection::vec(op(), 1..24),
    ) {
        let start = sample(magnetic, &lengths);
        let mut doc = Document::new(start.clone());
        let mut clipboard = None;
        for op in &ops {
            let Some(command) = command(doc.project(), op, &mut clipboard) else { continue };
            let _ = doc.execute(command);
        }
        let end = doc.project().clone();
        while doc.undo() {}
        prop_assert_eq!(doc.project(), &start);
        while doc.redo() {}
        prop_assert_eq!(doc.project(), &end);
    }
}

// ---- Following ------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Ripple {
    Delete { clip: usize, ripple: bool },
    DeleteGap { ms: i64 },
    TrimToPlayhead { ms: i64, right: bool },
    MagneticOn,
    Paste { clip: usize, ms: i64 },
    Duplicate { clip: usize },
}

fn ripple() -> impl Strategy<Value = Ripple> {
    prop_oneof![
        (0..16usize, any::<bool>()).prop_map(|(clip, ripple)| Ripple::Delete { clip, ripple }),
        (0..12_000i64).prop_map(|ms| Ripple::DeleteGap { ms }),
        (0..12_000i64, any::<bool>()).prop_map(|(ms, right)| Ripple::TrimToPlayhead { ms, right }),
        Just(Ripple::MagneticOn),
        (0..16usize, 0..30_000i64).prop_map(|(clip, ms)| Ripple::Paste { clip, ms }),
        (0..16usize).prop_map(|clip| Ripple::Duplicate { clip }),
    ]
}

/// Video clips on the main track (with gaps when not magnetic), unlinked followers on the
/// overlay and audio tracks, a linked overlay + sound pair, and a locked audio track.
fn following_sample(magnetic: bool, lengths: &[u8], followers: &[(u8, u16)]) -> Project {
    let mut p = empty();
    p.magnetic = magnetic;
    let mut at = 0.0;
    for &len in lengths {
        if !magnetic {
            at += f64::from(len % 2);
        }
        let len = 1.0 + f64::from(len % 4) * 0.75;
        p.push_clip(MAIN, video(&p, at, len, 0.5));
        at += len;
    }
    let locked = p.tracks.len();
    p.tracks.push(Track::new(TrackKind::Audio, "Audio 2"));
    for &(track, start) in followers {
        let start = f64::from(start % 200) * 0.1;
        let (index, clip) = match track % 3 {
            0 => (OVERLAY, color(start, 0.3)),
            1 => (AUDIO, sound(&p, start, 0.3, 0.0)),
            _ => (locked, sound(&p, start, 0.3, 0.0)),
        };
        if crate::timeline::is_free(&p.tracks[index], clip.start, clip.end(), &[]) {
            p.push_clip(index, clip);
        }
    }
    let o = color(at + 1.0, 0.5);
    let s = sound(&p, at + 1.25, 0.5, 0.0);
    if crate::timeline::is_free(&p.tracks[OVERLAY], o.start, o.end(), &[])
        && crate::timeline::is_free(&p.tracks[AUDIO], s.start, s.end(), &[])
    {
        let ids = [p.push_clip(OVERLAY, o), p.push_clip(AUDIO, s)];
        link(&mut p, &ids);
    }
    p.tracks[locked].locked = true;
    assert_eq!(p.validate(), Ok(()));
    p
}

fn ripple_command(project: &Project, op: &Ripple) -> Option<Box<dyn Command>> {
    let main = project.main()?;
    let main_clip =
        |i: usize| (!main.clips.is_empty()).then(|| main.clips[i % main.clips.len()].id);
    Some(match *op {
        Ripple::Delete { clip, ripple } => {
            Box::new(DeleteClips::new(vec![main_clip(clip)?], ripple))
        }
        Ripple::DeleteGap { ms: m } => Box::new(DeleteGap::new(main.id, ms(m))),
        Ripple::TrimToPlayhead { ms: m, right } => {
            let side = if right { Side::Right } else { Side::Left };
            Box::new(TrimToPlayhead::new(vec![], ms(m), side))
        }
        Ripple::MagneticOn => Box::new(SetMagnetic::new(true)),
        Ripple::Paste { clip, ms: m } => {
            let content = ClipboardContent::copy(project, &[main_clip(clip)?]).ok()?;
            Box::new(PasteClips::new(content, ms(m)))
        }
        Ripple::Duplicate { clip } => {
            Box::new(PasteClips::duplicate(project, &[main_clip(clip)?]).ok()?)
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A main-track ripple keeps every unclamped follower on the same content moment of its
    /// anchor, never changes a follower's shape, keeps link groups together, deletes only what
    /// the command deletes, and leaves locked tracks bit-identical.
    #[test]
    fn main_track_ripples_keep_followers_on_their_anchors(
        magnetic in any::<bool>(),
        lengths in prop::collection::vec(0u8..16, 1..7),
        followers in prop::collection::vec((0u8..3, 0u16..200), 0..12),
        op in ripple(),
    ) {
        let before = following_sample(magnetic, &lengths, &followers);
        let Some(mut command) = ripple_command(&before, &op) else { return Ok(()) };
        let mut after = before.clone();
        if command.apply(&mut after).is_err() {
            return Ok(());
        }
        prop_assert_eq!(after.validate(), Ok(()));

        let locked: Vec<_> = before.tracks.iter().filter(|t| t.locked).collect();
        for track in locked {
            prop_assert_eq!(after.track(track.id), Some(track), "locked tracks stay put");
        }

        let old_main = &before.main().unwrap().clips;
        let new_main = &after.main().unwrap().clips;
        let main_id = before.main_track;
        let deletes = matches!(op, Ripple::Delete { .. });
        let followers: Vec<&Clip> = before
            .tracks
            .iter()
            .filter(|t| t.id != main_id && !t.locked)
            .flat_map(|t| t.clips.iter())
            .collect();
        for old in &followers {
            let Some((_, new)) = after.clip(old.id) else {
                prop_assert!(deletes, "only a delete removes followers");
                continue;
            };
            prop_assert_eq!((new.duration, new.source_in()), (old.duration, old.source_in()), "no shape change");
            if old.link.is_some() {
                continue;
            }
            if let Some(Anchor::Inside { clip, content }) = anchor_of(old_main, old.start)
                && let Some(anchor) = new_main.iter().find(|c| c.id == clip)
            {
                let offset = (content - anchor.source_in()).flicks();
                if (0..=anchor.duration.flicks()).contains(&offset) {
                    prop_assert_eq!(new.start, anchor.start + Time(offset), "follower keeps its anchor offset");
                }
            }
        }
        // Link groups keep their internal offsets.
        for old in &followers {
            let Some(link) = old.link else { continue };
            let mates: Vec<&&Clip> = followers.iter().filter(|c| c.link == Some(link)).collect();
            let new: Vec<Option<Time>> = mates.iter().map(|c| after.clip(c.id).map(|(_, n)| n.start - c.start)).collect();
            prop_assert!(new.windows(2).all(|w| w[0] == w[1]), "groups move together");
        }
    }
}
