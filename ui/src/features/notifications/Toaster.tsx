import { CircleAlert, CircleCheck, Info, X } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { t } from "@/i18n";
import { cn } from "@/lib/utils";
import { TOAST_DURATION_MS, type Toast, type ToastTone, useToastStore } from "@/stores/toastStore";

import { openLogsFolder } from "./logs";

const ICONS: Readonly<Record<ToastTone, typeof Info>> = {
  info: Info,
  success: CircleCheck,
  error: CircleAlert,
};

/**
 * Stacked, non-blocking notifications in the bottom-right corner, above the status bar (F0.08).
 * That corner is the timeline or the inspector, never the native preview window in the default
 * layout, so the toasts need no preview occluder (ADR-0005).
 */
export function Toaster() {
  const toasts = useToastStore((s) => s.toasts);
  return (
    <section
      aria-label={t.toasts.region}
      className="pointer-events-none fixed right-3 bottom-8 z-50 flex w-toast max-w-[calc(100vw-1.5rem)] flex-col gap-2"
    >
      {toasts.map((toast) => (
        <ToastItem key={toast.id} toast={toast} />
      ))}
    </section>
  );
}

function ToastItem({ toast }: { toast: Toast }) {
  const dismiss = useToastStore((s) => s.dismiss);
  // Hover or keyboard focus keeps the toast; leaving restarts its full duration.
  const [paused, setPaused] = useState(false);
  const Icon = ICONS[toast.tone];

  useEffect(() => {
    if (paused) return;
    const timer = window.setTimeout(() => dismiss(toast.id), TOAST_DURATION_MS[toast.tone]);
    return () => window.clearTimeout(timer);
  }, [paused, dismiss, toast.id, toast.tone]);

  return (
    <div
      role={toast.tone === "error" ? "alert" : "status"}
      onPointerEnter={() => setPaused(true)}
      onPointerLeave={() => setPaused(false)}
      onFocus={() => setPaused(true)}
      onBlur={() => setPaused(false)}
      className="pointer-events-auto flex items-start gap-2 rounded-md border bg-popover p-3 text-popover-foreground shadow-lg data-[tone=error]:border-destructive/60"
      data-tone={toast.tone}
    >
      <Icon
        className={cn(
          "mt-0.5 size-4 shrink-0",
          toast.tone === "error" && "text-destructive",
          toast.tone === "success" && "text-success",
          toast.tone === "info" && "text-muted-foreground",
        )}
        aria-hidden
      />
      <div className="grid min-w-0 flex-1 gap-1">
        <p className="font-medium break-words">
          {toast.text}
          {toast.count > 1 && (
            <span className="ml-1.5 text-xs text-muted-foreground">{t.toasts.repeated(toast.count)}</span>
          )}
        </p>
        {toast.detail && <p className="text-xs break-words text-muted-foreground">{toast.detail}</p>}
        {toast.action === "openLogs" && (
          <Button variant="outline" size="xs" className="w-fit" onClick={() => void openLogsFolder()}>
            {t.toasts.openLogs}
          </Button>
        )}
      </div>
      <button
        type="button"
        aria-label={t.toasts.dismiss}
        onClick={() => dismiss(toast.id)}
        className="flex items-center rounded-sm text-muted-foreground hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
      >
        <X className="size-3.5" aria-hidden />
      </button>
    </div>
  );
}
