import { describe, expect, it } from "vitest";

import { containsPoint, fileNameOf, isLikelyImportable, toCssPoint } from "./dropTarget";

describe("fileNameOf", () => {
  it("takes the last segment of Windows and POSIX paths", () => {
    expect(fileNameOf("C:\\Users\\me\\Videos\\clip.MP4")).toBe("clip.MP4");
    expect(fileNameOf("/home/me/song.mp3")).toBe("song.mp3");
    expect(fileNameOf("C:\\Users\\me\\Videos\\")).toBe("Videos");
    expect(fileNameOf("clip.mov")).toBe("clip.mov");
  });
});

describe("isLikelyImportable", () => {
  it("accepts media extensions in any case and paths without one (folders)", () => {
    expect(isLikelyImportable(["C:\\a\\clip.MOV"])).toBe(true);
    expect(isLikelyImportable(["C:\\a\\Holiday 2026"])).toBe(true);
    expect(isLikelyImportable(["C:\\a\\notes.txt", "C:\\a\\photo.heic"])).toBe(true);
  });

  it("rejects drags with only non-media files", () => {
    expect(isLikelyImportable(["C:\\a\\notes.txt", "C:\\a\\setup.exe"])).toBe(false);
    expect(isLikelyImportable([])).toBe(false);
  });
});

describe("drop hit-testing", () => {
  it("converts physical to CSS pixels", () => {
    expect(toCssPoint({ x: 300, y: 150 }, 1.5)).toEqual({ x: 200, y: 100 });
    expect(toCssPoint({ x: 30, y: 15 }, 0)).toEqual({ x: 30, y: 15 });
  });

  it("includes the top-left edge and excludes the bottom-right edge", () => {
    const box = { left: 0, top: 40, right: 300, bottom: 500 };
    expect(containsPoint(box, { x: 0, y: 40 })).toBe(true);
    expect(containsPoint(box, { x: 300, y: 100 })).toBe(false);
    expect(containsPoint(box, { x: 10, y: 20 })).toBe(false);
  });
});
