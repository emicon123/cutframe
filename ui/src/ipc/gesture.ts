/**
 * Continuous edits (drag, trim, slider scrub) as one undo step (ADR-0006 §4).
 *
 * Rust owns the in-progress state: every `update` replaces the model from the gesture's base and
 * emits `project-changed`, so the UI and the native preview draw exactly what a `commit` would
 * record. This module only decides *when* to talk to Rust:
 *
 * - at most one `update` per animation frame, coalesced to the latest value, and never more than
 *   one in flight (a slow backend gets fewer, fresher updates instead of a backlog);
 * - all gesture traffic goes through one queue, so a pending update can never arrive after the
 *   `commit` or `cancelGesture` that ended it, nor after the next gesture has started;
 * - `cancelGesture` on Escape, window blur and lost pointer capture (see `startPointerGesture`);
 * - a failed `commit` leaves the Rust gesture open at its last valid state, so it is cancelled;
 * - only one gesture is active at a time: beginning a new one cancels the previous one.
 *
 * Plugging in a gesture command: Rust defines it with absolute parameters plus `phase: EditPhase`
 * (e.g. `set_clip_start(clip, start, phase)`), `pnpm bindings` exports it, and the UI passes a
 * sender built from the generated binding, for example in a canvas `onPointerDown`:
 *
 * ```ts
 * startPointerGesture(e.currentTarget, e.pointerId, {
 *   send: (start: Time, phase) => commands.setClipStart(clip.id, start, phase),
 *   valueAt: (move) => clip.start + snappedDelta(move.clientX - e.clientX),
 * });
 * ```
 *
 * A keyboard nudge or a typed value is a single commit without updates:
 * `beginGesture(send).commit(value)`.
 */
import { commands, type EditPhase } from "@/bindings";
import { logError } from "@/ipc/errors";
import { useAppStore } from "@/stores/appStore";

/** Sends one absolute value with a phase, usually a generated command from `bindings.ts`. */
export type GestureSender<V> = (value: V, phase: EditPhase) => Promise<unknown>;

export type Gesture<V> = {
  /** Shows `value` live. Coalesced: only the latest value per animation frame is sent. */
  update: (value: V) => void;
  /**
   * Records `value` as one history entry and ends the gesture. A pending update is dropped, since
   * the commit carries the final value. Resolves `false` if the gesture had already ended or the
   * commit failed (the failure is reported and the Rust gesture cancelled).
   */
  commit: (value: V) => Promise<boolean>;
  /** Ends the gesture and restores the state before it. Does nothing if it already ended. */
  cancel: () => Promise<void>;
  readonly active: boolean;
};

let queue: Promise<unknown> = Promise.resolve();
let current: Pick<Gesture<unknown>, "cancel"> | null = null;

/** Runs `task` after every previously queued gesture call has settled. */
function enqueue<T>(task: () => Promise<T>): Promise<T> {
  const result = queue.then(task);
  queue = result.catch(() => undefined);
  return result;
}

function sendCancel(): Promise<void> {
  return enqueue(() => commands.cancelGesture()).then(
    () => undefined,
    (error: unknown) => useAppStore.getState().reportError(error),
  );
}

/** Starts a gesture. Any gesture still active is cancelled first. */
export function beginGesture<V>(send: GestureSender<V>): Gesture<V> {
  if (current) void current.cancel();

  let active = true;
  let pending: { value: V } | null = null;
  let frame: number | null = null;
  let inFlight = false;

  const schedule = () => {
    if (frame === null && !inFlight && pending) frame = requestAnimationFrame(flush);
  };

  const flush = () => {
    frame = null;
    if (!active || !pending) return;
    const { value } = pending;
    pending = null;
    inFlight = true;
    void enqueue(() => send(value, "update"))
      // An invalid intermediate value leaves Rust at the last valid one; the commit reports.
      .catch(logError)
      .finally(() => {
        inFlight = false;
        if (active) schedule();
      });
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    void gesture.cancel();
  };
  const onBlur = () => void gesture.cancel();

  const end = () => {
    active = false;
    pending = null;
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
    window.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("blur", onBlur);
    if (current === gesture) current = null;
  };

  const gesture: Gesture<V> = {
    get active() {
      return active;
    },
    update(value) {
      if (!active) return;
      pending = { value };
      schedule();
    },
    commit(value) {
      if (!active) return Promise.resolve(false);
      end();
      // The cancel after a failure stays in the same queue slot, so it cannot hit a newer gesture.
      return enqueue(async () => {
        try {
          await send(value, "commit");
          return true;
        } catch (error) {
          useAppStore.getState().reportError(error);
          await commands.cancelGesture().catch(logError);
          return false;
        }
      });
    },
    cancel() {
      if (!active) return Promise.resolve();
      end();
      return sendCancel();
    },
  };

  // Capture phase, so Escape cancels the drag before anything else reacts to it.
  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("blur", onBlur);
  current = gesture;
  return gesture;
}

export type PointerGestureOptions<V> = {
  send: GestureSender<V>;
  /** The absolute value for a pointer position, e.g. the original start plus the snapped offset. */
  valueAt: (event: PointerEvent) => V;
};

/**
 * Drives a gesture from a pointer: captures it on `element`, sends updates on `pointermove`,
 * commits on `pointerup` and cancels on `pointercancel` or lost capture. Call it from
 * `pointerdown` with `e.currentTarget` and `e.pointerId`.
 */
export function startPointerGesture<V>(
  element: HTMLElement,
  pointerId: number,
  { send, valueAt }: PointerGestureOptions<V>,
): Gesture<V> {
  const gesture = beginGesture(send);
  const ours = (event: PointerEvent) => event.pointerId === pointerId;

  const onMove = (event: PointerEvent) => {
    if (ours(event)) gesture.update(valueAt(event));
  };
  const onUp = (event: PointerEvent) => {
    if (!ours(event)) return;
    detach();
    void gesture.commit(valueAt(event));
  };
  const onLost = (event: PointerEvent) => {
    if (!ours(event)) return;
    detach();
    void gesture.cancel();
  };
  const detach = () => {
    element.removeEventListener("pointermove", onMove);
    element.removeEventListener("pointerup", onUp);
    element.removeEventListener("pointercancel", onLost);
    element.removeEventListener("lostpointercapture", onLost);
  };

  element.addEventListener("pointermove", onMove);
  element.addEventListener("pointerup", onUp);
  element.addEventListener("pointercancel", onLost);
  element.addEventListener("lostpointercapture", onLost);
  element.setPointerCapture(pointerId);
  return gesture;
}
