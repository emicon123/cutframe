import { t } from "@/i18n";

/** One `jobProgress` value of the encoding stage and when it arrived (ms, any monotonic clock). */
export type ProgressSample = { at: number; progress: number };

/** The rate is measured over the last 5 s of progress (export.md §5.1). */
export const ETA_WINDOW_MS = 5000;

/** No estimate in the first 3 s of encoding: the start (decoder warm-up, fallback) is not typical. */
export const ETA_DELAY_MS = 3000;

/** The samples after `sample` arrives: only those inside the window ending at it. */
export function addSample(samples: readonly ProgressSample[], sample: ProgressSample): ProgressSample[] {
  return [...samples.filter((s) => s.at >= sample.at - ETA_WINDOW_MS), sample];
}

/**
 * Milliseconds left at `now`, or `null` while there is no estimate: in the first 3 s of encoding,
 * with fewer than two samples in the last 5 s (a stall), or with no progress over them. The rate
 * comes from the window, so a slow start or a short stall fades out instead of skewing the
 * estimate, and the time since the last sample is subtracted, so the estimate counts down between
 * progress events.
 */
export function estimateRemainingMs(
  samples: readonly ProgressSample[],
  encodingSince: number | null,
  now: number,
): number | null {
  if (encodingSince === null || now - encodingSince < ETA_DELAY_MS) return null;
  const recent = samples.filter((s) => s.at >= now - ETA_WINDOW_MS);
  const first = recent[0];
  const last = recent.at(-1);
  if (!first || !last || recent.length < 2) return null;
  const done = last.progress - first.progress;
  const elapsed = last.at - first.at;
  if (done <= 0 || elapsed <= 0) return null;
  const remaining = ((1 - last.progress) * elapsed) / done - (now - last.at);
  return Math.max(0, remaining);
}

/** "About 35 s left" (5 s steps), "About 4 min left", "About 1 h 5 min left". */
export function formatEta(ms: number): string {
  const seconds = Math.max(0, ms) / 1000;
  const texts = t.export.progress;
  if (seconds < 55) return texts.etaSeconds(Math.max(5, Math.ceil(seconds / 5) * 5));
  const minutes = Math.max(1, Math.round(seconds / 60));
  if (minutes < 60) return texts.etaMinutes(minutes);
  return texts.etaHours(Math.floor(minutes / 60), minutes % 60);
}
