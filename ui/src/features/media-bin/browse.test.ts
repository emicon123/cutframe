import { describe, expect, it } from "vitest";

import type { MediaKind, MediaView_Serialize } from "@/bindings";
import { FLICKS_PER_SECOND } from "@/lib/time";

import { type BrowseOptions, browseMedia, matchesQuery, toggleKind } from "./browse";

function item(id: string, kind: MediaKind, fileName: string, seconds: number | null): MediaView_Serialize {
  return {
    id,
    kind,
    fileName,
    path: `C:\\media\\${fileName}`,
    duration: seconds === null ? null : seconds * FLICKS_PER_SECOND,
    displayWidth: null,
    displayHeight: null,
    frameRate: null,
    videoCodec: null,
    hdr: null,
    audio: null,
    status: "online",
    usedBy: 0,
    previewVersion: "v",
    filmstrip: null,
  };
}

// Project (import) order.
const media = [
  item("a", "video", "Clip 10.mp4", 30),
  item("b", "audio", "music.mp3", 120),
  item("c", "image", "photo.HEIC", null),
  item("d", "video", "clip 2.mov", 30),
  item("e", "audio", "Voice.wav", 5),
];

const all: BrowseOptions = { sort: "imported", direction: "asc", kinds: [], query: "" };
const ids = (options: Partial<BrowseOptions>) => browseMedia(media, { ...all, ...options }).map((m) => m.id);

describe("browseMedia", () => {
  it("keeps import order by default and reverses it descending", () => {
    expect(ids({})).toEqual(["a", "b", "c", "d", "e"]);
    expect(ids({ direction: "desc" })).toEqual(["e", "d", "c", "b", "a"]);
  });

  it("sorts names case-insensitively with numbers in natural order", () => {
    expect(ids({ sort: "name" })).toEqual(["d", "a", "b", "c", "e"]);
  });

  it("sorts by duration with images first and ties in import order, both directions", () => {
    expect(ids({ sort: "duration" })).toEqual(["c", "e", "a", "d", "b"]);
    expect(ids({ sort: "duration", direction: "desc" })).toEqual(["b", "a", "d", "e", "c"]);
  });

  it("groups by kind (video, audio, image), keeping import order inside a kind", () => {
    expect(ids({ sort: "kind" })).toEqual(["a", "d", "b", "e", "c"]);
  });

  it("filters by kind; an empty filter shows everything", () => {
    expect(ids({ kinds: ["audio"] })).toEqual(["b", "e"]);
    expect(ids({ kinds: ["video", "image"] })).toEqual(["a", "c", "d"]);
  });

  it("searches names and combines with filter and sort", () => {
    expect(ids({ query: "clip" })).toEqual(["a", "d"]);
    expect(ids({ query: "  CLIP ", sort: "name" })).toEqual(["d", "a"]);
    expect(ids({ query: "o", kinds: ["audio"] })).toEqual(["e"]);
    expect(ids({ query: "nothing" })).toEqual([]);
  });

  it("does not change the input", () => {
    const before = media.map((m) => m.id);
    browseMedia(media, { ...all, sort: "name", direction: "desc" });
    expect(media.map((m) => m.id)).toEqual(before);
  });
});

describe("matchesQuery", () => {
  it("matches a blank query and substrings in any case", () => {
    expect(matchesQuery("Holiday.MP4", "")).toBe(true);
    expect(matchesQuery("Holiday.MP4", "   ")).toBe(true);
    expect(matchesQuery("Holiday.MP4", "day.mp")).toBe(true);
    expect(matchesQuery("Holiday.MP4", "days")).toBe(false);
  });
});

describe("toggleKind", () => {
  it("adds and removes kinds in canonical order", () => {
    expect(toggleKind([], "image")).toEqual(["image"]);
    expect(toggleKind(["image"], "video")).toEqual(["video", "image"]);
    expect(toggleKind(["video", "image"], "video")).toEqual(["image"]);
  });
});
