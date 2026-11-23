/**
 * Track header rules (timeline design §3.3, §4.15, §4.16): which flags a track offers, how a
 * track draws, and where a dragged header lands. Pure functions; Rust clamps and validates again.
 */
import type { TrackFlag, TrackKind } from "@/bindings";

import { RULER_HEIGHT, TRACK_GAP, TRACK_HEIGHT, trackTop } from "./geometry";

/** The flags a track's header offers: mute where it can sound, hide where it draws, lock always. */
export function trackFlags(kind: TrackKind): TrackFlag[] {
  if (kind === "audio") return ["muted", "locked"];
  if (kind === "text") return ["hidden", "locked"];
  return ["muted", "hidden", "locked"];
}

/** What these rules need of a track. */
export type FlagTrack = { id: string; kind: TrackKind; muted: boolean; hidden: boolean; locked: boolean };

/**
 * Whether the lane is drawn dimmed: the track adds nothing to playback (an audio track that is
 * muted, or a video or text track that is hidden). A muted video track still draws.
 */
export function trackDimmed(track: Pick<FlagTrack, "kind" | "muted" | "hidden">): boolean {
  return track.kind === "audio" ? track.muted : track.hidden;
}

/**
 * The indices a track can move to: its own section (overlays above the main track, audio below).
 * `null` for the main track, which never moves, or when the main track is unknown.
 */
export function moveRange(
  tracks: readonly { id: string }[],
  mainTrack: string | null,
  index: number,
): { min: number; max: number } | null {
  const main = tracks.findIndex((track) => track.id === mainTrack);
  if (main < 0 || index === main || index < 0 || index >= tracks.length) return null;
  return index < main ? { min: 0, max: main - 1 } : { min: main + 1, max: tracks.length - 1 };
}

/** A header drop: the final index to send to `moveTrack` and the boundary to draw the line at. */
export type TrackDrop = { index: number; boundary: number };

/**
 * Where a header dragged from `from` lands with the pointer at `y` (px from the top of the
 * header column, which starts with the ruler spacer like the canvas). The insertion boundary
 * (0 = above the first track) is the one nearest `y`, clamped into the track's section; the final
 * index accounts for the track leaving its old place. `null` when the track cannot move or the
 * drop leaves it where it is.
 */
export function trackDropAt(
  tracks: readonly { id: string }[],
  mainTrack: string | null,
  from: number,
  y: number,
): TrackDrop | null {
  const range = moveRange(tracks, mainTrack, from);
  if (!range) return null;
  const raw = Math.round((y - RULER_HEIGHT) / (TRACK_HEIGHT + TRACK_GAP));
  const boundary = Math.min(range.max + 1, Math.max(range.min, raw));
  const index = boundary > from ? boundary - 1 : boundary;
  return index === from ? null : { index, boundary };
}

/** The y of the insertion line for `boundary`, centred in the gap above that track. */
export function boundaryY(boundary: number): number {
  return trackTop(boundary) - TRACK_GAP / 2;
}
