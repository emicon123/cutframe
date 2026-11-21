import { FolderOpen, Upload } from "lucide-react";

import { Button } from "@/components/ui/button";
import { t } from "@/i18n";

export function MediaBin() {
  return (
    <section className="flex h-full flex-col bg-panel" aria-label={t.mediaBin.title}>
      <header className="flex h-9 shrink-0 items-center justify-between border-b px-3">
        <h2 className="font-semibold">{t.mediaBin.title}</h2>
        <Button variant="ghost" size="sm" disabled title={t.mediaBin.emptyHint}>
          <Upload className="size-3.5" aria-hidden />
          {t.mediaBin.import}
        </Button>
      </header>
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
        <FolderOpen className="size-8 opacity-60" aria-hidden />
        <p className="font-medium text-foreground">{t.mediaBin.empty}</p>
        <p className="text-xs">{t.mediaBin.emptyHint}</p>
      </div>
    </section>
  );
}
