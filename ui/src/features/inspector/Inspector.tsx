import { useState, type ReactNode } from "react";

import { commands } from "@/bindings";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { t } from "@/i18n";
import { formatFrameRate, formatTimecode } from "@/lib/time";
import { run } from "@/stores/appStore";
import { useProjectStore, type ProjectState } from "@/stores/projectStore";

export function Inspector() {
  const project = useProjectStore((s) => s.state?.project);

  return (
    <section className="flex h-full flex-col bg-panel" aria-label={t.inspector.title}>
      <header className="flex h-9 shrink-0 items-center border-b px-3">
        <h2 className="font-semibold">{t.inspector.title}</h2>
      </header>
      {project && (
        <div className="flex flex-col gap-4 overflow-y-auto p-3">
          {/* Remount on outside changes (undo/redo) so the field shows the model's name. */}
          <ProjectNameField key={project.name} name={project.name} />
          <Separator />
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2">
            <Row label={t.inspector.resolution}>
              {project.settings.width} × {project.settings.height}
            </Row>
            <Row label={t.inspector.frameRate}>{formatFrameRate(project.settings.frameRate)}</Row>
            <Row label={t.inspector.sampleRate}>{(project.settings.sampleRate / 1000).toFixed(1)} kHz</Row>
            <Row label={t.inspector.duration}>
              {formatTimecode(project.duration, project.settings.frameRate)}
            </Row>
            <Row label={t.inspector.tracks}>{project.tracks.length}</Row>
          </dl>
        </div>
      )}
    </section>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="text-right tabular-nums">{children}</dd>
    </>
  );
}

function ProjectNameField({ name }: { name: ProjectState["project"]["name"] }) {
  const [draft, setDraft] = useState(name);
  const commit = () => {
    const next = draft.trim();
    if (next && next !== name) void run(commands.renameProject(next));
    else setDraft(name);
  };
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs text-muted-foreground">{t.inspector.name}</span>
      <Input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") {
            setDraft(name);
            e.currentTarget.blur();
          }
        }}
      />
    </label>
  );
}
