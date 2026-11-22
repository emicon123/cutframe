import { MEDIA_EXTENSIONS } from "@/bindings";

const EXTENSIONS: ReadonlySet<string> = new Set(MEDIA_EXTENSIONS);

/** The last segment of a Windows or POSIX path. */
export function fileNameOf(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed.slice(Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/")) + 1) || path;
}

/**
 * Whether a drag is worth highlighting the panel for: some path has a media extension or none at
 * all (most likely a folder). Only a hint; Rust decides what is imported and reports the rest.
 */
export function isLikelyImportable(paths: readonly string[]): boolean {
  return paths.some((path) => {
    const name = fileNameOf(path);
    const dot = name.lastIndexOf(".");
    return dot <= 0 || EXTENSIONS.has(name.slice(dot + 1).toLowerCase());
  });
}

export type Point = { x: number; y: number };
export type Box = { left: number; top: number; right: number; bottom: number };

/** Tauri reports drag positions in physical pixels; the DOM uses CSS pixels. */
export function toCssPoint(physical: Point, devicePixelRatio: number): Point {
  const ratio = devicePixelRatio > 0 ? devicePixelRatio : 1;
  return { x: physical.x / ratio, y: physical.y / ratio };
}

export function containsPoint(box: Box, point: Point): boolean {
  return point.x >= box.left && point.x < box.right && point.y >= box.top && point.y < box.bottom;
}
