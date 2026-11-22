import { commands, events } from "@/bindings";
import { logError } from "@/ipc/errors";
import { useAppStore } from "@/stores/appStore";
import { useProjectStore } from "@/stores/projectStore";

/**
 * Keeps the project store in sync with Rust: subscribes to `projectChanged` first, cancels any
 * stale gesture, then fetches the current state, so no change is missed in between. Returns a
 * cleanup function.
 */
export function startProjectSync(): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  const { apply } = useProjectStore.getState();
  const { setInfo, reportError } = useAppStore.getState();

  void events.projectChanged
    .listen((event) => apply(event.payload))
    .then(async (stop) => {
      if (disposed) {
        stop();
        return;
      }
      unlisten = stop;
      // A gesture left open by a previous webview (reload, crash) would make every edit fail with
      // GestureInProgress (ADR-0006). Cancelling is a no-op otherwise; a failure must not block
      // the initial state, so it is only logged.
      await commands.cancelGesture().catch(logError);
      const [project, info] = await Promise.all([commands.getProject(), commands.getAppInfo()]);
      apply(project);
      setInfo(info);
    })
    .catch(reportError);

  return () => {
    disposed = true;
    unlisten?.();
  };
}
