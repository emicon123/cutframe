import { useEffect } from "react";

import { TooltipProvider } from "@/components/ui/tooltip";
import { EditorLayout } from "@/features/editor/EditorLayout";
import { installShortcuts } from "@/features/editor/shortcuts";
import { ShortcutsDialog } from "@/features/editor/ShortcutsDialog";
import { PreferencesDialog } from "@/features/preferences/PreferencesDialog";
import { loadPreferences } from "@/features/preferences/preferencesStore";
import { ProjectSettingsDialog } from "@/features/project/ProjectSettingsDialog";
import { RecoveryDialog } from "@/features/project/RecoveryDialog";
import { UnsavedChangesDialog } from "@/features/project/UnsavedChangesDialog";
import { useWindowTitle } from "@/features/project/useWindowTitle";
import { startJobSync } from "@/ipc/jobSync";
import { startProjectSync } from "@/ipc/projectSync";
import { useUiStore } from "@/stores/uiStore";

export function App() {
  useEffect(() => startJobSync(), []);
  useEffect(() => startProjectSync(), []);
  useEffect(() => installShortcuts(), []);
  useEffect(() => void loadPreferences(), []);
  useWindowTitle();
  const settingsOpen = useUiStore((s) => s.projectSettingsOpen);
  const setSettingsOpen = useUiStore((s) => s.setProjectSettingsOpen);

  return (
    <TooltipProvider delayDuration={400}>
      <EditorLayout />
      <ProjectSettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
      {/* Before ShortcutsDialog, which Preferences → Shortcuts opens on top of it. */}
      <PreferencesDialog />
      <ShortcutsDialog />
      <RecoveryDialog />
      <UnsavedChangesDialog />
    </TooltipProvider>
  );
}
