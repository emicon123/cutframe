import { commands, type MediaId, type Placement } from "@/bindings";
import { run } from "@/stores/appStore";
import { playheadStore } from "@/stores/playheadStore";

/** Puts media on the timeline as one undo step (`addClips`). Resolves the new clip ids. */
export async function addMediaClips(media: readonly MediaId[], placement: Placement) {
  if (media.length === 0) return undefined;
  return run(
    commands.addClips(
      media.map((id) => ({ kind: "media", media: id })),
      placement,
    ),
  );
}

/** The `+` button: the end of the main track (audio-only media: the first audio track). */
export const appendMedia = (media: readonly MediaId[]) => addMediaClips(media, { kind: "append" });

/** `+` with Alt: into the main track at the clip boundary nearest the playhead. */
export const insertMediaAtPlayhead = (media: readonly MediaId[]) =>
  addMediaClips(media, { kind: "insert", at: playheadStore.getState().time });
