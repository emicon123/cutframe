import { describe, expect, it } from "vitest";

import { RULER_HEIGHT, TRACK_GAP, TRACK_HEIGHT, trackTop } from "./geometry";
import { boundaryY, moveRange, trackDimmed, trackDropAt, trackFlags } from "./tracks";

// Two overlays, the main track, two audio tracks.
const tracks = [{ id: "o1" }, { id: "o2" }, { id: "main" }, { id: "a1" }, { id: "a2" }];
const ROW = TRACK_HEIGHT + TRACK_GAP;
/** A y near the boundary above track index `b`. */
const at = (b: number, offset = 0) => RULER_HEIGHT + b * ROW + offset;

describe("trackFlags", () => {
  it("offers mute where a track sounds, hide where it draws, lock everywhere", () => {
    expect(trackFlags("video")).toEqual(["muted", "hidden", "locked"]);
    expect(trackFlags("audio")).toEqual(["muted", "locked"]);
    expect(trackFlags("text")).toEqual(["hidden", "locked"]);
  });
});

describe("trackDimmed", () => {
  it("dims a muted audio track and a hidden visual track, not a muted video track", () => {
    expect(trackDimmed({ kind: "audio", muted: true, hidden: false })).toBe(true);
    expect(trackDimmed({ kind: "audio", muted: false, hidden: true })).toBe(false);
    expect(trackDimmed({ kind: "video", muted: true, hidden: false })).toBe(false);
    expect(trackDimmed({ kind: "video", muted: false, hidden: true })).toBe(true);
    expect(trackDimmed({ kind: "text", muted: false, hidden: true })).toBe(true);
  });
});

describe("moveRange", () => {
  it("is the track's own section, and null for the main track", () => {
    expect(moveRange(tracks, "main", 0)).toEqual({ min: 0, max: 1 });
    expect(moveRange(tracks, "main", 1)).toEqual({ min: 0, max: 1 });
    expect(moveRange(tracks, "main", 2)).toBeNull();
    expect(moveRange(tracks, "main", 4)).toEqual({ min: 3, max: 4 });
    expect(moveRange(tracks, "nope", 0)).toBeNull();
    expect(moveRange(tracks, "main", 9)).toBeNull();
  });
});

describe("trackDropAt", () => {
  it("moves an overlay down past its neighbour", () => {
    expect(trackDropAt(tracks, "main", 0, at(2))).toEqual({ index: 1, boundary: 2 });
  });

  it("moves an overlay up", () => {
    expect(trackDropAt(tracks, "main", 1, at(0, 5))).toEqual({ index: 0, boundary: 0 });
  });

  it("is null where the track would stay", () => {
    expect(trackDropAt(tracks, "main", 0, at(0))).toBeNull();
    expect(trackDropAt(tracks, "main", 0, at(1))).toBeNull();
    expect(trackDropAt(tracks, "main", 0, at(0, ROW / 2 - 1))).toBeNull();
  });

  it("clamps into the section: an overlay never goes below the main track", () => {
    expect(trackDropAt(tracks, "main", 0, at(5))).toEqual({ index: 1, boundary: 2 });
    // An audio track never goes above the main track.
    expect(trackDropAt(tracks, "main", 4, 0)).toEqual({ index: 3, boundary: 3 });
    expect(trackDropAt(tracks, "main", 3, at(9))).toEqual({ index: 4, boundary: 5 });
  });

  it("never moves the main track", () => {
    expect(trackDropAt(tracks, "main", 2, at(0))).toBeNull();
  });
});

describe("boundaryY", () => {
  it("is centred in the gap above a track", () => {
    expect(boundaryY(1)).toBe(trackTop(1) - TRACK_GAP / 2);
  });
});
