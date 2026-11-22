import { useEffect } from "react";

import { useAppStore } from "@/stores/appStore";

/**
 * Hides the native preview window while `active` (an open menu or dialog), because the preview
 * covers any DOM drawn over its area (ADR-0005). Every overlay that may overlap the preview uses
 * this instead of calling `setPreviewVisible` itself.
 */
export function usePreviewOccluder(active: boolean): void {
  useEffect(() => {
    if (!active) return;
    const { addPreviewOccluder, removePreviewOccluder } = useAppStore.getState();
    addPreviewOccluder();
    return removePreviewOccluder;
  }, [active]);
}
