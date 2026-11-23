import { createStore } from "zustand/vanilla";

import type { ClipId } from "@/bindings";
import { type GapSelection, locateGap } from "@/features/timeline/gap";
import { allClips, pruneSelection } from "@/features/timeline/selection";
import { useProjectStore } from "@/stores/projectStore";

/**
 * Selected clips, or one selected gap (timeline design §6.1). UI-only view state: not in the
 * model, not in history, not saved. A vanilla store, like `playheadStore`, so the timeline canvas
 * redraws without React renders. Every command takes the clip ids (or the gap) explicitly.
 * Selecting clips clears the gap and the other way round.
 */
export const selectionStore = createStore<{ clips: readonly ClipId[]; gap: GapSelection | null }>()(() => ({
  clips: [],
  gap: null,
}));

export function setSelection(clips: readonly ClipId[]) {
  const { clips: current, gap } = selectionStore.getState();
  if (gap === null && current.length === clips.length && current.every((id, i) => id === clips[i])) return;
  selectionStore.setState({ clips, gap: null });
}

/** Selects the gap that contains `gap.at` on `gap.track` (and no clips). */
export function setGapSelection(gap: GapSelection) {
  const current = selectionStore.getState().gap;
  if (current?.track === gap.track && current.at === gap.at) return;
  selectionStore.setState({ clips: [], gap });
}

export function clearSelection() {
  setSelection([]);
}

/** Ctrl+A: every clip on an unlocked track. */
export function selectAll() {
  setSelection(allClips(useProjectStore.getState().state?.project.tracks ?? []));
}

// After every `project-changed`, drop ids that no longer exist, and the gap once it is closed or
// filled, so undo, delete or opening another project never leaves ghost selections.
useProjectStore.subscribe((s) => {
  const { clips, gap } = selectionStore.getState();
  if (clips.length === 0 && gap === null) return;
  const tracks = s.state?.project.tracks ?? [];
  const kept = pruneSelection(tracks, clips);
  const keptGap = gap && locateGap(tracks, gap) ? gap : null;
  if (kept !== clips || keptGap !== gap) selectionStore.setState({ clips: kept, gap: keptGap });
});
