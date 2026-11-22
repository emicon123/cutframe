import { Download, Redo2, Undo2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { t } from "@/i18n";
import { useProjectStore } from "@/stores/projectStore";

import {
  type ActionContext,
  type ActionId,
  actionTooltip,
  getAction,
  isEnabled,
  useActionContext,
} from "./actions";
import { MainMenu } from "./MainMenu";

export function TopBar() {
  const name = useProjectStore((s) => s.state?.project.name);
  const ctx = useActionContext();

  return (
    <header className="flex h-11 shrink-0 items-center gap-2 border-b bg-panel px-3">
      <img src="/logo.svg" alt="" className="size-6" />
      <span className="font-semibold">{t.app.name}</span>
      <span className="text-muted-foreground">/</span>
      <span className="truncate">{name}</span>

      <div className="ml-4">
        <MainMenu />
      </div>
      <div className="flex items-center gap-0.5">
        <ActionButton id="undo" ctx={ctx}>
          <Undo2 className="size-4" aria-hidden />
        </ActionButton>
        <ActionButton id="redo" ctx={ctx}>
          <Redo2 className="size-4" aria-hidden />
        </ActionButton>
      </div>

      <Button className="ml-auto" size="sm" disabled title={t.topBar.exportSoon}>
        <Download className="size-4" aria-hidden />
        {t.topBar.export}
      </Button>
    </header>
  );
}

/** Icon button for a registered action; its tooltip shows the label and shortcut. */
function ActionButton({
  id,
  ctx,
  children,
}: {
  id: ActionId;
  ctx: ActionContext;
  children: React.ReactNode;
}) {
  const action = getAction(id);
  const disabled = !isEnabled(action, ctx);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* The span keeps the tooltip working while the button is disabled. */}
        <span tabIndex={disabled ? 0 : -1}>
          <Button
            variant="ghost"
            size="icon"
            aria-label={action.label}
            disabled={disabled}
            onClick={action.run}
          >
            {children}
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent>{actionTooltip(action, ctx)}</TooltipContent>
    </Tooltip>
  );
}
