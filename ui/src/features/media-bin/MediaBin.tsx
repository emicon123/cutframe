import { CircleAlert, Film, FolderOpen, Image, Music, Upload, X } from "lucide-react";
import { type KeyboardEvent, useEffect, useRef } from "react";

import type { MediaKind, MediaView_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { jobStageText, mediaErrorText, t } from "@/i18n";
import { formatDuration } from "@/lib/time";
import { cn } from "@/lib/utils";
import { type ImportJob, isCleanSummary, useImportStore } from "@/stores/importStore";
import { useProjectStore } from "@/stores/projectStore";

import { fileNameOf } from "./dropTarget";
import { cancelAllImports, cancelImport, importFromDialog, importPaths } from "./importActions";
import { summaryLines } from "./importSummary";
import { useFileDrop } from "./useFileDrop";

/** How long a result with nothing to read (every file added) stays visible. */
const CLEAN_RESULT_MS = 4000;

export function MediaBin() {
  const panel = useRef<HTMLElement>(null);
  const hovering = useFileDrop(panel, (paths) => void importPaths(paths));
  const media = useProjectStore((s) => s.state?.project.media);

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape" && cancelAllImports()) {
      event.preventDefault();
      event.stopPropagation();
    }
  };

  return (
    <section
      ref={panel}
      className={cn("relative flex h-full flex-col bg-panel", hovering && "ring-2 ring-primary ring-inset")}
      aria-label={t.mediaBin.title}
      onKeyDown={onKeyDown}
    >
      <header className="flex h-9 shrink-0 items-center justify-between border-b px-3">
        <h2 className="font-semibold">{t.mediaBin.title}</h2>
        <Button
          variant="ghost"
          size="sm"
          title={t.mediaBin.importTip}
          onClick={() => void importFromDialog()}
        >
          <Upload className="size-3.5" aria-hidden />
          {t.mediaBin.import}
        </Button>
      </header>
      <ImportJobs />
      {media && media.length > 0 ? <MediaList media={media} /> : <EmptyState />}
      <ImportResult />
      {hovering && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center bg-background/70 font-medium text-primary">
          {t.mediaBin.dropHint}
        </div>
      )}
    </section>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
      <FolderOpen className="size-8 opacity-60" aria-hidden />
      <p className="font-medium text-foreground">{t.mediaBin.empty}</p>
      <p className="text-xs">{t.mediaBin.emptyHint}</p>
    </div>
  );
}

const KIND_ICONS = { video: Film, audio: Music, image: Image } satisfies Record<MediaKind, unknown>;

function mediaDetails(item: MediaView_Serialize): string {
  const parts: string[] = [t.mediaBin.kinds[item.kind]];
  if (item.duration !== null) parts.push(formatDuration(item.duration));
  if (item.displayWidth !== null && item.displayHeight !== null) {
    parts.push(t.mediaBin.resolution(item.displayWidth, item.displayHeight));
  }
  return parts.join(t.mediaBin.detailSeparator);
}

function MediaList({ media }: { media: MediaView_Serialize[] }) {
  return (
    <ul className="min-h-0 flex-1 overflow-y-auto py-1" aria-label={t.mediaBin.list}>
      {media.map((item) => {
        const Icon = KIND_ICONS[item.kind];
        return (
          <li key={item.id} className="flex items-center gap-2 px-3 py-1.5" title={item.path}>
            <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
            <div className="min-w-0">
              <p className="truncate">{item.fileName}</p>
              <p className="truncate text-xs text-muted-foreground">{mediaDetails(item)}</p>
            </div>
          </li>
        );
      })}
    </ul>
  );
}

function ImportJobs() {
  const jobs = useImportStore((s) => s.jobs);
  if (jobs.length === 0) return null;
  return (
    <div className="flex shrink-0 flex-col gap-2 border-b px-3 py-2">
      {jobs.map((job) => (
        <JobProgressRow key={job.id} job={job} />
      ))}
    </div>
  );
}

function JobProgressRow({ job }: { job: ImportJob }) {
  const label = job.cancelling ? t.mediaBin.job.cancelling : jobStageText(job.stage ?? "probing");
  const percent = job.progress === null ? null : Math.round(job.progress * 100);
  return (
    <div className="flex items-center gap-2 text-xs">
      <div className="min-w-0 flex-1">
        <div className="flex justify-between gap-2 text-muted-foreground">
          <span className="truncate">{label}</span>
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
      <Button
        variant="ghost"
        size="sm"
        title={t.mediaBin.job.cancelTip}
        disabled={job.cancelling}
        onClick={() => cancelImport(job.id)}
      >
        {t.mediaBin.job.cancel}
      </Button>
    </div>
  );
}

function ImportResult() {
  const summary = useImportStore((s) => s.summary);
  const dispatch = useImportStore((s) => s.dispatch);
  const clean = summary !== null && isCleanSummary(summary);

  useEffect(() => {
    if (!clean) return;
    const timer = setTimeout(() => dispatch({ type: "dismissSummary" }), CLEAN_RESULT_MS);
    return () => clearTimeout(timer);
  }, [clean, summary, dispatch]);

  if (!summary) return null;
  const problem = summary.jobError !== null || summary.failed.length > 0;
  return (
    <div
      role="status"
      className="m-2 flex shrink-0 gap-2 rounded-md border bg-popover p-2 text-xs text-popover-foreground shadow-md"
    >
      {problem && <CircleAlert className="size-4 shrink-0 text-destructive" aria-hidden />}
      <div className="min-w-0 flex-1">
        {summaryLines(summary).map((line) => (
          <p key={line} className="font-medium">
            {line}
          </p>
        ))}
        {summary.failed.length > 0 && (
          <ul className="mt-1 max-h-32 overflow-y-auto text-muted-foreground">
            {summary.failed.map((file, index) => (
              <li key={`${file.path}-${index}`} className="truncate" title={file.path}>
                {t.mediaBin.result.fileReason(fileNameOf(file.path), mediaErrorText(file.error))}
              </li>
            ))}
          </ul>
        )}
      </div>
      <Button
        variant="ghost"
        size="icon"
        className="size-6 shrink-0"
        aria-label={t.mediaBin.result.dismiss}
        onClick={() => dispatch({ type: "dismissSummary" })}
      >
        <X className="size-3.5" aria-hidden />
      </Button>
    </div>
  );
}
