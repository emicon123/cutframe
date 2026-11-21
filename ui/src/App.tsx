import { useEffect } from "react";

import { TooltipProvider } from "@/components/ui/tooltip";
import { EditorLayout } from "@/features/editor/EditorLayout";
import { installShortcuts } from "@/features/editor/shortcuts";
import { startProjectSync } from "@/ipc/projectSync";

export function App() {
  useEffect(() => startProjectSync(), []);
  useEffect(() => installShortcuts(), []);

  return (
    <TooltipProvider delayDuration={400}>
      <EditorLayout />
    </TooltipProvider>
  );
}
