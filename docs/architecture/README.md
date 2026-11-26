# Architecture documentation

> **Audience:** every agent and every human working on Cutframe. Start here, then open the one
> document for the area you are about to touch.

---

## 1. What this folder is

`docs/architecture/` describes the system **as it is implemented today**: modules, data, contracts,
threads, files on disk and the rules that must not break. It is the "what is there" companion to two
other sets:

| Folder | Answers | Owner |
|---|---|---|
| `docs/architecture/` (this folder) | What exists, where, and what must not break | everyone, kept current after implementation |
| `docs/adr/` | Why a decision was taken, and what was rejected | `architect` |
| `docs/design/` | How a feature that is not built yet will work | `architect`, then implemented by `backend`/`frontend` |
| `docs/feature-catalog.md` | Which product features exist, are partial or missing | everyone, during audits |

If the code and a document disagree, the code is the truth for *behaviour* and the ADR is the truth
for *intent*. Say so in the task instead of silently following one of them.

## 2. The set

| Document | Read it when you touch |
|---|---|
| [00 — Repository and stack](00-repository-and-stack.md) | a new dependency, a crate boundary, toolchain, build or CI |
| [01 — System context](01-system-context.md) | process start-up, threads, windows, app-data folders, anything cross-cutting |
| [02 — Domain model](02-domain-model.md) | a model type, a time value, an invariant, the project file shape |
| [03 — Edit model](03-edit-model.md) | commands, undo/redo, gestures, timeline arranging, validation |
| [04 — Persistence and lifecycle](04-persistence-and-lifecycle.md) | open/save, migrations, autosave, recovery, recent projects, preferences |
| [05 — IPC contract](05-ipc-contract.md) | any `tauri` command or event, error kinds, view types, jobs |
| [06 — Media pipeline](06-media-pipeline.md) | import, probing, thumbnails, filmstrips, waveforms, cache, relink |
| [07 — Preview and render](07-preview-and-render.md) | wgpu, the preview window, device loss, offscreen rendering |
| [08 — UI architecture](08-ui-architecture.md) | React screens, stores, actions, the canvas timeline, dialogs |
| [diagrams/](diagrams/) | you want one picture of a whole flow instead of prose |

Deep dives in `diagrams/` are self-contained: each one is a single flow with its sequence, states
and failure paths. The numbered documents link to them instead of repeating them.

| Diagram | Shows |
|---|---|
| [010 — Start-up and shutdown](diagrams/010-startup-and-shutdown.md) | process start, the two windows, clean close vs crash |
| [020 — Timeline model](diagrams/020-timeline-model.md) | the document's object graph, track order, invariants |
| [030 — Command and gesture](diagrams/030-command-and-gesture.md) | the one path into the model, a drag as one history entry |
| [040 — Project lifecycle](diagrams/040-project-lifecycle.md) | open, save, autosave, recovery, backup rules |
| [050 — Media import](diagrams/050-media-import.md) | the import job, per-file outcomes, what the UI shows |
| [060 — `media://` request](diagrams/060-media-protocol.md) | routing, status codes, shared generation |
| [070 — Preview presentation](diagrams/070-preview-presentation.md) | show/hide rules, preview modes, device loss |

## 3. Conventions used in this set

- **Today vs Target.** A paragraph about behaviour that exists is written plainly. Anything that is
  designed in `docs/design/` or an ADR but not built is marked `**Target.**` and links to that
  document. There is no third category: if it is neither, it does not belong here.
- **Code references** look like `crates/app-core/src/document.rs:66` and are relative to the
  repository root. They are pointers, not copies: read the code for details.
- **Stable keys.** Anything the UI branches on (`HistoryLabel`, `IpcError.kind`, `JobStage`,
  `MediaErrorKind`, `GpuFallback`) is an API key, never display text. Renaming one breaks
  translations, so this set lists them as they are.
- **Mermaid diagrams** are rendered by GitHub and by most editors. Node labels are always quoted so
  spaces, parentheses and `+` are safe.
- **Numbers** (limits, timeouts, capacities) come from the code. When one changes, update the
  document in the same commit.

## 4. Keeping it current

A task that changes any of the following must update the matching document in the same commit:

| Change | Document |
|---|---|
| A crate boundary, workspace member, pinned tool or CI step | 00 |
| Threads, windows, start-up order, an app-data folder | 01 |
| A model type, field, unit or invariant | 02 |
| A command, what one undo step covers, a timeline rule | 03 |
| The project file, a schema migration, autosave or preferences | 04 |
| A `tauri` command, event, error kind or job stage | 05 |
| The media cache, a `media://` route, probe fields | 06 |
| The preview thread, adapter choice, surface or render targets | 07 |
| A screen, store, action or gesture | 08 |

The history of the system is in `docs/adr/` and in `docs/archive/`; this set only describes the
present, so delete paragraphs that stopped being true instead of appending corrections.
