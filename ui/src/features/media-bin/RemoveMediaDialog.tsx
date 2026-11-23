import { CircleAlert } from "lucide-react";
import { useState } from "react";

import type { MediaView_Serialize } from "@/bindings";
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

import { removeMedia } from "./importActions";

/** Most file names listed in the confirmation; the title carries the count. */
const LISTED = 8;

/**
 * Confirms removing `items` (open while not `null`), then sends one `removeMedia` (one undo step).
 * `onClose(true)` after a successful removal. Media that clips use gets a warning: its clips are
 * removed in the same step.
 */
export function RemoveMediaDialog({
  items,
  onClose,
}: {
  items: MediaView_Serialize[] | null;
  onClose: (removed: boolean) => void;
}) {
  const open = items !== null;
  const [busy, setBusy] = useState(false);
  usePreviewOccluder(open);

  const confirm = async () => {
    if (!items || busy) return;
    setBusy(true);
    const removed = await removeMedia(items.map((item) => item.id));
    setBusy(false);
    onClose(removed);
  };

  const first = items?.[0];
  // Removing media also removes its clips, in the same undo step (timeline design §4.9).
  const usedBy = items?.reduce((sum, item) => sum + item.usedBy, 0) ?? 0;
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose(false);
      }}
    >
      <DialogContent className="sm:max-w-md" closeLabel={t.mediaBin.remove.cancel}>
        <DialogHeader>
          <DialogTitle>{t.mediaBin.remove.title(items?.length ?? 0, first?.fileName ?? "")}</DialogTitle>
          <DialogDescription>{t.mediaBin.remove.description}</DialogDescription>
        </DialogHeader>
        {usedBy > 0 && (
          <p role="alert" className="flex gap-2 text-sm text-warning">
            <CircleAlert className="size-4 shrink-0" aria-hidden />
            {t.mediaBin.remove.used(usedBy)}
          </p>
        )}
        {items && items.length > 1 && (
          <ul className="max-h-40 overflow-y-auto text-sm text-muted-foreground">
            {items.slice(0, LISTED).map((item) => (
              <li key={item.id} className="truncate" title={item.path}>
                {item.fileName}
              </li>
            ))}
            {items.length > LISTED && <li>{t.mediaBin.remove.more(items.length - LISTED)}</li>}
          </ul>
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={() => onClose(false)}>
            {t.mediaBin.remove.cancel}
          </Button>
          <Button variant="destructive" autoFocus disabled={busy} onClick={() => void confirm()}>
            {t.mediaBin.remove.confirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
