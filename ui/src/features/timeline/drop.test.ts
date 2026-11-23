import { describe, expect, it } from "vitest";

import { FLICKS_PER_SECOND } from "@/lib/time";

import {
  accepts,
  clampNewTrackIndex,
  type DropInput,
  dropPayload,
  dropPlacement,
  dropPreview,
  type DropTrack,
  IMAGE_PREVIEW_DURATION,
  nearestBoundary,
  NEW_TRACK_EDGE,
  rowAt,
} from "./drop";
import { RULER_HEIGHT, TRACK_HEIGHT, trackTop } from "./geometry";

const S = FLICKS_PER_SECOND;
// 1 px = 1/100 s, so x = 100 is 1 s; frames are 1/100 s too.
const viewport = { scroll: 0, flicksPerPixel: S / 100 };
const frame = S / 100;

const overlay: DropTrack = {
  id: "ov",
  kind: "video",
  locked: false,
  clips: [{ start: 2 * S, duration: 2 * S }],
};
const main: DropTrack = {
  id: "main",
  kind: "video",
  locked: false,
  clips: [
    { start: 0, duration: 3 * S },
    { start: 3 * S, duration: 4 * S },
  ],
};
const music: DropTrack = { id: "music", kind: "audio", locked: false, clips: [] };
const tracks = [overlay, main, music];

const video = dropPayload([{ kind: "video", duration: 2 * S }]);
const sound = dropPayload([{ kind: "audio", duration: 3 * S }]);

/** The middle of track `index`. */
const mid = (index: number) => trackTop(index) + TRACK_HEIGHT / 2;

function input(overrides: Partial<DropInput>): DropInput {
  return {
    x: 0,
    y: mid(0),
    tracks,
    mainTrack: "main",
    magnetic: true,
    viewport,
    playhead: 0,
    frame,
    payload: video,
    snapping: false,
    ...overrides,
  };
}

describe("dropPayload", () => {
  it("sums durations, counting images as 5 s, and tells visual from audio-only", () => {
    expect(
      dropPayload([
        { kind: "video", duration: S },
        { kind: "image", duration: null },
      ]),
    ).toEqual({
      visual: true,
      audioOnly: false,
      duration: S + IMAGE_PREVIEW_DURATION,
    });
    expect(sound).toEqual({ visual: false, audioOnly: true, duration: 3 * S });
    const mixed = dropPayload([
      { kind: "video", duration: S },
      { kind: "audio", duration: S },
    ]);
    expect(mixed.visual || mixed.audioOnly).toBe(false);
  });
});

describe("rowAt", () => {
  it("finds a track in the middle of a row and a new track near its edges or below", () => {
    expect(rowAt(RULER_HEIGHT - 1, 3)).toBeNull();
    expect(rowAt(mid(1), 3)).toEqual({ track: 1 });
    expect(rowAt(trackTop(1) + NEW_TRACK_EDGE - 1, 3)).toEqual({ newAt: 1 });
    expect(rowAt(trackTop(1) + TRACK_HEIGHT - 1, 3)).toEqual({ newAt: 2 });
    expect(rowAt(trackTop(5), 3)).toEqual({ newAt: 3 });
  });
});

describe("track rules", () => {
  it("puts pictures on unlocked video tracks and sound on audio tracks", () => {
    expect(accepts(overlay, video)).toBe(true);
    expect(accepts(music, video)).toBe(false);
    expect(accepts(music, sound)).toBe(true);
    expect(accepts(main, sound)).toBe(false);
    expect(accepts({ ...overlay, locked: true }, video)).toBe(false);
  });

  it("clamps new tracks into the overlay or audio section", () => {
    expect(clampNewTrackIndex(3, 1, 3, video)).toBe(1);
    expect(clampNewTrackIndex(0, 1, 3, video)).toBe(0);
    expect(clampNewTrackIndex(0, 1, 3, sound)).toBe(2);
    expect(clampNewTrackIndex(9, 1, 3, sound)).toBe(3);
  });

  it("finds the nearest main-track boundary", () => {
    expect(nearestBoundary(main.clips, S)).toBe(0);
    expect(nearestBoundary(main.clips, 2 * S)).toBe(3 * S);
    expect(nearestBoundary(main.clips, 9 * S)).toBe(7 * S);
    expect(nearestBoundary([], 9 * S)).toBe(0);
  });
});

describe("dropPreview", () => {
  it("inserts at the nearest boundary of the magnetic main track", () => {
    const preview = dropPreview(input({ x: 520, y: mid(1) }));
    expect(preview).toMatchObject({ kind: "track", track: "main", start: 7 * S, insert: true, valid: true });
    expect(preview && dropPlacement(preview)).toEqual({
      kind: "drop",
      track: { kind: "existing", track: "main" },
      start: 7 * S,
    });
  });

  it("drops on an overlay at the pointer time rounded to frames, invalid where it overlaps", () => {
    const free = dropPreview(input({ x: 400.4, y: mid(0) }));
    expect(free).toMatchObject({ kind: "track", track: "ov", start: 4 * S, valid: true, insert: false });
    const clash = dropPreview(input({ x: 100, y: mid(0) }));
    expect(clash).toMatchObject({ kind: "track", track: "ov", start: S, valid: false });
    expect(clash && dropPlacement(clash)).toBeNull();
  });

  it("treats a non-magnetic main track like any other track", () => {
    expect(dropPreview(input({ x: 100, y: mid(1), magnetic: false }))).toMatchObject({
      valid: false,
      insert: false,
    });
    expect(dropPreview(input({ x: 800, y: mid(1), magnetic: false }))).toMatchObject({
      start: 8 * S,
      valid: true,
    });
  });

  it("snaps the ghost's start or end to clip edges and the playhead unless snapping is off", () => {
    // Ghost 2 s long at x = 395 (3.95 s): its end at 5.95 s is far, its start snaps to 4 s.
    const snapped = dropPreview(input({ x: 395, y: mid(0), snapping: true }));
    expect(snapped).toMatchObject({ start: 4 * S, snap: 4 * S });
    const playhead = dropPreview(input({ x: 603, y: mid(0), snapping: true, playhead: 6 * S }));
    expect(playhead).toMatchObject({ start: 6 * S, snap: 6 * S });
    expect(dropPreview(input({ x: 395, y: mid(0) }))).toMatchObject({ start: 3.95 * S, snap: null });
  });

  it("creates a new track near a track edge, below the tracks, or over a track that does not fit", () => {
    const above = dropPreview(input({ x: 100, y: trackTop(0) + 2 }));
    expect(above).toMatchObject({ kind: "newTrack", index: 0, valid: true });
    expect(above && dropPlacement(above)).toEqual({
      kind: "drop",
      track: { kind: "new", index: 0 },
      start: S,
    });
    // Video over the audio track: a new overlay just above the main track.
    expect(dropPreview(input({ y: mid(2) }))).toMatchObject({ kind: "newTrack", index: 1 });
    // Audio below everything: a new audio track at the bottom.
    expect(dropPreview(input({ y: trackTop(4), payload: sound }))).toMatchObject({
      kind: "newTrack",
      index: 3,
    });
    // Audio over the overlay: a new audio track right below the main track.
    expect(dropPreview(input({ y: mid(0), payload: sound }))).toMatchObject({ kind: "newTrack", index: 2 });
  });

  it("drops audio onto an audio track and rejects mixed selections", () => {
    expect(dropPreview(input({ x: 100, y: mid(2), payload: sound }))).toMatchObject({
      kind: "track",
      track: "music",
      valid: true,
    });
    const mixed = dropPayload([
      { kind: "video", duration: S },
      { kind: "audio", duration: S },
    ]);
    expect(dropPreview(input({ y: mid(0), payload: mixed }))).toMatchObject({ valid: false });
  });

  it("ignores the ruler", () => {
    expect(dropPreview(input({ y: 4 }))).toBeNull();
  });
});
