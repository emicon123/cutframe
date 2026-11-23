import { createStore } from "zustand/vanilla";

import { commands, type SeekMode, type Time } from "@/bindings";
import { isIpcError, logError } from "@/ipc/errors";

type PlayheadState = {
  /** Playhead position in flicks. */
  time: Time;
  /**
   * Generation returned by the newest answered `seek`, `null` before the first one. Playback
   * ticks of an older generation are stale (playback.md §8, task 0330).
   */
  seekGeneration: number | null;
};

/**
 * The playhead. A vanilla store on purpose: the timeline canvas subscribes to it directly, so
 * moving the playhead never re-renders React components.
 *
 * Until task 0330 the UI owns the playhead and drives the native preview through `seek`
 * (`seekTo`). With playback it becomes a mirror of the Rust transport fed by `playbackTick`.
 */
export const playheadStore = createStore<PlayheadState>()(() => ({ time: 0, seekGeneration: null }));

/** Moves the UI playhead only, without telling the engine. Use `seekTo` for user moves. */
export function setPlayhead(time: Time) {
  playheadStore.setState({ time: Math.max(0, Math.round(time)) });
}

// Every seek goes through one queue, so a scrub can never reach Rust after the exact seek that
// ended the drag (the engine keeps the newest seek). `seek` never blocks, so the wait is short.
let queue: Promise<void> = Promise.resolve();
let pendingScrub: Time | null = null;
let scrubFrame: number | null = null;
let scrubInFlight = false;

function logSeekError(error: unknown): void {
  // No preview engine: the preview simply stays empty, nothing for the user to act on.
  if (isIpcError(error) && error.kind === "unavailable") console.debug(`[ipc] seek: ${error.message}`);
  else logError(error);
}

function sendSeek(time: Time, mode: SeekMode): Promise<void> {
  queue = queue.then(() =>
    commands
      .seek(time, mode)
      .then((seekGeneration) => playheadStore.setState({ seekGeneration }), logSeekError),
  );
  return queue;
}

function scheduleScrub(): void {
  if (scrubFrame === null && !scrubInFlight && pendingScrub !== null) {
    scrubFrame = requestAnimationFrame(flushScrub);
  }
}

function flushScrub(): void {
  scrubFrame = null;
  if (pendingScrub === null) return;
  const time = pendingScrub;
  pendingScrub = null;
  scrubInFlight = true;
  void sendSeek(time, "scrub").finally(() => {
    scrubInFlight = false;
    scheduleScrub();
  });
}

/**
 * Moves the playhead at once (optimistic) and the native preview after it (fire and forget).
 *
 * - `scrub` while the playhead is dragged: at most one `seek` per animation frame and never more
 *   than one in flight, always with the latest time (a slow backend gets fewer, fresher seeks).
 * - `exact` for a click, a step, a jump and the end of a drag: sent at once. A pending scrub is
 *   dropped, since the exact seek carries the final time.
 */
export function seekTo(time: Time, mode: SeekMode): void {
  const before = playheadStore.getState().time;
  setPlayhead(time);
  const target = playheadStore.getState().time;
  if (mode === "scrub") {
    if (target === before && pendingScrub === null) return;
    pendingScrub = target;
    scheduleScrub();
    return;
  }
  pendingScrub = null;
  if (scrubFrame !== null) cancelAnimationFrame(scrubFrame);
  scrubFrame = null;
  void sendSeek(target, "exact");
}

/**
 * Keeps the engine's playhead equal to the UI's when the timeline duration changes. The engine
 * clamps every seek to `0..=duration` (playback.md §4.2), so a playhead placed beyond the end is
 * sent again once the timeline may reach it, and the first project state (startup, a webview
 * reload) sends it once. Call it with the duration before and after each project change.
 */
export function reseekOnDurationChange(before: Time | undefined, after: Time | undefined): void {
  if (after === undefined || after === before) return;
  const { time } = playheadStore.getState();
  if (before === undefined || time > before) seekTo(time, "exact");
}
