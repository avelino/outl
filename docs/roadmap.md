# Roadmap

Six phases.
Each one ends with something **usable standalone**.
If the next phase stalls, the previous one still delivers value.

## Phase 0 — Setup

Day zero.
Foundation only.

- [x] Cargo workspace with all phase-1 crates linked.
- [x] LICENSE (MIT, single-license).
- [x] README direct and not corporate.
- [x] CI: cargo test + clippy + fmt on PR.
- [x] `.claude/` infrastructure (agents, hooks, commands, settings).
- [x] CLAUDE.md root + per-crate.
- [x] docs/: architecture, crdt, markdown-format, storage, roadmap.
- [ ] Reserve `outl`, `outl-core`, `outl-md`, `outl-cli`, `outl-tui` on crates.io as `0.0.1` placeholder.
- [ ] Reserve `outl` on npm to prevent typosquatting.
- [ ] Issues #1–#4 opened.

## Phase 1 — Day-zero usable

The goal: Avelino replaces part of his Roam usage with outl on a single device.
Edit in VS Code or in the TUI, doesn't matter.
Journal works.

### `outl-core`
- `NodeId`, `ActorId`, `HLC`, fractional indexing types.
- `Op` enum with all 4 variants and `old_*` fields.
- `OpLog` (append-only, HLC-ordered).
- `tree.rs`: `do_op` / `undo_op` / `apply_op` / `creates_cycle`.
- `Storage` trait + `JsonlStorage` (persistent) + `MemoryStorage` (test double).
- Domain models: `Workspace`, `Page`, `Journal`, `Block`, `Property`, `Tag`.
- **100% coverage on the four CRDT functions.**
- **Test battery passing**: `convergence`, `cycle`, `cycle_chain`, `concurrent_edit_move`, `concurrent_delete_edit`, `late_op`, `idempotency`, `fractional_index`, `large_log`, `property_based`.

### `outl-md`
- Parse `.md` (clean, no IDs) → AST.
- Render AST → `.md` (clean).
- Read/write `.outl` sidecar (JSON, version `2`, reads v1).
- 3-level matching algorithm.
- Diff (old AST + new AST + old sidecar blocks) → minimal `Op` sequence; preserves `ref_handle` verbatim on level-1/2 matches.
- Block-level index (`((blk-XXXXXX))` + reverse refs) under `WorkspaceIndex` — O(1) resolve, linear `search_block_text`.
- Tests: `roundtrip`, `external_edit`, `duplicate_block`, `identical_blocks_swap`, `heavy_edit`.

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
- Outline panel (read/write — text editing, indent/outdent, move, delete).
- Inline backlinks below the outline (`B` toggle).
- Command palette + slash menu sharing one registry.
- Block-reference workflow: inline render of `((blk-XXXXXX))`, embed expansion (source block + children with `↳ ` prefix) for `!((blk-XXXXXX))`, `Enter` jumps to source block, `((` autocomplete in Insert, `y r` / `/refer` / `/refer-embed` copy handles to the OS clipboard (via `arboard`).
- Page properties visible and editable.
- Visual distinction page vs tag (different colors).

### Acceptance criteria

- [ ] `outl init ~/notes` creates a full workspace.
- [ ] `outl serve` runs in the background.
- [ ] `outl-tui` opens to today's journal.
- [ ] I can create a block with `[[refs]]`, `#tags`, `priority:: high`.
- [ ] I can open the `.md` in VS Code — clean, no visible IDs.
- [ ] I can edit, save, TUI reflects the change.
- [ ] I can duplicate a block in VS Code — reconcile assigns a new ULID to the duplicate.
- [ ] I can navigate `[` / `]` between journals.
- [ ] I can kill the process and reopen — state preserved.
- [ ] `outl doctor` reports integrity OK.

## Phase 2 — P2P sync (1 → N devices)

> Note: cross-device sync is already working today over iCloud Drive (see Phase 6 below — the mobile client landed early).
> The phase 2 work below replaces iCloud with iroh so the project stops depending on a third-party cloud and starts working across non-Apple devices.
> The `outl-actions::SyncEngine` interface stays the same.

- `outl-sync` crate with iroh transport.
- Discovery via shareable ticket (`outl share` generates, `outl join <ticket>` adds a peer).
- Handshake and incremental op exchange using `last_ts_per_actor`.
- E2E tests with 2–3 instances exchanging ops over loopback iroh.
- Encryption enabled by default (iroh QUIC).
- Document trust model: who can send me ops, how to revoke.

## Phase 3 — Queries and refinement

- `{{query: ...}}` inline with an outl-specific DSL.
- Built-in aggregations: count, group-by tag, filter by property.
- Journal-specific queries: "blocks from the last 7 days", "all tasks marked priority:: high".
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

## Phase 5 — Desktop (Tauri 2) — landed

Shipped as `outl-desktop`. Tauri 2 shell with a SolidJS + Tailwind
frontend, 3-pane layout (sidebar / outline / backlinks) and
OS-standard shortcuts (`Cmd/Ctrl+P` picker, `Cmd/Ctrl+B/\\/,/T/[/]`).

What's in:

- Workspace picker (`tauri-plugin-dialog`) — open any directory,
  persisted via `settings.json` so the next launch skips the picker.
- The whole `outl-actions` block surface (create / edit / TODO /
  indent / outdent / move / delete / paste / collapsed) as Tauri
  commands.
- Cross-client navigation (`open_ref`, journals, picker).
- Code-block execution via `outl-exec` (Python / Lisp / JS / Lua / Rust-wasm runtimes already shared with the TUI).
- Cross-platform FS watcher (`notify` + debouncer) emits `peer-ops-changed` on the Tauri side, which the frontend turns into `reload_workspace`.
  Replaces the iOS-only `NSMetadataQuery` dance the mobile side carries.
- Settings modal (vim mode toggle, theme, font size).
- Frontend code that's identical to mobile (DTO types, `MarkdownInline`, paste detection, autocomplete) lives in `@outl/shared` so the two clients never duplicate.
- CI: `.github/workflows/desktop.yml` checks on every PR with a Linux job (clippy + tests + Vitest + tsc + tauri bundle) plus a build matrix on macOS arm64 and Windows.
  The release dmg is universal (arm64 + x86_64 lipo'd) and built on a single `macos-latest` runner.

Future work (Phase 6 of the desktop roadmap, not the workspace
phase):

- Signed bundles + Homebrew cask.
- Graph view (visual backlinks).
- Block-level rich editing with live preview.

## Phase 6 — Mobile (landed early as Tauri 2 + Solid)

The mobile client shipped ahead of schedule and replaces the original `uniffi` + SwiftUI plan.
`outl-mobile` is a Tauri 2 app with a SolidJS + Tailwind frontend; the Rust side is a thin command shell over `outl-actions`, plus an Objective-C iCloud watcher (`NSMetadataQuery` + `NSFileCoordinator`) in `gen/apple/Sources/outl-mobile/main.mm`.
Sync today is iCloud Drive (per-actor `ops-<actor>.jsonl`); iroh becomes the wire transport when phase 2 lands. iOS is shipping; Android follows the same Tauri 2 surface and needs an Android-side equivalent of the iCloud watcher.

**iOS public beta on TestFlight:** <https://testflight.apple.com/join/P2GdWAMd>.

---

## Issues open since day zero

These are tracked publicly to signal intent and invite contributions:

- **#1** — Add ChronDbStorage backend (labels: `roadmap`, `storage`, `help wanted`)
- **#2** — Tauri desktop app (labels: `roadmap`, `phase-5`)
- **#4** — Plugin system with rhai (labels: `roadmap`, `phase-4`)
