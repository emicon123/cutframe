import { describe, expect, it } from "vitest";

import { FLICKS_PER_SECOND } from "@/lib/time";

import { type DragContext, moveValue, planMove, planTrim, snapTargets, trimValue } from "./editDrag";
import { TRACK_HEIGHT, trackTop } from "./geometry";
import type { HitTrack } from "./hitTest";

const S = FLICKS_PER_SECOND;
const frame = S / 100;
// 1 px = 1/100 s = one frame.
const viewport = { scroll: 0, flicksPerPixel: S / 100 };

const tracks: HitTrack[] = [
  { id: "ov", kind: "video", locked: false, clips: [{ id: "o1", start: 4 * S, duration: S }] },
  {
    id: "main",
    kind: "video",
    locked: false,
    clips: [
      { id: "m1", start: 0, duration: 2 * S, link: "L" },
      { id: "m2", start: 2 * S, duration: 3 * S },
    ],
  },
  { id: "music", kind: "audio", locked: false, clips: [{ id: "a1", start: 0, duration: 2 * S, link: "L" }] },
  { id: "locked", kind: "audio", locked: true, clips: [] },
];
const ctx: DragContext = {
  tracks,
  mainTrack: "main",
  viewport,
  playhead: 7 * S,
  frame,
  snapping: true,
};
const mid = (index: number) => trackTop(index) + TRACK_HEIGHT / 2;

describe("snapTargets", () => {
  it("lists 0, the playhead and the edges of clips not being dragged", () => {
    expect(snapTargets(tracks, new Set(["m1", "a1", "m2"]), 7 * S)).toEqual([0, 7 * S, 4 * S, 5 * S]);
  });
});

describe("planMove", () => {
  it("adds the anchor and measures the group with its linked members", () => {
    const plan = planMove(tracks, ["m2"], "o1");
    expect(plan).toMatchObject({ clips: ["m2", "o1"], anchor: "o1", anchorTrack: 0, vertical: false });
    expect(plan?.before).toBe(2 * S);
    expect(plan?.after).toBe(S);
  });

  it("allows a vertical move when the other clips are linked to the anchor's track", () => {
    const plan = planMove(tracks, ["m1", "a1"], "m1");
    expect(plan).toMatchObject({ vertical: true, kind: "video", anchorTrack: 1 });
    expect([...(plan?.group ?? [])].sort()).toEqual(["a1", "m1"]);
  });

  it("returns null for an unknown anchor", () => {
    expect(planMove(tracks, [], "nope")).toBeNull();
  });
});

describe("moveValue", () => {
  const plan = planMove(tracks, ["o1"], "o1");
  if (!plan) throw new Error("plan");

  it("moves by the pointer offset on the frame grid and stays on the track", () => {
    // 103.4 px = 1.034 s, rounded to the 1.03 s frame.
    const result = moveValue(plan, { ...ctx, snapping: false }, -103.4, mid(0));
    expect(result).toEqual({
      value: { start: 4 * S - 103 * frame, track: { kind: "existing", track: "ov" } },
      snap: null,
    });
  });

  it("snaps the group's start or end to a clip edge within 8 px", () => {
    // Start at 4.95 s: end 5.95 s; m2 ends at 5 s, 5 px from the start.
    const result = moveValue(plan, ctx, 95, mid(0));
    expect(result.value.start).toBe(5 * S);
    expect(result.snap).toBe(5 * S);
  });

  it("does not snap with snapping off (Alt)", () => {
    expect(moveValue(plan, { ...ctx, snapping: false }, 95, mid(0)).value.start).toBe(4 * S + 95 * frame);
  });

  it("never moves the group before 0", () => {
    const group = planMove(tracks, ["m2", "o1"], "o1");
    if (!group) throw new Error("plan");
    // The group starts 2 s before the anchor, so the anchor cannot go below 2 s.
    expect(moveValue(group, { ...ctx, snapping: false }, -400, mid(0)).value.start).toBe(2 * S);
  });

  it("targets another track of the same kind, a new track, or stays put", () => {
    const main = moveValue(plan, ctx, 0, mid(1)).value.track;
    expect(main).toEqual({ kind: "existing", track: "main" });
    // An audio track does not take a video clip.
    expect(moveValue(plan, ctx, 0, mid(2)).value.track).toEqual({ kind: "existing", track: "ov" });
    // Near the top edge of the main track: a new overlay track between.
    expect(moveValue(plan, ctx, 0, trackTop(1) + 2).value.track).toEqual({ kind: "new", index: 1 });
    // Below every track: a video track is clamped to just above main.
    expect(moveValue(plan, ctx, 0, trackTop(6)).value.track).toEqual({ kind: "new", index: 1 });
  });

  it("does not create a new track from the anchor's own row", () => {
    expect(moveValue(plan, ctx, 0, trackTop(0) + 2).value.track).toEqual({ kind: "existing", track: "ov" });
  });

  it("keeps a time-only move on the anchor's track", () => {
    const mixed = planMove(tracks, ["m2", "o1"], "o1");
    if (!mixed) throw new Error("plan");
    expect(moveValue(mixed, ctx, 0, mid(1)).value.track).toEqual({ kind: "existing", track: "ov" });
  });

  it("skips locked tracks", () => {
    const audio = planMove(tracks, ["a1"], "a1");
    if (!audio) throw new Error("plan");
    expect(moveValue(audio, ctx, 0, mid(3)).value.track).toEqual({ kind: "existing", track: "music" });
  });
});

describe("trimValue", () => {
  it("moves the edge by the pointer offset on the frame grid", () => {
    const plan = planTrim(tracks, "m2", "end");
    if (!plan) throw new Error("plan");
    expect(plan.edgeTime).toBe(5 * S);
    expect(trimValue(plan, { ...ctx, snapping: false }, 50.4)).toEqual({ time: 5.5 * S, snap: null });
  });

  it("snaps the dragged edge but not to its own clip or linked members", () => {
    const plan = planTrim(tracks, "m1", "end");
    if (!plan) throw new Error("plan");
    // m1's end (2 s) is excluded, as is a1's (linked); m2's start at 2 s is a target.
    expect(trimValue(plan, ctx, 5)).toEqual({ time: 2 * S, snap: 2 * S });
    const start = planTrim(tracks, "o1", "start");
    if (!start) throw new Error("plan");
    expect(trimValue(start, ctx, 104)).toEqual({ time: 5 * S, snap: 5 * S });
  });

  it("returns null for an unknown clip", () => {
    expect(planTrim(tracks, "nope", "start")).toBeNull();
  });
});
