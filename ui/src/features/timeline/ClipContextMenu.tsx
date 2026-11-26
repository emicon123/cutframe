/**
 * The right-click menu of the timeline lanes (F1.56): the clip actions of the registry for the
 * clip under the pointer (selected first, with its link group), or paste and select all on an
 * empty lane. Replace media opens a picker of compatible media from the media panel (F1.51).
 */
import { Replace } from "lucide-react";
import { type MouseEvent, type ReactNode, useState } from "react";

import type { ClipId } from "@/bindings";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  actionLabel,
  getAction,
  isEnabled,
  shortcutLabel,
  useActionContext,
} from "@/features/editor/actions";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { useProjectStore } from "@/stores/projectStore";
import { selectionStore, setSelection } from "@/stores/selectionStore";

import type { Hit } from "./hitTest";
import { canReplaceMedia, findClip } from "./linking";
import { contextMenuItems, type MenuEntry, type MenuTarget } from "./menus";
import { ReplaceMediaDialog } from "./ReplaceMediaDialog";
import { pressSelection } from "./selection";

const NO_MODIFIERS = { ctrl: false, shift: false };

export function ClipContextMenu({
  hitAt,
  children,
}: {
  /** What is under the client point, `null` outside the lanes. */
  hitAt: (clientX: number, clientY: number) => Hit | null;
  children: ReactNode;
}) {
  const [target, setTarget] = useState<MenuTarget>({ kind: "lane" });
  const [menuOpen, setMenuOpen] = useState(false);
  const [replacing, setReplacing] = useState<ClipId | null>(null);
  usePreviewOccluder(menuOpen);

  const onContextMenu = (event: MouseEvent) => {
    const hit = hitAt(event.clientX, event.clientY);
    if (!hit || hit.kind === "ruler") {
      // No menu on the ruler, and never the WebView's own menu.
      event.preventDefault();
      return;
    }
    if (hit.kind === "clip") {
      const tracks = useProjectStore.getState().state?.project.tracks ?? [];
      // An unselected clip is selected first (with its link group); a selected one keeps the selection.
      setSelection(pressSelection(tracks, selectionStore.getState().clips, hit.clip.id, NO_MODIFIERS));
      setTarget({ kind: "clip", clip: hit.clip.id });
    } else {
      setTarget({ kind: "lane" });
    }
  };

  return (
    <>
      <ContextMenu onOpenChange={setMenuOpen}>
        <ContextMenuTrigger asChild onContextMenu={onContextMenu}>
          {children}
        </ContextMenuTrigger>
        <ContextMenuContent className="min-w-56">
          {contextMenuItems(target.kind).map((entry, index) => (
            <MenuItem
              key={entry === "-" ? `sep-${index}` : entry}
              entry={entry}
              clip={target.kind === "clip" ? target.clip : null}
              onReplace={setReplacing}
            />
          ))}
        </ContextMenuContent>
      </ContextMenu>
      <ReplaceMediaDialog clip={replacing} onClose={() => setReplacing(null)} />
    </>
  );
}

function MenuItem({
  entry,
  clip,
  onReplace,
}: {
  entry: MenuEntry;
  clip: ClipId | null;
  onReplace: (clip: ClipId) => void;
}) {
  if (entry === "-") return <ContextMenuSeparator />;
  if (entry === "replace-media") return <ReplaceMediaItem clip={clip} onReplace={onReplace} />;
  return <ActionItem id={entry} />;
}

/** A registered action: the same label, shortcut and enabled state as in the Edit menu. */
function ActionItem({ id }: { id: Exclude<MenuEntry, "-" | "replace-media"> }) {
  const ctx = useActionContext();
  const action = getAction(id);
  const shortcut = shortcutLabel(action);
  return (
    <ContextMenuItem
      disabled={!isEnabled(action, ctx)}
      variant={id === "delete" || id === "ripple-delete" ? "destructive" : "default"}
      onSelect={action.run}
    >
      {actionLabel(action, ctx)}
      {shortcut && <ContextMenuShortcut>{shortcut}</ContextMenuShortcut>}
    </ContextMenuItem>
  );
}

/** Replace media acts on the right-clicked clip, when it is a media clip on a video or audio track. */
function ReplaceMediaItem({ clip, onReplace }: { clip: ClipId | null; onReplace: (clip: ClipId) => void }) {
  const enabled = useProjectStore((s) => {
    const found = clip && s.state && findClip(s.state.project.tracks, clip);
    return Boolean(found && canReplaceMedia(found.track.kind, found.clip));
  });
  return (
    <ContextMenuItem disabled={!enabled} onSelect={() => clip && onReplace(clip)}>
      <Replace aria-hidden />
      {t.timeline.replaceMedia.action}
    </ContextMenuItem>
  );
}
