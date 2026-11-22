import { useId, useState, type ReactNode } from "react";

import { commands, type ProjectSettings } from "@/bindings";
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
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { describeError, logError } from "@/ipc/errors";
import { formatFrameRate } from "@/lib/time";
import { cn } from "@/lib/utils";
import { useProjectStore } from "@/stores/projectStore";

import {
  formatAspectRatio,
  formFromSettings,
  frameRateKey,
  frameRateOptions,
  MAX_CANVAS_SIZE,
  MIN_CANVAS_SIZE,
  parseCanvasSize,
  RESOLUTION_PRESETS,
  type ResolutionPresetId,
  selectPreset,
  settingsFromForm,
  sizeErrorText,
} from "./settings";

const PRESET_IDS: readonly ResolutionPresetId[] = [...RESOLUTION_PRESETS.map((p) => p.id), "custom"];

const selectClass = cn(
  "h-9 w-full rounded-md border border-input bg-background px-3 text-sm shadow-xs outline-none dark:bg-input/30",
  "focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50",
);

/** Canvas size and frame rate. Apply sends one `setProjectSettings` (one undo entry). */
export function ProjectSettingsDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const settings = useProjectStore((s) => s.state?.project.settings);
  usePreviewOccluder(open);

  return (
    <Dialog open={open && settings !== undefined} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md" closeLabel={t.projectSettings.close}>
        <DialogHeader>
          <DialogTitle>{t.projectSettings.title}</DialogTitle>
          <DialogDescription>{t.projectSettings.description}</DialogDescription>
        </DialogHeader>
        {/* The content unmounts on close, so the form starts from the model each time. */}
        {settings && <SettingsForm initial={settings} onDone={() => onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  );
}

function SettingsForm({ initial, onDone }: { initial: ProjectSettings; onDone: () => void }) {
  const [form, setForm] = useState(() => formFromSettings(initial));
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const ids = { preset: useId(), width: useId(), height: useId(), rate: useId() };

  const rates = frameRateOptions(initial.frameRate);
  const width = parseCanvasSize(form.width);
  const height = parseCanvasSize(form.height);
  const custom = form.preset === "custom";

  const apply = async () => {
    const result = settingsFromForm(form, initial, rates);
    if (!result.ok) return;
    setPending(true);
    try {
      await commands.setProjectSettings(result.settings);
      onDone();
    } catch (error) {
      logError(error);
      setSubmitError(describeError(error));
      setPending(false);
    }
  };

  return (
    <form
      className="grid gap-4"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void apply();
      }}
    >
      <Field label={t.projectSettings.resolution} htmlFor={ids.preset}>
        <select
          id={ids.preset}
          className={selectClass}
          value={form.preset}
          onChange={(e) => setForm(selectPreset(form, e.target.value as ResolutionPresetId))}
        >
          {PRESET_IDS.map((id) => (
            <option key={id} value={id}>
              {t.projectSettings.presets[id]}
            </option>
          ))}
        </select>
      </Field>

      <div className="grid grid-cols-2 gap-3">
        <SizeInput
          id={ids.width}
          label={t.projectSettings.width}
          value={form.width}
          disabled={!custom}
          error={width.ok ? null : sizeErrorText(width.error)}
          onChange={(value) => setForm({ ...form, width: value })}
        />
        <SizeInput
          id={ids.height}
          label={t.projectSettings.height}
          value={form.height}
          disabled={!custom}
          error={height.ok ? null : sizeErrorText(height.error)}
          onChange={(value) => setForm({ ...form, height: value })}
        />
      </div>

      <div className="flex items-center justify-between text-sm">
        <span className="text-muted-foreground">{t.projectSettings.aspectRatio}</span>
        <span className="tabular-nums">
          {width.ok && height.ok ? formatAspectRatio(width.value, height.value) : "–"}
        </span>
      </div>

      <Field label={t.projectSettings.frameRate} htmlFor={ids.rate}>
        <select
          id={ids.rate}
          className={selectClass}
          value={form.frameRate}
          onChange={(e) => setForm({ ...form, frameRate: e.target.value })}
        >
          {rates.map((rate) => (
            <option key={frameRateKey(rate)} value={frameRateKey(rate)}>
              {formatFrameRate(rate)}
            </option>
          ))}
        </select>
      </Field>

      {submitError && (
        <p role="alert" className="text-sm text-destructive">
          {submitError}
        </p>
      )}

      <DialogFooter>
        <Button type="button" variant="ghost" onClick={onDone}>
          {t.projectSettings.cancel}
        </Button>
        <Button type="submit" disabled={pending || !width.ok || !height.ok}>
          {t.projectSettings.apply}
        </Button>
      </DialogFooter>
    </form>
  );
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

function SizeInput({
  id,
  label,
  value,
  disabled,
  error,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  disabled: boolean;
  error: string | null;
  onChange: (value: string) => void;
}) {
  const errorId = `${id}-error`;
  return (
    <Field label={label} htmlFor={id}>
      <Input
        id={id}
        type="number"
        inputMode="numeric"
        min={MIN_CANVAS_SIZE}
        max={MAX_CANVAS_SIZE}
        step={2}
        value={value}
        disabled={disabled}
        aria-invalid={error !== null}
        aria-describedby={error ? errorId : undefined}
        onChange={(e) => onChange(e.target.value)}
      />
      {error && (
        <span id={errorId} className="text-xs text-destructive">
          {error}
        </span>
      )}
    </Field>
  );
}
