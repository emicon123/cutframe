import { useEffect } from "react";

import { TooltipProvider } from "@/components/ui/tooltip";
import { EditorLayout } from "@/features/editor/EditorLayout";
import { installShortcuts } from "@/features/editor/shortcuts";
import { UnsavedChangesDialog } from "@/features/project/UnsavedChangesDialog";
import { useWindowTitle } from "@/features/project/useWindowTitle";
import { startProjectSync } from "@/ipc/projectSync";

export function App() {
  useEffect(() => startProjectSync(), []);
  useEffect(() => installShortcuts(), []);
  useWindowTitle();

  return (
    <TooltipProvider delayDuration={400}>
      <EditorLayout />
      <UnsavedChangesDialog />
    </TooltipProvider>
  );
}
