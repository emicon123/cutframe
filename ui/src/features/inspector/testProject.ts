/** Project fixtures for the inspector tests. */
import type {
  AudioProps_Serialize,
  ClipView_Serialize,
  MediaKind,
  MediaView_Serialize,
  TrackKind,
  TrackView_Serialize,
} from "@/bindings";
import { FLICKS_PER_SECOND } from "@/lib/time";
import type { ProjectState } from "@/stores/projectStore";

export const S = FLICKS_PER_SECOND;

export function audio(overrides: Partial<AudioProps_Serialize> = {}): AudioProps_Serialize {
  return { volume: { value: 1 }, fadeIn: 0, fadeOut: 0, preservePitch: true, ...overrides };
}

/** A 2 s clip of video `v` at 1× with sound, unless overridden. */
export function clip(id: string, overrides: Partial<ClipView_Serialize> = {}): ClipView_Serialize {
  return {
    id,
    name: id,
    start: 0,
    duration: 2 * S,
    source: { kind: "media", media: "v", sourceIn: 0, speed: 100, reverse: false },
    link: null,
    visual: null,
    audio: audio(),
    sourceDuration: 10 * S,
    sourceEnd: 2 * S,
    ...overrides,
  };
}

export function track(
  id: string,
  kind: TrackKind,
  clips: ClipView_Serialize[],
  locked = false,
): TrackView_Serialize {
  return { id, kind, name: id, muted: false, hidden: false, locked, clips };
}

export function media(id: string, kind: MediaKind): MediaView_Serialize {
  return {
    id,
    kind,
    fileName: `${id}.file`,
    path: `C:\\media\\${id}`,
    duration: kind === "image" ? null : 10 * S,
    displayWidth: null,
    displayHeight: null,
    frameRate: null,
    videoCodec: null,
    hdr: null,
    audio: null,
    status: "online",
    usedBy: 1,
    previewVersion: "1",
    filmstrip: null,
  };
}

export const MEDIA = [media("v", "video"), media("a", "audio"), media("img", "image")];

export function projectState(tracks: TrackView_Serialize[], revision = 1): ProjectState {
  return {
    revision,
    project: {
      id: "p",
      name: "Trip",
      settings: { width: 1920, height: 1080, frameRate: { num: 30, den: 1 }, sampleRate: 48000 },
      duration: 10 * S,
      background: { kind: "color", color: "#000000ff" },
      mainTrack: tracks[0]?.id ?? "main",
      magnetic: true,
      tracks,
      media: MEDIA,
    },
    history: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null, entries: [], current: 0 },
    filePath: null,
    dirty: false,
  };
}
