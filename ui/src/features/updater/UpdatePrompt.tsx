import { CircleAlert, Download, RotateCw } from "lucide-react";
import { useId, useState } from "react";

import type { UpdateInfo_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { formatBytes } from "@/features/preferences/preferences";
import { t } from "@/i18n";
import { cn } from "@/lib/utils";

import { downloadPercent, type UpdateStatus } from "./updateState";
import { hideUpdatePrompt, installUpdate, restartToUpdate, updateErrorText, useUpdateStore } from "./updater";

/**
 * The non-modal update prompt (F0.12): offer → download progress → "Restart to update", or the
 * failure with Retry. It sits in the toast corner (bottom right, over the timeline or inspector,
 * never the native preview), so it needs no preview occluder (ADR-0005).
 */
export function UpdatePrompt() {
  const status = useUpdateStore((s) => s.status);
  const hidden = useUpdateStore((s) => s.hidden);
  const restarting = useUpdateStore((s) => s.restarting);
  const titleId = useId();
  if (status.phase === "none" || hidden) return null;

  return (
    <section
      aria-labelledby={titleId}
      className="pointer-events-auto grid gap-2 rounded-md border bg-popover p-3 text-popover-foreground shadow-lg data-[phase=failed]:border-destructive/60"
      data-phase={status.phase}
    >
      <div className="flex items-start gap-2">
        <PhaseIcon phase={status.phase} />
        <div className="grid min-w-0 flex-1 gap-1">
          <h2 id={titleId} className="font-medium break-words">
            {title(status)}
          </h2>
          <Body status={status} />
        </div>
      </div>
      <div className="flex justify-end gap-2">
        <Button variant="ghost" size="xs" onClick={hideUpdatePrompt}>
          {t.updater.later}
        </Button>
        {status.phase === "available" && (
          <Button size="xs" onClick={() => void installUpdate()}>
            {t.updater.update}
          </Button>
        )}
        {status.phase === "failed" && (
          <Button size="xs" onClick={() => void installUpdate()}>
            {t.updater.retry}
          </Button>
        )}
        {status.phase === "ready" && (
          <Button size="xs" disabled={restarting} onClick={() => void restartToUpdate()}>
            {t.updater.restart}
          </Button>
        )}
      </div>
    </section>
  );
}

function title(status: Exclude<UpdateStatus, { phase: "none" }>): string {
  switch (status.phase) {
    case "available":
      return t.updater.available(status.info.version);
    case "downloading":
      return t.updater.downloading(status.info.version);
    case "ready":
      return t.updater.ready(status.info.version);
    case "failed":
      return t.updater.failed;
  }
}

function PhaseIcon({ phase }: { phase: UpdateStatus["phase"] }) {
  const Icon = phase === "failed" ? CircleAlert : phase === "ready" ? RotateCw : Download;
  return (
    <Icon
      className={cn("mt-0.5 size-4 shrink-0", phase === "failed" ? "text-destructive" : "text-primary")}
      aria-hidden
    />
  );
}

function Body({ status }: { status: Exclude<UpdateStatus, { phase: "none" }> }) {
  switch (status.phase) {
    case "available":
      return (
        <>
          <p className="text-xs text-muted-foreground">
            {t.updater.currentVersion(status.info.currentVersion)}
          </p>
          <ReleaseNotes info={status.info} />
        </>
      );
    case "downloading":
      return <DownloadProgress downloaded={status.downloaded} total={status.total} />;
    case "ready":
      return <p className="text-xs text-muted-foreground">{t.updater.readyHint}</p>;
    case "failed":
      return <p className="text-xs break-words text-muted-foreground">{updateErrorText(status.error)}</p>;
  }
}

/** Release notes as the release states them (not translated), collapsed by default. */
function ReleaseNotes({ info }: { info: UpdateInfo_Serialize }) {
  const [open, setOpen] = useState(false);
  const notesId = useId();
  const notes = info.notes?.trim();
  if (!notes) return null;
  return (
    <>
      <Button
        variant="link"
        size="xs"
        className="h-auto w-fit p-0 text-xs"
        aria-expanded={open}
        aria-controls={notesId}
        onClick={() => setOpen((o) => !o)}
      >
        {open ? t.updater.hideNotes : t.updater.showNotes}
      </Button>
      {open && (
        <p
          id={notesId}
          className="max-h-40 overflow-y-auto rounded-sm bg-muted p-2 text-xs break-words whitespace-pre-wrap"
        >
          {notes}
        </p>
      )}
    </>
  );
}

function DownloadProgress({ downloaded, total }: { downloaded: string; total: string | null }) {
  const percent = downloadPercent(downloaded, total);
  const text =
    percent !== null && total !== null
      ? t.updater.progressOf(formatBytes(downloaded), formatBytes(total), percent)
      : formatBytes(downloaded);
  return (
    <div className="grid gap-1 text-xs text-muted-foreground">
      <div
        role="progressbar"
        aria-label={t.updater.progressLabel}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        aria-valuetext={text}
        className="h-1 overflow-hidden rounded-full bg-muted"
      >
        <div
          className={cn("h-full rounded-full bg-primary", percent === null && "w-full animate-pulse")}
          style={percent === null ? undefined : { width: `${percent}%` }}
        />
      </div>
      <span>{text}</span>
    </div>
  );
}
