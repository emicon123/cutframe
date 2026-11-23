import { FolderSearch, Link, ListStart, Plus, Trash2, Upload } from "lucide-react";
import { type KeyboardEvent, type MouseEvent, type PointerEvent, useEffect, useRef, useState } from "react";

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
import { appendMedia, insertMediaAtPlayhead } from "@/features/timeline/addActions";
import { cn } from "@/lib/utils";
import { cancelMediaDrag, finishMediaDrag, moveMediaDrag, startMediaDrag } from "@/stores/mediaDragStore";
import type { MediaLayout } from "@/stores/uiStore";

import { importFromDialog, revealMedia } from "./importActions";
import { MediaDragChip } from "./MediaDragChip";
import { MediaItem } from "./MediaItem";
import { needsRelink } from "./relink";
import { openRelink } from "./relinkStore";
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

const isAddButton = (target: EventTarget | null) =>
  target instanceof Element && target.closest("[data-action='append']") !== null;

/** Pointer travel (CSS px) before a press on an item becomes a drag. */
const DRAG_THRESHOLD = 4;

type DragState = {
  pending: { id: MediaId; x: number; y: number } | null;
  dragging: boolean;
  suppressClick: boolean;
};

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
  const drag = useRef<DragState>({ pending: null, dragging: false, suppressClick: false });
  usePreviewOccluder(menuOpen);

  useEffect(() => {
    // Switching windows mid-drag drops nothing.
    const onBlur = () => {
      if (drag.current.dragging) cancelMediaDrag();
      drag.current = { pending: null, dragging: false, suppressClick: false };
    };
    window.addEventListener("blur", onBlur);
    return () => window.removeEventListener("blur", onBlur);
  }, []);

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

  /** The items an action on `id` is about: the selection if `id` is in it, else just `id`. */
  const targetsOf = (id: MediaId) => (selected.has(id) ? selectedIds : [id]);

  const onClick = (event: MouseEvent) => {
    if (drag.current.suppressClick) {
      drag.current.suppressClick = false;
      return;
    }
    const id = idOf(event.target);
    if (id !== null && isAddButton(event.target)) {
      const ids = targetsOf(id);
      void (event.altKey ? insertMediaAtPlayhead(ids) : appendMedia(ids));
      return;
    }
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

  // Dragging items to the timeline: pointer events only (media design §2), no HTML5 drag-and-drop.
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    const id = idOf(event.target);
    if (event.button !== 0 || id === null || isAddButton(event.target)) return;
    drag.current = {
      pending: { id, x: event.clientX, y: event.clientY },
      dragging: false,
      suppressClick: false,
    };
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const state = drag.current;
    if (state.dragging) {
      moveMediaDrag(event.clientX, event.clientY, event.altKey);
      return;
    }
    const pending = state.pending;
    if (!pending || (event.buttons & 1) === 0) return;
    if (Math.hypot(event.clientX - pending.x, event.clientY - pending.y) < DRAG_THRESHOLD) return;
    state.dragging = true;
    event.currentTarget.setPointerCapture(event.pointerId);
    if (!selected.has(pending.id)) apply({ type: "click", id: pending.id, ctrl: false, shift: false, order });
    startMediaDrag(targetsOf(pending.id), event.clientX, event.clientY, event.altKey);
  };

  const onPointerUp = () => {
    const state = drag.current;
    if (state.dragging) {
      finishMediaDrag();
      // The click after a drag lands on the list (pointer capture); it must not clear the selection.
      state.suppressClick = true;
    }
    state.pending = null;
    state.dragging = false;
  };

  const endDrag = () => {
    if (drag.current.dragging) cancelMediaDrag();
    drag.current = { pending: null, dragging: false, suppressClick: false };
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape" && drag.current.dragging) {
      endDrag();
      event.preventDefault();
      event.stopPropagation();
      return;
    }
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
  const toRelink = media.filter((item) => selected.has(item.id) && needsRelink(item));
  // One item: its own dialog; several: the dialog for every item that needs a relink.
  const relink = () => openRelink(toRelink.length === 1 && toRelink[0] ? toRelink[0].id : null);

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
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerCancel={endDrag}
            onLostPointerCapture={() => {
              if (drag.current.dragging) endDrag();
            }}
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
          <ContextMenuItem disabled={selectedIds.length === 0} onSelect={() => void appendMedia(selectedIds)}>
            <Plus aria-hidden />
            {t.mediaBin.add.append}
          </ContextMenuItem>
          <ContextMenuItem
            disabled={selectedIds.length === 0}
            onSelect={() => void insertMediaAtPlayhead(selectedIds)}
          >
            <ListStart aria-hidden />
            {t.mediaBin.add.insert}
          </ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem disabled={toRelink.length === 0} onSelect={relink}>
            <Link aria-hidden />
            {t.mediaBin.relink.action}
          </ContextMenuItem>
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
      <MediaDragChip media={media} />
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
