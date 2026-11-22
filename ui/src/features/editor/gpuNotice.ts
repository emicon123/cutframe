import type { AppInfo_Serialize } from "@/bindings";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";

type AppInfo = AppInfo_Serialize;

/** A runtime GPU event the status bar tells the user about (design `platform-integration.md` §2). */
export type GpuNotice =
  /** The GPU was lost and the preview restarted on `adapter`. */
  | { kind: "gpuRestarted"; adapter: string; software: boolean }
  /** The preview gave up after repeated losses. */
  | { kind: "previewLost" };

/**
 * The notice an `appInfoChanged` payload deserves, or `null` for none. Every `deviceLost` payload
 * is a new loss (Rust emits one per recovery), so each gets its notice; the preview turning
 * `unavailable` is reported only on the change.
 */
export function gpuNoticeFor(previous: AppInfo | null, next: AppInfo): GpuNotice | null {
  if (next.previewMode === "unavailable") {
    return previous?.previewMode === "unavailable" ? null : { kind: "previewLost" };
  }
  if (next.gpu?.fallback === "deviceLost") {
    return { kind: "gpuRestarted", adapter: next.gpu.adapter, software: next.gpu.software };
  }
  return null;
}

export function gpuNoticeText(notice: GpuNotice): string {
  if (notice.kind === "previewLost") return t.status.previewLost;
  return notice.software ? t.status.gpuRestartedSoftware : t.status.gpuRestarted(notice.adapter);
}

/** Tooltip of the status bar's warning icon: why the GPU in use is not the first choice. */
export function gpuWarningText(info: AppInfo | null): string | null {
  const fallback = info?.gpu?.fallback;
  return fallback ? t.gpu.fallbacks[fallback] : null;
}

/** `appInfoChanged`: replaces the stored `AppInfo` and posts the notice it deserves, if any. */
export function applyAppInfoChange(next: AppInfo): void {
  const { info, setInfo, showGpuNotice } = useAppStore.getState();
  const notice = gpuNoticeFor(info, next);
  setInfo(next);
  if (notice) showGpuNotice(notice);
}
