import { events } from "@/bindings";
import { useAppStore } from "@/stores/appStore";
import { useImportStore } from "@/stores/importStore";

/**
 * Feeds job events (`job-progress`, `media-job-report`, `job-finished`) into the import store.
 * Start it before any job can start. Returns a cleanup function.
 */
export function startJobSync(): () => void {
  const { dispatch } = useImportStore.getState();
  const subscriptions = [
    events.jobProgress.listen((e) => dispatch({ type: "progress", event: e.payload })),
    events.mediaJobReport.listen((e) => dispatch({ type: "report", event: e.payload })),
    events.jobFinished.listen((e) => dispatch({ type: "finished", event: e.payload })),
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
