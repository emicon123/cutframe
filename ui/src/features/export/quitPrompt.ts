import { create } from "zustand";

type QuitPromptStore = {
  /** Resolves the open prompt; `null` while no prompt is shown. */
  resolve: ((cancelAndQuit: boolean) => void) | null;
};

/** State of "An export is running. Cancel it and quit?", rendered by `QuitExportDialog`. */
export const useQuitPrompt = create<QuitPromptStore>()(() => ({ resolve: null }));

/** Shows the prompt and waits; `true` means cancel the export and quit. An open prompt is answered "stay". */
export function askCancelExport(): Promise<boolean> {
  useQuitPrompt.getState().resolve?.(false);
  return new Promise((resolve) => useQuitPrompt.setState({ resolve }));
}

/** Answers the open prompt and closes it. Does nothing when no prompt is open. */
export function answerCancelExport(cancelAndQuit: boolean): void {
  const { resolve } = useQuitPrompt.getState();
  useQuitPrompt.setState({ resolve: null });
  resolve?.(cancelAndQuit);
}
