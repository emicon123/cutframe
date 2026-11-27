import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";

import { Button } from "@/components/ui/button";
import { jobStageText, t } from "@/i18n";

import { estimateRemainingMs, formatEta } from "./eta";
import { cancelExport, useExportStore } from "./exportStore";

/** How often the ETA text is recomputed between progress events. */
const ETA_TICK_MS = 1000;

/**
 * The running export in the editor header, never over the preview (export.md §7): stage, percent,
 * ETA and Cancel. Re-renders when the whole percent changes, plus once a second for the ETA.
 */
export function ExportProgress() {
  const job = useExportStore(
    useShallow((s) =>
      s.job
        ? {
            stage: s.job.stage,
            percent: s.job.progress === null ? null : Math.floor(s.job.progress * 100),
            cancelling: s.job.cancelling,
          }
        : null,
    ),
  );
  if (!job) return null;
  const { stage, percent, cancelling } = job;
  const encoding = stage === "exportEncoding";

  return (
    <div role="group" aria-label={t.export.progress.label} className="flex items-center gap-2 text-xs">
      <span className="text-muted-foreground">
        {stage ? jobStageText(stage) : jobStageText("exportPreparing")}
      </span>
      <div
        role="progressbar"
        aria-label={t.export.progress.label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        className="h-1.5 w-28 overflow-hidden rounded-full bg-muted"
      >
        <div
          className="h-full rounded-full bg-primary transition-[width]"
          style={{ width: `${encoding ? (percent ?? 0) : stage === "exportFinalizing" ? 100 : 0}%` }}
        />
      </div>
      {encoding && percent !== null && (
        <span className="w-8 text-right tabular-nums">{t.export.progress.percent(percent)}</span>
      )}
      {encoding && <EtaText />}
      <Button
        variant="ghost"
        size="xs"
        title={t.export.progress.cancelTip}
        disabled={cancelling}
        onClick={() => void cancelExport()}
      >
        <X aria-hidden />
        {cancelling ? t.export.progress.cancelling : t.export.progress.cancel}
      </Button>
    </div>
  );
}

/** Reads the samples outside React once a second, so progress events do not re-render it. */
function EtaText() {
  const [text, setText] = useState<string>(t.export.progress.estimating);
  useEffect(() => {
    const update = () => {
      const job = useExportStore.getState().job;
      const ms = job ? estimateRemainingMs(job.samples, job.encodingSince, performance.now()) : null;
      setText(ms === null ? t.export.progress.estimating : formatEta(ms));
    };
    update();
    const timer = window.setInterval(update, ETA_TICK_MS);
    return () => window.clearInterval(timer);
  }, []);
  return <span className="min-w-28 text-muted-foreground tabular-nums">{text}</span>;
}
