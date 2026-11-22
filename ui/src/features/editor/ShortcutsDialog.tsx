import { Fragment } from "react";

import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { useUiStore } from "@/stores/uiStore";

import { shortcutGroups } from "./actions";

/** Shortcut reference, generated from the action registry and grouped by menu. */
export function ShortcutsDialog() {
  const open = useUiStore((s) => s.shortcutsOpen);
  const setOpen = useUiStore((s) => s.setShortcutsOpen);
  usePreviewOccluder(open);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="sm:max-w-md" closeLabel={t.shortcutsDialog.close}>
        <DialogHeader>
          <DialogTitle>{t.shortcutsDialog.title}</DialogTitle>
          <DialogDescription>{t.shortcutsDialog.description}</DialogDescription>
        </DialogHeader>
        <div className="flex max-h-[60vh] flex-col gap-4 overflow-y-auto">
          {shortcutGroups().map((group) => (
            <section key={group.id} aria-labelledby={`shortcuts-${group.id}`}>
              <h3 id={`shortcuts-${group.id}`} className="mb-1.5 text-xs font-semibold text-muted-foreground">
                {group.label}
              </h3>
              <dl className="grid grid-cols-[1fr_auto] gap-x-4 gap-y-1.5">
                {group.rows.map((row) => (
                  <Fragment key={row.id}>
                    <dt>{row.label}</dt>
                    <dd className="flex items-center justify-end gap-1.5 text-muted-foreground">
                      {row.keys.map((key, i) => (
                        <Fragment key={key}>
                          {i > 0 && <span className="text-xs">{t.shortcutsDialog.or}</span>}
                          <kbd className="rounded-sm border bg-muted px-1.5 py-0.5 font-sans text-xs text-foreground">
                            {key}
                          </kbd>
                        </Fragment>
                      ))}
                    </dd>
                  </Fragment>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
