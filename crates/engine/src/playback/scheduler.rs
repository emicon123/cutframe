//! The decode-ahead scheduler's lookahead (playback.md §7): which video clips start within the
//! next 1.5 s of playback (times the rate, wrapping at the loop end) and which source time each
//! needs first. The render thread hands the result to the reader pool
//! (`ReaderPool::prime`), which pre-seeks a reader for each one. Pure.
//!
//! A clip that continues its predecessor on the same track in the source (a split point: same
//! media, the previous clip ends where this one starts, in the timeline and in the source) needs
//! nothing: the reader of the previous clip gets there by decoding forward.

use app_core::media::MediaKind;
use app_core::{Clip, ClipSource, MediaId, Project, Time, TrackKind};

use super::transport::Rate;

/// How far ahead the scheduler looks at 1x.
pub const LOOKAHEAD: Time = Time(app_core::FLICKS_PER_SECOND * 3 / 2);

/// A clip that starts soon and needs a primed reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Need {
    pub media: MediaId,
    /// The sample time of the clip's first frame (as `build_plan` will ask for it).
    pub sample_time: Time,
    /// Timeline time at which the clip starts (after a loop wrap: in the next loop).
    pub starts: Time,
}

/// The clips of `project` that start within the lookahead after `t` while playing at `rate`,
/// earliest first, into `needs` (cleared first). Nothing in reverse: reverse playback is a timed
/// scrub, one request per presented frame.
pub fn lookahead(project: &Project, t: Time, rate: Rate, looping: bool, needs: &mut Vec<Need>) {
    needs.clear();
    if !rate.is_forward() {
        return;
    }
    let span = Time(LOOKAHEAD.flicks() * i64::from(rate.get()));
    let end = t + span;
    let duration = project.duration();
    collect(project, t, end, false, Time::ZERO, needs);
    if looping && duration > Time::ZERO && end > duration {
        // After the wrap the clips at 0 start too.
        let rest = (end - duration).min(duration);
        collect(project, Time::ZERO, rest, true, duration, needs);
    }
    needs.sort_by_key(|need| need.starts);
}

/// Clips starting in `(from, to]` (`[from, to]` when `inclusive`), with `starts` offset by
/// `offset`.
fn collect(
    project: &Project,
    from: Time,
    to: Time,
    inclusive: bool,
    offset: Time,
    needs: &mut Vec<Need>,
) {
    let rate = project.settings.frame_rate;
    let frame = rate.frame_duration();
    let half_frame = Time(frame.flicks() / 2);
    let drawn = project
        .main_index()
        .map_or(project.tracks.len(), |main| main + 1);
    for track in project.tracks[..drawn]
        .iter()
        .filter(|track| track.kind == TrackKind::Video && !track.hidden)
    {
        let first = track.clips.partition_point(|c| c.start < from);
        for (index, clip) in track.clips.iter().enumerate().skip(first) {
            if clip.start > to {
                break;
            }
            if !inclusive && clip.start == from {
                continue;
            }
            // TODO(0370 step 3): prime freeze frames (a still pace) too.
            let ClipSource::Media { media, .. } = clip.source else {
                continue;
            };
            if !is_video(project, media) {
                continue;
            }
            if index > 0 && continues(&track.clips[index - 1], clip) {
                continue;
            }
            // The first frame whose start lies inside the clip.
            let n = clip.start.flicks().div_euclid(frame.flicks())
                + i64::from(clip.start.flicks().rem_euclid(frame.flicks()) != 0);
            let first_frame = Time::from_frames(n, rate);
            let Some(source) = clip.source_time(first_frame) else {
                continue;
            };
            needs.push(Need {
                media,
                sample_time: source + half_frame,
                starts: clip.start + offset,
            });
        }
    }
}

fn is_video(project: &Project, media: MediaId) -> bool {
    project
        .media_asset(media)
        .is_some_and(|asset| asset.kind == MediaKind::Video && asset.info.video.is_some())
}

/// `next` follows `previous` directly in the timeline and in the same media's source.
// TODO(0370 step 4): the halves of a split reversed clip continue downwards.
fn continues(previous: &Clip, next: &Clip) -> bool {
    previous.end() == next.start
        && previous.source.media().is_some()
        && previous.source.media() == next.source.media()
        && previous.source_end() == Some(next.source_in())
}

#[cfg(test)]
mod tests {
    use app_core::media::samples;
    use app_core::{FLICKS_PER_SECOND, Rgba8};

    use super::*;

    fn s(seconds: f64) -> Time {
        Time((seconds * FLICKS_PER_SECOND as f64).round() as i64)
    }

    fn media_clip(media: MediaId, start: Time, duration: Time, source_in: Time) -> Clip {
        Clip {
            source: ClipSource::media_at(media, source_in),
            ..Clip::color("V", start, duration, Rgba8::BLACK)
        }
    }

    /// One video media; the main track holds `clips` as (start, duration, source_in) seconds.
    fn one_media(clips: &[(f64, f64, f64)]) -> (Project, MediaId) {
        let mut project = Project::new("Test");
        let asset = samples::video(1);
        let media = asset.id;
        project.media.push(asset);
        let main = project.main_index().unwrap();
        for &(start, duration, source_in) in clips {
            project.tracks[main]
                .clips
                .push(media_clip(media, s(start), s(duration), s(source_in)));
        }
        (project, media)
    }

    fn needs(project: &Project, t: f64, rate: i8, looping: bool) -> Vec<Need> {
        let mut needs = Vec::new();
        lookahead(project, s(t), Rate::new(rate).unwrap(), looping, &mut needs);
        needs
    }

    const HALF: i64 = FLICKS_PER_SECOND / 60;

    #[test]
    fn a_jump_cut_in_the_window_needs_a_primed_reader() {
        // 0..2 s plays source 0..2, 2..4 s jumps to source 10..12.
        let (project, media) = one_media(&[(0.0, 2.0, 0.0), (2.0, 2.0, 10.0)]);
        assert_eq!(needs(&project, 0.0, 1, false), [], "the cut is 2 s away");
        assert_eq!(
            needs(&project, 0.6, 1, false),
            [Need {
                media,
                sample_time: s(10.0) + Time(HALF),
                starts: s(2.0)
            }]
        );
        // The active clip itself needs nothing.
        assert_eq!(needs(&project, 2.0, 1, false), []);
        // At 2x the window is 3 s.
        assert_eq!(needs(&project, 0.1, 1, false), []);
        assert_eq!(needs(&project, 0.1, 2, false).len(), 1);
        assert_eq!(
            needs(&project, 0.6, -1, false),
            [],
            "reverse is a timed scrub"
        );
    }

    #[test]
    fn a_split_point_continues_the_reader() {
        // A split at 2 s: the second half continues the source.
        let (project, _) = one_media(&[(0.0, 2.0, 5.0), (2.0, 2.0, 7.0)]);
        assert_eq!(needs(&project, 1.0, 1, false), []);
        // A gap in the timeline is a new start even with continuous source times.
        let (project, _) = one_media(&[(0.0, 2.0, 5.0), (2.5, 2.0, 7.0)]);
        assert_eq!(needs(&project, 1.0, 1, false).len(), 1);
    }

    #[test]
    fn the_window_wraps_at_the_loop_end() {
        let (project, media) = one_media(&[(0.0, 2.0, 3.0), (2.0, 2.0, 0.0)]);
        // 0.5 s before the end: the clip at 0 starts the next loop 0.5 s from now.
        let wrapped = needs(&project, 3.5, 1, true);
        assert_eq!(
            wrapped,
            [Need {
                media,
                sample_time: s(3.0) + Time(HALF),
                starts: s(4.0)
            }]
        );
        assert_eq!(needs(&project, 3.5, 1, false), []);
    }

    #[test]
    fn hidden_tracks_images_and_colour_clips_need_nothing() {
        let (mut project, _) = one_media(&[(0.0, 1.0, 0.0)]);
        let image = samples::image(2);
        let image_id = image.id;
        project.media.push(image);
        let main = project.main_index().unwrap();
        project.tracks[main]
            .clips
            .push(media_clip(image_id, s(1.0), s(1.0), Time::ZERO));
        project.tracks[main]
            .clips
            .push(Clip::color("C", s(2.0), s(1.0), Rgba8::BLACK));
        assert_eq!(needs(&project, 0.5, 2, false), []);

        let (mut project, _) = one_media(&[(0.0, 1.0, 0.0), (1.0, 1.0, 9.0)]);
        project.tracks[main].hidden = true;
        assert_eq!(needs(&project, 0.5, 1, false), []);
    }

    #[test]
    fn a_clip_off_the_frame_grid_starts_at_its_first_whole_frame() {
        let frame = FLICKS_PER_SECOND / 30;
        let (project, media) = one_media(&[(0.0, 1.0, 0.0)]);
        let mut project = project;
        let main = project.main_index().unwrap();
        // Starts a third of a frame into frame 30.
        project.tracks[main].clips.push(media_clip(
            media,
            Time(30 * frame + frame / 3),
            s(1.0),
            s(4.0),
        ));
        let need = needs(&project, 0.5, 1, false)[0];
        assert_eq!(
            need.sample_time,
            s(4.0) + Time(frame - frame / 3) + Time(HALF)
        );
    }
}
