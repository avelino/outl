# Roadmap

Six phases. Each one ends with something **usable standalone**. If the
next phase stalls, the previous one still delivers value.

## Phase 0 — Setup

Day zero. Foundation only.

- [x] Cargo workspace with all phase-1 crates linked.
- [x] LICENSE (MIT, single-license).
- [x] README direct and not corporate.
- [x] CI: cargo test + clippy + fmt on PR.
- [x] `.claude/` infrastructure (agents, hooks, commands, settings).
- [x] CLAUDE.md root + per-crate.
- [x] docs/: architecture, crdt, markdown-format, storage, roadmap.
- [ ] Reserve `outl`, `outl-core`, `outl-md`, `outl-cli`, `outl-tui` on
  crates.io as `0.0.1` placeholder.
- [ ] Reserve `outl` on npm to prevent typosquatting.
- [ ] Issues #1–#4 opened.

## Phase 1 — Day-zero usable

The goal: Avelino replaces part of his Roam usage with outl on a single
device. Edit in VS Code or in the TUI, doesn't matter. Journal works.

### `outl-core`
- `NodeId`, `ActorId`, `HLC`, fractional indexing types.
- `Op` enum with all 4 variants and `old_*` fields.
- `OpLog` (append-only, HLC-ordered).
- `tree.rs`: `do_op` / `undo_op` / `apply_op` / `creates_cycle`.
- `Storage` trait + `SqliteStorage` implementation.
- Domain models: `Workspace`, `Page`, `Journal`, `Block`, `Property`, `Tag`.
- **100% coverage on the four CRDT functions.**
- **Test battery passing**:
  `convergence`, `cycle`, `cycle_chain`, `concurrent_edit_move`,
  `concurrent_delete_edit`, `late_op`, `idempotency`,
  `fractional_index`, `large_log`, `property_based`.

### `outl-md`
- Parse `.md` (clean, no IDs) → AST.
- Render AST → `.md` (clean).
- Read/write `.outl` sidecar (JSON).
- 3-level matching algorithm.
- Diff (old AST + new AST) → minimal `Op` sequence.
- Tests: `roundtrip`, `external_edit`, `duplicate_block`,
  `identical_blocks_swap`, `heavy_edit`.

### `outl-cli`
- `outl init <path>` — scaffold workspace.
- `outl serve` — run file watcher, debounce 200ms.
- `outl doctor` — integrity check.
- `outl reconcile` — delegate to TUI for orphan resolution.
- `outl export` — placeholder for phase 4.

### `outl-tui`
- Journal of today opens by default.
- Sidebar of pages.
- Navigation between journals (`[`, `]`, `t`, `g j`).
- Tag panel.
- Outline panel (read-only first pass).
- Backlinks panel.
- Command palette (basic).
- Page properties visible and editable.
- Visual distinction page vs tag (different colors).

### Acceptance criteria

- [ ] `outl init ~/notes` creates a full workspace.
- [ ] `outl serve` runs in the background.
- [ ] `outl-tui` opens to today's journal.
- [ ] I can create a block with `[[refs]]`, `#tags`, `priority:: high`.
- [ ] I can open the `.md` in VS Code — clean, no visible IDs.
- [ ] I can edit, save, TUI reflects the change.
- [ ] I can duplicate a block in VS Code — reconcile assigns a new ULID to
  the duplicate.
- [ ] I can navigate `[` / `]` between journals.
- [ ] I can kill the process and reopen — state preserved.
- [ ] `outl doctor` reports integrity OK.

## Phase 2 — P2P sync (1 → N devices)

- `outl-sync` crate with iroh transport.
- Discovery via shareable ticket (`outl share` generates,
  `outl join <ticket>` adds a peer).
- Handshake and incremental op exchange using `last_ts_per_actor`.
- E2E tests with 2–3 instances exchanging ops over loopback iroh.
- Encryption enabled by default (iroh QUIC).
- Document trust model: who can send me ops, how to revoke.

## Phase 3 — Queries and refinement

- `{{query: ...}}` inline with an outl-specific DSL.
- Built-in aggregations: count, group-by tag, filter by property.
- Journal-specific queries: "blocks from the last 7 days",
  "all tasks marked priority:: high".
- Backlinks page generated on-demand.
- TUI commands to save and reuse queries.

## Phase 4 — TUI polish + plugins

- Themes (config in `.outl/config.toml`).
- Export to Hugo (pipeline targeting avelino.run).
- Minimal plugin system using `rhai`:
  - Plugins consume the op stream.
  - Can register new query types.
  - Can register render hooks for the TUI.
- Plugin discovery from `.outl/plugins/`.

## Phase 5 — Desktop (Tauri 2)

- Tauri shell over `outl-core` + `outl-md`.
- Block-level rich editing with live preview.
- Graph view (visual backlinks).
- macOS + Linux + Windows builds.

## Phase 6 — Mobile

- `uniffi` FFI surface over `outl-core` + `outl-md`.
- SwiftUI app (iOS, native).
- Compose app (Android, native).
- Sync via iroh (same protocol as desktop).

---

## Issues open since day zero

These are tracked publicly to signal intent and invite contributions:

- **#1** — Add ChronDbStorage backend
  (labels: `roadmap`, `storage`, `help wanted`)
- **#2** — Tauri desktop app
  (labels: `roadmap`, `phase-5`)
- **#3** — Mobile via uniffi
  (labels: `roadmap`, `phase-6`)
- **#4** — Plugin system with rhai
  (labels: `roadmap`, `phase-4`)
