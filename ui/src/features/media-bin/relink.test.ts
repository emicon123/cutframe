import { describe, expect, it } from "vitest";

import { countMissing, needsRelink, parentDirOf, previewKey, summarizeRelink } from "./relink";

describe("needsRelink and countMissing", () => {
  it("counts offline and changed items; online ones need nothing", () => {
    expect(needsRelink({ status: "online" })).toBe(false);
    expect(needsRelink({ status: "offline" })).toBe(true);
    expect(needsRelink({ status: "changed" })).toBe(true);
    const statuses = ["online", "offline", "changed", "offline"] as const;
    expect(countMissing(statuses.map((status) => ({ status })))).toEqual({ offline: 2, changed: 1 });
    expect(countMissing([])).toEqual({ offline: 0, changed: 0 });
  });
});

describe("summarizeRelink", () => {
  it("sorts a folder report into relinked and not found", () => {
    const result = summarizeRelink({
      files: [
        { path: "D:\\new\\a.mp4", outcome: { kind: "relinked", media: "m1" } },
        { path: "C:\\old\\b.mp4", outcome: { kind: "notFound", media: "m2" } },
        { path: "D:\\new\\c.wav", outcome: { kind: "relinked", media: "m3" } },
      ],
      error: null,
    });
    expect(result).toEqual({
      relinked: ["m1", "m3"],
      notFound: ["m2"],
      mismatch: null,
      failed: [],
      jobError: null,
      cancelled: false,
    });
  });

  it("reports a content mismatch and per-file failures", () => {
    expect(
      summarizeRelink({
        files: [{ path: "D:\\x.mp4", outcome: { kind: "fingerprintMismatch", media: "m1" } }],
        error: null,
      }).mismatch,
    ).toBe("m1");
    const failed = summarizeRelink({
      files: [{ path: "D:\\song.mp3", outcome: { kind: "failed", error: "kindMismatch", detail: "audio" } }],
      error: null,
    });
    expect(failed.failed).toEqual([{ path: "D:\\song.mp3", error: "kindMismatch" }]);
  });

  it("separates a failed job from a cancelled one", () => {
    expect(summarizeRelink({ files: [], error: { kind: "fileNotFound", message: "" } }).jobError).toBe(
      "fileNotFound",
    );
    const cancelled = summarizeRelink({ files: [], error: { kind: "cancelled", message: "" } });
    expect(cancelled.cancelled).toBe(true);
    expect(cancelled.jobError).toBeNull();
  });
});

describe("previewKey", () => {
  it("changes when an item comes back online, so a failed preview is asked again", () => {
    const url = "http://media.localhost/poster/m1?v=abc";
    expect(previewKey(url, "offline")).not.toBe(previewKey(url, "online"));
    expect(previewKey(url, "online")).toBe(previewKey(url, "online"));
  });
});

describe("parentDirOf", () => {
  it("returns the folder of Windows and POSIX paths", () => {
    expect(parentDirOf("C:\\clips\\trip\\a.mp4")).toBe("C:\\clips\\trip");
    expect(parentDirOf("/home/me/a.mp4")).toBe("/home/me");
    expect(parentDirOf("a.mp4")).toBeUndefined();
  });
});
