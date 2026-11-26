import { FolderOpen, Plus } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { fileActions } from "@/features/project/fileActions";
import { t } from "@/i18n";

import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import { type CardAction, ProjectCard } from "./ProjectCard";
import { type Recent, recent as appRecent, type RecentProject } from "./recent";
import { RenameDialog } from "./RenameDialog";

/**
 * The start screen (design `home-screen.md`): New project, Open… and the recent projects. The
 * editor (and with it the native preview) is unmounted while this is shown, so nothing here needs
 * a preview occluder. The list is fetched on every mount and after every action.
 */
export function HomeScreen({ recent = appRecent }: { recent?: Recent }) {
  const projects = recent.store((s) => s.projects);
  const loaded = recent.store((s) => s.loaded);
  const [renaming, setRenaming] = useState<RecentProject | null>(null);
  const [deleting, setDeleting] = useState<RecentProject | null>(null);

  useEffect(() => void recent.load(), [recent]);

  const onAction = (action: CardAction, project: RecentProject) => {
    switch (action) {
      case "open":
        void recent.open(project.path);
        break;
      case "rename":
        setRenaming(project);
        break;
      case "duplicate":
        void recent.duplicate(project);
        break;
      case "reveal":
        void recent.reveal(project.path);
        break;
      case "delete":
        setDeleting(project);
        break;
      case "remove":
        void recent.remove(project.path);
        break;
    }
  };

  return (
    <div className="flex h-full flex-col bg-background">
      <header className="flex h-11 shrink-0 items-center gap-2 border-b bg-panel px-3">
        <img src="/logo.svg" alt="" className="size-6" />
        <span className="font-semibold">{t.app.name}</span>
      </header>
      <main className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto grid max-w-6xl gap-6 p-6">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="mr-auto text-lg font-semibold">{t.home.title}</h1>
            <Button onClick={() => void fileActions.newProject()}>
              <Plus className="size-4" aria-hidden />
              {t.home.newProject}
            </Button>
            <Button variant="secondary" onClick={() => void fileActions.openProject()}>
              <FolderOpen className="size-4" aria-hidden />
              {t.home.open}
            </Button>
          </div>
          <section aria-labelledby="home-recent" className="grid gap-3">
            <h2 id="home-recent" className="text-sm font-medium text-muted-foreground">
              {t.home.recent}
            </h2>
            {loaded && projects.length === 0 ? (
              <div className="rounded-lg border border-dashed p-8 text-center">
                <p className="font-medium">{t.home.empty}</p>
                <p className="text-sm text-muted-foreground">{t.home.emptyHint}</p>
              </div>
            ) : (
              <ul className="grid grid-cols-[repeat(auto-fill,minmax(var(--spacing-project-card),1fr))] gap-3">
                {projects.map((project) => (
                  <ProjectCard key={project.path} project={project} onAction={onAction} />
                ))}
              </ul>
            )}
          </section>
        </div>
      </main>
      <RenameDialog project={renaming} rename={recent.rename} onClose={() => setRenaming(null)} />
      <DeleteConfirmDialog project={deleting} trash={recent.trash} onClose={() => setDeleting(null)} />
    </div>
  );
}
