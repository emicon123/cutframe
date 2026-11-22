import type { MediaKind, MediaView_Serialize } from "@/bindings";

/**
 * How the media panel orders and narrows `ProjectState.media`. View state only: the model order
 * (import order) stays in Rust and nothing here changes the project.
 */
export type MediaSortKey = "imported" | "name" | "duration" | "kind";
export type SortDirection = "asc" | "desc";

export const MEDIA_SORT_KEYS: readonly MediaSortKey[] = ["imported", "name", "duration", "kind"];
export const MEDIA_KINDS: readonly MediaKind[] = ["video", "audio", "image"];

export type BrowseOptions = {
  sort: MediaSortKey;
  direction: SortDirection;
  /** Kinds to show; empty shows every kind. */
  kinds: readonly MediaKind[];
  /** Case-insensitive substring of the file name; blank shows everything. */
  query: string;
};

const KIND_ORDER: Record<MediaKind, number> = { video: 0, audio: 1, image: 2 };

const names = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

type Indexed = { item: MediaView_Serialize; index: number };

function compareBy(key: MediaSortKey): (a: Indexed, b: Indexed) => number {
  switch (key) {
    case "imported":
      return (a, b) => a.index - b.index;
    case "name":
      return (a, b) => names.compare(a.item.fileName, b.item.fileName);
    case "duration":
      // Images have no duration: they sort as the shortest.
      return (a, b) => (a.item.duration ?? -1) - (b.item.duration ?? -1);
    case "kind":
      return (a, b) => KIND_ORDER[a.item.kind] - KIND_ORDER[b.item.kind];
  }
}

/** True when `fileName` matches the search `query` (trimmed, case-insensitive substring). */
export function matchesQuery(fileName: string, query: string): boolean {
  const needle = query.trim().toLocaleLowerCase();
  return needle === "" || fileName.toLocaleLowerCase().includes(needle);
}

/**
 * The items the panel shows, in display order. Ties keep import order (also when descending), so
 * the order is stable while thumbnails fill in.
 */
export function browseMedia(
  media: readonly MediaView_Serialize[],
  { sort, direction, kinds, query }: BrowseOptions,
): MediaView_Serialize[] {
  const compare = compareBy(sort);
  const sign = direction === "asc" ? 1 : -1;
  return media
    .map((item, index) => ({ item, index }))
    .filter(
      ({ item }) => (kinds.length === 0 || kinds.includes(item.kind)) && matchesQuery(item.fileName, query),
    )
    .sort((a, b) => sign * compare(a, b) || a.index - b.index)
    .map(({ item }) => item);
}

/** Toggles `kind` in a kind filter, keeping the canonical kind order. */
export function toggleKind(kinds: readonly MediaKind[], kind: MediaKind): MediaKind[] {
  const next = kinds.includes(kind) ? kinds.filter((k) => k !== kind) : [...kinds, kind];
  return MEDIA_KINDS.filter((k) => next.includes(k));
}
