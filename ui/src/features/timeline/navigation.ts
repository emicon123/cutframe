/**
 * Keyboard navigation of the playhead (F1.54, playback design §2): frame steps on the project
 * frame grid, previous/next edit point, start and end. Pure functions over flicks; the actions
 * in `navActions.ts` seek with the results.
 */

/** A clip as navigation sees it. */
type Span = { start: number; duration: number };

/**
 * The playhead `frames` frames away from `time` (negative: back), on the frame grid of `frame`
 * flicks. A playhead between two frames first snaps to the neighbouring frame in that direction.
 * Clamped to `0..=end`.
 */
export function frameStep(time: number, frames: number, frame: number, end: number): number {
  if (frame <= 0 || frames === 0) return time;
  const index = frames > 0 ? Math.floor(time / frame) : Math.ceil(time / frame);
  return Math.min(Math.max(0, end), Math.max(0, (index + frames) * frame));
}

/** End: the start of the last frame of a timeline `duration` long (0 when it is empty). */
export function lastFrame(duration: number, frame: number): number {
  if (duration <= 0 || frame <= 0) return 0;
  return Math.max(0, Math.ceil(duration / frame) * frame - frame);
}

/** Every edit point: 0, the timeline end and each clip's start and end on all tracks, sorted. */
export function editPoints(tracks: readonly { clips: readonly Span[] }[], duration: number): number[] {
  const points = new Set<number>([0, duration]);
  for (const track of tracks) {
    for (const clip of track.clips) {
      points.add(clip.start);
      points.add(clip.start + clip.duration);
    }
  }
  return [...points].sort((a, b) => a - b);
}

/** Up: the nearest edit point before `time`; 0 when there is none. */
export function previousEdit(points: readonly number[], time: number): number {
  let result = 0;
  for (const point of points) {
    if (point >= time) break;
    result = point;
  }
  return result;
}

/** Down: the nearest edit point after `time`; `time` itself when there is none. */
export function nextEdit(points: readonly number[], time: number): number {
  return points.find((point) => point > time) ?? time;
}
