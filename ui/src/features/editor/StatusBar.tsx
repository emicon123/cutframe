import { CircleAlert, Cpu, TriangleAlert, X } from "lucide-react";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";

import { gpuNoticeText, gpuWarningText } from "./gpuNotice";

/** Below the timeline, so nothing here (tooltips included) can reach the native preview. */
export function StatusBar() {
  const info = useAppStore((s) => s.info);
  const lastError = useAppStore((s) => s.lastError);
  const notice = useAppStore((s) => s.gpuNotice);
  const dismissNotice = useAppStore((s) => s.dismissGpuNotice);
  const warning = gpuWarningText(info);

  return (
    <footer className="flex h-6 shrink-0 items-center gap-3 border-t bg-panel px-3 text-xs text-muted-foreground">
      {info && (
        <>
          <span className="flex items-center gap-1.5">
            <Cpu className="size-3.5" aria-hidden />
            {info.gpu ? t.status.gpu(info.gpu.adapter, info.gpu.backend) : t.status.noGpu}
            {info.gpu?.software && ` (${t.status.software})`}
            {warning && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    aria-label={warning}
                    className="flex items-center rounded-sm text-warning focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
                  >
                    <TriangleAlert className="size-3.5" aria-hidden />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="top">{warning}</TooltipContent>
              </Tooltip>
            )}
          </span>
          <span>{t.status.previewMode[info.previewMode]}</span>
        </>
      )}
      {notice && (
        <span role="status" className="flex min-w-0 items-center gap-1.5 text-warning">
          <TriangleAlert className="size-3.5 shrink-0" aria-hidden />
          <span className="truncate">{gpuNoticeText(notice)}</span>
          <button
            type="button"
            aria-label={t.status.dismiss}
            onClick={dismissNotice}
            className="flex items-center rounded-sm hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
          >
            <X className="size-3.5" aria-hidden />
          </button>
        </span>
      )}
      {lastError && (
        <span role="alert" className="flex min-w-0 items-center gap-1.5 text-destructive">
          <CircleAlert className="size-3.5 shrink-0" aria-hidden />
          <span className="truncate">{lastError}</span>
        </span>
      )}
      {info && <span className="ml-auto">{t.status.version(info.version)}</span>}
    </footer>
  );
}
