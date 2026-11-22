import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useId, useState, type ReactNode } from "react";

import type { UiLanguage } from "@/bindings";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { resetPanelLayout } from "@/features/editor/layout";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { describeError, logError } from "@/ipc/errors";
import { useAppStore } from "@/stores/appStore";
import { useUiStore } from "@/stores/uiStore";

import {
  AUTOSAVE_RANGE,
  errorField,
  type FieldErrors,
  type FormField,
  formatBytes,
  formFromPreferences,
  IMAGE_DURATION_RANGE,
  type Preferences,
  preferencesFromForm,
  type PreferencesForm,
  type SecondsRange,
  secondsErrorText,
  TRANSITION_DURATION_RANGE,
  validateForm,
} from "./preferences";
import {
  clearCache,
  loadPreferences,
  refreshCacheInfo,
  savePreferences,
  usePreferencesStore,
} from "./preferencesStore";

type TabId = "general" | "editing" | "performance" | "cache" | "shortcuts";

const TABS: readonly TabId[] = ["general", "editing", "performance", "cache", "shortcuts"];

/** The tab that shows each validated field, so a rejected Save can switch to it. */
const FIELD_TAB: Readonly<Record<FormField, TabId>> = {
  autosaveSeconds: "general",
  imageSeconds: "editing",
  transitionSeconds: "editing",
};

const LANGUAGES: readonly UiLanguage[] = ["en"];

/**
 * Edit → Preferences… (design `preferences.md` §2). Save sends the whole preferences object; the
 * cache folder and Clear cache act at once. Not an undo step. Hides the native preview while open.
 */
export function PreferencesDialog() {
  const open = useUiStore((s) => s.preferencesOpen);
  const setOpen = useUiStore((s) => s.setPreferencesOpen);
  const preferences = usePreferencesStore((s) => s.preferences);
  const [loadFailed, setLoadFailed] = useState(false);
  usePreviewOccluder(open);

  // Normally loaded at startup; this covers a dialog opened before that finished or after it failed.
  useEffect(() => {
    if (!open || usePreferencesStore.getState().preferences) return;
    let current = true;
    void loadPreferences().then((loaded) => {
      if (current) setLoadFailed(loaded === null);
    });
    return () => {
      current = false;
    };
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="sm:max-w-xl" closeLabel={t.preferences.close}>
        <DialogHeader>
          <DialogTitle>{t.preferences.title}</DialogTitle>
          <DialogDescription>{t.preferences.description}</DialogDescription>
        </DialogHeader>
        {/* The content unmounts on close, so the draft starts from what Rust has each time. */}
        {preferences ? (
          <PreferencesBody base={preferences} onDone={() => setOpen(false)} />
        ) : (
          <p role={loadFailed ? "alert" : "status"} className="text-sm text-muted-foreground">
            {loadFailed ? t.preferences.loadFailed : t.preferences.loading}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}

function PreferencesBody({ base, onDone }: { base: Preferences; onDone: () => void }) {
  const [initial] = useState(() => formFromPreferences(base));
  const [form, setForm] = useState(initial);
  const [tab, setTab] = useState<TabId>("general");
  const [rejected, setRejected] = useState<FormField | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const errors = validateForm(form);

  const edit = (patch: Partial<PreferencesForm>) => {
    setForm({ ...form, ...patch });
    setRejected(null);
    setSubmitError(null);
  };

  const save = async () => {
    // `base` is the latest from Rust (a cache folder change while open is included).
    const next = preferencesFromForm(form, base, initial);
    const firstInvalid = (Object.keys(errors) as FormField[])[0];
    if (next === null) {
      if (firstInvalid) setTab(FIELD_TAB[firstInvalid]);
      return;
    }
    setPending(true);
    try {
      await savePreferences(next);
      onDone();
    } catch (error) {
      logError(error);
      const field = errorField(error);
      setRejected(field);
      if (field) setTab(FIELD_TAB[field]);
      else setSubmitError(describeError(error));
      setPending(false);
    }
  };

  return (
    <form
      className="grid gap-4"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <Tabs value={tab} onValueChange={(value) => setTab(value as TabId)}>
        <TabsList className="w-full">
          {TABS.map((id) => (
            <TabsTrigger key={id} value={id}>
              {t.preferences.tabs[id]}
            </TabsTrigger>
          ))}
        </TabsList>
        <div className="min-h-64 pt-2">
          <TabsContent value="general">
            <GeneralTab form={form} errors={errors} rejected={rejected} edit={edit} />
          </TabsContent>
          <TabsContent value="editing">
            <EditingTab form={form} errors={errors} rejected={rejected} edit={edit} />
          </TabsContent>
          <TabsContent value="performance">
            <PerformanceTab />
          </TabsContent>
          <TabsContent value="cache">
            <CacheTab base={base} />
          </TabsContent>
          <TabsContent value="shortcuts">
            <ShortcutsTab />
          </TabsContent>
        </div>
      </Tabs>

      {submitError && (
        <p role="alert" className="text-sm text-destructive">
          {submitError}
        </p>
      )}

      <DialogFooter>
        <Button type="button" variant="ghost" onClick={onDone}>
          {t.preferences.cancel}
        </Button>
        <Button type="submit" disabled={pending}>
          {t.preferences.save}
        </Button>
      </DialogFooter>
    </form>
  );
}

type FormTabProps = {
  form: PreferencesForm;
  errors: FieldErrors;
  rejected: FormField | null;
  edit: (patch: Partial<PreferencesForm>) => void;
};

/** The text under a seconds field: a local validation hint, or Rust's rejection. */
function fieldError(
  field: FormField,
  range: SecondsRange,
  { errors, rejected }: FormTabProps,
): string | null {
  const error = errors[field];
  if (error) return secondsErrorText(error, range);
  return rejected === field ? t.preferences.fieldErrors.rejected : null;
}

function GeneralTab(props: FormTabProps) {
  const { form, edit } = props;
  const ids = { language: useId(), autosave: useId(), interval: useId() };
  return (
    <div className="grid gap-4">
      <Field label={t.preferences.general.language} htmlFor={ids.language}>
        <NativeSelect
          id={ids.language}
          value={form.language}
          onChange={(e) => edit({ language: e.target.value as UiLanguage })}
        >
          {LANGUAGES.map((language) => (
            <option key={language} value={language}>
              {t.preferences.general.languages[language]}
            </option>
          ))}
        </NativeSelect>
      </Field>

      <div className="grid gap-2">
        <label htmlFor={ids.autosave} className="flex items-center gap-2 text-sm">
          <input
            id={ids.autosave}
            type="checkbox"
            className="size-4 accent-primary"
            checked={form.autosaveEnabled}
            onChange={(e) => edit({ autosaveEnabled: e.target.checked })}
          />
          {t.preferences.general.autosave}
        </label>
        {form.autosaveEnabled ? (
          <SecondsInput
            id={ids.interval}
            label={t.preferences.general.autosaveInterval}
            value={form.autosaveSeconds}
            range={AUTOSAVE_RANGE}
            step={1}
            error={fieldError("autosaveSeconds", AUTOSAVE_RANGE, props)}
            onChange={(autosaveSeconds) => edit({ autosaveSeconds })}
          />
        ) : (
          <p className="text-xs text-muted-foreground">{t.preferences.general.autosaveOffHint}</p>
        )}
      </div>

      <div className="grid gap-1.5">
        <span className="text-xs text-muted-foreground">{t.preferences.general.panelLayout}</span>
        <div>
          <Button type="button" variant="outline" size="sm" onClick={resetPanelLayout}>
            {t.preferences.general.resetPanelLayout}
          </Button>
        </div>
      </div>
    </div>
  );
}

function EditingTab(props: FormTabProps) {
  const { form, edit } = props;
  const ids = { image: useId(), transition: useId() };
  return (
    <div className="grid gap-4">
      <SecondsInput
        id={ids.image}
        label={t.preferences.editing.imageDuration}
        value={form.imageSeconds}
        range={IMAGE_DURATION_RANGE}
        step={0.1}
        error={fieldError("imageSeconds", IMAGE_DURATION_RANGE, props)}
        onChange={(imageSeconds) => edit({ imageSeconds })}
      />
      <SecondsInput
        id={ids.transition}
        label={t.preferences.editing.transitionDuration}
        value={form.transitionSeconds}
        range={TRANSITION_DURATION_RANGE}
        step={0.1}
        error={fieldError("transitionSeconds", TRANSITION_DURATION_RANGE, props)}
        onChange={(transitionSeconds) => edit({ transitionSeconds })}
      />
      <p className="text-xs text-muted-foreground">{t.preferences.editing.hint}</p>
    </div>
  );
}

/** Read-only GPU capability report (design `platform-integration.md` §6). */
function PerformanceTab() {
  const info = useAppStore((s) => s.info);
  const gpu = info?.gpu;
  const p = t.preferences.performance;
  const yesNo = (value: boolean) => (value ? p.yes : p.no);
  const acceleration = gpu?.videoAcceleration;

  return (
    <div className="grid gap-3 text-sm">
      {!gpu && <p className="text-muted-foreground">{p.noGpu}</p>}
      <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5">
        {gpu && (
          <>
            <Row label={p.adapter}>{gpu.adapter}</Row>
            <Row label={p.backend}>{gpu.backend}</Row>
            <Row label={p.deviceType}>{t.gpu.deviceTypes[gpu.deviceType]}</Row>
            <Row label={p.driver}>{gpu.driver || p.notReported}</Row>
            <Row label={p.software}>{yesNo(gpu.software)}</Row>
            <Row label={p.presentsToWindow}>{yesNo(gpu.presentsToWindow)}</Row>
            <Row label={p.fallback}>{gpu.fallback ? t.gpu.fallbacks[gpu.fallback] : p.noFallback}</Row>
          </>
        )}
        {info && <Row label={p.previewMode}>{t.status.previewMode[info.previewMode]}</Row>}
        <Row label={p.videoAcceleration}>
          {acceleration ? (
            <>
              <span className="block">{p.decoders(acceleration.decoders.join(", ") || p.none)}</span>
              <span className="block">{p.encoders(acceleration.encoders.join(", ") || p.none)}</span>
            </>
          ) : (
            p.videoAccelerationUnknown
          )}
        </Row>
      </dl>
    </div>
  );
}

/** Cache folder, size and Clear cache. Every button here acts at once, not via Save. */
function CacheTab({ base }: { base: Preferences }) {
  const info = usePreferencesStore((s) => s.cacheInfo);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const c = t.preferences.cache;

  /** Runs one cache action at a time; a failure is logged and shown in the tab. */
  const act = async (action: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await action();
      setError(null);
    } catch (failure) {
      logError(failure);
      setError(describeError(failure));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    let current = true;
    refreshCacheInfo().catch((failure: unknown) => {
      logError(failure);
      if (current) setError(describeError(failure));
    });
    return () => {
      current = false;
    };
  }, []);

  /** Sends the stored preferences (not the unsaved draft) with the new folder. */
  const setFolder = async (cacheDir: string | null) => {
    await savePreferences({ ...base, cacheDir });
    await refreshCacheInfo();
  };

  const change = async () => {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: c.pickTitle,
      defaultPath: base.cacheDir ?? undefined,
    });
    if (typeof picked === "string") await setFolder(picked);
  };

  return (
    <div className="grid gap-4 text-sm">
      <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5">
        <Row label={c.folder}>
          <span className="break-all select-text">{info?.path ?? "–"}</span>
        </Row>
        <Row label={c.size}>{info ? `${formatBytes(info.sizeBytes)} · ${c.files(info.files)}` : "–"}</Row>
      </dl>
      <p className="text-xs text-muted-foreground">{c.hint}</p>

      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => void act(change)}>
          {c.change}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={busy || base.cacheDir === null}
          onClick={() => void act(() => setFolder(null))}
        >
          {c.resetFolder}
        </Button>
        {!confirming && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => setConfirming(true)}
          >
            {c.clear}
          </Button>
        )}
      </div>

      {confirming && (
        <div role="group" aria-label={c.confirmClear} className="grid gap-2 rounded-md border p-3">
          <p>{c.confirmClear}</p>
          <div className="flex gap-2">
            <Button
              type="button"
              variant="destructive"
              size="sm"
              disabled={busy}
              onClick={() => {
                setConfirming(false);
                void act(clearCache);
              }}
            >
              {c.confirm}
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={() => setConfirming(false)}>
              {c.cancel}
            </Button>
          </div>
        </div>
      )}

      {error && (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}

function ShortcutsTab() {
  const showShortcuts = useUiStore((s) => s.setShortcutsOpen);
  return (
    <div className="grid gap-3 text-sm">
      <p className="text-muted-foreground">{t.preferences.shortcuts.description}</p>
      <div>
        <Button type="button" variant="outline" size="sm" onClick={() => showShortcuts(true)}>
          {t.preferences.shortcuts.show}
        </Button>
      </div>
    </div>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0">{children}</dd>
    </>
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

function SecondsInput({
  id,
  label,
  value,
  range,
  step,
  error,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  range: SecondsRange;
  step: number;
  error: string | null;
  onChange: (value: string) => void;
}) {
  const errorId = `${id}-error`;
  return (
    <Field label={label} htmlFor={id}>
      <div className="flex items-center gap-2">
        <Input
          id={id}
          type="number"
          inputMode="decimal"
          className="w-32"
          min={range.min}
          max={range.max}
          step={step}
          value={value}
          aria-invalid={error !== null}
          aria-describedby={error ? errorId : undefined}
          onChange={(e) => onChange(e.target.value)}
        />
        <span className="text-xs text-muted-foreground">{t.preferences.seconds}</span>
      </div>
      {error && (
        <span id={errorId} className="text-xs text-destructive">
          {error}
        </span>
      )}
    </Field>
  );
}
