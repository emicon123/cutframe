import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ChevronDown, CircleAlert, TriangleAlert } from "lucide-react";
import { type ReactNode, useEffect, useId, useState } from "react";

import {
  commands,
  type ExportCheck,
  type ExportResolution,
  type ExportSettings_Deserialize,
  type MediaId,
  type ProjectView,
  type VideoCodec,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { openRelink } from "@/features/media-bin/relinkStore";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { describeError, isIpcError, logError } from "@/ipc/errors";
import { formatFrameRate } from "@/lib/time";
import { cn } from "@/lib/utils";
import { useProjectStore } from "@/stores/projectStore";

import { closeExportDialog, exportStarted, mediaName, setCapabilities, useExportStore } from "./exportStore";
import { defaultExportFolder, rememberFolder } from "./folder";
import { type ExportPresetId, PRESET_IDS, presetLabel, type VideoChoice } from "./presets";
import {
  applyPreset,
  availableEncoders,
  buildRequest,
  type ExportForm,
  exportFileName,
  fixedRateOptions,
  frameRateValue,
  hevcAvailable,
  initialForm,
  parseFrameRateValue,
  projectExportRate,
  QUALITIES,
  RESOLUTIONS,
  withCodec,
} from "./settings";
import { freeSpaceText, type IssueView, issueViews, summaryLine } from "./summary";

/** `checkExport` runs this long after the last change (export.md §7). */
export const CHECK_DEBOUNCE_MS = 150;

const CODECS: readonly VideoCodec[] = ["h264", "hevc"];

/**
 * File → Export… (export.md §2, §7). Hides the native preview while open (ADR-0005 §4); the form
 * mounts with the dialog, so it starts from the project and the remembered folder each time.
 */
export function ExportDialog() {
  const open = useExportStore((s) => s.dialogOpen);
  const project = useProjectStore((s) => (open ? (s.state?.project ?? null) : null));
  usePreviewOccluder(open);

  return (
    <Dialog open={open && project !== null} onOpenChange={(next) => !next && closeExportDialog()}>
      <DialogContent className="sm:max-w-xl" closeLabel={t.export.close}>
        <DialogHeader>
          <DialogTitle>{t.export.title}</DialogTitle>
          <DialogDescription>{t.export.description}</DialogDescription>
        </DialogHeader>
        {project && <ExportFormView project={project} />}
      </DialogContent>
    </Dialog>
  );
}

type Checked = { key: string; check: ExportCheck } | { key: string; error: unknown };

function ExportFormView({ project }: { project: ProjectView }) {
  const capabilities = useExportStore((s) => s.capabilities);
  const revision = useProjectStore((s) => s.state?.revision ?? 0);
  const [form, setForm] = useState<ExportForm>(() => initialForm(project.name, project.settings, ""));
  const [customOpen, setCustomOpen] = useState(false);
  const [checked, setChecked] = useState<Checked | null>(null);
  const [replaceKey, setReplaceKey] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const ids = {
    preset: useId(),
    name: useId(),
    folder: useId(),
    resolution: useId(),
    rate: useId(),
    codec: useId(),
    quality: useId(),
    bitrate: useId(),
    encoder: useId(),
  };

  // The session's probe starts with the first call; its result also arrives as an event.
  useEffect(() => {
    commands.getExportCapabilities().then(setCapabilities, logError);
  }, []);

  useEffect(() => {
    let live = true;
    void defaultExportFolder().then((folder) => {
      if (live) setForm((f) => (f.folder === "" ? { ...f, folder } : f));
    });
    return () => {
      live = false;
    };
  }, []);

  const request = buildRequest(form);
  const requestKey = request.ok ? JSON.stringify(request.settings) : null;

  // Debounced; a newer change or the dialog closing drops an answer still in flight. The project
  // revision (a relink, media going offline) and the probe finishing re-check the same settings.
  useEffect(() => {
    if (requestKey === null) return;
    let live = true;
    const timer = window.setTimeout(() => {
      const settings = JSON.parse(requestKey) as ExportSettings_Deserialize;
      commands.checkExport(settings).then(
        (check) => live && setChecked({ key: requestKey, check }),
        (error: unknown) => live && setChecked({ key: requestKey, error }),
      );
    }, CHECK_DEBOUNCE_MS);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [requestKey, revision, capabilities?.probed]);

  const current = checked !== null && checked.key === requestKey ? checked : null;
  const check = checked && "check" in checked ? checked.check : null;
  const checkError = current && "error" in current ? current.error : null;
  const currentCheck = current && "check" in current ? current.check : null;
  const label = presetLabel(form.video, form.preset);
  const confirming = replaceKey !== null && replaceKey === requestKey;
  const canStart = request.ok && currentCheck !== null && currentCheck.canStart && !starting;
  const fileName = exportFileName(form.name);

  const setVideo = (video: VideoChoice) => setForm((f) => ({ ...f, video }));

  const start = async (overwrite: boolean) => {
    if (!request.ok || requestKey === null) return;
    setStarting(true);
    setStartError(null);
    try {
      const id = await commands.startExport(request.settings, overwrite);
      rememberFolder(form.folder);
      exportStarted(id);
      closeExportDialog();
    } catch (error) {
      setStarting(false);
      if (isIpcError(error) && error.kind === "fileExists") {
        // The file appeared after the last check: ask now.
        setReplaceKey(requestKey);
        return;
      }
      logError(error);
      setStartError(describeError(error));
    }
  };

  const submit = () => {
    if (!canStart) return;
    if (confirming) {
      void start(true);
    } else if (currentCheck.issues.some((issue) => issue.kind === "fileExists")) {
      setReplaceKey(requestKey);
    } else {
      void start(false);
    }
  };

  const browse = async () => {
    try {
      const folder = await openDialog({
        directory: true,
        multiple: false,
        title: t.export.browseTitle,
        defaultPath: form.folder.trim() || undefined,
      });
      if (folder !== null) setForm((f) => ({ ...f, folder }));
    } catch (error) {
      logError(error);
      setStartError(describeError(error));
    }
  };

  const relink = (media: MediaId) => {
    closeExportDialog();
    openRelink(media);
  };

  const fieldError = !request.ok ? t.export.formErrors[request.error] : null;
  const projectRate = projectExportRate(project.settings.frameRate);
  const hevc = hevcAvailable(capabilities);
  const encoders = availableEncoders(capabilities, form.video.codec);
  const autoEncoder = form.video.encoder === null ? (currentCheck?.encoder ?? null) : null;
  const issues = check ? issueViews(check, label, mediaName) : [];

  return (
    <form
      className="grid gap-4"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <Field label={t.export.preset} htmlFor={ids.preset}>
        <NativeSelect
          id={ids.preset}
          value={label}
          onChange={(e) => {
            const value = e.target.value;
            if (value === "custom") setCustomOpen(true);
            else setForm((f) => applyPreset(f, value as ExportPresetId));
          }}
        >
          {[...PRESET_IDS, "custom" as const].map((id) => (
            <option key={id} value={id}>
              {t.export.presets[id]}
            </option>
          ))}
        </NativeSelect>
      </Field>

      <Field label={t.export.fileName} htmlFor={ids.name}>
        <div className="flex items-center gap-1.5">
          <Input
            id={ids.name}
            value={form.name}
            spellCheck={false}
            aria-invalid={request.ok ? undefined : request.error === "name"}
            onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
          />
          <span className="text-muted-foreground">{t.export.extension}</span>
        </div>
      </Field>

      <Field label={t.export.folder} htmlFor={ids.folder}>
        <div className="flex gap-2">
          <Input
            id={ids.folder}
            value={form.folder}
            spellCheck={false}
            aria-invalid={request.ok ? undefined : request.error === "folder"}
            onChange={(e) => setForm((f) => ({ ...f, folder: e.target.value }))}
          />
          <Button type="button" variant="outline" onClick={() => void browse()}>
            {t.export.browse}
          </Button>
        </div>
      </Field>

      <div className="grid gap-3">
        <button
          type="button"
          aria-expanded={customOpen}
          onClick={() => setCustomOpen((o) => !o)}
          className="flex w-fit items-center gap-1 rounded-sm text-sm text-muted-foreground hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
        >
          <ChevronDown
            className={cn("size-4 transition-transform", !customOpen && "-rotate-90")}
            aria-hidden
          />
          {t.export.custom}
        </button>
        {customOpen && (
          <div className="grid grid-cols-2 gap-3">
            <Field label={t.export.resolution} htmlFor={ids.resolution}>
              <NativeSelect
                id={ids.resolution}
                value={form.video.resolution}
                onChange={(e) => setVideo({ ...form.video, resolution: e.target.value as ExportResolution })}
              >
                {RESOLUTIONS.map((r) => (
                  <option key={r} value={r}>
                    {t.export.resolutions[r]}
                  </option>
                ))}
              </NativeSelect>
            </Field>
            <Field label={t.export.frameRate} htmlFor={ids.rate}>
              <NativeSelect
                id={ids.rate}
                value={frameRateValue(form.video.frameRate)}
                onChange={(e) => setVideo({ ...form.video, frameRate: parseFrameRateValue(e.target.value) })}
              >
                <option value={frameRateValue(null)}>
                  {t.export.projectRate(formatFrameRate(projectRate))}
                </option>
                {fixedRateOptions(project.settings.frameRate).map((rate) => (
                  <option key={frameRateValue(rate)} value={frameRateValue(rate)}>
                    {formatFrameRate(rate)}
                  </option>
                ))}
              </NativeSelect>
            </Field>
            <Field label={t.export.codec} htmlFor={ids.codec}>
              <NativeSelect
                id={ids.codec}
                value={form.video.codec}
                onChange={(e) => setVideo(withCodec(form.video, e.target.value as VideoCodec, capabilities))}
              >
                {CODECS.map((codec) => (
                  <option key={codec} value={codec} disabled={codec === "hevc" && !hevc}>
                    {codec === "hevc" && !hevc ? t.export.hevcUnavailable : t.export.codecs[codec]}
                  </option>
                ))}
              </NativeSelect>
            </Field>
            <Field label={t.export.quality} htmlFor={ids.quality}>
              <NativeSelect
                id={ids.quality}
                value={form.video.quality}
                onChange={(e) =>
                  setVideo({ ...form.video, quality: e.target.value as VideoChoice["quality"] })
                }
              >
                {QUALITIES.map((q) => (
                  <option key={q} value={q}>
                    {t.export.qualities[q]}
                  </option>
                ))}
              </NativeSelect>
            </Field>
            <Field label={t.export.encoder} htmlFor={ids.encoder}>
              <NativeSelect
                id={ids.encoder}
                value={form.video.encoder ?? "auto"}
                onChange={(e) =>
                  setVideo({
                    ...form.video,
                    encoder: e.target.value === "auto" ? null : (e.target.value as VideoChoice["encoder"]),
                  })
                }
              >
                <option value="auto">
                  {autoEncoder ? t.export.automaticWith(t.export.encoders[autoEncoder]) : t.export.automatic}
                </option>
                {encoders.map((encoder) => (
                  <option key={encoder} value={encoder}>
                    {t.export.encoders[encoder]}
                  </option>
                ))}
              </NativeSelect>
            </Field>
            {form.video.quality === "custom" && (
              <Field label={t.export.customBitrate} htmlFor={ids.bitrate}>
                <Input
                  id={ids.bitrate}
                  inputMode="decimal"
                  value={form.customMbps}
                  aria-invalid={request.ok ? undefined : request.error === "bitrate"}
                  onChange={(e) => setForm((f) => ({ ...f, customMbps: e.target.value }))}
                />
              </Field>
            )}
            {!capabilities?.probed && (
              <p className="col-span-2 text-xs text-muted-foreground">{t.export.probing}</p>
            )}
            {form.video.codec === "hevc" && (
              <p className="col-span-2 text-xs text-muted-foreground">{t.export.hevcHint}</p>
            )}
          </div>
        )}
      </div>

      <section aria-live="polite" className="grid min-h-12 gap-1.5 rounded-md border bg-muted/40 p-3 text-sm">
        {fieldError ? (
          <p className="text-destructive">{fieldError}</p>
        ) : checkError !== null ? (
          <p className="text-destructive">{checkErrorText(checkError, form.video.quality === "custom")}</p>
        ) : check ? (
          <>
            <p className={cn("tabular-nums", current === null && "text-muted-foreground")}>
              {summaryLine(check, form.video.codec, project.settings)}
            </p>
            {freeSpaceText(check) && <p className="text-xs text-muted-foreground">{freeSpaceText(check)}</p>}
          </>
        ) : (
          <p className="text-muted-foreground">{t.export.checking}</p>
        )}
        {!fieldError && checkError === null && issues.length > 0 && (
          <ul className="grid gap-1">
            {issues.map((issue) => (
              <IssueRow key={issue.key} issue={issue} onRelink={relink} />
            ))}
          </ul>
        )}
      </section>

      {startError && (
        <p role="alert" className="text-sm text-destructive">
          {startError}
        </p>
      )}

      <DialogFooter className="items-center">
        {confirming ? (
          <>
            <p role="alert" className="mr-auto text-sm">
              {t.export.replaceQuestion(fileName)}
            </p>
            <Button type="button" variant="ghost" onClick={() => setReplaceKey(null)}>
              {t.export.back}
            </Button>
            <Button type="submit" variant="destructive" disabled={!canStart}>
              {t.export.replace}
            </Button>
          </>
        ) : (
          <>
            <Button type="button" variant="ghost" onClick={closeExportDialog}>
              {t.export.cancel}
            </Button>
            <Button type="submit" disabled={!canStart}>
              {t.export.start}
            </Button>
          </>
        )}
      </DialogFooter>
    </form>
  );
}

function IssueRow({ issue, onRelink }: { issue: IssueView; onRelink: (media: MediaId) => void }) {
  const media = issue.relink;
  return (
    <li className="flex items-start gap-1.5">
      {issue.tone === "error" ? (
        <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
      ) : (
        <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
      )}
      <span className="flex-1">{issue.text}</span>
      {media !== undefined && (
        <Button type="button" variant="outline" size="xs" onClick={() => onRelink(media)}>
          {t.export.relink}
        </Button>
      )}
    </li>
  );
}

/** A rejected `checkExport`: `invalidEdit` can only be a custom bitrate out of range. */
function checkErrorText(error: unknown, customBitrate: boolean): string {
  if (isIpcError(error) && error.kind === "invalidEdit" && customBitrate) return t.export.bitrateRange;
  return t.export.checkFailed;
}

function Field({ label, htmlFor, children }: { label: string; htmlFor: string; children: ReactNode }) {
  return (
    <div className="grid gap-1.5">
      <label htmlFor={htmlFor} className="text-xs text-muted-foreground">
        {label}
      </label>
      {children}
    </div>
  );
}
