import { describe, expect, it } from "vitest";

import { SECTIONS } from "./registry";
import { inspectorItems, type InspectorSection, shared, visibleSections } from "./sections";
import { audio, clip, projectState, S, track } from "./testProject";

const { project } = projectState([
  track("main", "video", [
    clip("video", { link: "l1", audio: null }),
    clip("image", {
      start: 2 * S,
      audio: null,
      source: { kind: "media", media: "img", sourceIn: 0, speed: 100, reverse: false },
    }),
    clip("color", { start: 4 * S, audio: null, source: { kind: "color", color: "#ff0000ff" } }),
    clip("freeze", { start: 6 * S, audio: null, source: { kind: "freeze", media: "v", sourceTime: S } }),
    clip("sounding", { start: 8 * S }),
  ]),
  track("music", "audio", [
    clip("detached", {
      link: "l1",
      source: { kind: "media", media: "v", sourceIn: 0, speed: 200, reverse: false },
    }),
    clip("song", {
      start: 4 * S,
      source: { kind: "media", media: "a", sourceIn: 0, speed: 100, reverse: false },
    }),
  ]),
]);

const ids = (selected: string[]) =>
  visibleSections(SECTIONS, inspectorItems(project, selected)).map(({ section, targets }) => [
    section.id,
    targets.map((item) => item.clip.id),
  ]);

describe("inspectorItems", () => {
  it("lists the selected clips in timeline order with their track and media", () => {
    const items = inspectorItems(project, ["song", "video", "color"]);
    expect(items.map((item) => [item.clip.id, item.track.id, item.media?.id ?? null])).toEqual([
      ["video", "main", "v"],
      ["color", "main", null],
      ["song", "music", "a"],
    ]);
  });
});

describe("visibleSections", () => {
  it("shows Audio and Speed for a sounding video clip, Duration for stills", () => {
    expect(ids(["sounding"])).toEqual([
      ["audio", ["sounding"]],
      ["speed", ["sounding"]],
    ]);
    expect(ids(["image"])).toEqual([["duration", ["image"]]]);
    expect(ids(["color"])).toEqual([["duration", ["color"]]]);
    expect(ids(["freeze"])).toEqual([["duration", ["freeze"]]]);
  });

  it("treats a link group as one item: a video with its detached audio keeps Audio", () => {
    expect(ids(["video", "detached"])).toEqual([
      ["audio", ["detached"]],
      ["speed", ["video", "detached"]],
    ]);
  });

  it("shows only the sections every selected item has in common", () => {
    expect(ids(["sounding", "song"])).toEqual([
      ["audio", ["sounding", "song"]],
      ["speed", ["sounding", "song"]],
    ]);
    expect(ids(["sounding", "color"])).toEqual([]);
    expect(ids(["video", "detached", "song"])).toEqual([
      ["audio", ["detached", "song"]],
      ["speed", ["video", "detached", "song"]],
    ]);
  });

  it("shows a single-clip section only for one target", () => {
    expect(ids(["image", "color"])).toEqual([]);
  });

  it("is empty without a selection", () => {
    expect(visibleSections(SECTIONS, [])).toEqual([]);
  });

  it("picks up a new registered section without other changes (task 0350's Transform)", () => {
    const transform: InspectorSection = {
      id: "transform",
      title: "Transform",
      applies: (item) => item.track.kind === "video",
      Component: () => null,
    };
    const shown = visibleSections([transform, ...SECTIONS], inspectorItems(project, ["image"]));
    expect(shown.map(({ section }) => section.id)).toEqual(["transform", "duration"]);
  });
});

describe("shared", () => {
  it("reports the first value and whether the others differ", () => {
    const items = inspectorItems(project, ["sounding", "song"]);
    expect(shared(items, (item) => item.clip.duration, 0)).toEqual({ value: 2 * S, mixed: false });
    const louder = inspectorItems(
      projectState([
        track("t", "audio", [clip("x", { audio: audio({ fadeIn: S }) }), clip("y", { start: 5 * S })]),
      ]).project,
      ["x", "y"],
    );
    expect(shared(louder, (item) => item.clip.audio?.fadeIn, 0)).toEqual({ value: S, mixed: true });
    expect(shared([], (item) => item.clip.duration, 7)).toEqual({ value: 7, mixed: false });
  });
});
