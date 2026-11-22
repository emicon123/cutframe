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

import { formatSavedAt, type Recovery, recovery as appRecovery, type RecoveryEntry } from "./recovery";

/**
 * "Recover unsaved work": the autosaves of instances that did not exit cleanly, offered once per
 * webview load. Escape, ✕ and Later close it and keep the files for the next start.
 */
export function RecoveryDialog({ recovery = appRecovery }: { recovery?: Recovery }) {
  const entries = recovery.store((s) => s.entries);
  const error = recovery.store((s) => s.error);
  const busy = recovery.store((s) => s.busy);
  const open = entries.length > 0;
  usePreviewOccluder(open);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) recovery.later();
      }}
    >
      <DialogContent className="sm:max-w-lg" closeLabel={t.recovery.later}>
        <DialogHeader>
          <DialogTitle>{t.recovery.title}</DialogTitle>
          <DialogDescription>{t.recovery.description}</DialogDescription>
        </DialogHeader>
        <ul className="grid max-h-80 gap-2 overflow-y-auto">
          {entries.map((entry) => (
            <EntryRow
              key={entry.id}
              entry={entry}
              disabled={busy}
              onRestore={() => void recovery.restore(entry.id)}
              onDiscard={() => void recovery.discard(entry.id)}
            />
          ))}
        </ul>
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button variant="ghost" disabled={busy} onClick={() => recovery.later()}>
            {t.recovery.later}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function EntryRow({
  entry,
  disabled,
  onRestore,
  onDiscard,
}: {
  entry: RecoveryEntry;
  disabled: boolean;
  onRestore: () => void;
  onDiscard: () => void;
}) {
  return (
    <li className="flex items-center gap-3 rounded-md border border-border p-3">
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">{entry.projectName}</p>
        <p className="truncate text-xs text-muted-foreground" title={entry.filePath ?? undefined}>
          {entry.filePath ?? t.recovery.notSavedYet}
        </p>
        <p className="text-xs text-muted-foreground tabular-nums">
          {t.recovery.savedAt(formatSavedAt(entry.savedAtMs))}
        </p>
      </div>
      <Button
        variant="secondary"
        size="sm"
        disabled={disabled}
        aria-label={t.recovery.discardItem(entry.projectName)}
        onClick={onDiscard}
      >
        {t.recovery.discard}
      </Button>
      <Button
        size="sm"
        disabled={disabled}
        aria-label={t.recovery.restoreItem(entry.projectName)}
        onClick={onRestore}
      >
        {t.recovery.restore}
      </Button>
    </li>
  );
}
