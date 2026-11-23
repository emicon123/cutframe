import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { t } from "@/i18n";

import type { RecentProject } from "./recent";

/**
 * "Move <name> to the Recycle Bin?" (open while `project` is not `null`). Only the confirm button
 * calls `trash`; Escape, ✕ and Cancel do nothing.
 */
export function DeleteConfirmDialog({
  project,
  trash,
  onClose,
}: {
  project: RecentProject | null;
  trash: (path: string) => Promise<void>;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);

  const confirm = async () => {
    if (!project || busy) return;
    setBusy(true);
    await trash(project.path);
    setBusy(false);
    onClose();
  };

  return (
    <Dialog
      open={project !== null}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="sm:max-w-md" closeLabel={t.home.delete.cancel}>
        <DialogHeader>
          <DialogTitle>{t.home.delete.title(project?.name ?? "")}</DialogTitle>
          <DialogDescription>{t.home.delete.description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t.home.delete.cancel}
          </Button>
          <Button variant="destructive" autoFocus disabled={busy} onClick={() => void confirm()}>
            {t.home.delete.confirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
