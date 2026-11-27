import { describe, expect, it } from "vitest";

import type { ExportCheck, ExportIssue } from "@/bindings";
import { t } from "@/i18n";
import { FLICKS_PER_SECOND } from "@/lib/time";

import { formatBitrate, freeSpaceText, isBlocking, issueText, issueViews, summaryLine } from "./summary";

const check = (overrides: Partial<ExportCheck> = {}): ExportCheck => ({
  width: 1920,
  height: 1080,
  frameRate: { num: 30, den: 1 },
  duration: 42 * FLICKS_PER_SECOND,
  frames: 1260,
  videoKbps: 12000,
  audioKbps: 384,
  sampleRate: 48000,
  estimatedBytes: "67600000",
  freeBytes: "228000000000",
  encoder: "nvenc",
  issues: [],
  canStart: true,
  ...overrides,
});

const names: Record<string, string> = { v: "beach.mp4", a: "song.flac" };
const mediaName = (id: string) => names[id] ?? "?";

describe("formatBitrate", () => {
  it("shows kbps below 1 Mbps and Mbps with at most one decimal above", () => {
    expect(formatBitrate(384)).toBe("384 kbps");
    expect(formatBitrate(12000)).toBe("12 Mbps");
    expect(formatBitrate(67500)).toBe("67.5 Mbps");
    expect(formatBitrate(7040)).toBe("7 Mbps");
    expect(formatBitrate(1000)).toBe("1 Mbps");
  });
});

describe("summaryLine", () => {
  const canvas = { width: 1920, height: 1080 };

  it("joins length, size, rate, codec and bitrates and the estimated file size", () => {
    expect(summaryLine(check(), "h264", canvas)).toBe(
      "0:42 · 1920 × 1080 · 30 fps · H.264 12 Mbps · AAC 384 kbps · about 64.4 MB",
    );
  });

  it("says upscaled when the output is larger than the canvas", () => {
    const line = summaryLine(check({ width: 3840, height: 2160, videoKbps: 67500 }), "hevc", canvas);
    expect(line).toContain("HEVC 67.5 Mbps");
    expect(line.endsWith(` · ${t.export.summary.upscaled}`)).toBe(true);
    expect(summaryLine(check({ width: 1280, height: 720 }), "h264", canvas)).not.toContain(
      t.export.summary.upscaled,
    );
  });

  it("shows the free space when Rust knows it", () => {
    expect(freeSpaceText(check())).toBe("212.3 GB free on this drive");
    expect(freeSpaceText(check({ freeBytes: null }))).toBeNull();
  });
});

describe("issues", () => {
  const all: ExportIssue[] = [
    { kind: "emptyTimeline" },
    { kind: "invalidPath" },
    { kind: "folderMissing" },
    { kind: "outputIsSource", media: "v" },
    { kind: "mediaOffline", media: "a" },
    { kind: "codecUnavailable" },
    { kind: "fileLocked" },
    { kind: "fileExists" },
    { kind: "lowDiskSpace" },
  ];

  it("words every issue, naming the media from the project", () => {
    for (const issue of all) expect(issueText(issue, mediaName)).not.toBe("");
    expect(issueText({ kind: "mediaOffline", media: "a" }, mediaName)).toBe(
      t.export.issues.mediaOffline("song.flac"),
    );
    expect(issueText({ kind: "outputIsSource", media: "v" }, mediaName)).toContain("beach.mp4");
    expect(issueText({ kind: "fileLocked" }, mediaName)).toBe(t.export.issues.fileLocked);
  });

  it("blocks on everything but an existing file and low disk space", () => {
    expect(all.filter((i) => !isBlocking(i)).map((i) => i.kind)).toEqual(["fileExists", "lowDiskSpace"]);
  });

  it("lists blockers first and offers relink for offline media", () => {
    const views = issueViews(
      check({ issues: [{ kind: "lowDiskSpace" }, { kind: "mediaOffline", media: "a" }], canStart: false }),
      "youtube1080",
      mediaName,
    );
    expect(views.map((v) => [v.tone, v.relink])).toEqual([
      ["error", "a"],
      ["warning", undefined],
    ]);
  });

  it("warns about Shorts that are not vertical or longer than 3 minutes", () => {
    const shorts = (c: ExportCheck) => issueViews(c, "shorts", mediaName).map((v) => v.key);
    expect(shorts(check({ width: 1080, height: 1920 }))).toEqual([]);
    expect(shorts(check())).toEqual(["shortsNotVertical"]);
    expect(shorts(check({ width: 1080, height: 1920, duration: 181 * FLICKS_PER_SECOND }))).toEqual([
      "shortsTooLong",
    ]);
    expect(issueViews(check({ duration: 600 * FLICKS_PER_SECOND }), "youtube1080", mediaName)).toEqual([]);
  });
});
