# Task 0095: Architecture documentation set

**Tier:** 0 · **Agents:** `architect` (with `backend`/`frontend` review) · **Depends on:** —

**Spec:** `CLAUDE.md`, `docs/adr/`, `docs/design/`, the code as it is today

## Why

`docs/architecture.md` was a one-page overview written at stage 0. It still claimed there was no media
pipeline, no FFmpeg and no preferences, which stopped being true several tasks ago, and it mixed
today's code with the target. There was no single place that described the implemented system at the
level of detail the ADRs and designs use.

## Scope

- Replace the one-pager with `docs/architecture/`: numbered topic files (`00`–`08`), a `README.md`
  index with conventions and a keeping-current table, and `diagrams/` with one self-contained flow
  per diagram.
- Follow the conventions of the reference repository: H1 `# NN — Title`, an `> **Audience:**`
  blockquote, `---`, numbered `## N.` sections, Mermaid diagrams with quoted labels, a final
  `## Links` section to ADRs, designs and tasks.
- Describe **today** (what the code does) and mark anything designed but not built as `**Target.**`
  with a link; add `> Drift:` notes where older docs and the code disagree.
- Cover at minimum what is implemented: repository and stack, system context and threads, domain
  model, edit model and gestures, persistence and lifecycle, IPC contract, media pipeline, preview
  and render, UI architecture.
- Delete `docs/architecture.md` and redirect every reference to the new set.

## Files

- `docs/architecture/README.md`, `00-repository-and-stack.md` … `08-ui-architecture.md`
- `docs/architecture/diagrams/010-startup-and-shutdown.md` … `070-preview-presentation.md`
- `CLAUDE.md`, `README.md`, `.claude/agents/{architect,backend,frontend,devops}.md`,
  `docs/architecture-review.md`, `docs/design/export.md` (references)
- deleted: `docs/architecture.md`

## Done when

- every implemented subsystem has a topic file that matches the code, with code references
- every target paragraph is marked and links to the design or ADR that decided it
- all relative links in the new set resolve; no file references the deleted page
- the index states which document must be updated when a given area changes
- `pnpm check` passes (documentation-only change, no Rust or UI code touched)
