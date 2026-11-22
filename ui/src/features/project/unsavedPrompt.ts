import { create } from "zustand";

/** The user's answer to "Save changes?". */
export type UnsavedChoice = "save" | "discard" | "cancel";

type UnsavedPromptStore = {
  /** Resolves the open prompt; `null` while no prompt is shown. */
  resolve: ((choice: UnsavedChoice) => void) | null;
};

/** State of the unsaved-changes prompt, rendered by `UnsavedChangesDialog`. */
export const useUnsavedPrompt = create<UnsavedPromptStore>()(() => ({ resolve: null }));

/** Shows the prompt and waits for the answer. A prompt that is already open is cancelled. */
export function askUnsaved(): Promise<UnsavedChoice> {
  useUnsavedPrompt.getState().resolve?.("cancel");
  return new Promise((resolve) => useUnsavedPrompt.setState({ resolve }));
}

/** Answers the open prompt and closes it. Does nothing when no prompt is open. */
export function answerUnsaved(choice: UnsavedChoice): void {
  const { resolve } = useUnsavedPrompt.getState();
  useUnsavedPrompt.setState({ resolve: null });
  resolve?.(choice);
}
