import type { FilmstripLayout } from "@/bindings";

/** Where a hover position lands in a filmstrip: the frame, its tile and its slot in that tile. */
export type SkimFrame = {
  frame: number;
  tile: number;
  /** 0-based position inside the tile. */
  slot: number;
  /** Frames in this tile (the last tile may hold fewer than `framesPerTile`). */
  framesInTile: number;
};

/**
 * Maps a horizontal position over the thumbnail (`fraction`, 0 = left edge, 1 = right edge) to a
 * filmstrip frame. Positions outside are clamped. `null` for an empty filmstrip.
 */
export function skimFrame(layout: FilmstripLayout, fraction: number): SkimFrame | null {
  const { frames, framesPerTile } = layout;
  if (frames <= 0 || framesPerTile <= 0) return null;
  const clamped = Number.isFinite(fraction) ? Math.min(1, Math.max(0, fraction)) : 0;
  const frame = Math.min(frames - 1, Math.floor(clamped * frames));
  const tile = Math.floor(frame / framesPerTile);
  const framesInTile = Math.min(framesPerTile, frames - tile * framesPerTile);
  return { frame, tile, slot: frame - tile * framesPerTile, framesInTile };
}

/**
 * CSS background size and position that show slot `slot` of a tile holding `framesInTile` frames
 * side by side, scaled to the element (which has the frame's aspect ratio).
 */
export function tileBackground({ slot, framesInTile }: Pick<SkimFrame, "slot" | "framesInTile">): {
  backgroundSize: string;
  backgroundPosition: string;
} {
  const x = framesInTile > 1 ? (slot / (framesInTile - 1)) * 100 : 0;
  return { backgroundSize: `${framesInTile * 100}% 100%`, backgroundPosition: `${x}% 0%` };
}
