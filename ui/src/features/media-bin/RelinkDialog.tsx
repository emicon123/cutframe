import { CircleAlert } from "lucide-react";
import { useShallow } from "zustand/react/shallow";

import type { MediaView_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { errorKindText, jobStageText, mediaErrorText, t } from "@/i18n";
import { cn } from "@/lib/utils";
import { useImportStore } from "@/stores/importStore";
import { useProjectStore } from "@/stores/projectStore";

import { fileNameOf } from "./dropTarget";
import { needsRelink, type RelinkResult } from "./relink";
import {
  acceptMismatch,
  cancelRelink,
  closeRelink,
  keepAsIs,
  locateFile,
  rejectMismatch,
  searchFolder,
  useRelinkStore,
} from "./relinkStore";

const EMPTY: MediaView_Serialize[] = [];

/**
 * Relink one item (Locate file…, or "use as it is now" for a changed file) or every missing item
 * at once (Search folder…), with the results and the content-mismatch question (design §6.6).
 */
export function RelinkDialog() {
  const { open, focus, running, mismatch, result } = useRelinkStore(
    useShallow((s) => ({
      open: s.open,
      focus: s.focus,
      running: s.running,
      mismatch: s.mismatch,
      result: s.result,
    })),
  );
  const media = useProjectStore((s) => s.state?.project.media ?? EMPTY);
  usePreviewOccluder(open);

  const focused = focus === null ? undefined : media.find((item) => item.id === focus);
  // The focused item stays listed after it is relinked; the full list shrinks as files are found.
  const items = focus === null ? media.filter(needsRelink) : focused ? [focused] : [];
  const anyOffline = media.some((item) => item.status === "offline");
  const busy = running !== null;

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) closeRelink();
      }}
    >
      <DialogContent className="sm:max-w-lg" closeLabel={t.mediaBin.relink.close}>
        <DialogHeader>
          <DialogTitle>
            {focused ? t.mediaBin.relink.titleOne(focused.fileName) : t.mediaBin.relink.titleAll}
          </DialogTitle>
          <DialogDescription>
            {focused && focused.status !== "online"
              ? t.mediaBin.status.tip[focused.status]
              : t.mediaBin.relink.description}
          </DialogDescription>
        </DialogHeader>

        {items.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t.mediaBin.relink.allFound}</p>
        ) : (
          <ul
            className="max-h-60 divide-y overflow-y-auto rounded-md border text-sm"
            aria-label={t.mediaBin.relink.titleAll}
          >
            {items.map((item) => (
              <RelinkRow key={item.id} item={item} disabled={busy} />
            ))}
          </ul>
        )}

        {mismatch && !busy && (
          <div role="alert" className="flex flex-col gap-2 rounded-md border border-warning/50 p-2 text-sm">
            <p className="flex gap-2">
              <CircleAlert className="size-4 shrink-0 text-warning" aria-hidden />
              {t.mediaBin.relink.mismatch}
            </p>
            <p className="truncate text-xs text-muted-foreground" title={mismatch.path}>
              {mismatch.path}
            </p>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={rejectMismatch}>
                {t.mediaBin.relink.cancel}
              </Button>
              <Button size="sm" onClick={() => void acceptMismatch()}>
                {t.mediaBin.relink.useAnyway}
              </Button>
            </div>
          </div>
        )}

        {busy && <RelinkProgress jobId={running.jobId} cancellable={running.mode === "folder"} />}
        {result && !busy && !mismatch && <RelinkResultLines result={result} media={media} />}

        <DialogFooter>
          <Button variant="secondary" disabled={busy || !anyOffline} onClick={() => void searchFolder()}>
            {t.mediaBin.relink.searchFolder}
          </Button>
          <Button variant="ghost" onClick={closeRelink}>
            {t.mediaBin.relink.close}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function RelinkRow({ item, disabled }: { item: MediaView_Serialize; disabled: boolean }) {
  return (
    <li className="flex items-center gap-2 px-2 py-1.5">
      <div className="min-w-0 flex-1">
        <p className="truncate font-medium">{item.fileName}</p>
        <p className="truncate text-xs text-muted-foreground" title={item.path}>
          {item.path}
        </p>
      </div>
      {item.status !== "online" && (
        <span
          className={cn(
            "text-xs font-semibold",
            item.status === "offline" ? "text-destructive" : "text-warning",
          )}
        >
          {t.mediaBin.status.badge[item.status]}
        </span>
      )}
      {item.status === "changed" && (
        <Button
          variant="ghost"
          size="xs"
          disabled={disabled}
          aria-label={t.mediaBin.relink.keepAsIsItem(item.fileName)}
          onClick={() => void keepAsIs(item.id)}
        >
          {t.mediaBin.relink.keepAsIs}
        </Button>
      )}
      <Button
        variant="outline"
        size="xs"
        disabled={disabled}
        aria-label={t.mediaBin.relink.locateItem(item.fileName)}
        onClick={() => void locateFile(item.id)}
      >
        {t.mediaBin.relink.locate}
      </Button>
    </li>
  );
}

function RelinkProgress({ jobId, cancellable }: { jobId: string | null; cancellable: boolean }) {
  const job = useImportStore(useShallow((s) => s.jobs.find((j) => j.id === jobId) ?? null));
  const percent = job?.progress == null ? null : Math.round(job.progress * 100);
  const label = jobStageText(job?.stage ?? (cancellable ? "searchingFolder" : "probing"));
  return (
    <div className="flex items-center gap-2 text-xs">
      <div className="min-w-0 flex-1">
        <div className="flex justify-between text-muted-foreground">
          <span>{label}</span>
          {percent !== null && <span>{t.mediaBin.job.percent(percent)}</span>}
        </div>
        <div
          role="progressbar"
          aria-label={label}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent ?? undefined}
          className="mt-1 h-1 overflow-hidden rounded-full bg-muted"
        >
          <div
            className={cn("h-full rounded-full bg-primary", percent === null && "w-full animate-pulse")}
            style={percent === null ? undefined : { width: `${percent}%` }}
          />
        </div>
      </div>
      {cancellable && (
        <Button variant="ghost" size="sm" disabled={jobId === null} onClick={cancelRelink}>
          {t.mediaBin.relink.cancel}
        </Button>
      )}
    </div>
  );
}

function RelinkResultLines({
  result,
  media,
}: {
  result: RelinkResult;
  media: readonly MediaView_Serialize[];
}) {
  const { outcomes } = t.mediaBin.relink;
  const name = (id: string) => media.find((item) => item.id === id)?.fileName ?? id;
  const lines: string[] = [];
  if (result.cancelled) lines.push(outcomes.cancelled);
  if (result.jobError) lines.push(t.mediaBin.relink.jobFailed(errorKindText(result.jobError)));
  if (result.relinked.length > 0) lines.push(outcomes.relinked(result.relinked.length));
  if (result.notFound.length > 0) lines.push(outcomes.notFound(result.notFound.length));
  if (lines.length === 0 && result.failed.length === 0) lines.push(outcomes.nothingMissing);
  const problem = result.jobError !== null || result.failed.length > 0 || result.notFound.length > 0;
  return (
    <div role="status" className="flex gap-2 rounded-md bg-muted/50 p-2 text-sm">
      {problem && <CircleAlert className="size-4 shrink-0 text-destructive" aria-hidden />}
      <div className="min-w-0">
        {lines.map((line) => (
          <p key={line} className="font-medium">
            {line}
          </p>
        ))}
        <ul className="text-xs text-muted-foreground">
          {result.notFound.map((id) => (
            <li key={id} className="truncate">
              {name(id)}
            </li>
          ))}
          {result.failed.map((file, index) => (
            <li key={`${file.path}-${index}`} className="truncate" title={file.path}>
              {t.mediaBin.result.fileReason(fileNameOf(file.path), mediaErrorText(file.error))}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
