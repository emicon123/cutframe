/**
 * Where media dragged onto the timeline would land (timeline design §2, §4.1), as pure functions.
 * The preview mirrors what `addClips` does so the ghost shows the result; Rust stays the judge and
 * rejects anything that does not fit.
 */
import type { MediaKind, Placement, TrackId, TrackKind } from "@/bindings";
import { FLICKS_PER_SECOND } from "@/lib/time";

import { RULER_HEIGHT, TRACK_GAP, TRACK_HEIGHT, type Viewport, xToTime } from "./geometry";
import { SNAP_PIXELS, snap, toFrameGrid } from "./snapping";

/** Rows within this many px of a track's top or bottom edge mean "a new track here". */
export const NEW_TRACK_EDGE = 8;

/**
 * Length a still image gets on the timeline, for the ghost only (Rust's `DEFAULT_IMAGE_DURATION`,
 * 5 s, decides the real clip).
 */
export const IMAGE_PREVIEW_DURATION = 5 * FLICKS_PER_SECOND;

export type DropPayload = {
  /** Every item has a picture (video, image): it goes on video tracks. */
  visual: boolean;
  /** Every item is audio-only: it goes on audio tracks. */
  audioOnly: boolean;
  /** Total length of the clips, placed one after another. */
  duration: number;
};

/** What a list of media items needs from the timeline. Mixed kinds fit no single track. */
export function dropPayload(media: readonly { kind: MediaKind; duration: number | null }[]): DropPayload {
  const audio = media.filter((m) => m.kind === "audio").length;
  return {
    visual: media.length > 0 && audio === 0,
    audioOnly: media.length > 0 && audio === media.length,
    duration: media.reduce(
      (sum, m) => sum + (m.kind === "image" ? IMAGE_PREVIEW_DURATION : (m.duration ?? 0)),
      0,
    ),
  };
}

export type DropTrack = {
  id: TrackId;
  kind: TrackKind;
  locked: boolean;
  clips: readonly { start: number; duration: number }[];
};

export type DropInput = {
  /** CSS px relative to the canvas's top-left corner. */
  x: number;
  y: number;
  tracks: readonly DropTrack[];
  mainTrack: TrackId;
  magnetic: boolean;
  viewport: Viewport;
  playhead: number;
  /** Flicks per project frame; drop times land on frames. */
  frame: number;
  payload: DropPayload;
  /** Off while Alt is held or when snapping is turned off. */
  snapping: boolean;
};

export type DropPreview =
  | {
      kind: "track";
      index: number;
      track: TrackId;
      start: number;
      duration: number;
      valid: boolean;
      /** Time of the snap line, if the drop snapped. */
      snap: number | null;
      /** An insertion into the magnetic main track at a clip boundary (`start`). */
      insert: boolean;
    }
  | { kind: "newTrack"; index: number; start: number; duration: number; valid: boolean; snap: number | null };

type Row = { track: number } | { newAt: number };

/** The track row under `y`, or a new track between rows (near an edge, or below the last track). */
export function rowAt(y: number, trackCount: number): Row | null {
  if (y < RULER_HEIGHT) return null;
  const pitch = TRACK_HEIGHT + TRACK_GAP;
  const index = Math.floor((y - RULER_HEIGHT) / pitch);
  if (index >= trackCount) return { newAt: trackCount };
  const within = y - RULER_HEIGHT - index * pitch;
  if (within < NEW_TRACK_EDGE) return { newAt: index };
  if (within >= TRACK_HEIGHT - NEW_TRACK_EDGE) return { newAt: index + 1 };
  return { track: index };
}

export function accepts(track: DropTrack, payload: DropPayload): boolean {
  if (track.locked) return false;
  if (payload.audioOnly) return track.kind === "audio";
  return payload.visual && track.kind === "video";
}

/** A new track's index clamped into the section the clips fit: overlays above main, audio below. */
export function clampNewTrackIndex(
  index: number,
  mainIndex: number,
  trackCount: number,
  payload: DropPayload,
): number {
  return payload.audioOnly
    ? Math.min(trackCount, Math.max(mainIndex + 1, index))
    : Math.min(mainIndex, Math.max(0, index));
}

/** The clip boundary of `clips` (time 0 and every clip end) nearest to `time`. */
export function nearestBoundary(clips: readonly { start: number; duration: number }[], time: number): number {
  let best = 0;
  for (const clip of clips) {
    const end = clip.start + clip.duration;
    if (Math.abs(end - time) < Math.abs(best - time)) best = end;
  }
  return best;
}

function overlaps(clips: DropTrack["clips"], start: number, duration: number): boolean {
  return clips.some((clip) => clip.start < start + duration && start < clip.start + clip.duration);
}

/** Snap targets: time 0, the playhead and every clip edge. */
function snapTargets(tracks: readonly DropTrack[], playhead: number): number[] {
  const targets = [0, playhead];
  for (const track of tracks)
    for (const clip of track.clips) targets.push(clip.start, clip.start + clip.duration);
  return targets;
}

export function dropPreview(input: DropInput): DropPreview | null {
  const { tracks, payload, viewport } = input;
  const row = rowAt(input.y, tracks.length);
  if (!row) return null;
  const mainIndex = Math.max(
    0,
    tracks.findIndex((track) => track.id === input.mainTrack),
  );
  const fits = payload.visual || payload.audioOnly;

  const timed = (): { start: number; snap: number | null } => {
    const raw = toFrameGrid(xToTime(input.x, viewport), input.frame);
    if (!input.snapping) return { start: raw, snap: null };
    const { delta, target } = snap(
      [raw, raw + payload.duration],
      snapTargets(tracks, input.playhead),
      SNAP_PIXELS * viewport.flicksPerPixel,
    );
    return { start: Math.max(0, raw + delta), snap: target };
  };

  const track = "track" in row ? tracks[row.track] : undefined;
  if (track && "track" in row && accepts(track, payload)) {
    if (track.id === input.mainTrack && input.magnetic) {
      const start = nearestBoundary(track.clips, xToTime(input.x, viewport));
      return {
        kind: "track",
        index: row.track,
        track: track.id,
        start,
        duration: payload.duration,
        valid: true,
        snap: null,
        insert: true,
      };
    }
    const { start, snap: line } = timed();
    const valid = !overlaps(track.clips, start, payload.duration);
    return {
      kind: "track",
      index: row.track,
      track: track.id,
      start,
      duration: payload.duration,
      valid,
      snap: line,
      insert: false,
    };
  }

  // A new track: near an edge, below the tracks, or over a track the clips cannot go on.
  const wanted = "newAt" in row ? row.newAt : row.track;
  const index = clampNewTrackIndex(wanted, mainIndex, tracks.length, payload);
  const { start, snap: line } = timed();
  return { kind: "newTrack", index, start, duration: payload.duration, valid: fits, snap: line };
}

/** The `addClips` placement for a preview; `null` when the drop would be rejected. */
export function dropPlacement(preview: DropPreview): Placement | null {
  if (!preview.valid) return null;
  return preview.kind === "track"
    ? { kind: "drop", track: { kind: "existing", track: preview.track }, start: preview.start }
    : { kind: "drop", track: { kind: "new", index: preview.index }, start: preview.start };
}
