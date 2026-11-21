import { CircleAlert, Cpu } from "lucide-react";

import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";

export function StatusBar() {
  const info = useAppStore((s) => s.info);
  const lastError = useAppStore((s) => s.lastError);

  return (
    <footer className="flex h-6 shrink-0 items-center gap-3 border-t bg-panel px-3 text-xs text-muted-foreground">
      {info && (
        <>
          <span className="flex items-center gap-1.5">
            <Cpu className="size-3.5" aria-hidden />
            {info.gpu ? t.status.gpu(info.gpu.adapter, info.gpu.backend) : t.status.noGpu}
            {info.gpu?.software && ` (${t.status.software})`}
          </span>
          <span>{t.status.previewMode[info.previewMode]}</span>
        </>
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
