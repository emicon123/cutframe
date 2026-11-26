import { describe, expect, it } from "vitest";

import {
  canDetachAudio,
  canReplaceMedia,
  canReplaceWith,
  findClip,
  type LinkClip,
  type LinkTrack,
  linkToggle,
  replacementMedia,
} from "./linking";

const media = (id: string, extra: Partial<LinkClip> = {}): LinkClip => ({
  id,
  duration: 100,
  source: { kind: "media", media: "m1", sourceIn: 0 },
  audio: {},
  ...extra,
});
const color: LinkClip["source"] = { kind: "color", color: "#ff0000ff" };

const tracks: LinkTrack[] = [
  {
    id: "main",
    kind: "video",
    clips: [
      media("v1", { link: "L" }),
      media("v2"),
      media("v3", { audio: null }),
      media("c", { source: color, audio: null }),
    ],
  },
  { id: "over", kind: "video", clips: [media("o1")] },
  { id: "music", kind: "audio", clips: [media("a1", { link: "L" }), media("a2")] },
  {
    id: "titles",
    kind: "text",
    clips: [{ id: "t1", duration: 50, source: { kind: "text", content: "Hi" } }],
  },
];

describe("linkToggle", () => {
  it("unlinks when any selected clip is linked", () => {
    expect(linkToggle(tracks, ["v1", "a1"])).toBe("unlink");
    expect(linkToggle(tracks, ["v2", "a1"])).toBe("unlink");
    expect(linkToggle(tracks, ["v1"])).toBe("unlink");
  });

  it("links two or more unlinked clips on different tracks", () => {
    expect(linkToggle(tracks, ["v2", "a2"])).toBe("link");
    expect(linkToggle(tracks, ["v2", "o1", "a2"])).toBe("link");
  });

  it("does nothing for one clip, clips sharing a track or no selection", () => {
    expect(linkToggle(tracks, ["v2"])).toBeNull();
    expect(linkToggle(tracks, ["v2", "v3"])).toBeNull();
    expect(linkToggle(tracks, [])).toBeNull();
    expect(linkToggle(tracks, ["gone", "v2"])).toBeNull();
  });
});

describe("canDetachAudio", () => {
  it("applies to video-track media clips that still make sound", () => {
    expect(canDetachAudio(tracks, ["v2"])).toBe(true);
    expect(canDetachAudio(tracks, ["v3", "v2"])).toBe(true);
  });

  it("skips detached, colour, audio-track and text clips", () => {
    expect(canDetachAudio(tracks, ["v3"])).toBe(false);
    expect(canDetachAudio(tracks, ["c"])).toBe(false);
    expect(canDetachAudio(tracks, ["a2"])).toBe(false);
    expect(canDetachAudio(tracks, ["t1"])).toBe(false);
    expect(canDetachAudio(tracks, [])).toBe(false);
  });
});

describe("findClip", () => {
  it("finds a clip with its track", () => {
    expect(findClip(tracks, "a2")?.track.id).toBe("music");
    expect(findClip(tracks, "nope")).toBeNull();
  });
});

describe("replace media compatibility", () => {
  const item = (id: string, kind: "video" | "audio" | "image", duration: number | null, sound: boolean) => ({
    id,
    kind,
    duration,
    audio: sound ? { codec: "aac", channels: 2, sampleRate: 48_000, streamCount: 1 } : null,
  });
  const library = [
    item("m1", "video", 500, true), // the clip's own media
    item("long", "video", 500, true),
    item("silent", "video", 500, false),
    item("short", "video", 50, true),
    item("pic", "image", null, false),
    item("song", "audio", 500, true),
    item("blip", "audio", 10, true),
  ];
  const ids = (list: { id: string }[]) => list.map((m) => m.id);

  it("offers video and images on a video track, long enough, not the current media", () => {
    expect(ids(replacementMedia("video", media("x"), library))).toEqual(["long", "silent", "pic"]);
  });

  it("offers media with sound on an audio track", () => {
    expect(ids(replacementMedia("audio", media("x"), library))).toEqual(["long", "song"]);
  });

  it("offers nothing on a text track or for non-media clips", () => {
    expect(replacementMedia("text", media("x"), library)).toEqual([]);
    expect(canReplaceWith("video", media("c", { source: color }), item("long", "video", 500, true))).toBe(
      false,
    );
  });

  it("knows which clips can be replaced at all", () => {
    expect(canReplaceMedia("video", media("x"))).toBe(true);
    expect(canReplaceMedia("audio", media("x"))).toBe(true);
    expect(canReplaceMedia("video", media("c", { source: color }))).toBe(false);
    expect(canReplaceMedia("text", media("x"))).toBe(false);
  });
});
