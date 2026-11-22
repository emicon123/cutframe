import { FolderSearch, Trash2, Upload } from "lucide-react";
import { type KeyboardEvent, type MouseEvent, useRef, useState } from "react";

import type { MediaId, MediaView_Serialize } from "@/bindings";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { cn } from "@/lib/utils";
import type { MediaLayout } from "@/stores/uiStore";

import { importFromDialog, revealMedia } from "./importActions";
import { MediaItem } from "./MediaItem";
import { RemoveMediaDialog } from "./RemoveMediaDialog";
import {
  emptySelection,
  liveSelection,
  type Selection,
  type SelectionAction,
  selectionReducer,
} from "./selection";

const idOf = (target: EventTarget | null): MediaId | null =>
  target instanceof Element
    ? (target.closest<HTMLElement>("[data-media-id]")?.dataset.mediaId ?? null)
    : null;

/** Columns of the grid as laid out, so Up/Down move by a row. */
function gridColumns(element: HTMLElement | null): number {
  if (!element) return 1;
  const columns = getComputedStyle(element).gridTemplateColumns.split(" ").filter(Boolean).length;
  return Math.max(1, columns);
}

/**
 * The media items (already sorted and filtered) as a multi-select listbox: click, Ctrl/Shift-click,
 * arrow keys, Ctrl+A, Delete, and a context menu with Show in Explorer and Remove.
 */
export function MediaBrowser({ media, layout }: { media: MediaView_Serialize[]; layout: MediaLayout }) {
  const list = useRef<HTMLDivElement>(null);
  const [selection, setSelection] = useState<Selection>(emptySelection);
  const [menuOpen, setMenuOpen] = useState(false);
  const [removing, setRemoving] = useState<MediaView_Serialize[] | null>(null);
  usePreviewOccluder(menuOpen);

  const order = media.map((item) => item.id);
  const selectedIds = liveSelection(selection, order);
  const selected = new Set(selectedIds);
  const focusId =
    selection.focus !== null && order.includes(selection.focus) ? selection.focus : (order[0] ?? null);

  const apply = (action: SelectionAction, moveFocus = false) => {
    const next = selectionReducer(selection, action);
    setSelection(next);
    if (moveFocus && next.focus !== null) {
      const items = list.current?.querySelectorAll<HTMLElement>("[data-media-id]") ?? [];
      for (const element of items) if (element.dataset.mediaId === next.focus) element.focus();
    }
  };

  const askRemove = (ids: readonly MediaId[]) => {
    const items = media.filter((item) => ids.includes(item.id));
    if (items.length > 0) setRemoving(items);
  };

  const onClick = (event: MouseEvent) => {
    const id = idOf(event.target);
    if (id === null) {
      apply({ type: "clear" });
      return;
    }
    apply({ type: "click", id, ctrl: event.ctrlKey || event.metaKey, shift: event.shiftKey, order });
  };

  const onContextMenu = (event: MouseEvent) => {
    const id = idOf(event.target);
    apply(id === null ? { type: "clear" } : { type: "context", id });
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.altKey) return;
    const ctrl = event.ctrlKey || event.metaKey;
    const row = layout === "grid" ? gridColumns(list.current) : 1;
    const steps: Record<string, number> = {
      ArrowLeft: -1,
      ArrowRight: 1,
      ArrowUp: -row,
      ArrowDown: row,
      Home: -order.length,
      End: order.length,
    };
    const step = steps[event.key];
    let handled = true;
    if (step !== undefined && !ctrl) apply({ type: "move", step, shift: event.shiftKey, order }, true);
    else if (ctrl && event.key.toLowerCase() === "a") apply({ type: "selectAll", order });
    else if (event.key === "Delete" && !ctrl) askRemove(selectedIds);
    else handled = false;
    if (handled) {
      event.preventDefault();
      event.stopPropagation();
    }
  };

  const single = selectedIds.length === 1 ? selectedIds[0] : undefined;

  return (
    <>
      <ContextMenu onOpenChange={setMenuOpen}>
        <ContextMenuTrigger asChild>
          <div
            ref={list}
            role="listbox"
            aria-multiselectable
            aria-label={t.mediaBin.list}
            className={cn(
              "min-h-0 flex-1 content-start overflow-y-auto p-2",
              layout === "grid"
                ? "grid grid-cols-[repeat(auto-fill,minmax(var(--spacing-media-tile),1fr))] gap-1"
                : "flex flex-col",
            )}
            onClick={onClick}
            onContextMenu={onContextMenu}
            onKeyDown={onKeyDown}
          >
            {media.map((item) => (
              <MediaItem
                key={item.id}
                item={item}
                layout={layout}
                selected={selected.has(item.id)}
                tabbable={item.id === focusId}
              />
            ))}
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent className="min-w-52">
          <ContextMenuItem onSelect={() => void importFromDialog()}>
            <Upload aria-hidden />
            {t.file.importMedia}
          </ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem disabled={single === undefined} onSelect={() => single && revealMedia(single)}>
            <FolderSearch aria-hidden />
            {t.mediaBin.actions.reveal}
          </ContextMenuItem>
          <ContextMenuItem
            variant="destructive"
            disabled={selectedIds.length === 0}
            onSelect={() => askRemove(selectedIds)}
          >
            <Trash2 aria-hidden />
            {selectedIds.length > 1
              ? t.mediaBin.actions.removeMany(selectedIds.length)
              : t.mediaBin.actions.remove}
            <ContextMenuShortcut>{t.keys.delete}</ContextMenuShortcut>
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
      <RemoveMediaDialog
        items={removing}
        onClose={(removed) => {
          setRemoving(null);
          if (removed) setSelection(emptySelection);
        }}
      />
    </>
  );
}
