/**
 * Clip selection rules (timeline design §6.1), as pure functions over the track list. Selection
 * is view state: never in the model, never in history.
 */
import type { ClipId } from "@/bindings";

import type { HitTrack } from "./hitTest";

/** `ids` plus every clip linked to one of them, in timeline order, without duplicates. */
export function withLinked(tracks: readonly HitTrack[], ids: readonly ClipId[]): ClipId[] {
  const wanted = new Set(ids);
  const links = new Set<string>();
  for (const track of tracks)
    for (const clip of track.clips) if (wanted.has(clip.id) && clip.link) links.add(clip.link);
  const result: ClipId[] = [];
  for (const track of tracks)
    for (const clip of track.clips)
      if (wanted.has(clip.id) || (clip.link && links.has(clip.link))) result.push(clip.id);
  return result;
}

export type ClickModifiers = { ctrl: boolean; shift: boolean };

/**
 * The selection after a click (pointer up without a drag) on `clip`: plain selects its link group
 * only, Shift adds the group, Ctrl toggles it.
 */
export function clickSelection(
  tracks: readonly HitTrack[],
  selected: readonly ClipId[],
  clip: ClipId,
  { ctrl, shift }: ClickModifiers,
): ClipId[] {
  const group = withLinked(tracks, [clip]);
  if (ctrl) {
    if (selected.includes(clip)) {
      const removed = new Set(group);
      return selected.filter((id) => !removed.has(id));
    }
    return withLinked(tracks, [...selected, ...group]);
  }
  if (shift) return withLinked(tracks, [...selected, ...group]);
  return group;
}

/**
 * The selection when a press on `clip` starts: a clip already selected keeps the selection (so a
 * group can be dragged); otherwise its group replaces the selection, or joins it with Ctrl/Shift.
 */
export function pressSelection(
  tracks: readonly HitTrack[],
  selected: readonly ClipId[],
  clip: ClipId,
  { ctrl, shift }: ClickModifiers,
): ClipId[] {
  if (selected.includes(clip)) return [...selected];
  const group = withLinked(tracks, [clip]);
  return ctrl || shift ? withLinked(tracks, [...selected, ...group]) : group;
}

/** Every clip on an unlocked track (Ctrl+A). */
export function allClips(tracks: readonly HitTrack[]): ClipId[] {
  return tracks.flatMap((track) => (track.locked ? [] : track.clips.map((clip) => clip.id)));
}

/** `selected` without the ids that no longer exist; the same array when nothing was dropped. */
export function pruneSelection(tracks: readonly HitTrack[], selected: readonly ClipId[]): readonly ClipId[] {
  if (selected.length === 0) return selected;
  const existing = new Set(tracks.flatMap((track) => track.clips.map((clip) => clip.id)));
  const kept = selected.filter((id) => existing.has(id));
  return kept.length === selected.length ? selected : kept;
}
