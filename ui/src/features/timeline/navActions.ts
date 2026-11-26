/**
 * Playhead navigation (F1.54) and zoom (F1.32) actions. They read the project, the playhead and
 * the timeline view from their stores and apply the pure rules of `navigation.ts` and
 * `geometry.ts`. Nothing here is an edit: no command, no history.
 */
import { frameDuration } from "@/lib/time";
import { playheadStore, setPlayhead } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";
import { setViewport, timelineViewStore } from "@/stores/timelineViewStore";

import { revealTime, ZOOM_STEP, zoomAnchor, zoomAt, zoomTo, zoomToFit } from "./geometry";
import { editPoints, frameStep, lastFrame, nextEdit, previousEdit } from "./navigation";

const project = () => useProjectStore.getState().state?.project;

/** Seeks and scrolls the timeline so the new playhead is in view. */
function seek(time: number) {
  setPlayhead(time);
  const { viewport, width } = timelineViewStore.getState();
  if (width > 0) setViewport(revealTime(viewport, width, playheadStore.getState().time));
}

/** Left / Right (Shift: 10 frames): step `frames` frames on the project frame grid. */
export function stepFrames(frames: number) {
  const p = project();
  if (!p) return;
  seek(frameStep(playheadStore.getState().time, frames, frameDuration(p.settings.frameRate), p.duration));
}

/** Up (`-1`) / Down (`1`): the previous or next clip boundary on any track, 0 or the end. */
export function jumpToEdit(direction: -1 | 1) {
  const p = project();
  if (!p) return;
  const points = editPoints(p.tracks, p.duration);
  const time = playheadStore.getState().time;
  seek(direction < 0 ? previousEdit(points, time) : nextEdit(points, time));
}

/** Home: the start of the timeline. */
export function goToStart() {
  seek(0);
}

/** End: the last frame of the timeline. */
export function goToEnd() {
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
