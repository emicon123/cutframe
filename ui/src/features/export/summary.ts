import type { ExportCheck, ExportIssue, MediaId, VideoCodec } from "@/bindings";
import { formatBytes } from "@/features/preferences/preferences";
import { t } from "@/i18n";
import { FLICKS_PER_SECOND, formatDuration, formatFrameRate } from "@/lib/time";

import type { PresetLabel } from "./presets";

/** YouTube publishes longer uploads as regular videos, not Shorts. */
export const SHORTS_MAX_DURATION = 180 * FLICKS_PER_SECOND;

/** A bitrate for display: `384 kbps`, `12 Mbps`, `67.5 Mbps`. */
export function formatBitrate(kbps: number): string {
  if (kbps < 1000) return t.export.units.kbps(String(kbps));
  return t.export.units.mbps(String(Math.round(kbps / 100) / 10));
}

/**
 * The summary line: `0:42 · 1920 × 1080 · 30 fps · H.264 12 Mbps · AAC 384 kbps · about 64.5 MB`,
 * with "upscaled" when the output is larger than the canvas. All numbers come from `checkExport`.
 */
export function summaryLine(
  check: ExportCheck,
  codec: VideoCodec,
  canvas: { width: number; height: number },
): string {
  const s = t.export.summary;
  const parts = [
    formatDuration(check.duration),
    s.size(check.width, check.height),
    formatFrameRate(check.frameRate),
    s.video(t.export.codecs[codec], formatBitrate(check.videoKbps)),
    s.audio(formatBitrate(check.audioKbps)),
    s.estimate(formatBytes(check.estimatedBytes)),
  ];
  if (check.width * check.height > canvas.width * canvas.height) parts.push(s.upscaled);
  return parts.join(s.separator);
}

/** "212.4 GB free on this drive", or `null` when Rust could not tell. */
export function freeSpaceText(check: ExportCheck): string | null {
  const free = check.freeBytes ?? null;
  return free === null ? null : t.export.summary.free(formatBytes(free));
}

/** One line under the summary: a blocker, a warning or a note. */
export type IssueView = {
  key: string;
  tone: "error" | "warning";
  text: string;
  /** A missing source the user can relink from here. */
  relink?: MediaId;
};

/** `fileExists` (asks first) and `lowDiskSpace` are warnings; every other issue blocks. */
export function isBlocking(issue: ExportIssue): boolean {
  return issue.kind !== "fileExists" && issue.kind !== "lowDiskSpace";
}

/** Readable text of an issue; `mediaName` names a media id from the project. */
export function issueText(issue: ExportIssue, mediaName: (id: MediaId) => string): string {
  const texts = t.export.issues;
  switch (issue.kind) {
    case "outputIsSource":
      return texts.outputIsSource(mediaName(issue.media));
    case "mediaOffline":
      return texts.mediaOffline(mediaName(issue.media));
    default:
      return texts[issue.kind];
  }
}

/**
 * Everything to list under the summary, blockers first: Rust's issues, and the Shorts rules the
 * UI applies to the returned size and duration (vertical, at most 3 minutes).
 */
export function issueViews(
  check: ExportCheck,
  preset: PresetLabel,
  mediaName: (id: MediaId) => string,
): IssueView[] {
  const views: IssueView[] = check.issues.map((issue, index) => ({
    key: `${issue.kind}-${index}`,
    tone: isBlocking(issue) ? "error" : "warning",
    text: issueText(issue, mediaName),
    ...(issue.kind === "mediaOffline" ? { relink: issue.media } : {}),
  }));
  if (preset === "shorts") {
    if (check.width > check.height) {
      views.push({ key: "shortsNotVertical", tone: "warning", text: t.export.issues.shortsNotVertical });
    }
    if (check.duration > SHORTS_MAX_DURATION) {
      views.push({ key: "shortsTooLong", tone: "warning", text: t.export.issues.shortsTooLong });
    }
  }
  return views.sort((a, b) => Number(a.tone === "warning") - Number(b.tone === "warning"));
}
