/**
 * Transport actions (playback.md §2): play/pause, J/K/L shuttle and the loop toggle. The engine
 * owns the transport; the new state comes back as a `playbackTick` (`playheadStore`), so nothing
 * here changes the store. Seeks and frame steps live in `playheadStore` (they carry generations).
 */
import { commands, type ShuttleKey } from "@/bindings";
import { logTransportError, playheadStore } from "@/stores/playheadStore";

/** Space and the Play button: play at 1x from the playhead, or pause. */
export function togglePlayback(): void {
  commands.togglePlayback().catch(logTransportError("togglePlayback"));
}

/** J (`reverse`), K (`pause`), L (`forward`); repeated J or L presses speed up to 8x. */
export function shuttle(key: ShuttleKey): void {
  commands.shuttle(key).catch(logTransportError("shuttle"));
}

/** The loop button: loop the whole timeline on or off. */
export function toggleLoop(): void {
  commands.setLoop(!playheadStore.getState().looping).catch(logTransportError("setLoop"));
}
