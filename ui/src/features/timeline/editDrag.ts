/**
 * The absolute values a clip drag sends to Rust (`moveClips`, `trimClip`), as pure functions of
 * the drag-start state and the pointer (timeline design §4.2, §4.3, §6.2). Everything is computed
 * from the snapshot taken when the drag started, never from the live draft, so a value depends
 * only on where the pointer is (ADR-0006: absolute values, no deltas). Rust stays the judge: it
 * rejects overlaps and clamps trims.
 */
import type { ClipId, Edge, TrackId, TrackKind, TrackTarget } from "@/bindings";

import { clampNewTrackIndex, rowAt } from "./drop";
import { trackAt, type Viewport } from "./geometry";
import type { HitTrack } from "./hitTest";
import { withLinked } from "./selection";
import { SNAP_PIXELS, snap, toFrameGrid } from "./snapping";

export type DragContext = {
  /** The tracks when the drag started. */
  tracks: readonly HitTrack[];
  mainTrack: TrackId;
  viewport: Viewport;
  playhead: number;
  /** Flicks per project frame: edits land on frames. */
  frame: number;
  /** The snapping preference, and Alt not held. */
  snapping: boolean;
};

/** Snap targets: time 0, the playhead and every clip edge except those of `excluded` clips. */
export function snapTargets(
  tracks: readonly HitTrack[],
  excluded: ReadonlySet<ClipId>,
  playhead: number,
): number[] {
  const targets = [0, playhead];
  for (const track of tracks)
    for (const clip of track.clips)
      if (!excluded.has(clip.id)) targets.push(clip.start, clip.start + clip.duration);
  return targets;
}

export type MovePlan = {
  /** The clips to send (the selection with the anchor in it). */
  clips: ClipId[];
  anchor: ClipId;
  anchorStart: number;
  /** Index of the anchor's track and its kind. */
  anchorTrack: number;
  kind: TrackKind;
  /** Every listed clip sits on the anchor's track or is linked to one there. */
  vertical: boolean;
  /** The dragged clips with their linked members: they are not snap targets. */
  group: ReadonlySet<ClipId>;
  /** Earliest start and latest end of `group` relative to the anchor's start. */
  before: number;
  after: number;
};

/** The move of `selection` grabbed at `anchor`; `null` if the anchor is not on any track. */
export function planMove(
  tracks: readonly HitTrack[],
  selection: readonly ClipId[],
  anchor: ClipId,
): MovePlan | null {
  const anchorTrack = tracks.findIndex((track) => track.clips.some((clip) => clip.id === anchor));
  const track = tracks[anchorTrack];
  const anchorClip = track?.clips.find((clip) => clip.id === anchor);
  if (!track || !anchorClip) return null;
  const clips = selection.includes(anchor) ? [...selection] : [...selection, anchor];
  const group = new Set(withLinked(tracks, clips));

  const listed = new Set(clips);
  const linksOnAnchorTrack = new Set(track.clips.flatMap((clip) => (clip.link ? [clip.link] : [])));
  let earliest = anchorClip.start;
  let latest = anchorClip.start + anchorClip.duration;
  let vertical = true;
  for (const other of tracks) {
    for (const clip of other.clips) {
      if (group.has(clip.id)) {
        earliest = Math.min(earliest, clip.start);
        latest = Math.max(latest, clip.start + clip.duration);
      }
      if (listed.has(clip.id) && other !== track && !(clip.link && linksOnAnchorTrack.has(clip.link)))
        vertical = false;
    }
  }
  return {
    clips,
    anchor,
    anchorStart: anchorClip.start,
    anchorTrack,
    kind: track.kind,
    vertical,
    group,
    before: anchorClip.start - earliest,
    after: latest - anchorClip.start,
  };
}

export type MoveValue = { start: number; track: TrackTarget };

export type MoveResult = {
  value: MoveValue;
  /** Time of the snap line, if the move snapped. */
  snap: number | null;
};

/** Where the anchor's clips go for the pointer at `y` (CSS px from the canvas top). */
function moveTrack(plan: MovePlan, ctx: DragContext, y: number): TrackTarget {
  const own = ctx.tracks[plan.anchorTrack];
  const stay: TrackTarget = { kind: "existing", track: own?.id ?? "" };
  if (!plan.vertical || trackAt(y, ctx.tracks.length) === plan.anchorTrack) return stay;
  const row = rowAt(y, ctx.tracks.length);
  if (!row) return stay;
  if ("track" in row) {
    const target = ctx.tracks[row.track];
    return target && target.kind === plan.kind && !target.locked
      ? { kind: "existing", track: target.id }
      : stay;
  }
  const mainIndex = Math.max(
    0,
    ctx.tracks.findIndex((track) => track.id === ctx.mainTrack),
  );
  const audio = plan.kind === "audio";
  const index = clampNewTrackIndex(row.newAt, mainIndex, ctx.tracks.length, {
    visual: !audio,
    audioOnly: audio,
    duration: 0,
  });
  return { kind: "new", index };
}

/**
 * The `moveClips` value for a pointer `dx` px right of the press and at `y`: the anchor's start on
 * the frame grid, snapped by the group's outer edges, and never moving the group before 0.
 */
export function moveValue(plan: MovePlan, ctx: DragContext, dx: number, y: number): MoveResult {
  const fpp = ctx.viewport.flicksPerPixel;
  const raw = toFrameGrid(plan.anchorStart + dx * fpp, ctx.frame);
  let start = raw;
  let line: number | null = null;
  if (ctx.snapping) {
    const { delta, target } = snap(
      [raw - plan.before, raw + plan.after],
      snapTargets(ctx.tracks, plan.group, ctx.playhead),
      SNAP_PIXELS * fpp,
    );
    start = raw + delta;
    line = target;
  }
  if (start - plan.before < 0) {
    start = plan.before;
    line = null;
  }
  return { value: { start, track: moveTrack(plan, ctx, y) }, snap: line };
}

export type TrimPlan = {
  clip: ClipId;
  edge: Edge;
  /** The edge's time when the drag started. */
  edgeTime: number;
  /** The clip and its linked members: they are not snap targets. */
  group: ReadonlySet<ClipId>;
};

export function planTrim(tracks: readonly HitTrack[], clipId: ClipId, edge: Edge): TrimPlan | null {
  for (const track of tracks) {
    const clip = track.clips.find((c) => c.id === clipId);
    if (!clip) continue;
    return {
      clip: clipId,
      edge,
      edgeTime: edge === "start" ? clip.start : clip.start + clip.duration,
      group: new Set(withLinked(tracks, [clipId])),
    };
  }
  return null;
}

export type TrimResult = { time: number; snap: number | null };

/**
 * The `trimClip` edge time for a pointer `dx` px right of the press, on the frame grid and
 * snapped. Rust clamps it to the source range, one frame and the neighbours.
 */
export function trimValue(plan: TrimPlan, ctx: DragContext, dx: number): TrimResult {
  const fpp = ctx.viewport.flicksPerPixel;
  const raw = toFrameGrid(plan.edgeTime + dx * fpp, ctx.frame);
  if (!ctx.snapping) return { time: raw, snap: null };
  const { delta, target } = snap([raw], snapTargets(ctx.tracks, plan.group, ctx.playhead), SNAP_PIXELS * fpp);
  return { time: raw + delta, snap: target };
}
