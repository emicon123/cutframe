/**
 * Speed, reverse and freeze frames on the timeline (clip-speed-audio design §2, §6): which clips
 * can be retimed, the timeline badges, and the targets of the freeze-frame and reverse actions.
 * Pure functions over the track list; Rust validates again and stays the source of truth.
 */
import type { ClipId, ClipSource, MediaId, MediaKind, Speed, TrackKind } from "@/bindings";
import { t } from "@/i18n";

/** `Speed` is a percent of normal: 100 = 1×, within 10 ..= 10,000 (0.1× ..= 100×). */
export const NORMAL_SPEED: Speed = 100;
export const MIN_SPEED: Speed = 10;
export const MAX_SPEED: Speed = 10_000;
/** Voices faster than this are silent in preview and export (design §5.3, `MAX_SOUNDING_SPEED`). */
export const MAX_SOUNDING_SPEED: Speed = 400;

/** The media kind of each media id, for the rules below. */
export type MediaKinds = ReadonlyMap<MediaId, MediaKind>;

export function mediaKinds(media: readonly { id: MediaId; kind: MediaKind }[]): MediaKinds {
  return new Map(media.map((item) => [item.id, item.kind]));
}

/**
 * Whether a clip can change speed and play reversed: video and audio media clips. Images, freeze
 * frames, colour and text clips cannot (`setClipSpeed` and `setClipReverse` skip them).
 */
export function isRetimable(source: ClipSource, kinds: MediaKinds): boolean {
  return source.kind === "media" && isRetimableKind(kinds.get(source.media));
}

/** Whether media of `kind` can be retimed: video and audio, not images. */
export function isRetimableKind(kind: MediaKind | undefined): boolean {
  return kind === "video" || kind === "audio";
}

/** A speed as a factor with at most two decimals and no trailing zeros: 200 → "2", 25 → "0.25". */
export function speedFactorText(speed: Speed): string {
  return String(Number((speed / 100).toFixed(2)));
}

const NO_BADGES: readonly string[] = [];
const speedBadges = new Map<Speed, string>();

function speedBadge(speed: Speed): string {
  let text = speedBadges.get(speed);
  if (text === undefined) {
    text = t.timeline.badges.speed(speedFactorText(speed));
    speedBadges.set(speed, text);
  }
  return text;
}

/**
 * The badges drawn on a clip: its speed when not 1× ("2×"), a reverse marker, a freeze-frame
 * marker. A plain clip gets one shared empty list, so the draw path allocates nothing for it.
 */
export function clipBadges(source: ClipSource): readonly string[] {
  if (source.kind === "freeze") return [t.timeline.badges.freeze];
  if (source.kind !== "media" || (source.speed === NORMAL_SPEED && !source.reverse)) return NO_BADGES;
  const badges: string[] = [];
  if (source.speed !== NORMAL_SPEED) badges.push(speedBadge(source.speed));
  if (source.reverse) badges.push(t.timeline.badges.reverse);
  return badges;
}

/** What the rules below need of a track. */
export type RetimeTrack = {
  kind: TrackKind;
  clips: readonly { id: ClipId; start: number; duration: number; source: ClipSource }[];
};

/**
 * The clip the freeze-frame action freezes (design §6): the selected clip when exactly one video
 * clip is selected and it is under `at`; otherwise `null`, so Rust picks the main-track clip
 * under `at`. A linked audio clip in the selection does not count.
 */
export function freezeTarget(
  tracks: readonly RetimeTrack[],
  kinds: MediaKinds,
  selected: readonly ClipId[],
  at: number,
): ClipId | null {
  if (selected.length === 0) return null;
  const wanted = new Set(selected);
  const videos = tracks.flatMap((track) =>
    track.kind === "video"
      ? track.clips.filter(
          (clip) =>
            wanted.has(clip.id) && clip.source.kind === "media" && kinds.get(clip.source.media) === "video",
        )
      : [],
  );
  const [only] = videos;
  if (videos.length !== 1 || !only) return null;
  return only.start <= at && at < only.start + only.duration ? only.id : null;
}

/**
 * What the reverse toggle does with `selected`: `null` when no selected clip can be reversed,
 * otherwise whether they all play reversed already (then the toggle turns reverse off).
 */
export function reverseState(
  tracks: readonly RetimeTrack[],
  kinds: MediaKinds,
  selected: readonly ClipId[],
): { reversed: boolean } | null {
  if (selected.length === 0) return null;
  const wanted = new Set(selected);
  let any = false;
  let all = true;
  for (const track of tracks)
    for (const clip of track.clips) {
      if (!wanted.has(clip.id) || !isRetimable(clip.source, kinds)) continue;
      any = true;
      if (clip.source.kind === "media" && !clip.source.reverse) all = false;
    }
  return any ? { reversed: all } : null;
}
