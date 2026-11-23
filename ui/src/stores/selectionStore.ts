import { createStore } from "zustand/vanilla";

import type { ClipId } from "@/bindings";
import { allClips, pruneSelection } from "@/features/timeline/selection";
import { useProjectStore } from "@/stores/projectStore";

/**
 * Selected clips (timeline design §6.1). UI-only view state: not in the model, not in history, not
 * saved. A vanilla store, like `playheadStore`, so the timeline canvas redraws without React
 * renders. Every command takes the clip ids explicitly.
 */
export const selectionStore = createStore<{ clips: readonly ClipId[] }>()(() => ({ clips: [] }));

export function setSelection(clips: readonly ClipId[]) {
  const current = selectionStore.getState().clips;
  if (current.length === clips.length && current.every((id, i) => id === clips[i])) return;
  selectionStore.setState({ clips });
}

export function clearSelection() {
  setSelection([]);
}

/** Ctrl+A: every clip on an unlocked track. */
export function selectAll() {
  setSelection(allClips(useProjectStore.getState().state?.project.tracks ?? []));
}

// After every `project-changed`, drop ids that no longer exist, so undo, delete or opening another
// project never leaves ghost selections.
useProjectStore.subscribe((s) => {
  const { clips } = selectionStore.getState();
  if (clips.length === 0) return;
  const kept = pruneSelection(s.state?.project.tracks ?? [], clips);
  if (kept !== clips) selectionStore.setState({ clips: kept });
});
