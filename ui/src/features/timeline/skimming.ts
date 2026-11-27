/**
 * Hover skimming on the timeline (F1.55, playback.md §2, §4.2): while paused, the preview shows the
 * frame under the pointer without moving the playhead. The engine honours `setSkim` only while
 * paused and clears it itself on play, a target change and a new project.
 *
 * At most one `setSkim` per animation frame, always with the latest time; `null` when the pointer
 * leaves, a drag starts, playback starts or skimming is switched off, so the preview shows the
 * playhead again. Nothing is sent while skimming is not allowed (`canSkim`).
 */
import { commands, type Time } from "@/bindings";
import { logTransportError, playheadStore } from "@/stores/playheadStore";
import { useUiStore } from "@/stores/uiStore";

export type SkimmerDeps = {
  send: (time: Time | null) => void;
  /** Skimming is on, the transport is paused and the timeline (not a media item) plays. */
  canSkim: () => boolean;
  requestFrame?: (callback: () => void) => number;
  cancelFrame?: (handle: number) => void;
};

export type Skimmer = {
  /** The pointer hovers over `time` (lanes or ruler, no button held). */
  hover: (time: Time) => void;
  /** The pointer left, a drag started or skimming stopped: show the playhead again. */
  leave: () => void;
  /** The engine cleared the skim itself (playback started): forget it without sending. */
  reset: () => void;
};

export function createSkimmer({
  send,
  canSkim,
  requestFrame = (callback) => requestAnimationFrame(callback),
  cancelFrame = (handle) => cancelAnimationFrame(handle),
}: SkimmerDeps): Skimmer {
  let pending: Time | null = null;
  let frame: number | null = null;
  /** The time the engine shows, `null` when it shows the playhead. */
  let shown: Time | null = null;

  const cancel = () => {
    if (frame !== null) cancelFrame(frame);
    frame = null;
    pending = null;
  };

  const leave = () => {
    cancel();
    if (shown === null) return;
    shown = null;
    send(null);
  };

  const flush = () => {
    frame = null;
    const time = pending;
    pending = null;
    if (time === null) return;
    if (!canSkim()) {
      leave();
      return;
    }
    if (time === shown) return;
    shown = time;
    send(time);
  };

  return {
    hover: (time) => {
      if (!canSkim()) {
        leave();
        return;
      }
      pending = Math.max(0, Math.round(time));
      frame ??= requestFrame(flush);
    },
    leave,
    reset: () => {
      cancel();
      shown = null;
    },
  };
}

/** The app's skimmer: the stores decide when skimming applies, `setSkim` carries it. */
export function createTimelineSkimmer(): Skimmer {
  return createSkimmer({
    send: (time) => void commands.setSkim(time).catch(logTransportError("setSkim")),
    canSkim: () => {
      const transport = playheadStore.getState();
      return useUiStore.getState().skimming && transport.state === "paused" && transport.source === null;
    },
  });
}
