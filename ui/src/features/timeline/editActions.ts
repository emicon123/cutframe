/**
 * Split, delete, trim to playhead and clipboard actions (timeline design §4.4–4.8, §6.3). Each one
 * is a single command call with the selection and the playhead passed explicitly; Rust does the
 * edit and pushes the new state through `project-changed`. The deciding logic is pure and tested.
 */
import { type ClipId, commands, type MediaId, type TrackFlag, type TrackId } from "@/bindings";
import { run } from "@/stores/appStore";
import { playheadStore, seekTo } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";
import { clearSelection, selectionStore, setSelection } from "@/stores/selectionStore";

import type { GapSelection } from "./gap";
import type { HitTrack } from "./hitTest";
import { linkToggle } from "./linking";
import { freezeTarget, mediaKinds, reverseState } from "./retime";

export type DeleteRequest =
  { kind: "gap"; track: TrackId; at: number } | { kind: "clips"; clips: ClipId[]; ripple: boolean };

/** What Delete (or Shift+Delete with `ripple`) removes: the selected gap, else the selected clips. */
export function deleteRequest(
  selection: { clips: readonly ClipId[]; gap: GapSelection | null },
  ripple: boolean,
): DeleteRequest | null {
  if (selection.gap) return { kind: "gap", ...selection.gap };
  if (selection.clips.length > 0) return { kind: "clips", clips: [...selection.clips], ripple };
  return null;
}

/**
 * Where the playhead goes after Q (delete left of the playhead): on the magnetic main track the
 * trimmed clip keeps its start and the content that was at `at` moves there, so the playhead
 * follows it. `null` when it stays (not magnetic, or no main-track clip is trimmed). The trimmed
 * clip is a selected main-track clip under `at`, or with no selection the main-track clip there.
 */
export function playheadAfterTrimLeft(
  project: { tracks: readonly HitTrack[]; mainTrack: TrackId; magnetic: boolean },
  selected: readonly ClipId[],
  at: number,
): number | null {
  if (!project.magnetic) return null;
  const main = project.tracks.find((track) => track.id === project.mainTrack);
  const clip = main?.clips.find(
    (c) => (selected.length === 0 || selected.includes(c.id)) && c.start < at && at < c.start + c.duration,
  );
  return clip ? clip.start : null;
}

const selectedClips = () => [...selectionStore.getState().clips];
const playhead = () => playheadStore.getState().time;

/** Selects the clips a command created; nothing on failure (`run` already reported it). */
const selectCreated = (ids: ClipId[] | undefined) => {
  if (ids) setSelection(ids);
};

/** Ctrl+B: the selected clips, or every clip under the playhead; selects the right halves. */
export async function splitAtPlayhead() {
  selectCreated(await run(commands.splitClips(selectedClips(), playhead())));
}

/** Delete / Shift+Delete: the selected gap, or the selected clips (with or without ripple). */
export async function deleteSelection(ripple: boolean) {
  const request = deleteRequest(selectionStore.getState(), ripple);
  if (!request) return;
  if (request.kind === "gap") {
    const done = await run(commands.deleteGap(request.track, request.at));
    if (done !== undefined) clearSelection();
  } else {
    await run(commands.deleteClips(request.clips, request.ripple));
  }
}

/** Q (`left`) / W (`right`): delete the part of the selected clips (or the main clip) past the playhead. */
export async function trimToPlayhead(side: "left" | "right") {
  const clips = selectedClips();
  const at = playhead();
  const project = useProjectStore.getState().state?.project;
  const target = side === "left" && project ? playheadAfterTrimLeft(project, clips, at) : null;
  const done = await run(commands.trimToPlayhead(clips, at, side));
  if (done !== undefined && target !== null) seekTo(target, "exact");
}

export async function copySelection() {
  await run(commands.copyClips(selectedClips()));
}

export async function cutSelection() {
  await run(commands.cutClips(selectedClips()));
}

/** Ctrl+V at the playhead; selects the pasted clips. */
export async function pasteAtPlayhead() {
  selectCreated(await run(commands.pasteClips(playhead())));
}

/** Ctrl+D: copies of the selection right after it; selects them. */
export async function duplicateSelection() {
  selectCreated(await run(commands.duplicateClips(selectedClips())));
}

export async function toggleMagnetic() {
  const magnetic = useProjectStore.getState().state?.project.magnetic;
  if (magnetic === undefined) return;
  await run(commands.setMagnetic(!magnetic));
}

/** The link toggle (F1.49): unlinks the selection when it holds a linked clip, else links it. */
export async function toggleLinkSelection() {
  const clips = selectedClips();
  const tracks = useProjectStore.getState().state?.project.tracks ?? [];
  const next = linkToggle(tracks, clips);
  if (next === "link") await run(commands.linkClips(clips));
  else if (next === "unlink") await run(commands.unlinkClips(clips));
}

/** F1.50: moves the sound of the selected video clips to linked audio clips; selects those. */
export async function detachSelectedAudio() {
  selectCreated(await run(commands.detachAudio(selectedClips())));
}

/**
 * F1.82 (Shift+F): inserts a still of the frame under the playhead and selects it. Freezes the
 * selected video clip when exactly one is selected under the playhead, else Rust picks the
 * main-track clip there.
 */
export async function freezeAtPlayhead() {
  const project = useProjectStore.getState().state?.project;
  const at = playhead();
  const clip = project ? freezeTarget(project.tracks, mediaKinds(project.media), selectedClips(), at) : null;
  const created = await run(commands.insertFreezeFrame(clip, at));
  if (created !== undefined) setSelection([created]);
}

/** F1.81: plays the selected clips (and their linked members) backwards, or forwards again. */
export async function toggleReverseSelection() {
  const clips = selectedClips();
  const project = useProjectStore.getState().state?.project;
  const state = project ? reverseState(project.tracks, mediaKinds(project.media), clips) : null;
  if (state) await run(commands.setClipReverse(clips, !state.reversed));
}

/** F1.51: points `clip` at `media`, keeping its place, length and properties. */
export async function replaceClipMedia(clip: ClipId, media: MediaId): Promise<boolean> {
  return (await run(commands.replaceClipMedia(clip, media))) !== undefined;
}

/** F1.52: mute, hide or lock a track. */
export async function setTrackFlag(track: TrackId, flag: TrackFlag, value: boolean) {
  await run(commands.setTrackFlag(track, flag, value));
}

/** F1.53: moves a track to its final `index` (Rust clamps it into the track's section). */
export async function moveTrack(track: TrackId, index: number) {
  await run(commands.moveTrack(track, index));
}

export async function removeTrack(track: TrackId) {
  await run(commands.removeTrack(track));
}
