import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { frameDuration } from "@/lib/time";
import { playheadStore, setPlayhead } from "@/stores/playheadStore";
import { type ProjectState, useProjectStore } from "@/stores/projectStore";
import { setLaneWidth, setViewport, timelineViewStore } from "@/stores/timelineViewStore";

import { DEFAULT_VIEWPORT, FIT_PADDING, timeToX } from "./geometry";
import {
  fitTimeline,
  goToEnd,
  goToStart,
  jumpToEdit,
  setZoomLevel,
  stepFrames,
  zoomStep,
} from "./navActions";

const frame = frameDuration({ num: 25, den: 1 });
const clip = (start: number, duration: number) => ({ start, duration });

const project = (duration: number) =>
  ({
    project: {
      duration,
      settings: { frameRate: { num: 25, den: 1 } },
      tracks: [
        { clips: [clip(0, 50 * frame), clip(50 * frame, 50 * frame)] },
        { clips: [clip(20 * frame, 10 * frame)] },
      ],
    },
  }) as unknown as ProjectState;

const playhead = () => playheadStore.getState().time;
const view = () => timelineViewStore.getState().viewport;

beforeEach(() => {
  useProjectStore.setState({ state: project(100 * frame) });
  setViewport(DEFAULT_VIEWPORT);
  setLaneWidth(1000);
  setPlayhead(0);
});

afterEach(() => useProjectStore.setState({ state: null }));

describe("playhead navigation (F1.54)", () => {
  it("steps frames on the project grid", () => {
    stepFrames(1);
    expect(playhead()).toBe(frame);
    stepFrames(10);
    expect(playhead()).toBe(11 * frame);
    stepFrames(-1);
    expect(playhead()).toBe(10 * frame);
  });

  it("jumps between edit points on all tracks", () => {
    jumpToEdit(1);
    expect(playhead()).toBe(20 * frame);
    jumpToEdit(1);
    expect(playhead()).toBe(30 * frame);
    jumpToEdit(-1);
    expect(playhead()).toBe(20 * frame);
  });

  it("goes to the start and to the last frame", () => {
    goToEnd();
    expect(playhead()).toBe(99 * frame);
    goToStart();
    expect(playhead()).toBe(0);
  });

  it("scrolls the timeline to keep the new playhead in view", () => {
    goToEnd();
    const x = timeToX(playhead(), view());
    expect(x).toBeGreaterThan(0);
    expect(x).toBeLessThanOrEqual(1000);
    goToStart();
    expect(view().scroll).toBe(0);
  });

  it("does nothing without a project", () => {
    useProjectStore.setState({ state: null });
    setPlayhead(5 * frame);
    stepFrames(1);
    goToEnd();
    expect(playhead()).toBe(5 * frame);
  });
});

describe("zoom actions (F1.32)", () => {
  it("zooms in and out around a visible playhead", () => {
    setPlayhead(2 * frame);
    const x = timeToX(playhead(), view());
    zoomStep(1);
    expect(view().flicksPerPixel).toBeLessThan(DEFAULT_VIEWPORT.flicksPerPixel);
    expect(timeToX(playhead(), view())).toBeCloseTo(x, 0);
    zoomStep(-1);
    expect(view().flicksPerPixel).toBeCloseTo(DEFAULT_VIEWPORT.flicksPerPixel);
  });

  it("sets the slider's zoom level", () => {
    setZoomLevel(DEFAULT_VIEWPORT.flicksPerPixel * 4);
    expect(view().flicksPerPixel).toBeCloseTo(DEFAULT_VIEWPORT.flicksPerPixel * 4);
  });

  it("fits the whole timeline", () => {
    setViewport({ scroll: 12345, flicksPerPixel: DEFAULT_VIEWPORT.flicksPerPixel });
    fitTimeline();
    expect(view().scroll).toBe(0);
    expect(timeToX(100 * frame, view())).toBeCloseTo(1000 - FIT_PADDING);
  });
});
