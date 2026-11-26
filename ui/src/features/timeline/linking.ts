/**
 * Which clip-level actions apply to a selection: the link toggle (timeline design §3.5, §4.12),
 * detach audio (§4.13) and replace media (§4.14). Pure functions over the track list; Rust
 * validates again and stays the source of truth, these only decide what the UI offers.
 */
import type { ClipId, ClipSource, MediaView_Serialize, TrackKind } from "@/bindings";

/** What these rules need of a clip. */
export type LinkClip = {
  id: ClipId;
  duration: number;
  link?: string | null;
  source: ClipSource;
  /** `null` once the clip's sound is detached, or for clips that never make sound. */
  audio?: unknown;
};

/** What these rules need of a track. */
export type LinkTrack = { id: string; kind: TrackKind; clips: readonly LinkClip[] };

/** The link toggle's next step for `selected`: unlink, link, or nothing to do. */
export type LinkToggle = "link" | "unlink" | null;

/**
 * Unlink when any selected clip is linked (the UI sends the whole selection, so the whole group
 * leaves). Link when at least two clips are selected, each on its own track. Otherwise nothing.
 */
export function linkToggle(tracks: readonly LinkTrack[], selected: readonly ClipId[]): LinkToggle {
  if (selected.length === 0) return null;
  const wanted = new Set(selected);
  const tracksUsed = new Set<string>();
  let count = 0;
  let sameTrack = false;
  for (const track of tracks)
    for (const clip of track.clips) {
      if (!wanted.has(clip.id)) continue;
      if (clip.link) return "unlink";
      count++;
      if (tracksUsed.has(track.id)) sameTrack = true;
      tracksUsed.add(track.id);
    }
  return count >= 2 && !sameTrack ? "link" : null;
}

/** Whether a clip on a `kind` track still plays its own sound, so detach audio applies to it. */
export function hasDetachableAudio(kind: TrackKind, clip: LinkClip): boolean {
  return kind === "video" && clip.source.kind === "media" && clip.audio != null;
}

/** Whether detach audio has anything to do for `selected`. */
export function canDetachAudio(tracks: readonly LinkTrack[], selected: readonly ClipId[]): boolean {
  if (selected.length === 0) return false;
  const wanted = new Set(selected);
  return tracks.some((track) =>
    track.clips.some((clip) => wanted.has(clip.id) && hasDetachableAudio(track.kind, clip)),
  );
}

/** The clip `id` and its track, or `null`. */
export function findClip<T extends LinkTrack>(
  tracks: readonly T[],
  id: ClipId,
): { track: T; clip: T["clips"][number] } | null {
  for (const track of tracks) {
    const clip = track.clips.find((c) => c.id === id);
    if (clip) return { track, clip };
  }
  return null;
}

type ReplaceMedia = Pick<MediaView_Serialize, "id" | "kind" | "duration" | "audio">;

/**
 * Whether `media` can replace the source of `clip` on a `kind` track (the rules of
 * `replaceClipMedia`): only media clips; video tracks take video and images, audio tracks take
 * media with sound, text tracks take none; timed media must be at least as long as the clip.
 * The clip's current media is left out.
 */
export function canReplaceWith(kind: TrackKind, clip: LinkClip, media: ReplaceMedia): boolean {
  if (clip.source.kind !== "media" || clip.source.media === media.id) return false;
  const fits =
    kind === "video"
      ? media.kind === "video" || media.kind === "image"
      : kind === "audio" && media.audio !== null;
  return fits && (media.duration === null || media.duration >= clip.duration);
}

/** Whether replace media applies to `clip` at all (a media clip on a video or audio track). */
export function canReplaceMedia(kind: TrackKind, clip: LinkClip): boolean {
  return clip.source.kind === "media" && kind !== "text";
}

/** The media items that can replace the source of `clip`, in media-panel order. */
export function replacementMedia<M extends ReplaceMedia>(
  kind: TrackKind,
  clip: LinkClip,
  media: readonly M[],
): M[] {
  return media.filter((item) => canReplaceWith(kind, clip, item));
}
