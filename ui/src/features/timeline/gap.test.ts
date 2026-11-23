import { describe, expect, it } from "vitest";

import { gapAt, gapOnTrack, locateGap } from "./gap";
import type { HitTrack } from "./hitTest";

const clip = (id: string, start: number, duration: number) => ({ id, start, duration });
const track = (id: string, clips: HitTrack["clips"], locked = false): HitTrack => ({
  id,
  kind: "video",
  locked,
  clips,
});

describe("gapAt", () => {
  const clips = [clip("b", 300, 100), clip("a", 100, 100)];

  it("finds the gap before the first clip and between clips, whatever the clip order", () => {
    expect(gapAt(clips, 0)).toEqual({ start: 0, end: 100 });
    expect(gapAt(clips, 99)).toEqual({ start: 0, end: 100 });
    expect(gapAt(clips, 200)).toEqual({ start: 200, end: 300 });
    expect(gapAt(clips, 299)).toEqual({ start: 200, end: 300 });
  });

  it("has no gap inside a clip, after the last clip, between touching clips or before 0", () => {
    expect(gapAt(clips, 150)).toBeNull();
    expect(gapAt(clips, 400)).toBeNull();
    expect(gapAt(clips, 10_000)).toBeNull();
    expect(gapAt([clip("a", 0, 100), clip("b", 100, 100)], 100)).toBeNull();
    expect(gapAt(clips, -1)).toBeNull();
    expect(gapAt([], 0)).toBeNull();
  });
});

describe("gapOnTrack", () => {
  it("never selects a gap on a locked track", () => {
    expect(gapOnTrack(track("t", [clip("a", 100, 10)]), 50)).toEqual({ start: 0, end: 100 });
    expect(gapOnTrack(track("t", [clip("a", 100, 10)], true), 50)).toBeNull();
    expect(gapOnTrack(undefined, 50)).toBeNull();
  });
});

describe("locateGap", () => {
  const tracks = [track("t1", [clip("a", 0, 100)]), track("t2", [clip("b", 200, 100)])];

  it("returns the track index and the current span of a selected gap", () => {
    expect(locateGap(tracks, { track: "t2", at: 50 })).toEqual({ index: 1, start: 0, end: 200 });
  });

  it("is null once the gap is gone, its track is gone or nothing is selected", () => {
    expect(locateGap(tracks, { track: "t1", at: 50 })).toBeNull();
    expect(locateGap(tracks, { track: "gone", at: 50 })).toBeNull();
    expect(locateGap(tracks, null)).toBeNull();
  });
});
