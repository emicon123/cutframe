/**
 * The media picker of "Replace media…" (F1.51): the media-panel items that can replace the clip's
 * source (same kind of track, long enough). Picking one sends one `replaceClipMedia`.
 */
import { useState } from "react";

import type { ClipId } from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { mediaDetails } from "@/features/media-bin/MediaItem";
import { Thumbnail } from "@/features/media-bin/Thumbnail";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { useProjectStore } from "@/stores/projectStore";

import { replaceClipMedia } from "./editActions";
import { findClip, replacementMedia } from "./linking";

export function ReplaceMediaDialog({ clip, onClose }: { clip: ClipId | null; onClose: () => void }) {
  const project = useProjectStore((s) => s.state?.project);
  const [busy, setBusy] = useState(false);
  const found = clip && project ? findClip(project.tracks, clip) : null;
  const open = found !== null;
  usePreviewOccluder(open);

  const choices = found && project ? replacementMedia(found.track.kind, found.clip, project.media) : [];

  const pick = async (media: string) => {
    if (!found || busy) return;
    setBusy(true);
    const done = await replaceClipMedia(found.clip.id, media);
    setBusy(false);
    if (done) onClose();
  };

  const text = t.timeline.replaceMedia;
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="sm:max-w-lg" closeLabel={text.cancel}>
        <DialogHeader>
          <DialogTitle>{text.title(found?.clip.name ?? "")}</DialogTitle>
          <DialogDescription>{text.description}</DialogDescription>
        </DialogHeader>
        {choices.length === 0 ? (
          <p className="text-sm text-muted-foreground">{text.none}</p>
        ) : (
          <ul aria-label={text.list} className="flex max-h-80 flex-col gap-1 overflow-y-auto">
            {choices.map((item) => (
              <li key={item.id}>
                <button
                  type="button"
                  disabled={busy}
                  title={item.path}
                  className="flex w-full items-center gap-2 rounded-sm px-2 py-1 text-left outline-none hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
                  onClick={() => void pick(item.id)}
                >
                  <Thumbnail item={item} className="aspect-video w-14 shrink-0" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate">{item.fileName}</span>
                    <span className="block truncate text-xs text-muted-foreground">{mediaDetails(item)}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {text.cancel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
