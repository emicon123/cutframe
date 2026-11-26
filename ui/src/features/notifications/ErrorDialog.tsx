import { CircleAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { fileActions } from "@/features/project/fileActions";
import { t } from "@/i18n";

import { closeErrorDialog, useErrorDialog } from "./errorDialogStore";
import { openLogsFolder } from "./logs";

/**
 * A failure that leaves the user with a decision (F0.08): a project that did not open, or changes
 * that were not saved (Save As… elsewhere). Other failures are toasts.
 */
export function ErrorDialog() {
  const content = useErrorDialog((s) => s.content);
  const open = content !== null;
  usePreviewOccluder(open);

  const saveAs = () => {
    closeErrorDialog();
    void fileActions.saveAs();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) closeErrorDialog();
      }}
    >
      {content && (
        <DialogContent className="sm:max-w-md" closeLabel={t.errorDialog.dismiss}>
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <CircleAlert className="size-5 shrink-0 text-destructive" aria-hidden />
              {content.title}
            </DialogTitle>
            <DialogDescription>{content.reason}</DialogDescription>
          </DialogHeader>
          {(content.hint !== undefined || content.logs) && (
            <div className="grid gap-1 text-sm text-muted-foreground">
              {content.hint && <p>{content.hint}</p>}
              {content.logs && <p>{t.errorDialog.detailsInLog}</p>}
            </div>
          )}
          <DialogFooter>
            {content.logs && (
              <Button variant="ghost" className="sm:mr-auto" onClick={() => void openLogsFolder()}>
                {t.errorDialog.openLogs}
              </Button>
            )}
            {content.saveAs && (
              <Button variant="secondary" onClick={saveAs}>
                {t.errorDialog.saveAs}
              </Button>
            )}
            <Button autoFocus onClick={closeErrorDialog}>
              {t.errorDialog.close}
            </Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  );
}
