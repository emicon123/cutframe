/**
 * The edit phases of an inspector slider (ADR-0006, clip-speed-audio design §6, §10.1), kept
 * apart from the Radix slider so they can be tested without layout:
 *
 * - a pointer drag is one gesture: `update` at most once per animation frame (`beginGesture`
 *   coalesces), `commit` with the last value on pointer-up;
 * - Escape and window blur cancel it (`beginGesture` listens), and so does lost pointer capture;
 * - a keyboard step (arrows, Home/End, Page keys) is a single `commit`.
 *
 * The gesture opens on the first value change, so a click that moves nothing sends nothing.
 */
import { beginGesture, type Gesture, type GestureSender } from "@/ipc/gesture";

export type SliderGesture = {
  /** `pointerdown` on the slider, before the slider computes a value. */
  pointerDown: () => void;
  /** `keydown` on the slider, before the slider steps. */
  keyDown: () => void;
  /** The slider's value changed (Radix `onValueChange`), as a model value. */
  change: (value: number, send: GestureSender<number>) => void;
  /** The slider committed a value (Radix `onValueCommit`): a keyboard step. */
  commit: (value: number, send: GestureSender<number>) => void;
  /** `pointerup`: commits the drag. */
  pointerUp: () => void;
  /** `lostpointercapture` (pointer cancelled, capture taken away) or unmount: cancels the drag. */
  cancel: () => void;
};

export function createSliderGesture(): SliderGesture {
  let keyboard = false;
  let drag: Gesture<number> | null = null;
  let last: number | null = null;

  return {
    pointerDown() {
      keyboard = false;
      last = null;
      // A drag whose release never arrived: the new one starts clean.
      if (drag?.active) void drag.cancel();
      drag = null;
    },
    keyDown() {
      keyboard = true;
    },
    change(value, send) {
      // A keyboard step changes and commits at once; `commit` sends it.
      if (keyboard) return;
      drag ??= beginGesture(send);
      if (!drag.active) return; // cancelled by Escape or blur: ignore the rest of this drag
      last = value;
      drag.update(value);
    },
    commit(value, send) {
      // A drag commits on pointer-up with its last value instead.
      if (keyboard) void beginGesture(send).commit(value);
    },
    pointerUp() {
      const gesture = drag;
      drag = null;
      if (gesture?.active && last !== null) void gesture.commit(last);
    },
    cancel() {
      const gesture = drag;
      drag = null;
      if (gesture?.active) void gesture.cancel();
    },
  };
}
