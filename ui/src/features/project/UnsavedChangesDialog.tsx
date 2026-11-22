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
import { useProjectStore } from "@/stores/projectStore";

import { answerUnsaved, useUnsavedPrompt } from "./unsavedPrompt";

/** Save / Don't save / Cancel, shown while `askUnsaved()` waits. Escape and ✕ mean Cancel. */
export function UnsavedChangesDialog() {
  const open = useUnsavedPrompt((s) => s.resolve !== null);
  const name = useProjectStore((s) => s.state?.project.name ?? "");
  usePreviewOccluder(open);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) answerUnsaved("cancel");
      }}
    >
      <DialogContent className="sm:max-w-md" closeLabel={t.unsaved.cancel}>
        <DialogHeader>
          <DialogTitle>{t.unsaved.title(name)}</DialogTitle>
          <DialogDescription>{t.unsaved.description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="ghost" onClick={() => answerUnsaved("cancel")}>
            {t.unsaved.cancel}
          </Button>
          <Button variant="secondary" onClick={() => answerUnsaved("discard")}>
            {t.unsaved.discard}
          </Button>
          <Button autoFocus onClick={() => answerUnsaved("save")}>
            {t.unsaved.save}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
