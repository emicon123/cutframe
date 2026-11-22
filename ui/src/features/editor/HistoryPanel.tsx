import { X } from "lucide-react";

import { commands } from "@/bindings";
import { Button } from "@/components/ui/button";
import { t } from "@/i18n";
import { cn } from "@/lib/utils";
import { run } from "@/stores/appStore";
import { useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

import { historyRows, type HistoryRow } from "./history";

const NO_HISTORY = { entries: [], current: 0 };

/**
 * Undo history as a list; clicking a row jumps there in one `jumpToHistory` call. It is laid out
 * beside the native preview, never over it, so it needs no preview occluder.
 */
export function HistoryPanel() {
  const history = useProjectStore((s) => s.state?.history);
  const close = useUiStore((s) => s.setHistoryPanelOpen);
  const rows = historyRows(history ?? NO_HISTORY);

  return (
    <section className="flex min-h-0 flex-col border-t bg-panel" aria-label={t.historyPanel.title}>
      <header className="flex h-9 shrink-0 items-center border-b pr-1 pl-3">
        <h2 className="font-semibold">{t.historyPanel.title}</h2>
        <Button
          variant="ghost"
          size="icon"
          className="ml-auto size-7"
          aria-label={t.historyPanel.close}
          onClick={() => close(false)}
        >
          <X className="size-4" aria-hidden />
        </Button>
      </header>
      <ol className="flex min-h-0 flex-col overflow-y-auto p-1">
        {rows.map((row) => (
          <HistoryItem key={row.index} row={row} />
        ))}
      </ol>
    </section>
  );
}

function HistoryItem({ row }: { row: HistoryRow }) {
  const current = row.status === "current";
  return (
    <li>
      <button
        type="button"
        aria-current={current ? "step" : undefined}
        className={cn(
          "w-full truncate rounded-sm px-2 py-1 text-left text-sm",
          "hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
          current && "bg-accent font-medium text-accent-foreground",
          row.status === "undone" && "text-muted-foreground italic",
        )}
        onClick={() => {
          if (!current) void run(commands.jumpToHistory(row.index));
        }}
      >
        {row.label}
        {row.status === "undone" && <span className="sr-only"> {t.historyPanel.undone}</span>}
      </button>
    </li>
  );
}
