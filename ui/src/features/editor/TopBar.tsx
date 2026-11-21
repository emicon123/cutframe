import { Download, Redo2, Undo2 } from "lucide-react";

import { commands } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { t } from "@/i18n";
import { run } from "@/stores/appStore";
import { useProjectStore } from "@/stores/projectStore";

export function TopBar() {
  const name = useProjectStore((s) => s.state?.project.name);
  const history = useProjectStore((s) => s.state?.history);

  const undoTip = history?.undoLabel ? t.topBar.undoAction(history.undoLabel) : t.topBar.nothingToUndo;
  const redoTip = history?.redoLabel ? t.topBar.redoAction(history.redoLabel) : t.topBar.nothingToRedo;

  return (
    <header className="flex h-11 shrink-0 items-center gap-2 border-b bg-panel px-3">
      <img src="/logo.svg" alt="" className="size-6" />
      <span className="font-semibold">{t.app.name}</span>
      <span className="text-muted-foreground">/</span>
      <span className="truncate">{name}</span>

      <div className="ml-4 flex items-center gap-0.5">
        <HistoryButton
          label={t.topBar.undo}
          tip={`${undoTip} (Ctrl+Z)`}
          disabled={!history?.canUndo}
          onClick={() => void run(commands.undo())}
        >
          <Undo2 className="size-4" aria-hidden />
        </HistoryButton>
        <HistoryButton
          label={t.topBar.redo}
          tip={`${redoTip} (Ctrl+Shift+Z)`}
          disabled={!history?.canRedo}
          onClick={() => void run(commands.redo())}
        >
          <Redo2 className="size-4" aria-hidden />
        </HistoryButton>
      </div>

      <Button className="ml-auto" size="sm" disabled title={t.topBar.exportSoon}>
        <Download className="size-4" aria-hidden />
        {t.topBar.export}
      </Button>
    </header>
  );
}

function HistoryButton({
  label,
  tip,
  disabled,
  onClick,
  children,
}: {
  label: string;
  tip: string;
  disabled: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* The span keeps the tooltip working while the button is disabled. */}
        <span tabIndex={disabled ? 0 : -1}>
          <Button variant="ghost" size="icon" aria-label={label} disabled={disabled} onClick={onClick}>
            {children}
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent>{tip}</TooltipContent>
    </Tooltip>
  );
}
