#!/usr/bin/env bash
# SessionStart hook: inject critical context every session.
#
# Reminds Claude of the invariants that MUST NOT be violated and the
# current state of the project. Keeps the model from drifting on long
# sessions where the original spec scrolls out of the immediate window.

set -uo pipefail

cat <<'EOF'
# outl session context

You are working on **outl**, a local-first outliner with CRDT-based tree sync.

## CRITICAL invariants (NEVER violate)

1. **Op log is source of truth.** All mutations go through `Op` → `apply_op` → log.
   The `.md` file is a projection. Never edit `.md` directly to "fix" state.

2. **Markdown stays 100% clean.** No `id::`, no UUID, no HTML comments.
   IDs live ONLY in the `.outl` sidecar (JSON dotfile).

3. **The CRDT algorithm follows Kleppmann et al. 2022 literally.**
   `do_op` / `undo_op` / `apply_op` / `creates_cycle` must match the paper.
   100% test coverage on these four functions is non-negotiable.

4. **A move that creates a cycle is a deterministic NO-OP, but the op
   stays in the log.** Removing the op breaks reordering correctness.

5. **Storage is a trait, not a struct.** Never call into `rusqlite`
   from `outl-core`. Everything goes through the `Storage` trait.

## Reminders

- Read `CLAUDE.md` in the crate you're touching before making changes.
- Per-crate invariants are in `crates/<name>/CLAUDE.md`.
- The paper: <https://martin.kleppmann.com/papers/move-op.pdf>
- Tests in `crates/outl-core/tests/` are spec, not afterthought.

## State

TUI, CLI, desktop (Tauri 2), mobile (iOS), and the JavaScript plugin system
(Boa) are all shipped, along with code-block execution and iCloud Drive sync.
Not yet built: P2P sync via iroh (iCloud Drive is the transport today),
the query DSL (`{{query}}`), ChronDB storage, Android, and graph view.
EOF

exit 0
