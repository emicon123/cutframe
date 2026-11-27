/**
 * Playhead navigation (F1.54) and zoom (F1.32) actions. They read the project, the playhead and
 * the timeline view from their stores and apply the pure rules of `navigation.ts` and
 * `geometry.ts`. Nothing here is an edit: no history. Playhead moves go to the engine (`seek`,
 * `stepFrames`), which pauses or keeps playing as playback.md §2 says.
 */
import { frameDuration } from "@/lib/time";
import { playheadStore, seekSource, seekTo, stepSource, stepTo } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";
import { setViewport, timelineViewStore } from "@/stores/timelineViewStore";

import { revealTime, ZOOM_STEP, zoomAnchor, zoomAt, zoomTo, zoomToFit } from "./geometry";
import { editPoints, frameStep, lastFrame, nextEdit, previousEdit } from "./navigation";

const project = () => useProjectStore.getState().state?.project;

/** The media item playing on its own (source preview), if any, with its project view. */
function sourceMedia() {
  const source = playheadStore.getState().source;
  if (!source) return null;
  const p = project();
  return { source, media: p?.media.find((item) => item.id === source.media), project: p };
}

/** Scrolls the timeline so the (new) playhead is in view. */
function reveal() {
  const { viewport, width } = timelineViewStore.getState();
  if (width > 0) setViewport(revealTime(viewport, width, playheadStore.getState().time));
}

/** An exact seek, then the playhead scrolled into view. */
function seek(time: number) {
  seekTo(time, "exact");
  reveal();
}

/**
 * Left / Right (Shift: 10 frames): the engine pauses and steps `frames` frames on the project
 * frame grid; the UI shows where it expects the step to land until the tick confirms it.
 */
export function stepFrames(frames: number) {
  // A source preview steps in media time; the timeline playhead stays.
  if (sourceMedia()) {
    stepSource(frames);
    return;
  }
  const p = project();
  if (!p) return;
  stepTo(
    frames,
    frameStep(playheadStore.getState().time, frames, frameDuration(p.settings.frameRate), p.duration),
  );
  reveal();
}

/** Up (`-1`) / Down (`1`): the previous or next clip boundary on any track, 0 or the end. */
export function jumpToEdit(direction: -1 | 1) {
  const p = project();
  if (!p) return;
  const points = editPoints(p.tracks, p.duration);
  const time = playheadStore.getState().time;
  seek(direction < 0 ? previousEdit(points, time) : nextEdit(points, time));
}

/** Home: the start of the timeline (of the media item in a source preview). */
export function goToStart() {
  if (sourceMedia()) seekSource(0);
  else seek(0);
}

/** End: the start of the last frame of the timeline (or of the previewed media), in integer flicks. */
export function goToEnd() {
  const preview = sourceMedia();
  if (preview) {
    const { media, project: p } = preview;
    const rate = media?.frameRate ?? p?.settings.frameRate;
    if (media && media.duration !== null && rate) seekSource(lastFrame(media.duration, frameDuration(rate)));
    return;
  }
  const p = project();
  if (!p) return;
  seek(lastFrame(p.duration, frameDuration(p.settings.frameRate)));
}

/** + / -: zoom in (`1`) or out (`-1`) one step, anchored at the playhead or the lanes' middle. */
export function zoomStep(direction: -1 | 1) {
  const { viewport, width } = timelineViewStore.getState();
  const factor = direction > 0 ? 1 / ZOOM_STEP : ZOOM_STEP;
  setViewport(zoomAt(viewport, factor, zoomAnchor(viewport, width, playheadStore.getState().time)));
}

/** The zoom slider: set the zoom level, anchored like `zoomStep`. */
export function setZoomLevel(flicksPerPixel: number) {
  const { viewport, width } = timelineViewStore.getState();
  setViewport(zoomTo(viewport, flicksPerPixel, zoomAnchor(viewport, width, playheadStore.getState().time)));
}

/** Shift+Z: show the whole timeline. */
export function fitTimeline() {
  const { width } = timelineViewStore.getState();
  setViewport(zoomToFit(project()?.duration ?? 0, width));
}
