import { commands, events } from "@/bindings";
import { applyAppInfoChange } from "@/features/editor/gpuNotice";
import { fileActions } from "@/features/project/fileActions";
import { recovery } from "@/features/project/recovery";
import { logError } from "@/ipc/errors";
import { useAppStore } from "@/stores/appStore";
import { reseekOnDurationChange } from "@/stores/playheadStore";
import { useProjectStore } from "@/stores/projectStore";

/**
 * Keeps the project store in sync with Rust: subscribes to `projectChanged` (and `quitRequested`,
 * answered with the unsaved-changes prompt; `openFileRequested`, a project double-clicked while
 * the app runs; `appInfoChanged`, a GPU change at runtime) first, cancels any stale gesture, then
 * fetches the current state, so no change is missed in between. Then it opens the project file the
 * app was launched with, if any, and only after that offers crash recovery. It also re-sends the
 * playhead to the engine on the first state and when a duration change may have clamped it.
 * Returns a cleanup function.
 */
export function startProjectSync(): () => void {
  let disposed = false;
  const unlisteners: (() => void)[] = [
    useProjectStore.subscribe((s, prev) =>
      reseekOnDurationChange(prev.state?.project.duration, s.state?.project.duration),
    ),
  ];
  const { apply } = useProjectStore.getState();
  const { setInfo, reportError } = useAppStore.getState();

  void Promise.all([
    events.projectChanged.listen((event) => apply(event.payload)),
    events.quitRequested.listen(() => void fileActions.requestClose()),
    events.openFileRequested.listen((event) => void fileActions.openPath(event.payload.path)),
    events.appInfoChanged.listen((event) => applyAppInfoChange(event.payload)),
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
      const [project, info] = await Promise.all([commands.getProject(), commands.getAppInfo()]);
      apply(project);
      setInfo(info);
      // After the first state, so the unsaved checks see the real document. The launch path
      // first (design `platform-integration.md` §8): the recovery prompt still appears after it.
      const launchPath = await commands.takeLaunchPath().catch((error: unknown) => {
        logError(error);
        return null;
      });
      if (launchPath !== null) await fileActions.openPath(launchPath);
      void recovery.load();
    })
    .catch(reportError);

  return () => {
    disposed = true;
    for (const stop of unlisteners) stop();
  };
}
