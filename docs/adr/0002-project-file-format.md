# ADR-0002: Project file format and schema migrations

- **Status:** Accepted (2026-11-22, with the amendment below)
- **Date:** 2026-11-21

## Context

ADR-0001 fixes the project format as "JSON (`serde`) with a `schemaVersion` field + migrations".
`crates/app-core/src/serialization.rs` already does this. It reads `schemaVersion`, rejects newer
files (`LoadError::TooNew`), runs a migration chain over `serde_json::Value` and deserializes into
the model. A v1 fixture is loaded in a test. The model types themselves (`Project`, `Track`, `Clip`)
derive `Serialize`/`Deserialize`, so their serde form *is* the file format.

Several hard-to-reverse questions are still open: what goes into the file and what stays out, how
media is referenced, how saving avoids corrupt files, and what loading checks. Once users have
project files on disk, every one of these choices becomes a migration.

## Decision

1. **One project = one UTF-8 JSON file.** The file extension is chosen together with the final
   product name, under the same deadline as the app identifier (see CLAUDE.md). Until then the app
   uses `.json`.
2. **The serde form of the `app-core` model is the file format.** There is no separate DTO layer.
   Any change that alters the serialized output bumps `CURRENT_SCHEMA_VERSION` and adds one migration
   step `v(N) -> v(N+1)` on `serde_json::Value`. It also adds a fixture `tests/fixtures/project-vN.json`,
   written by the old version and kept forever. A snapshot test of a canonical project's serialized
   form catches format changes made by accident.
3. **Load pipeline:** parse JSON → read `schemaVersion` (missing means error) → reject newer
   versions → migrate step by step → deserialize → **validate invariants** (`Project::validate`: ids
   unique, clips on a track sorted and non-overlapping, durations > 0, frame rate flick-aligned, and
   so on). An invalid file is reported as an error and is never repaired silently.
4. **No forward compatibility.** An app opens files of its own version or older. Saving always
   writes `CURRENT_SCHEMA_VERSION`. The first time a file loaded from an older version is saved, the
   original is copied to `<file>.v<N>.bak`, so an upgrade can be undone by hand.
5. **What the file contains:** only the document, meaning the project model. It never contains undo
   history, caches (thumbnails, waveforms, proxies, the SQLite media index), UI layout, zoom or
   playhead. Caches live in the app cache directory, keyed by media identity, and can be regenerated.
6. **Values:** time is integer flicks (`Time`, an `i64`). Ids are UUID v7 strings. Enums are
   camelCase strings. Effects are referenced by a stable effect id plus the effect's version from
   `effect.json`, and parameters by name. When an effect's parameters change, the change is migrated
   per effect.
7. **Media references:** each media item stores the path relative to the project file and the
   absolute path. Loading tries the relative path first, then the absolute one. Missing media is
   marked offline and can be relinked. The media-import design doc decides the identity fingerprint
   used for relinking and cache keys.
8. **Saving is atomic and stays outside `app-core`.** `app-core` converts to and from strings only.
   `src-tauri` writes `<file>.tmp` in the target directory, flushes it (`sync_all`) and renames it
   over the target. Saving is not an undo step. `AppState` records the session revision (ADR-0003)
   of the last save, which gives the "unsaved changes" state. During an open gesture, saving writes
   the gesture's base state (ADR-0006). Autosave, when it comes, writes to the app-data recovery
   directory and never over the user's file.

## Rationale

- JSON is diffable, easy to read when debugging and simple to migrate as a `Value` tree. This matches
  ADR-0001.
- Using the model types as the format avoids maintaining two parallel type trees. The cost is that
  model refactors need migrations, and the fixtures and the snapshot test keep that cost visible.
- Validation on load protects every later command from states it assumes cannot exist.
- Atomic rename is the standard way to avoid half-written files if the app crashes or the machine
  loses power.

Rejected:
- **Binary format (bincode, MessagePack):** smaller and faster, but not diffable, and migrations
  are harder. Project files are small compared with the media they reference.
- **SQLite project file:** gives transactional saves, but every change still needs a migration, the
  file is harder to diff, and it is heavier than needed. SQLite stays for the media index and cache
  (ADR-0001).
- **Project bundle directory (project plus caches):** caches can be regenerated, and a directory is
  harder to share. A zip container can be added later if assets need to be embedded.
- **Separate file DTOs:** worth reconsidering only if the in-memory model and the file format need
  to diverge.
- **Forward compatibility (an older app ignoring unknown fields):** it silently drops data that a
  newer app wrote.

## Consequences and risks

- `Project::validate` must be added before open/save ships. Fuzzing it with `proptest` on generated
  projects is cheap.
- Every PR that changes serde attributes or model fields must bump the version, add a migration and
  add a fixture. Reviewers check this, and the snapshot test catches what they miss.
- Choosing the extension late means the first public release fixes it. Changing it afterwards breaks
  file associations.

## Amendment 2026-11-22

Accepted for task 0100 (new, open, save, project settings). The decisions above stand, with these
changes. Points 1 and 8 of the original text are historical where they disagree with this section.
Implementation details are in `docs/design/project-lifecycle.md`.

1. **Working file extension: `.cutframe`.** It replaces "use `.json` until the final name". It lives
   in exactly one place, `app_core::PROJECT_FILE_EXTENSION = "cutframe"` (no leading dot), and is
   exported to `ui/src/bindings.ts` with `Builder::constant`. The UI never writes the literal. It is a
   working choice: it is renamed under the same deadline as the app identifier (CLAUDE.md), before the
   first public release and before file associations (F0.15) ship.
   - *Why `.cutframe`:* it is self-describing in Explorer, collides with no known format, and matches
     the codename that is already in the identifier, so both change together in one step. Rejected:
     **`.json`** (Explorer opens it in a text editor, no file association possible, any JSON file
     looks like a project); **a neutral extension such as `.vproj` / `.cfp`** (does not avoid the
     rename, since it would still be tied to the final product, and short ones are taken by other
     tools).
   - *Existing `.json` files:* no build has ever written a project file (there was no save command),
     so there is nothing to migrate. The open dialog filters for `.cutframe` only. `open_project`
     itself does not check the extension: content decides (load pipeline, §3).
2. **"Unsaved changes" is tracked by history position, not by session revision** (replaces the last
   sentences of §8). The revision also changes on save, gesture updates and cancel, so "revision
   differs from the saved revision" would stay dirty after undoing back to the saved state.
   `Document` gives every history entry an id, and its *history position* is the id of the newest
   applied entry. `AppState` records the position at the last save or open. The project is dirty
   when the current position differs or a gesture is open.
3. **Open and save run off the main thread.** They are `async` IPC commands whose file I/O runs in
   `spawn_blocking` (ADR-0004: the main thread does no I/O). They are not `JobId` jobs: project files
   are small (well under 1 MB, ADR-0003 budget) and finish in milliseconds. The document lock is held
   only to take the snapshot to save, or to swap in the opened project.
4. **Backup naming:** the copy made on the first save of an older-schema file is
   `<file>.v<N>.bak` next to the file, where N is the schema version that was loaded (unchanged). An
   existing backup of that name is never overwritten.
