import { events } from "@/bindings";
import { finishExport, setCapabilities, useExportStore } from "@/features/export/exportStore";
import { useAppStore } from "@/stores/appStore";

/**
 * Feeds the export events into the export store: `jobProgress` (the export stages; other jobs are
 * ignored there), `exportReport`, `jobFinished` (the card goes away and the result toast shows)
 * and `exportCapabilitiesChanged` (the session's encoder probe). Start it before an export can
 * start. Returns a cleanup function.
 */
export function startExportSync(): () => void {
  const { dispatch } = useExportStore.getState();
  const subscriptions = [
    events.jobProgress.listen((e) =>
      dispatch({ type: "progress", event: e.payload, now: performance.now() }),
    ),
    events.exportReport.listen((e) => dispatch({ type: "report", event: e.payload })),
    events.jobFinished.listen((e) => finishExport(e.payload)),
    events.exportCapabilitiesChanged.listen((e) => setCapabilities(e.payload)),
  ];
  for (const subscription of subscriptions) {
    subscription.catch(useAppStore.getState().reportError);
  }
  return () => {
    for (const subscription of subscriptions) {
      void subscription.then((stop) => stop()).catch(() => undefined);
    }
  };
}
