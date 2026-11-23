/**
 * Hit-testing on the timeline canvas, as pure functions (timeline design §12.2). Coordinates are
 * CSS px relative to the canvas's top-left corner; times are flicks.
 */
import type { ClipId, TrackKind } from "@/bindings";

import { RULER_HEIGHT, TRACK_HEIGHT, timeToX, trackAt, trackTop, type Viewport } from "./geometry";

/** Width of the grab zone at each clip edge that starts a trim instead of a move. */
export const TRIM_EDGE_PIXELS = 6;

/** A pointer press becomes a drag (move, trim, marquee) after this many px. */
export const DRAG_THRESHOLD = 3;

/** What hit-testing needs of a clip. */
export type HitClip = { id: ClipId; start: number; duration: number; link?: string | null };

/** What hit-testing needs of a track. */
export type HitTrack = { id: string; kind: TrackKind; locked: boolean; clips: readonly HitClip[] };

export type ClipZone = "body" | "start" | "end";

export type Hit =
  | { kind: "ruler" }
  | { kind: "clip"; track: number; clip: HitClip; zone: ClipZone }
  /** An empty spot on a lane (`track` set) or below/between lanes (`track` null). */
  | { kind: "lane"; track: number | null };

/**
 * The trim zone of each edge: `TRIM_EDGE_PIXELS` wide, but never more than a third of a narrow
 * clip, so even a tiny clip keeps a body to grab.
 */
function edgeZone(width: number): number {
  return Math.min(TRIM_EDGE_PIXELS, width / 3);
}

/** The ruler, a clip (and which part of it) or an empty lane under (`x`, `y`). */
export function hitTest(x: number, y: number, tracks: readonly HitTrack[], viewport: Viewport): Hit {
  if (y < RULER_HEIGHT) return { kind: "ruler" };
  const index = trackAt(y, tracks.length);
  const track = index === null ? undefined : tracks[index];
  if (index === null || !track) return { kind: "lane", track: null };
  // Later clips are drawn on top, so they win where clips touch.
  for (let i = track.clips.length - 1; i >= 0; i--) {
    const clip = track.clips[i];
    if (!clip) continue;
    const left = timeToX(clip.start, viewport);
    const width = clip.duration / viewport.flicksPerPixel;
    if (x < left || x > left + width) continue;
    const edge = edgeZone(width);
    const zone: ClipZone = x <= left + edge ? "start" : x >= left + width - edge ? "end" : "body";
    return { kind: "clip", track: index, clip, zone };
  }
  return { kind: "lane", track: index };
}

export type Rect = { x0: number; y0: number; x1: number; y1: number };

/** The rectangle spanned by two corners, in any order. */
export function normalizeRect(rect: Rect): Rect {
  return {
    x0: Math.min(rect.x0, rect.x1),
    y0: Math.min(rect.y0, rect.y1),
    x1: Math.max(rect.x0, rect.x1),
    y1: Math.max(rect.y0, rect.y1),
  };
}

/** Clips on unlocked tracks whose drawn box intersects the marquee `rect`. */
export function clipsInRect(rect: Rect, tracks: readonly HitTrack[], viewport: Viewport): ClipId[] {
  const r = normalizeRect(rect);
  const ids: ClipId[] = [];
  tracks.forEach((track, index) => {
    if (track.locked) return;
    const top = trackTop(index);
    if (r.y1 < top || r.y0 > top + TRACK_HEIGHT) return;
    for (const clip of track.clips) {
      const left = timeToX(clip.start, viewport);
      const right = left + clip.duration / viewport.flicksPerPixel;
      if (r.x1 >= left && r.x0 <= right) ids.push(clip.id);
    }
  });
  return ids;
}

/** Whether the pointer moved far enough from the press to count as a drag. */
export function pastThreshold(dx: number, dy: number): boolean {
  return Math.hypot(dx, dy) > DRAG_THRESHOLD;
}
