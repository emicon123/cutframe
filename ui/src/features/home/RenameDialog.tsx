import { type SyntheticEvent, useState } from "react";

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
import { t } from "@/i18n";

import type { RecentProject } from "./recent";

/**
 * Renames a recent project and its file (open while `project` is not `null`). Prefilled with the
 * current name; Enter applies. `rename` resolves the error text to show, or `null` on success.
 */
export function RenameDialog({
  project,
  rename,
  onClose,
}: {
  project: RecentProject | null;
  rename: (path: string, name: string) => Promise<string | null>;
  onClose: () => void;
}) {
  return (
    <Dialog
      open={project !== null}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="sm:max-w-md" closeLabel={t.home.rename.cancel}>
        {/* Keyed by path, so the form starts from the project's name each time it opens. */}
        {project && <RenameForm key={project.path} project={project} rename={rename} onClose={onClose} />}
      </DialogContent>
    </Dialog>
  );
}

function RenameForm({
  project,
  rename,
  onClose,
}: {
  project: RecentProject;
  rename: (path: string, name: string) => Promise<string | null>;
  onClose: () => void;
}) {
  const [name, setName] = useState(project.name);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const empty = name.trim() === "";

  const submit = async (event: SyntheticEvent) => {
    event.preventDefault();
    if (empty || busy) return;
    if (name.trim() === project.name) {
      onClose();
      return;
    }
    setBusy(true);
    const message = await rename(project.path, name);
    setBusy(false);
    if (message === null) onClose();
    else setError(message);
  };

  return (
    <form onSubmit={(event) => void submit(event)} className="grid gap-4">
      <DialogHeader>
        <DialogTitle>{t.home.rename.title}</DialogTitle>
        <DialogDescription>{t.home.rename.description}</DialogDescription>
      </DialogHeader>
      <Input
        aria-label={t.home.rename.label}
        value={name}
        autoFocus
        onFocus={(event) => event.currentTarget.select()}
        aria-invalid={error !== null}
        onChange={(event) => {
          setName(event.target.value);
          setError(null);
        }}
      />
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <DialogFooter>
        <Button type="button" variant="ghost" onClick={onClose}>
          {t.home.rename.cancel}
        </Button>
        <Button type="submit" disabled={empty || busy}>
          {t.home.rename.apply}
        </Button>
      </DialogFooter>
    </form>
  );
}
