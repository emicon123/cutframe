import { describe, expect, it } from "vitest";

import type { ClipSource } from "@/bindings";
import { t } from "@/i18n";

import {
  clipBadges,
  freezeTarget,
  isRetimable,
  mediaKinds,
  type RetimeTrack,
  reverseState,
  speedFactorText,
} from "./retime";

const media = (speed = 100, reverse = false, id = "v"): ClipSource => ({
  kind: "media",
  media: id,
  sourceIn: 0,
  speed,
  reverse,
});

const kinds = mediaKinds([
  { id: "v", kind: "video" },
  { id: "a", kind: "audio" },
  { id: "img", kind: "image" },
]);

describe("isRetimable", () => {
  it("takes video and audio media clips only", () => {
    expect(isRetimable(media(), kinds)).toBe(true);
    expect(isRetimable(media(100, false, "a"), kinds)).toBe(true);
    expect(isRetimable(media(100, false, "img"), kinds)).toBe(false);
    expect(isRetimable({ kind: "freeze", media: "v", sourceTime: 0 }, kinds)).toBe(false);
    expect(isRetimable({ kind: "color", color: "#000000ff" }, kinds)).toBe(false);
    expect(isRetimable({ kind: "text", content: "Hi" }, kinds)).toBe(false);
  });
});

describe("clip badges (F1.79, F1.81, F1.82)", () => {
  it("formats the speed as a short factor", () => {
    expect(speedFactorText(200)).toBe("2");
    expect(speedFactorText(50)).toBe("0.5");
    expect(speedFactorText(25)).toBe("0.25");
    expect(speedFactorText(125)).toBe("1.25");
    expect(speedFactorText(10)).toBe("0.1");
    expect(speedFactorText(10_000)).toBe("100");
  });

  it("shows the speed when it is not 1×, a reverse and a freeze marker", () => {
    expect(clipBadges(media(200))).toEqual(["2×"]);
    expect(clipBadges(media(50, true))).toEqual(["0.5×", t.timeline.badges.reverse]);
    expect(clipBadges(media(100, true))).toEqual([t.timeline.badges.reverse]);
    expect(clipBadges({ kind: "freeze", media: "v", sourceTime: 5 })).toEqual([t.timeline.badges.freeze]);
  });

  it("gives plain clips one shared empty list", () => {
    expect(clipBadges(media())).toEqual([]);
    expect(clipBadges(media())).toBe(clipBadges({ kind: "color", color: "#ffffffff" }));
  });
});

describe("freezeTarget (F1.82)", () => {
  const tracks: RetimeTrack[] = [
    {
      kind: "video",
      clips: [
        { id: "over", start: 0, duration: 100, source: media() },
        { id: "still", start: 200, duration: 100, source: media(100, false, "img") },
      ],
    },
    {
      kind: "video",
      clips: [
        { id: "main", start: 0, duration: 100, source: media() },
        { id: "next", start: 100, duration: 100, source: media() },
      ],
    },
    { kind: "audio", clips: [{ id: "sound", start: 0, duration: 100, source: media(100, false, "a") }] },
  ];

  it("freezes the one selected video clip under the playhead, its linked audio aside", () => {
    expect(freezeTarget(tracks, kinds, ["over"], 50)).toBe("over");
    expect(freezeTarget(tracks, kinds, ["main", "sound"], 0)).toBe("main");
  });

  it("leaves the choice to Rust otherwise (null: the main-track clip under the playhead)", () => {
    expect(freezeTarget(tracks, kinds, [], 50)).toBeNull();
    expect(freezeTarget(tracks, kinds, ["over", "main"], 50)).toBeNull();
    expect(freezeTarget(tracks, kinds, ["main"], 100)).toBeNull();
    expect(freezeTarget(tracks, kinds, ["still"], 250)).toBeNull();
    expect(freezeTarget(tracks, kinds, ["sound"], 50)).toBeNull();
  });
});

describe("reverseState (F1.81)", () => {
  const tracks: RetimeTrack[] = [
    {
      kind: "video",
      clips: [
        { id: "fwd", start: 0, duration: 100, source: media() },
        { id: "back", start: 100, duration: 100, source: media(100, true) },
        { id: "img", start: 200, duration: 100, source: media(100, false, "img") },
      ],
    },
  ];

  it("turns reverse on unless every reversible selected clip already plays reversed", () => {
    expect(reverseState(tracks, kinds, ["fwd", "back"])).toEqual({ reversed: false });
    expect(reverseState(tracks, kinds, ["back", "img"])).toEqual({ reversed: true });
  });

  it("does not apply to images or an empty selection", () => {
    expect(reverseState(tracks, kinds, ["img"])).toBeNull();
    expect(reverseState(tracks, kinds, [])).toBeNull();
  });
});
