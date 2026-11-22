import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect } from "react";

import { logError } from "@/ipc/errors";
import { useProjectStore } from "@/stores/projectStore";

import { formatWindowTitle } from "./fileActions";

/** Keeps the native window title at `*<name> - Cutframe` (asterisk while dirty). */
export function useWindowTitle(): void {
  const name = useProjectStore((s) => s.state?.project.name);
  const dirty = useProjectStore((s) => s.state?.dirty ?? false);

  useEffect(() => {
    const title = formatWindowTitle(name === undefined ? null : { name, dirty });
    getCurrentWindow().setTitle(title).catch(logError);
  }, [name, dirty]);
}
