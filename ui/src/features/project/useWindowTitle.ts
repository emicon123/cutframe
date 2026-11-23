import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect } from "react";

import { logError } from "@/ipc/errors";
import { useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

import { formatWindowTitle } from "./fileActions";

/**
 * Keeps the native window title at `*<name> - Cutframe` (asterisk while dirty) in the editor, and
 * plain `Cutframe` on the home screen (its untitled project is not shown there).
 */
export function useWindowTitle(): void {
  const editor = useUiStore((s) => s.view === "editor");
  const name = useProjectStore((s) => s.state?.project.name);
  const dirty = useProjectStore((s) => s.state?.dirty ?? false);

  useEffect(() => {
    const title = formatWindowTitle(editor && name !== undefined ? { name, dirty } : null);
    getCurrentWindow().setTitle(title).catch(logError);
  }, [editor, name, dirty]);
}
