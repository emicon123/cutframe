import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { shortcutLabel } from "@/features/editor/shortcuts";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { useProjectStore } from "@/stores/projectStore";

import { fileActions } from "./fileActions";
import { ProjectSettingsDialog } from "./ProjectSettingsDialog";

/** File menu in the top bar. Task 0130 moves these items into the main menu. */
export function FileMenu() {
  const [menuOpen, setMenuOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const loaded = useProjectStore((s) => s.state !== null);
  usePreviewOccluder(menuOpen);

  return (
    <>
      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm">
            {t.file.menu}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-52">
          <MenuItem
            label={t.file.new}
            shortcut="new-project"
            onSelect={() => void fileActions.newProject()}
          />
          <MenuItem
            label={t.file.open}
            shortcut="open-project"
            onSelect={() => void fileActions.openProject()}
          />
          <DropdownMenuSeparator />
          <MenuItem
            label={t.file.save}
            shortcut="save"
            disabled={!loaded}
            onSelect={() => void fileActions.save()}
          />
          <MenuItem
            label={t.file.saveAs}
            shortcut="save-as"
            disabled={!loaded}
            onSelect={() => void fileActions.saveAs()}
          />
          <DropdownMenuSeparator />
          <MenuItem
            label={t.file.projectSettings}
            disabled={!loaded}
            onSelect={() => setSettingsOpen(true)}
          />
        </DropdownMenuContent>
      </DropdownMenu>
      <ProjectSettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
    </>
  );
}

function MenuItem({
  label,
  shortcut,
  disabled,
  onSelect,
}: {
  label: string;
  shortcut?: string;
  disabled?: boolean;
  onSelect: () => void;
}) {
  return (
    <DropdownMenuItem disabled={disabled} onSelect={onSelect}>
      {label}
      {shortcut && <DropdownMenuShortcut>{shortcutLabel(shortcut)}</DropdownMenuShortcut>}
    </DropdownMenuItem>
  );
}
