import { CircleAlert, FolderOpen, Loader, Upload, X } from "lucide-react";
import { type KeyboardEvent, useDeferredValue, useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";

import type { MediaView_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { jobStageText, mediaErrorText, t } from "@/i18n";
import { cn } from "@/lib/utils";
import {
  type ImportJob,
  isBackgroundJob,
  isCleanSummary,
  isImportJob,
  useImportStore,
} from "@/stores/importStore";
import { useProjectStore } from "@/stores/projectStore";
import { useUiStore } from "@/stores/uiStore";

import { browseMedia } from "./browse";
import { fileNameOf } from "./dropTarget";
import { cancelAllImports, cancelImport, importFromDialog, importPaths } from "./importActions";
import { summaryLines } from "./importSummary";
import { MediaBrowser } from "./MediaBrowser";
import { MediaToolbar } from "./MediaToolbar";
import { countMissing } from "./relink";
import { RelinkDialog } from "./RelinkDialog";
import { openRelink } from "./relinkStore";
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
      // Timeline shortcuts (Delete, Ctrl+C/X/V/D, Ctrl+A) do not act while focus is in here.
      data-shortcut-scope="media-bin"
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
      {media && <MissingNotice media={media} />}
      {media && media.length > 0 ? <MediaContent media={media} /> : <EmptyState />}
      <ImportResult />
      <BackgroundJobs />
      <RelinkDialog />
      {hovering && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center bg-background/70 font-medium text-primary">
          {t.mediaBin.dropHint}
        </div>
      )}
    </section>
  );
}

/** "N files missing" after open (or when a file disappears), with a way into the relink dialog. */
function MissingNotice({ media }: { media: MediaView_Serialize[] }) {
  const { offline, changed } = countMissing(media);
  const key = `${offline}/${changed}`;
  const [dismissed, setDismissed] = useState<string | null>(null);
  if (offline + changed === 0 || dismissed === key) return null;
  return (
    <div
      role="status"
      className="flex shrink-0 items-center gap-2 border-b bg-destructive/10 px-3 py-1.5 text-xs"
    >
      <CircleAlert className="size-4 shrink-0 text-destructive" aria-hidden />
      <span className="min-w-0 flex-1 truncate font-medium">{t.mediaBin.notice.text(offline, changed)}</span>
      <Button variant="outline" size="xs" onClick={() => openRelink(null)}>
        {t.mediaBin.notice.relink}
      </Button>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={t.mediaBin.notice.dismiss}
        onClick={() => setDismissed(key)}
      >
        <X aria-hidden />
      </Button>
    </div>
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

/** Search, filter and sort over the project's media, then the grid or list. */
function MediaContent({ media }: { media: MediaView_Serialize[] }) {
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const { layout, sort, direction, kinds, setKinds } = useUiStore(
    useShallow((s) => ({
      layout: s.mediaLayout,
      sort: s.mediaSort,
      direction: s.mediaSortDirection,
      kinds: s.mediaKinds,
      setKinds: s.setMediaKinds,
    })),
  );
  const visible = browseMedia(media, { sort, direction, kinds, query: deferredQuery });

  return (
    <>
      <MediaToolbar query={query} onQueryChange={setQuery} />
      {visible.length > 0 ? (
        <MediaBrowser media={visible} layout={layout} />
      ) : (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-xs text-muted-foreground">
          <p>{t.mediaBin.noMatches}</p>
          <Button
            variant="outline"
            size="xs"
            onClick={() => {
              setQuery("");
              setKinds([]);
            }}
          >
            {t.mediaBin.clearFilters}
          </Button>
        </div>
      )}
    </>
  );
}

/** Slim, unobtrusive progress of preview generation that Rust runs by itself (F1.23). */
function BackgroundJobs() {
  const jobs = useImportStore(useShallow((s) => s.jobs.filter(isBackgroundJob)));
  const job = jobs[0];
  if (!job) return null;
  const percent = job.progress === null ? null : Math.round(job.progress * 100);
  const label = jobStageText(job.stage ?? "generatingPreviews");
  return (
    <div
      className="flex shrink-0 items-center gap-2 border-t px-3 py-1 text-xs text-muted-foreground"
      title={t.mediaBin.background}
    >
      <Loader className="size-3 shrink-0 animate-spin" aria-hidden />
      <span className="truncate">{label}</span>
      <div
        role="progressbar"
        aria-label={t.mediaBin.background}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        className="h-0.5 min-w-8 flex-1 overflow-hidden rounded-full bg-muted"
      >
        <div
          className={cn("h-full rounded-full bg-primary/70", percent === null && "w-full animate-pulse")}
          style={percent === null ? undefined : { width: `${percent}%` }}
        />
      </div>
      {percent !== null && <span className="tabular-nums">{t.mediaBin.job.percent(percent)}</span>}
    </div>
  );
}

function ImportJobs() {
  const jobs = useImportStore(useShallow((s) => s.jobs.filter(isImportJob)));
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
