//! What the preview plays (playback.md §8, task 0340): the timeline, or one media item on its
//! own (source preview, F1.17).
//!
//! [`PlaySource`] is the project every playback thread reads: the render thread (plans, the
//! lookahead), the transport (its range and frame grid), the audio feeder (the mixer's voices)
//! and the silent clock. For the [`PlaybackTarget::Timeline`] it is the shell's snapshot. For a
//! [`PlaybackTarget::Media`] target it is a **source project** built from that media
//! ([`source_project`]): one clip of the whole media on one track, so the media plays through
//! the unchanged render graph and mixer:
//!
//! - the canvas is the media's display size (audio-only media: the timeline's canvas, black), the
//!   clip is fitted, an image is shown still for [`DEFAULT_IMAGE_DURATION`];
//! - the frame grid is the media's own frame rate when the flick grid allows it (else the nearest
//!   common rate within 1 %, else the timeline's rate), so steps move by source frames;
//! - the clip has audio props when the media has sound: the mixer's only voice, which reads the
//!   media's default audio stream (the first listed, `MediaInfo::audio`).
//!
//! The source project is rebuilt lazily, lock-free, when the shell publishes a new timeline
//! snapshot (relink, offline status, removal). Media removed from the project keeps its clip but
//! loses its asset: an offline placeholder, silent.

use std::sync::Arc;

use app_core::model::DEFAULT_IMAGE_DURATION;
use app_core::{
    AudioProps, CanvasBackground, Clip, ClipId, ClipSource, FrameRate, MediaAsset, MediaId,
    MediaKind, Project, ProjectId, Track, TrackId, TrackKind, VisualProps,
};
use arc_swap::{ArcSwap, ArcSwapOption};

/// What the preview plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaybackTarget {
    Timeline,
    /// One media item on its own; transport times are media times.
    Media(MediaId),
}

/// A project the engine threads read without locks.
pub trait ProjectSource: Send + Sync {
    /// The project now. Cheap (an `Arc` clone); may allocate when a new snapshot arrived.
    fn project(&self) -> Arc<Project>;
}

impl ProjectSource for ArcSwap<Project> {
    fn project(&self) -> Arc<Project> {
        self.load_full()
    }
}

/// `set_target` named media that is not in the timeline project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("media {0} is not in the project")]
pub struct UnknownMedia(pub MediaId);

/// A media target's source project and the timeline snapshot it was built from.
#[derive(Debug)]
struct Programme {
    media: MediaId,
    from: Arc<Project>,
    project: Arc<Project>,
}

/// The project the preview plays. See the module docs.
#[derive(Debug)]
pub struct PlaySource {
    timeline: Arc<ArcSwap<Project>>,
    /// `None`: the timeline plays.
    media: ArcSwapOption<Programme>,
}

impl PlaySource {
    /// Plays the timeline in `timeline` (the shell's snapshot) until a media target is set.
    pub fn new(timeline: Arc<ArcSwap<Project>>) -> Self {
        Self {
            timeline,
            media: ArcSwapOption::empty(),
        }
    }

    /// The shell's timeline snapshot.
    pub fn timeline(&self) -> &Arc<ArcSwap<Project>> {
        &self.timeline
    }

    pub fn target(&self) -> PlaybackTarget {
        self.media
            .load()
            .as_ref()
            .map_or(PlaybackTarget::Timeline, |p| PlaybackTarget::Media(p.media))
    }

    /// Switches what [`ProjectSource::project`] returns. A media target must be in the current
    /// timeline snapshot. The caller (the transport) serialises target changes.
    pub(crate) fn set_target(&self, target: PlaybackTarget) -> Result<(), UnknownMedia> {
        let prepared = self.prepare(target)?;
        self.switch(prepared);
        Ok(())
    }

    /// The first half of [`Self::set_target`]: checks and builds the target without switching.
    pub(crate) fn prepare(&self, target: PlaybackTarget) -> Result<Prepared, UnknownMedia> {
        Ok(Prepared(match target {
            PlaybackTarget::Timeline => None,
            PlaybackTarget::Media(media) => {
                let from = self.timeline.load_full();
                let asset = from.media_asset(media).ok_or(UnknownMedia(media))?;
                let project = Arc::new(source_project(&from, asset));
                Some(Arc::new(Programme {
                    media,
                    from,
                    project,
                }))
            }
        }))
    }

    /// The second half of [`Self::set_target`]: switches to a prepared target.
    pub(crate) fn switch(&self, prepared: Prepared) {
        self.media.store(prepared.0);
    }
}

/// A checked target, ready to switch to ([`PlaySource::prepare`]).
#[derive(Debug)]
pub(crate) struct Prepared(Option<Arc<Programme>>);

impl ProjectSource for PlaySource {
    fn project(&self) -> Arc<Project> {
        let Some(programme) = self.media.load_full() else {
            return self.timeline.load_full();
        };
        let timeline = self.timeline.load_full();
        if Arc::ptr_eq(&programme.from, &timeline) {
            return Arc::clone(&programme.project);
        }
        let project = Arc::new(match timeline.media_asset(programme.media) {
            Some(asset) => source_project(&timeline, asset),
            None => {
                let mut gone = Project::clone(&programme.project);
                gone.media.clear();
                gone
            }
        });
        let next = Arc::new(Programme {
            media: programme.media,
            from: timeline,
            project: Arc::clone(&project),
        });
        // Only if no target change came in between; another reader's identical rebuild may win.
        let current = Some(programme);
        self.media.compare_and_swap(&current, Some(next));
        project
    }
}

/// Common frame rates a probed average rate snaps to when it is not on the flick grid.
const COMMON_RATES: [(u32, u32); 11] = [
    (24_000, 1001),
    (24, 1),
    (25, 1),
    (30_000, 1001),
    (30, 1),
    (48, 1),
    (50, 1),
    (60_000, 1001),
    (60, 1),
    (100, 1),
    (120, 1),
];

/// The frame grid of `asset`'s source project: its own rate (above 0 and at most 240 fps) when
/// the flick grid allows it, else the nearest [`COMMON_RATES`] entry within 1 %, else `None`.
fn source_frame_rate(asset: &MediaAsset) -> Option<FrameRate> {
    let rate = asset.info.video.as_ref()?.frame_rate?;
    let fps = rate.as_f64();
    if !(fps > 0.0 && fps <= 240.0) {
        return None;
    }
    FrameRate::new(rate.num(), rate.den()).ok().or_else(|| {
        COMMON_RATES
            .iter()
            .filter_map(|&(num, den)| FrameRate::new(num, den).ok())
            .map(|common| (((common.as_f64() - fps) / fps).abs(), common))
            .filter(|(error, _)| *error <= 0.01)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, common)| common)
    })
}

/// The source project of `asset` (see the module docs). Ids are derived from the media id, so
/// rebuilds give equal plans and keep the readers.
pub fn source_project(timeline: &Project, asset: &MediaAsset) -> Project {
    let id = asset.id.0;
    let mut settings = timeline.settings;
    if let Some(video) = &asset.info.video {
        (settings.width, settings.height) = video.display_size();
    }
    if let Some(rate) = source_frame_rate(asset) {
        settings.frame_rate = rate;
    }
    let duration = match asset.kind {
        MediaKind::Image => DEFAULT_IMAGE_DURATION,
        MediaKind::Video | MediaKind::Audio => asset
            .info
            .duration
            .filter(|d| *d > app_core::Time::ZERO)
            .unwrap_or(DEFAULT_IMAGE_DURATION),
    };
    let sounds = asset.kind != MediaKind::Image && !asset.info.audio.is_empty();
    let clip = Clip {
        id: ClipId(id),
        name: asset.file_name().to_owned(),
        start: app_core::Time::ZERO,
        duration,
        source: ClipSource::media_at(asset.id, app_core::Time::ZERO),
        link: None,
        visual: Some(VisualProps::default()),
        audio: sounds.then(AudioProps::default),
    };
    let track = Track {
        id: TrackId(id),
        clips: vec![clip],
        ..Track::new(TrackKind::Video, "Source")
    };
    Project {
        schema_version: timeline.schema_version,
        id: ProjectId(id),
        name: asset.file_name().to_owned(),
        settings,
        background: CanvasBackground::default(),
        main_track: track.id,
        magnetic: false,
        tracks: vec![track],
        media: vec![asset.clone()],
    }
}

#[cfg(test)]
mod tests {
    use app_core::media::samples;
    use app_core::{Rational, Time};

    use super::*;

    fn timeline_with(media: Vec<MediaAsset>) -> Arc<ArcSwap<Project>> {
        let mut project = Project::new("Timeline");
        project.settings.frame_rate = FrameRate::FPS_25;
        project.media = media;
        Arc::new(ArcSwap::from_pointee(project))
    }

    #[test]
    fn a_video_plays_alone_on_its_own_canvas_grid_and_length() {
        let video = samples::video(1);
        let timeline = Project::new("T");
        let project = source_project(&timeline, &video);
        assert_eq!(
            (project.settings.width, project.settings.height),
            (1920, 1080)
        );
        assert_eq!(project.settings.frame_rate, FrameRate::FPS_30);
        assert_eq!(project.duration(), Time::from_seconds(5));
        assert_eq!(project.tracks.len(), 1);
        assert_eq!(project.media, vec![video.clone()]);
        let clip = &project.tracks[0].clips[0];
        assert_eq!(clip.source, ClipSource::media_at(video.id, Time::ZERO));
        assert!(clip.audio.is_some(), "its sound is the only voice");
        // One video layer, fitted to the whole canvas.
        let plan = crate::render::build_plan(&project, Time::from_seconds(1), (960, 540));
        assert_eq!(plan.layers.len(), 1);
        assert_eq!(
            plan.layers[0].transform,
            crate::render::Affine2::rect(0.0, 0.0, 1920.0, 1080.0)
        );
        assert!(matches!(
            plan.layers[0].content,
            crate::render::LayerContent::Video { media, .. } if media == video.id
        ));
    }

    #[test]
    fn images_are_still_and_silent_and_audio_is_the_only_voice() {
        let timeline = Project::new("T");
        let image = source_project(&timeline, &samples::image(2));
        assert_eq!(image.duration(), DEFAULT_IMAGE_DURATION);
        assert_eq!((image.settings.width, image.settings.height), (4032, 3024));
        assert_eq!(image.settings.frame_rate, timeline.settings.frame_rate);
        assert!(image.tracks[0].clips[0].audio.is_none());
        let plan = crate::render::build_plan(&image, Time::from_seconds(3), (400, 300));
        assert!(matches!(
            plan.layers[0].content,
            crate::render::LayerContent::Image { .. }
        ));

        let song = samples::audio(3);
        let audio = source_project(&timeline, &song);
        assert_eq!(audio.duration(), Time::from_seconds(3));
        assert_eq!((audio.settings.width, audio.settings.height), (1920, 1080));
        let plan = crate::render::build_plan(&audio, Time::ZERO, (960, 540));
        assert!(plan.layers.is_empty(), "nothing to draw");
        // The mixer's voices: exactly the one clip.
        let voices: Vec<_> = audio
            .clips()
            .filter(|c| c.audio.is_some() && c.source.media() == Some(song.id))
            .collect();
        assert_eq!(voices.len(), 1);
    }

    #[test]
    fn frame_rates_off_the_flick_grid_snap_to_a_common_rate() {
        let mut video = samples::video(1);
        let set = |video: &mut MediaAsset, num, den| {
            video.info.video.as_mut().unwrap().frame_rate = Some(Rational::new(num, den).unwrap());
        };
        set(&mut video, 30_000, 1001);
        assert_eq!(source_frame_rate(&video), Some(FrameRate::NTSC_30));
        // A phone's variable-rate average.
        set(&mut video, 2_987, 100);
        assert_eq!(source_frame_rate(&video), Some(FrameRate::NTSC_30));
        set(&mut video, 1_300, 97);
        assert_eq!(source_frame_rate(&video), None, "13.4 fps is not common");
        set(&mut video, 90_000, 1);
        assert_eq!(source_frame_rate(&video), None, "a time base, not a rate");
    }

    #[test]
    fn targets_switch_the_played_project_and_keep_the_timeline() {
        let video = samples::video(1);
        let timeline = timeline_with(vec![video.clone()]);
        let source = PlaySource::new(Arc::clone(&timeline));
        assert_eq!(source.target(), PlaybackTarget::Timeline);
        assert!(Arc::ptr_eq(&source.project(), &timeline.load_full()));

        let unknown = MediaId::new();
        assert_eq!(
            source.set_target(PlaybackTarget::Media(unknown)),
            Err(UnknownMedia(unknown))
        );
        assert_eq!(source.target(), PlaybackTarget::Timeline);

        source.set_target(PlaybackTarget::Media(video.id)).unwrap();
        assert_eq!(source.target(), PlaybackTarget::Media(video.id));
        let first = source.project();
        assert_eq!(first.media, vec![video.clone()]);
        assert!(Arc::ptr_eq(&first, &source.project()), "cached");

        // A new timeline snapshot rebuilds an equal project.
        let mut next = Project::clone(&timeline.load());
        next.name = "Renamed".into();
        timeline.store(Arc::new(next.clone()));
        let rebuilt = source.project();
        assert!(!Arc::ptr_eq(&first, &rebuilt));
        assert_eq!(*first, *rebuilt);

        // Removed from the project: the clip stays, its asset goes (offline placeholder).
        next.media.clear();
        timeline.store(Arc::new(next));
        let gone = source.project();
        assert!(gone.media.is_empty());
        assert_eq!(gone.duration(), Time::from_seconds(5));
        let plan = crate::render::build_plan(&gone, Time::ZERO, (960, 540));
        assert!(matches!(
            plan.layers[0].content,
            crate::render::LayerContent::Placeholder(_)
        ));

        source.set_target(PlaybackTarget::Timeline).unwrap();
        assert!(Arc::ptr_eq(&source.project(), &timeline.load_full()));
    }
}
