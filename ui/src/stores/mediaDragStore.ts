import { createStore } from "zustand/vanilla";

import type { MediaId } from "@/bindings";

/**
 * A drag of media items from the media panel, done with pointer events: WebView2 delivers no
 * HTML5 drag-and-drop inside the page while native file drop is on (media design §2). A vanilla
 * store, so the timeline canvas follows the pointer without React renders.
 */
export type MediaDrag = {
  media: MediaId[];
  /** Client (viewport) CSS pixels. */
  x: number;
  y: number;
  /** Alt held: no snapping. */
  alt: boolean;
};

export const mediaDragStore = createStore<{ drag: MediaDrag | null }>()(() => ({ drag: null }));

/** Takes a finished drag; returns true when it consumed it (the pointer was over the zone). */
export type DropZone = (drag: MediaDrag) => boolean;

const zones = new Set<DropZone>();

export function registerDropZone(zone: DropZone): () => void {
  zones.add(zone);
  return () => zones.delete(zone);
}

export function startMediaDrag(media: MediaId[], x: number, y: number, alt: boolean): void {
  mediaDragStore.setState({ drag: { media, x, y, alt } });
}

export function moveMediaDrag(x: number, y: number, alt: boolean): void {
  const { drag } = mediaDragStore.getState();
  if (drag) mediaDragStore.setState({ drag: { ...drag, x, y, alt } });
}

/** Ends the drag (pointer up) and hands it to the first zone that takes it. */
export function finishMediaDrag(): boolean {
  const { drag } = mediaDragStore.getState();
  if (!drag) return false;
  mediaDragStore.setState({ drag: null });
  for (const zone of zones) if (zone(drag)) return true;
  return false;
}

/** Escape, blur or a lost pointer: drop nothing. */
export function cancelMediaDrag(): void {
  if (mediaDragStore.getState().drag) mediaDragStore.setState({ drag: null });
}
