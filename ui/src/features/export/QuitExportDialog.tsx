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
import { t } from "@/i18n";

import { answerCancelExport, useQuitPrompt } from "./quitPrompt";

/** Closing while an export runs (export.md §2 step 8). Escape and ✕ keep exporting. */
export function QuitExportDialog() {
  const open = useQuitPrompt((s) => s.resolve !== null);
  usePreviewOccluder(open);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) answerCancelExport(false);
      }}
    >
      <DialogContent className="sm:max-w-md" closeLabel={t.export.close}>
        <DialogHeader>
          <DialogTitle>{t.export.quit.title}</DialogTitle>
          <DialogDescription>{t.export.quit.description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button autoFocus variant="ghost" onClick={() => answerCancelExport(false)}>
            {t.export.quit.keep}
          </Button>
          <Button variant="destructive" onClick={() => answerCancelExport(true)}>
            {t.export.quit.confirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
