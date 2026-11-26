/**
 * F1.31: the timeline follows the playhead during playback, page by page. When the playhead
 * reaches the far edge (the right one at forward rates, the left one in reverse) or is out of
 * view, the view jumps so the playhead sits `margin` px from the edge it comes from and has a
 * whole page to travel. Paging instead of a smooth follow keeps the clips still while they are
 * watched, and the viewport changes only once per page.
 *
 * While the user scrolls (wheel) or drags on the timeline, following holds off, and comes back
 * `USER_HOLD_MS` after the last scroll.
 */
import { timeToX, type Viewport } from "./geometry";

/** How long a manual scroll holds following off. */
export const USER_HOLD_MS = 1500;

/** The viewport that keeps `time`, moving at `rate`, in view; `viewport` itself when it is. */
export function followPlayhead(
  viewport: Viewport,
  width: number,
  time: number,
  rate: number,
  margin = 48,
): Viewport {
  if (width <= 0 || rate === 0) return viewport;
  const x = timeToX(time, viewport);
  const inset = Math.min(margin, width / 4);
  const inView = rate > 0 ? x >= 0 && x <= width - inset : x >= inset && x <= width;
  if (inView) return viewport;
  const at = rate > 0 ? inset : width - inset;
  return { ...viewport, scroll: Math.max(0, Math.round(time - at * viewport.flicksPerPixel)) };
}

/** Tracks the user's own scrolling and dragging, which hold following off. */
export type AutoScroll = {
  pointerDown: () => void;
  pointerUp: () => void;
  /** A manual scroll at `now` (`performance.now()`). */
  scrolled: (now: number) => void;
  /** `followPlayhead`, unless the user is busy with the view. */
  follow: (viewport: Viewport, width: number, time: number, rate: number, now: number) => Viewport;
};

export function createAutoScroll(): AutoScroll {
  let dragging = false;
  let lastScroll = -Infinity;
  return {
    pointerDown: () => {
      dragging = true;
    },
    pointerUp: () => {
      dragging = false;
    },
    scrolled: (now) => {
      lastScroll = now;
    },
    follow: (viewport, width, time, rate, now) =>
      dragging || now - lastScroll < USER_HOLD_MS ? viewport : followPlayhead(viewport, width, time, rate),
  };
}
