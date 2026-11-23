import type { Time } from "@/bindings";
import { playheadStore, seekTo } from "@/stores/playheadStore";

/**
 * Moves the playhead with a pointer, from its `pointerdown` on `element`: the press is an exact
 * seek (a click shows the exact frame), every move while captured a scrub, and the release an
 * exact seek again if the pointer moved, so a drag always ends on the exact frame. Losing the
 * capture (pointercancel, window switch) ends the drag the same way at the last position.
 */
export function startPlayheadDrag(
  element: HTMLElement,
  down: PointerEvent,
  timeAt: (event: PointerEvent) => Time,
) {
  const { pointerId } = down;
  const ours = (event: PointerEvent) => event.pointerId === pointerId;
  let moved = false;

  const onMove = (event: PointerEvent) => {
    if (!ours(event)) return;
    moved = true;
    seekTo(timeAt(event), "scrub");
  };
  const onUp = (event: PointerEvent) => {
    if (ours(event)) end(timeAt(event));
  };
  const onLost = (event: PointerEvent) => {
    if (ours(event)) end(playheadStore.getState().time);
  };
  const end = (time: Time) => {
    element.removeEventListener("pointermove", onMove);
    element.removeEventListener("pointerup", onUp);
    element.removeEventListener("pointercancel", onLost);
    element.removeEventListener("lostpointercapture", onLost);
    if (moved) seekTo(time, "exact");
  };

  element.addEventListener("pointermove", onMove);
  element.addEventListener("pointerup", onUp);
  element.addEventListener("pointercancel", onLost);
  element.addEventListener("lostpointercapture", onLost);
  element.setPointerCapture(pointerId);
  seekTo(timeAt(down), "exact");
}
