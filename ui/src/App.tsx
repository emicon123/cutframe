import { useEffect } from "react";

import { TooltipProvider } from "@/components/ui/tooltip";
import { AboutDialog } from "@/features/about/AboutDialog";
import { EditorLayout } from "@/features/editor/EditorLayout";
import { installShortcuts } from "@/features/editor/shortcuts";
import { ShortcutsDialog } from "@/features/editor/ShortcutsDialog";
import { HomeScreen } from "@/features/home/HomeScreen";
import { ErrorDialog } from "@/features/notifications/ErrorDialog";
import { Toaster } from "@/features/notifications/Toaster";
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
  const view = useUiStore((s) => s.view);
  // Nothing until startup has opened the launch path, so it does not flash the home screen.
  const started = useUiStore((s) => s.started);

  return (
    <TooltipProvider delayDuration={400}>
      {/* Home unmounts the editor, and with it the native preview (`setPreviewVisible(false)`). */}
      {view === "editor" ? <EditorLayout /> : started && <HomeScreen />}
      <ProjectSettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
      {/* Before ShortcutsDialog, which Preferences → Shortcuts opens on top of it. */}
      <PreferencesDialog />
      <ShortcutsDialog />
      <RecoveryDialog />
      <UnsavedChangesDialog />
      <AboutDialog />
      {/* After the unsaved prompt: a failed save from it shows on top. */}
      <ErrorDialog />
      <Toaster />
    </TooltipProvider>
  );
}
