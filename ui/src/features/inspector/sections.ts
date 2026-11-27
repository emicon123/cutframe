/**
 * The selection-driven inspector (clip-speed-audio design §6): which sections show for the
 * selected clips and which clips each one edits. Pure; the registry with the section components
 * is `registry.ts`.
 */
import type { ComponentType } from "react";

import type {
  ClipId,
  ClipView_Serialize,
  MediaView_Serialize,
  ProjectView_Serialize,
  TrackView_Serialize,
} from "@/bindings";
import { isRetimableKind } from "@/features/timeline/retime";

/** A selected clip with its track and media (`null` for colour and text clips). */
export type InspectorItem = {
  clip: ClipView_Serialize;
  track: TrackView_Serialize;
  media: MediaView_Serialize | null;
};

/** The selected clips in timeline order (top track first), with their tracks and media. */
export function inspectorItems(
  project: Pick<ProjectView_Serialize, "tracks" | "media">,
  selected: readonly ClipId[],
): InspectorItem[] {
  if (selected.length === 0) return [];
  const wanted = new Set(selected);
  const media = new Map(project.media.map((item) => [item.id, item]));
  return project.tracks.flatMap((track) =>
    track.clips
      .filter((clip) => wanted.has(clip.id))
      .map((clip) => ({
        clip,
        track,
        media:
          clip.source.kind === "media" || clip.source.kind === "freeze"
            ? (media.get(clip.source.media) ?? null)
            : null,
      })),
  );
}

export type SectionProps = {
  /** The selected clips this section edits (at least one). */
  targets: readonly InspectorItem[];
  project: ProjectView_Serialize;
};

/**
 * One inspector section. Register it once in `SECTIONS` (`registry.ts`); the shell shows it for
 * every selection it applies to.
 */
export type InspectorSection = {
  id: string;
  title: string;
  /** Whether the section edits this clip. */
  applies: (item: InspectorItem) => boolean;
  /** Only for exactly one clip it applies to (its command takes one clip, e.g. `trimClip`). */
  single?: boolean;
  Component: ComponentType<SectionProps>;
};

/** The clips that play sound: volume and fades (Audio section). */
export const hasAudio = (item: InspectorItem): boolean => item.clip.audio != null;

/** Video and audio media clips: speed, preserve pitch and reverse (Speed section). */
export const isRetimableItem = (item: InspectorItem): boolean =>
  item.clip.source.kind === "media" && isRetimableKind(item.media?.kind);

/** Images, colour cards and freeze frames: a length typed by hand (Duration section). */
export const hasStillDuration = (item: InspectorItem): boolean => {
  const { kind } = item.clip.source;
  return kind === "color" || kind === "freeze" || (kind === "media" && item.media?.kind === "image");
};

export type VisibleSection = { section: InspectorSection; targets: InspectorItem[] };

/**
 * The sections the selection has in common, each with the clips it edits. A link group counts as
 * one selected item (clicking a clip selects its group): a section shows when every selected
 * group has a clip it applies to, so a video with its detached audio still shows Audio and
 * Speed, while a video plus a colour card shows only what both have.
 */
export function visibleSections(
  sections: readonly InspectorSection[],
  items: readonly InspectorItem[],
): VisibleSection[] {
  if (items.length === 0) return [];
  const groups = new Map<string, InspectorItem[]>();
  for (const item of items) {
    const key = item.clip.link ?? item.clip.id;
    const group = groups.get(key);
    if (group) group.push(item);
    else groups.set(key, [item]);
  }
  return sections.flatMap((section) => {
    const targets = items.filter(section.applies);
    const common = [...groups.values()].every((group) => group.some(section.applies));
    if (!common || targets.length === 0 || (section.single && targets.length !== 1)) return [];
    return [{ section, targets }];
  });
}

/** A value of the edited clips: the first one's, and whether the others differ from it. */
export type Shared<T> = { value: T; mixed: boolean };

export function shared<T>(
  items: readonly InspectorItem[],
  get: (item: InspectorItem) => T,
  fallback: T,
): Shared<T> {
  const [first] = items;
  if (!first) return { value: fallback, mixed: false };
  const value = get(first);
  return { value, mixed: items.some((item) => get(item) !== value) };
}

/** Edits of clips on a locked track fail (`trackLocked`), so their controls are disabled. */
export function anyLocked(items: readonly InspectorItem[]): boolean {
  return items.some((item) => item.track.locked);
}
