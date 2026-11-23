import {
  Copy,
  Ellipsis,
  FolderOpen,
  FolderSearch,
  ImageOff,
  type LucideIcon,
  Pencil,
  Trash2,
  X,
} from "lucide-react";
import { useState } from "react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { t } from "@/i18n";
import { mediaUrl } from "@/ipc/media";
import { formatDuration } from "@/lib/time";
import { cn } from "@/lib/utils";

import { formatModified, isMissing, type RecentProject } from "./recent";

export type CardAction = "open" | "rename" | "duplicate" | "reveal" | "delete" | "remove";

type MenuEntry = { action: CardAction; label: string; icon: LucideIcon; destructive?: boolean } | "-";

const FULL_MENU: readonly MenuEntry[] = [
  { action: "open", label: t.home.actions.open, icon: FolderOpen },
  { action: "rename", label: t.home.actions.rename, icon: Pencil },
  { action: "duplicate", label: t.home.actions.duplicate, icon: Copy },
  { action: "reveal", label: t.home.actions.reveal, icon: FolderSearch },
  "-",
  { action: "delete", label: t.home.actions.delete, icon: Trash2, destructive: true },
  { action: "remove", label: t.home.actions.remove, icon: X },
];

/** A missing file can only leave the list. */
const MISSING_MENU: readonly MenuEntry[] = [{ action: "remove", label: t.home.actions.remove, icon: X }];

/** The card menu's entries; the same for right-click and the `⋯` button. */
export function cardMenu(project: RecentProject): readonly MenuEntry[] {
  return isMissing(project) ? MISSING_MENU : FULL_MENU;
}

/**
 * One recent project: cover, name, duration and modified date (or "Missing"). A click opens it;
 * right-click and the `⋯` button show the same menu. At most 30 cards and nothing per frame, so
 * no memoization.
 */
export function ProjectCard({
  project,
  onAction,
}: {
  project: RecentProject;
  onAction: (action: CardAction, project: RecentProject) => void;
}) {
  const missing = isMissing(project);
  const entries = cardMenu(project);
  const run = (action: CardAction) => onAction(action, project);

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <li
          className={cn(
            "relative rounded-lg border bg-card text-card-foreground transition-colors",
            !missing && "hover:border-primary/60",
          )}
        >
          <button
            type="button"
            className="block w-full rounded-lg text-left focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none disabled:cursor-default"
            aria-label={t.home.openItem(project.name)}
            title={project.path}
            disabled={missing}
            onClick={() => run("open")}
          >
            <Cover hash={project.cover} dimmed={missing} />
            <div className="grid gap-0.5 p-2.5 pr-9">
              <p className="truncate text-sm font-medium">{project.name}</p>
              <p className="flex items-center gap-1.5 truncate text-xs text-muted-foreground tabular-nums">
                <span>{formatDuration(project.duration)}</span>
                <span aria-hidden>·</span>
                {project.modifiedMs === null ? (
                  <span className="font-medium text-warning" title={t.home.missingHint}>
                    {t.home.missing}
                  </span>
                ) : (
                  <span className="truncate">{t.home.modified(formatModified(project.modifiedMs))}</span>
                )}
              </p>
            </div>
          </button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                aria-label={t.home.cardMenu(project.name)}
                className="absolute right-1.5 bottom-2.5 flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                <Ellipsis className="size-4" aria-hidden />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-48">
              {entries.map((entry, index) =>
                entry === "-" ? (
                  <DropdownMenuSeparator key={`sep-${index}`} />
                ) : (
                  <DropdownMenuItem
                    key={entry.action}
                    variant={entry.destructive ? "destructive" : "default"}
                    onSelect={() => run(entry.action)}
                  >
                    <entry.icon aria-hidden />
                    {entry.label}
                  </DropdownMenuItem>
                ),
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        </li>
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-48">
        {entries.map((entry, index) =>
          entry === "-" ? (
            <ContextMenuSeparator key={`sep-${index}`} />
          ) : (
            <ContextMenuItem
              key={entry.action}
              variant={entry.destructive ? "destructive" : "default"}
              onSelect={() => run(entry.action)}
            >
              <entry.icon aria-hidden />
              {entry.label}
            </ContextMenuItem>
          ),
        )}
      </ContextMenuContent>
    </ContextMenu>
  );
}

/** The cover JPEG from `media://cover/<hash>`; a placeholder when there is none or it fails (404). */
function Cover({ hash, dimmed }: { hash: string | null; dimmed: boolean }) {
  const [failed, setFailed] = useState<string | null>(null);
  const show = hash !== null && failed !== hash;

  return (
    <div className="relative aspect-video overflow-hidden rounded-t-lg bg-muted">
      {show ? (
        <img
          src={mediaUrl(`cover/${hash}`)}
          alt=""
          loading="lazy"
          draggable={false}
          className={cn("size-full object-cover", dimmed && "opacity-40")}
          onError={() => setFailed(hash)}
        />
      ) : (
        <div className="absolute inset-0 flex items-center justify-center text-muted-foreground">
          <ImageOff className="size-6 opacity-50" aria-hidden />
          <span className="sr-only">{t.home.noCover}</span>
        </div>
      )}
    </div>
  );
}
