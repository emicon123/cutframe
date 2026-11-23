import { describe, expect, it } from "vitest";

import type { HitTrack } from "./hitTest";
import { allClips, clickSelection, pressSelection, pruneSelection, withLinked } from "./selection";

const tracks: HitTrack[] = [
  {
    id: "v",
    kind: "video",
    locked: false,
    clips: [
      { id: "v1", start: 0, duration: 10, link: "L" },
      { id: "v2", start: 10, duration: 10 },
    ],
  },
  { id: "a", kind: "audio", locked: true, clips: [{ id: "a1", start: 0, duration: 10, link: "L" }] },
];
const none = { ctrl: false, shift: false };

describe("withLinked", () => {
  it("adds linked members once, in timeline order", () => {
    expect(withLinked(tracks, ["a1", "v1"])).toEqual(["v1", "a1"]);
    expect(withLinked(tracks, ["v2"])).toEqual(["v2"]);
  });
});

describe("clickSelection", () => {
  it("selects the clicked clip's group", () => {
    expect(clickSelection(tracks, ["v2"], "v1", none)).toEqual(["v1", "a1"]);
  });

  it("adds with Shift and toggles with Ctrl", () => {
    expect(clickSelection(tracks, ["v2"], "a1", { ctrl: false, shift: true })).toEqual(["v1", "v2", "a1"]);
    expect(clickSelection(tracks, ["v2"], "v1", { ctrl: true, shift: false })).toEqual(["v1", "v2", "a1"]);
    expect(clickSelection(tracks, ["v1", "v2", "a1"], "v1", { ctrl: true, shift: false })).toEqual(["v2"]);
  });
});

describe("pressSelection", () => {
  it("keeps a selection that contains the pressed clip, so the group can be dragged", () => {
    expect(pressSelection(tracks, ["v1", "v2", "a1"], "v2", none)).toEqual(["v1", "v2", "a1"]);
  });

  it("replaces or extends the selection with an unselected clip's group", () => {
    expect(pressSelection(tracks, ["v2"], "v1", none)).toEqual(["v1", "a1"]);
    expect(pressSelection(tracks, ["v2"], "v1", { ctrl: true, shift: false })).toEqual(["v1", "v2", "a1"]);
  });
});

describe("allClips", () => {
  it("selects every clip on unlocked tracks", () => {
    expect(allClips(tracks)).toEqual(["v1", "v2"]);
  });
});

describe("pruneSelection", () => {
  it("drops ids that no longer exist", () => {
    expect(pruneSelection(tracks, ["v2", "gone"])).toEqual(["v2"]);
  });

  it("returns the same array when nothing changed", () => {
    const selected = ["v1", "a1"];
    expect(pruneSelection(tracks, selected)).toBe(selected);
  });
});
