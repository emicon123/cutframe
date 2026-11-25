# Diagram 040 — Project lifecycle

> **Audience:** `backend`, `frontend`. How one document moves between never-saved, saved, dirty,
> autosaved and recovered, and what the file on disk looks like at each step.

---

## 1. One document, four durable states

The Rust model always holds exactly one project — the home screen shows an empty untitled one. What
changes is where the current committed state exists on disk.

```mermaid
stateDiagram-v2
  [*] --> Untitled: start-up creates an empty project
  Untitled --> Dirty: first edit
  Untitled --> Saved: Save As (file did not exist)
  Saved --> Dirty: edit or gesture commit (new history position)
  Dirty --> Saved: Save or Save As (position stored)
  Dirty --> Dirty: autosave writes recovery/session.json
  Dirty --> Recovered: crash, next start finds an orphan
  Saved --> Recovered: crash after an edit that was autosaved
  Recovered --> Dirty: restore_recovery adopts the orphan
  Recovered --> Saved: discard_recovery deletes it
  Saved --> [*]: clean close, no recovery envelope left
  Dirty --> [*]: close after Save or Don't save
```

- "Dirty" is `HistoryPosition != saved position` **or** a gesture is open. Undoing back to the saved
  position makes the project clean without writing anything ([03](030-command-and-gesture.md) §3).
- A clean close deletes this session's recovery envelope and lock. A crash skips that, which is what
  makes the prompt appear on the next start.

## 2. Open: parse, migrate, validate

`serialization::load` reads `schemaVersion` first. A file this build cannot understand is refused,
never repaired.

```mermaid
flowchart TB
  read["read the file as UTF-8"] --> parse{"parse JSON"}
  parse -->|"no"| malformed["malformedProject"]
  parse --> version{"schemaVersion in 1..=3"}
  version -->|"missing"| malformed
  version -->|"> 3"| tooNew["projectTooNew"]
  version -->|"1 or 2"| migrate["migrate one step at a time"]
  version -->|"3"| deser["deserialize into Project"]
  migrate --> deser
  deser --> validate{"Project::validate"}
  validate -->|"violation"| invalid["invalidProject"]
  validate -->|"ok"| loaded["Loaded { project, source_version }"]
  loaded --> resolve["resolve relative media paths"]
  resolve --> swap["replace the open project, record recent entry"]
  swap --> jobs["background media check + preview prefetch"]
```

`source_version` is what the file *was*; the first save over an older file writes
`<name>.cutframe.v<n>.bak` before touching it. A newer **recent list** is not an error — the app
starts with an empty list and backs the file up on the next write.

## 3. Save and autosave

Both take a `SaveSnapshot` under a short document lock and write outside it. They differ in what
they do with the result and in how the file is replaced.

```mermaid
sequenceDiagram
  autonumber
  participant UI
  participant Cmd as Save / Save As
  participant Auto as autosave thread
  participant Doc as Document
  participant Disk

  UI->>Cmd: Ctrl+S
  Cmd->>Doc: save_lock, then SaveSnapshot
  Doc-->>Cmd: committed project, position, generation, file, source_version
  alt project has no file yet
    Cmd-->>UI: false — the UI runs Save As
  else file exists
    Cmd->>Disk: to_json with relative paths, write .tmp, sync_all, rename
    Cmd->>Doc: mark_saved(generation, position)
    Cmd->>Disk: update recent.json
    Cmd-->>UI: true
  end

  loop every autosaveInterval, default 60 s
    Auto->>Doc: SaveSnapshot (short lock)
    alt not dirty, or the same generation and position as last write
      Auto->>Auto: nothing to write
    else
      Auto->>Disk: write recovery/session.json, .tmp then rename
    end
  end
```

- `SaveSnapshot.project` is `Document::committed()`: an open gesture's **base**, never the live drag,
  so neither a save nor an autosave can persist a half-finished move.
- A save that finishes after New or Open belongs to another generation and marks nothing
  (`mark_saved` compares generations).
- Autosave retries after a failed write because a failed write records no key. A stuck disk delays
  shutdown by at most `SHUTDOWN_TIMEOUT = 2 s`.
- `SetInterval(None)` and `Discard` disable autosave and delete this session's envelope.

## 4. Recovery

An envelope is an **orphan** when its `.lock` can be opened (the previous session is gone) and it
parses. `should_offer` shows it when the project was never saved, its file is missing, or the
autosave is newer than the project file's mtime.

```mermaid
flowchart LR
  start["start-up"] --> scan["scan recovery/ for orphans"]
  scan --> none["no orphans: nothing shown"]
  scan --> offer{"should_offer?"}
  offer -->|"no"| keep["keep the envelope, do not prompt"]
  offer -->|"yes"| prompt["RecoveryDialog"]
  prompt -->|"Restore"| load["load the project, replace the open one"]
  load --> adopt["autosave Adopt: write under this session, then delete the old entry"]
  prompt -->|"Discard"| del["delete the orphan"]
  prompt -->|"later"| keep
```

Recovery is offered after the launch path is handled and over whichever view ends up visible, so a
project opened from Explorer never competes with the prompt for the editor. A file written by a newer
app is kept rather than discarded; only a corrupt envelope is removed.

## 5. The two backup rules

| Situation | Written | Never happens |
|---|---|---|
| first save over a file loaded from schema `n < 3` | `<name>.cutframe.v<n>.bak`, once | the backup is not overwritten by a later save |
| save over the same schema | only the project file | no `.bak` churn |
| preferences or recent list from an older version | a backup on the next write | the old file is not deleted first |
| any failed write | the old file, then the temporary file is removed | no mixed or truncated file |

## Links

- [ADR-0002 — Project file format](../../adr/0002-project-file-format.md)
- [04 — Persistence and lifecycle](../04-persistence-and-lifecycle.md)
- [project-lifecycle design](../../design/project-lifecycle.md),
  [autosave-recovery design](../../design/autosave-recovery.md)
- [diagrams/010 — Start-up and shutdown](010-startup-and-shutdown.md)
