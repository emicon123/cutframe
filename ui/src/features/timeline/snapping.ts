/**
 * Snapping is a view concern (it depends on the zoom), computed in the UI (timeline design §6.2).
 * Times are flicks.
 */

/** Snap distance in CSS pixels; multiply by `flicksPerPixel` for the threshold. */
export const SNAP_PIXELS = 8;

export type SnapResult = { delta: number; target: number | null };

/**
 * Finds the candidate–target pair closest together within `threshold` flicks and returns the
 * delta that moves the candidate onto the target. No pair in range: `{ delta: 0, target: null }`.
 */
export function snap(
  candidates: readonly number[],
  targets: readonly number[],
  threshold: number,
): SnapResult {
  let best: SnapResult = { delta: 0, target: null };
  let bestDistance = Infinity;
  for (const candidate of candidates) {
    for (const target of targets) {
      const distance = Math.abs(target - candidate);
      if (distance <= threshold && distance < bestDistance) {
        bestDistance = distance;
        best = { delta: target - candidate, target };
      }
    }
  }
  return best;
}

/** Rounds `time` to the nearest frame of a grid with `frame` flicks per frame (never below 0). */
export function toFrameGrid(time: number, frame: number): number {
  if (!(frame > 0)) return Math.max(0, Math.round(time));
  return Math.max(0, Math.round(time / frame) * frame);
}
