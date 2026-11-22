# Architecture Decision Records

One file per decision: `NNNN-kebab-case-title.md`, numbered sequentially and never renumbered.
A superseded ADR stays in place with `Status: Superseded by ADR-NNNN`. A smaller change to an
accepted ADR is an amendment: a dated note on its Status line plus an `Amendment YYYY-MM-DD` section
at the end; the original text is kept and marked historical rather than rewritten.

Sections: Status, Date, Context, Decision, Rationale (incl. rejected options), Consequences and risks.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-tech-stack.md) | Tech stack: Rust + Tauri 2 + React 19 + wgpu | Accepted (amended 2026-11-22) |
| [0002](0002-project-file-format.md) | Project file format and schema migrations | Accepted (amended 2026-11-22) |
| [0003](0003-ipc-conventions.md) | IPC conventions | Proposed |
| [0004](0004-threading-and-model-ownership.md) | Threading and model ownership | Proposed |
| [0005](0005-preview-presentation-per-platform.md) | Preview presentation per platform | Accepted (amended 2026-11-22) |
| [0006](0006-edit-transactions.md) | Edit transactions for gestures and compound edits | Accepted |
