/**
 * Gaps on a track (timeline design §4.6, §6.1), as pure functions. A gap is the free span before
 * the first clip or between two clips, never after the last one: the same rule as `deleteGap`.
 */
import type { TrackId } from "@/bindings";

import type { HitClip, HitTrack } from "./hitTest";

/** A selected gap: the track and a time inside it, the arguments `deleteGap` takes. */
export type GapSelection = { track: TrackId; at: number };

export type GapSpan = { start: number; end: number };

/** The gap that contains `at` among `clips` (start inclusive, end exclusive); `null` if none. */
export function gapAt(clips: readonly HitClip[], at: number): GapSpan | null {
  if (at < 0) return null;
  const sorted = [...clips].sort((a, b) => a.start - b.start);
  let start = 0;
  for (const clip of sorted) {
    if (at < clip.start) return at >= start ? { start, end: clip.start } : null;
    start = Math.max(start, clip.start + clip.duration);
  }
  return null;
}

/** The selectable gap at `at` on `track`: none on a locked track, since it cannot be deleted. */
export function gapOnTrack(track: HitTrack | undefined, at: number): GapSpan | null {
  if (!track || track.locked) return null;
  return gapAt(track.clips, at);
}

/** Where a selected gap is now: its track index and span, or `null` when it no longer exists. */
export function locateGap(
  tracks: readonly HitTrack[],
  gap: GapSelection | null,
): (GapSpan & { index: number }) | null {
  if (!gap) return null;
  const index = tracks.findIndex((track) => track.id === gap.track);
  const span = gapOnTrack(tracks[index], gap.at);
  return span ? { index, ...span } : null;
}
