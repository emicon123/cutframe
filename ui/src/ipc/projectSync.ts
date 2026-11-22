import { commands, events } from "@/bindings";
import { fileActions } from "@/features/project/fileActions";
import { recovery } from "@/features/project/recovery";
import { logError } from "@/ipc/errors";
import { useAppStore } from "@/stores/appStore";
import { useProjectStore } from "@/stores/projectStore";

/**
 * Keeps the project store in sync with Rust: subscribes to `projectChanged` (and `quitRequested`,
 * answered with the unsaved-changes prompt) first, cancels any stale gesture, then fetches the
 * current state, so no change is missed in between, and then offers crash recovery. Returns a cleanup function.
 */
export function startProjectSync(): () => void {
  let disposed = false;
  const unlisteners: (() => void)[] = [];
  const { apply } = useProjectStore.getState();
  const { setInfo, reportError } = useAppStore.getState();

  void Promise.all([
    events.projectChanged.listen((event) => apply(event.payload)),
    events.quitRequested.listen(() => void fileActions.requestClose()),
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
      // After the first state, so a restore's unsaved check sees the real document.
      void recovery.load();
    })
    .catch(reportError);

  return () => {
    disposed = true;
    for (const stop of unlisteners) stop();
  };
}
