import { createStore } from "zustand/vanilla";

import { DEFAULT_VIEWPORT, type Viewport } from "@/features/timeline/geometry";

/**
 * The timeline's scroll and zoom, and the width of its lanes. A vanilla store like
 * `playheadStore`: the canvas reads and subscribes to it directly, so scrolling and zooming never
 * re-render React; only the zoom slider subscribes to `flicksPerPixel`. UI state, not the model.
 */
export const timelineViewStore = createStore<{ viewport: Viewport; width: number }>()(() => ({
  viewport: DEFAULT_VIEWPORT,
  width: 0,
}));

export function setViewport(viewport: Viewport) {
  timelineViewStore.setState({ viewport });
}

export function setLaneWidth(width: number) {
  timelineViewStore.setState({ width });
}
