import { createStore } from "zustand/vanilla";

/**
 * Playhead position in flicks. A vanilla store on purpose: the timeline canvas subscribes to it
 * directly, so moving the playhead never re-renders React components.
 *
 * Stage 0 keeps the playhead in the UI. With playback it becomes Rust-owned transport state fed
 * by `playbackTick` events.
 */
export const playheadStore = createStore<{ time: number }>()(() => ({ time: 0 }));

export function setPlayhead(time: number) {
  playheadStore.setState({ time: Math.max(0, Math.round(time)) });
}
