import { commands, events } from "@/bindings";
import { applyAppInfoChange } from "@/features/editor/gpuNotice";
import { fileActions } from "@/features/project/fileActions";
import { recovery } from "@/features/project/recovery";
import { logError } from "@/ipc/errors";
import { useAppStore } from "@/stores/appStore";
import { applyTick } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

/**
 * Keeps the project store in sync with Rust: subscribes to `projectChanged` (and `quitRequested`,
 * answered with the unsaved-changes prompt; `openFileRequested`, a project double-clicked while
 * the app runs; `appInfoChanged`, a GPU change at runtime) first, cancels any stale gesture, then
 * fetches the current state, so no change is missed in between. Then it opens the project file the
 * app was launched with, if any (it shows the editor; otherwise the home screen stays), and only
 * after that offers crash recovery over either view. The playhead mirrors the engine's transport
 * the same way: `playbackTick` first, then `getPlayback` once (playback.md §8). Returns a cleanup
 * function.
 */
export function startProjectSync(): () => void {
  let disposed = false;
  /** A tick is newer than the `getPlayback` snapshot taken before it. */
  let ticked = false;
  const unlisteners: (() => void)[] = [];
  const { apply } = useProjectStore.getState();
  const { setInfo, reportError } = useAppStore.getState();

  void Promise.all([
    events.projectChanged.listen((event) => apply(event.payload)),
    events.quitRequested.listen(() => void fileActions.requestClose()),
    events.openFileRequested.listen((event) => void fileActions.openPath(event.payload.path)),
    events.appInfoChanged.listen((event) => applyAppInfoChange(event.payload)),
    events.playbackTick.listen((event) => {
      ticked = true;
      applyTick(event.payload);
    }),
  ])
    .then(async (stops) => {
      if (disposed) {
        for (const stop of stops) stop();
        return;
      }
      unlisteners.push(...stops);
      // A gesture left open by a previous webview (reload, crash) would make every edit fail with
      // GestureInProgress (ADR-0006). Cancelling is a no-op otherwise; a failure must not block
      // the initial state, so it is only logged.
      await commands.cancelGesture().catch(logError);
      const [project, info, playback] = await Promise.all([
        commands.getProject(),
        commands.getAppInfo(),
        // The playhead stays where it is if this fails; the next tick brings the state.
        commands.getPlayback().catch((error: unknown) => {
          logError(error);
          return null;
        }),
      ]);
      apply(project);
      setInfo(info);
      if (playback !== null && !ticked) applyTick(playback);
      // After the first state, so the unsaved checks see the real document. The launch path
      // first (design `platform-integration.md` §8): the recovery prompt still appears after it.
      const launchPath = await commands.takeLaunchPath().catch((error: unknown) => {
        logError(error);
        return null;
      });
      if (launchPath !== null) await fileActions.openPath(launchPath);
      useUiStore.getState().setStarted();
      void recovery.load();
    })
    .catch(reportError)
    // A failed startup still shows a screen (the home screen, with the error).
    .finally(() => useUiStore.getState().setStarted());

  return () => {
    disposed = true;
    for (const stop of unlisteners) stop();
  };
}
