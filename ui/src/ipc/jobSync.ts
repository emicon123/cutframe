import { events } from "@/bindings";
import { useAppStore } from "@/stores/appStore";
import { useImportStore } from "@/stores/importStore";

import { jobResults } from "./jobResults";

/**
 * Feeds job events (`job-progress`, `media-job-report`, `job-finished`) into the import store and `jobResults`.
 * Start it before any job can start. Returns a cleanup function.
 */
export function startJobSync(): () => void {
  const { dispatch } = useImportStore.getState();
  const subscriptions = [
    events.jobProgress.listen((e) => dispatch({ type: "progress", event: e.payload })),
    events.mediaJobReport.listen((e) => {
      jobResults.report(e.payload);
      dispatch({ type: "report", event: e.payload });
    }),
    events.jobFinished.listen((e) => {
      jobResults.finished(e.payload);
      dispatch({ type: "finished", event: e.payload });
    }),
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
