import { createStore } from "zustand/vanilla";

import { commands, type PlaybackTick, type SeekMode, type Time, type TransportStateView } from "@/bindings";
import { isIpcError, logError } from "@/ipc/errors";
import { FLICKS_PER_SECOND } from "@/lib/time";

export type PlayheadState = {
  /**
   * Playhead position in flicks: the engine's playhead while paused, the start of the frame on
   * screen while playing, or a local move the engine has not answered yet.
   */
  time: Time;
  /** `performance.now()` when `time` last moved: the base of the extrapolation while playing. */
  at: number;
  state: TransportStateView;
  /** -8..=8, 0 while paused. */
  rate: number;
  looping: boolean;
  /** Frames dropped since the last play start. */
  droppedFrames: number;
  /**
   * The newest seek generation known: answered by `seek` / `stepFrames` or seen in a tick, `null`
   * before the first one. Ticks of an older generation are stale (playback.md §8).
   */
  seekGeneration: number | null;
};

/**
 * The playhead and the transport. A vanilla store on purpose: the timeline canvas and the
 * timecode subscribe to it directly, so playback never re-renders React components.
 *
 * A mirror of the Rust transport (task 0330), fed by `playbackTick` through `applyTick`. Seeks and
 * steps move it at once (optimistic); the engine's ticks confirm or correct it.
 */
export const playheadStore = createStore<PlayheadState>()(() => ({
  time: 0,
  at: 0,
  state: "paused",
  rate: 0,
  looping: false,
  droppedFrames: 0,
  seekGeneration: null,
}));

/** Moves the UI playhead only, without telling the engine. Use `seekTo` for user moves. */
export function setPlayhead(time: Time) {
  playheadStore.setState({ time: Math.max(0, Math.round(time)), at: performance.now() });
}

/**
 * The store after `tick`, or `null` when the tick is older than the newest seek generation (it was
 * emitted before a seek or step the UI already shows). The extrapolation base `at` moves only when
 * the shown frame or the motion changes, so repeated ticks of one frame do not hold it back.
 */
export function reduceTick(s: PlayheadState, tick: PlaybackTick, now: number): PlayheadState | null {
  if (s.seekGeneration !== null && tick.seek < s.seekGeneration) return null;
  const rate = tick.state === "playing" ? tick.rate : 0;
  const moved = tick.time !== s.time || tick.state !== s.state || rate !== s.rate;
  return {
    time: tick.time,
    at: moved ? now : s.at,
    state: tick.state,
    rate,
    looping: tick.looping,
    droppedFrames: tick.droppedFrames,
    seekGeneration: Math.max(s.seekGeneration ?? tick.seek, tick.seek),
  };
}

/** Upper bound of the extrapolation: a little more than one frame at 24 fps. */
export const MAX_EXTRAPOLATION_MS = 50;

/**
 * The playhead to draw at `now` (`performance.now()`): while playing, `time + elapsed × rate`
 * between ticks, so the playhead moves smoothly; it snaps back to the next tick. Clamped to
 * `0..=end`. View math only: a float, never stored or sent.
 */
export function displayTime(
  s: Pick<PlayheadState, "time" | "at" | "state" | "rate">,
  now: number,
  end: Time,
): number {
  if (s.state !== "playing" || s.rate === 0) return s.time;
  const elapsed = Math.min(Math.max(0, now - s.at), MAX_EXTRAPOLATION_MS);
  const time = s.time + (elapsed / 1000) * FLICKS_PER_SECOND * s.rate;
  return Math.min(Math.max(0, time), Math.max(end, s.time));
}

// Every seek and step goes through one queue, so a scrub can never reach Rust after the exact seek
// that ended the drag (the engine keeps the newest seek). They never block, so the wait is short.
let queue: Promise<void> = Promise.resolve();
let pendingScrub: Time | null = null;
let scrubFrame: number | null = null;
let scrubInFlight = false;
/** Seeks and steps queued or in flight: the UI shows a playhead the engine has not answered. */
let unanswered = 0;
/** The newest tick that arrived while a local move was unanswered. */
let heldTick: PlaybackTick | null = null;

/**
 * Logs a failed transport command. No preview engine (`unavailable`): the preview simply stays
 * empty, nothing for the user to act on.
 */
export function logTransportError(command: string): (error: unknown) => void {
  return (error) => {
    if (isIpcError(error) && error.kind === "unavailable")
      console.debug(`[ipc] ${command}: ${error.message}`);
    else logError(error);
  };
}

/**
 * Applies a `playbackTick` (or the `getPlayback` answer). While a local seek or step is
 * unanswered, the tick is held: it may predate the move, and its generation can only be judged
 * once the engine has answered. The newest held tick is applied then.
 */
export function applyTick(tick: PlaybackTick, now = performance.now()): void {
  if (unanswered > 0 || pendingScrub !== null) {
    heldTick = tick;
    return;
  }
  heldTick = null;
  const next = reduceTick(playheadStore.getState(), tick, now);
  if (next) playheadStore.setState(next);
}

function releaseHeldTick(): void {
  if (heldTick !== null) applyTick(heldTick);
}

function send(command: string, call: () => Promise<number>): Promise<void> {
  unanswered += 1;
  queue = queue.then(() =>
    call()
      .then((generation) =>
        playheadStore.setState((s) => ({
          seekGeneration: Math.max(s.seekGeneration ?? generation, generation),
        })),
      )
      .catch(logTransportError(command))
      .finally(() => {
        unanswered -= 1;
        releaseHeldTick();
      }),
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
  void send("seek", () => commands.seek(time, "scrub")).finally(() => {
    scrubInFlight = false;
    scheduleScrub();
  });
}

function dropPendingScrub(): void {
  pendingScrub = null;
  if (scrubFrame !== null) cancelAnimationFrame(scrubFrame);
  scrubFrame = null;
}

/**
 * Moves the playhead at once (optimistic) and the engine after it (fire and forget).
 *
 * - `scrub` while the playhead is dragged: at most one `seek` per animation frame and never more
 *   than one in flight, always with the latest time (a slow backend gets fewer, fresher seeks).
 *   The engine pauses playback on the first one and plays short audio snippets.
 * - `exact` for a click, a jump and the end of a drag: sent at once (while playing, playback goes
 *   on from there). A pending scrub is dropped, since the exact seek carries the final time.
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
  dropPendingScrub();
  void send("seek", () => commands.seek(target, "exact"));
}

/**
 * Left / Right (Shift: 10 frames): the engine pauses and steps `frames` whole frames from the
 * frame on screen, clamped. `expected` is where the UI thinks it lands, shown until the tick.
 */
export function stepTo(frames: number, expected: Time): void {
  dropPendingScrub();
  setPlayhead(expected);
  playheadStore.setState({ state: "paused", rate: 0 });
  void send("stepFrames", () => commands.stepFrames(frames));
}
