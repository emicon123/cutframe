import { ChevronRight, Copy, FolderOpen } from "lucide-react";
import { useDeferredValue, useEffect, useMemo, useState } from "react";

import { type AboutInfo_Serialize, commands } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { openLogsFolder } from "@/features/notifications/logs";
import { usePreviewOccluder } from "@/features/preview/occlusion";
import { t } from "@/i18n";
import { logError } from "@/ipc/errors";
import { showToast } from "@/stores/toastStore";
import { useUiStore } from "@/stores/uiStore";

import {
  filterPackages,
  LICENSE_PAGE_SIZE,
  type LicensesLoader,
  loadLicenses as loadGenerated,
  packageKey,
  packageLink,
  pageOf,
  textsOf,
  type ThirdPartyLicenses,
  type ThirdPartyPackage,
} from "./licenses";

type AboutDialogProps = {
  /** Injected by tests; the app loads the generated file. */
  loadLicenses?: LicensesLoader;
};

/**
 * Help → About (F0.10): version, build, FFmpeg and the log folder, and the third-party licences
 * generated at build time, including the FFmpeg LGPL notice and source offer.
 */
export function AboutDialog({ loadLicenses = loadGenerated }: AboutDialogProps) {
  const open = useUiStore((s) => s.aboutOpen);
  const setOpen = useUiStore((s) => s.setAboutOpen);
  usePreviewOccluder(open);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="flex h-about-dialog flex-col sm:max-w-3xl" closeLabel={t.about.close}>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <img src="/logo.svg" alt="" className="size-6" />
            {t.about.title}
          </DialogTitle>
          <DialogDescription>{t.about.description}</DialogDescription>
        </DialogHeader>
        {/* Inactive tabs unmount, so the licence file is fetched only when its tab is shown. */}
        <Tabs defaultValue="about" className="min-h-0 flex-1">
          <TabsList className="w-full">
            <TabsTrigger value="about">{t.about.tabs.about}</TabsTrigger>
            <TabsTrigger value="licenses">{t.about.tabs.licenses}</TabsTrigger>
          </TabsList>
          <TabsContent value="about" className="min-h-0 overflow-y-auto">
            <AboutTab />
          </TabsContent>
          <TabsContent value="licenses" className="min-h-0 overflow-y-auto pr-1">
            <LicensesTab load={loadLicenses} />
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    showToast({ tone: "success", text: t.about.copied });
  } catch (error) {
    logError(error);
    showToast({ tone: "error", text: t.about.copyFailed });
  }
}

function CopyButton({ text, label }: { text: string; label: string }) {
  return (
    <Button variant="ghost" size="icon-xs" aria-label={label} onClick={() => void copyText(text)}>
      <Copy aria-hidden />
    </Button>
  );
}

/**
 * A URL as selectable text with a copy button. The webview has no permission to open external
 * links (no opener capability), and navigating the main window away would lose the editor.
 */
function LinkRow({ label, url }: { label: string; url: string }) {
  return (
    <div className="flex min-w-0 items-center gap-2">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <code className="min-w-0 flex-1 truncate font-mono text-xs select-text" title={url}>
        {url}
      </code>
      <CopyButton text={url} label={t.about.copyLink(label)} />
    </div>
  );
}

function AboutTab() {
  const [info, setInfo] = useState<AboutInfo_Serialize | null>(null);

  useEffect(() => {
    let current = true;
    commands
      .getAboutInfo()
      .then((next) => {
        if (current) setInfo(next);
      })
      .catch(logError);
    return () => {
      current = false;
    };
  }, []);

  if (!info) return <p className="text-sm text-muted-foreground">{t.about.loading}</p>;

  return (
    <div className="grid gap-4 py-2">
      <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
        <dt className="text-muted-foreground">{t.about.version}</dt>
        <dd className="select-text">{info.version}</dd>
        <dt className="text-muted-foreground">{t.about.build}</dt>
        <dd>{t.about.buildProfiles[info.buildProfile]}</dd>
        {info.commit !== null && (
          <>
            <dt className="text-muted-foreground">{t.about.commit}</dt>
            <dd className="font-mono text-xs select-text">{info.commit}</dd>
          </>
        )}
        <dt className="text-muted-foreground">{t.about.ffmpeg}</dt>
        <dd className="select-text">{info.ffmpegVersion}</dd>
        <dt className="text-muted-foreground">{t.about.ffmpegLicense}</dt>
        <dd className="select-text">{info.ffmpegLicense}</dd>
        <dt className="text-muted-foreground">{t.about.logFolder}</dt>
        <dd className="flex min-w-0 items-center gap-2">
          {info.logDir === null ? (
            t.about.noLogFolder
          ) : (
            <>
              <code className="min-w-0 truncate font-mono text-xs select-text" title={info.logDir}>
                {info.logDir}
              </code>
              <CopyButton text={info.logDir} label={t.about.copyPath} />
            </>
          )}
        </dd>
      </dl>
      {info.logDir !== null && (
        <Button variant="secondary" size="sm" className="w-fit" onClick={() => void openLogsFolder()}>
          <FolderOpen aria-hidden />
          {t.about.openLogs}
        </Button>
      )}
    </div>
  );
}

type LoadState =
  | { status: "loading" }
  | { status: "missing" }
  | { status: "failed" }
  | { status: "ready"; data: ThirdPartyLicenses };

function LicensesTab({ load }: { load: LicensesLoader }) {
  const [state, setState] = useState<LoadState>({ status: "loading" });

  useEffect(() => {
    let current = true;
    load()
      .then((data) => {
        if (current) setState(data ? { status: "ready", data } : { status: "missing" });
      })
      .catch((error: unknown) => {
        logError(error);
        if (current) setState({ status: "failed" });
      });
    return () => {
      current = false;
    };
  }, [load]);

  switch (state.status) {
    case "loading":
      return <p className="py-2 text-sm text-muted-foreground">{t.about.loading}</p>;
    case "missing":
      return <p className="py-2 text-sm text-muted-foreground">{t.about.licenses.missing}</p>;
    case "failed":
      return <p className="py-2 text-sm text-destructive">{t.about.licenses.failed}</p>;
    case "ready":
      return (
        <div className="grid gap-6 py-2">
          <FfmpegSection data={state.data} />
          <PackageList data={state.data} />
        </div>
      );
  }
}

function FfmpegSection({ data }: { data: ThirdPartyLicenses }) {
  const { ffmpeg } = data;
  return (
    <section aria-labelledby="about-ffmpeg" className="grid gap-3 text-sm">
      <h3 id="about-ffmpeg" className="font-semibold">
        {t.about.licenses.ffmpegHeading}
      </h3>
      <pre className="rounded-md border bg-muted p-3 font-sans text-xs whitespace-pre-wrap select-text">
        {ffmpeg.notice}
      </pre>
      <div className="grid gap-1">
        <h4 className="font-medium">{t.about.licenses.sourceOffer}</h4>
        <p className="text-muted-foreground select-text">{ffmpeg.sourceOffer}</p>
      </div>
      <div className="grid gap-1">
        <LinkRow label={t.about.licenses.sourceCode} url={ffmpeg.sourceCode} />
        <LinkRow label={t.about.licenses.buildScripts} url={ffmpeg.buildScripts} />
      </div>
      <LicenseTexts data={data} ids={ffmpeg.licenseTextIds} />
    </section>
  );
}

function LicenseTexts({ data, ids }: { data: ThirdPartyLicenses; ids: readonly string[] }) {
  const texts = textsOf(data, ids);
  if (texts.length === 0)
    return <p className="text-xs text-muted-foreground">{t.about.licenses.noLicenseText}</p>;
  return (
    <div className="grid gap-2">
      {texts.map((text) => (
        <section key={text.id} aria-label={text.name} className="grid gap-1">
          <h5 className="text-xs font-medium text-muted-foreground">{text.name}</h5>
          <pre className="max-h-64 overflow-auto rounded-md border bg-muted p-2 font-mono text-2xs whitespace-pre-wrap select-text">
            {text.text}
          </pre>
        </section>
      ))}
    </div>
  );
}

function PackageList({ data }: { data: ThirdPartyLicenses }) {
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(LICENSE_PAGE_SIZE);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const deferredQuery = useDeferredValue(query);
  const matches = useMemo(() => filterPackages(data.packages, deferredQuery), [data.packages, deferredQuery]);
  const { visible, remaining } = pageOf(matches, limit);

  const toggle = (key: string) =>
    setExpanded((keys) => {
      const next = new Set(keys);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  return (
    <section aria-labelledby="about-packages" className="grid gap-2 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <h3 id="about-packages" className="mr-auto font-semibold">
          {t.about.licenses.packagesHeading}
        </h3>
        <span className="text-xs text-muted-foreground" aria-live="polite">
          {t.about.licenses.count(matches.length, data.packages.length)}
        </span>
      </div>
      <Input
        type="search"
        value={query}
        aria-label={t.about.licenses.search}
        placeholder={t.about.licenses.searchPlaceholder}
        onChange={(event) => {
          setQuery(event.target.value);
          setLimit(LICENSE_PAGE_SIZE);
        }}
      />
      {matches.length === 0 ? (
        <p className="py-2 text-muted-foreground">{t.about.licenses.none}</p>
      ) : (
        <ul className="divide-y rounded-md border">
          {visible.map((pkg) => {
            const key = packageKey(pkg);
            return (
              <PackageRow
                key={key}
                pkg={pkg}
                data={data}
                expanded={expanded.has(key)}
                onToggle={() => toggle(key)}
              />
            );
          })}
        </ul>
      )}
      {remaining > 0 && (
        <Button
          variant="secondary"
          size="sm"
          className="w-fit"
          onClick={() => setLimit((n) => n + LICENSE_PAGE_SIZE)}
        >
          {t.about.licenses.showMore(Math.min(remaining, LICENSE_PAGE_SIZE))}
        </Button>
      )}
    </section>
  );
}

type PackageRowProps = {
  pkg: ThirdPartyPackage;
  data: ThirdPartyLicenses;
  expanded: boolean;
  onToggle: () => void;
};

/** Only expanded rows render their licence texts (163 texts, some over 10 KB). */
function PackageRow({ pkg, data, expanded, onToggle }: PackageRowProps) {
  const link = packageLink(pkg);
  return (
    <li>
      <button
        type="button"
        aria-expanded={expanded}
        aria-label={t.about.licenses.showLicense(pkg.name)}
        onClick={onToggle}
        className="flex w-full min-w-0 items-center gap-2 px-2 py-1.5 text-left hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
      >
        <ChevronRight
          className="size-3.5 shrink-0 text-muted-foreground transition-transform data-[open=true]:rotate-90"
          data-open={expanded}
          aria-hidden
        />
        <span className="w-12 shrink-0 text-2xs text-muted-foreground uppercase">
          {t.about.licenses.ecosystems[pkg.ecosystem]}
        </span>
        <span className="min-w-0 truncate font-medium">{pkg.name}</span>
        <span className="shrink-0 text-xs text-muted-foreground">{pkg.version}</span>
        <span className="ml-auto min-w-0 truncate text-xs text-muted-foreground">{pkg.license}</span>
      </button>
      {expanded && (
        <div className="grid gap-2 px-2 pt-1 pb-3 pl-7">
          {link && <LinkRow label={pkg.name} url={link} />}
          <LicenseTexts data={data} ids={pkg.licenseTextIds} />
        </div>
      )}
    </li>
  );
}
