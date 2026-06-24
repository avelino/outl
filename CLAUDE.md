# CLAUDE.md — outl

Context for Claude Code sessions working on this repo.
Read this before making any change.

## What this project is

**outl** is a local-first outliner (Roam/Logseq replacement) with:

- **Markdown as source of truth** — `.md` files are 100% clean, no visible IDs.
- **Conflict-free sync** via a tree CRDT (Kleppmann et al. 2022).
- **Trait-based storage** — JSONL (one file per actor) is the only persistent backend; ChronDB on the roadmap.
- **TUI as a first-class citizen**, not an afterthought.
- **Journal-first** — daily notes are the primary entry point.

Full spec lives in the README and `docs/`.
Don't skim — read.

## Critical invariants (NEVER violate)

These are the non-negotiables.
Violating any one breaks user trust irreversibly.

1. **Op log is source of truth.**
   All mutations go through `Op` → `apply_op` → log.
   The materialized tree and `.md` files are projections.
   Never edit `.md` to "fix" state.

2. **Markdown stays 100% clean.**
   No `id::`, no UUID inline, no HTML comments, nothing.
   IDs live ONLY in the `.outl` sidecar (JSON file next to the `.md`, e.g. `pages/foo.outl`).
   The sidecar is **not** a dotfile, because iCloud Documents (when used as the file transport) drops dotted paths during cross-device sync and silently breaks multi-device workspaces.
   Same rule applies to `ops/`.

3. **CRDT follows Kleppmann 2022 literally.**
   `do_op` / `undo_op` / `apply_op` / `creates_cycle` must match the paper.
   100% coverage on these four is non-negotiable.

4. **Move that creates a cycle is a no-op on the materialized tree, but the op still goes into the log.**
   Removing it breaks correctness of future reordering.

5. **Storage is a trait, not a struct.**
   `JsonlStorage` is the only persistent impl, and tests use `MemoryStorage`.
   Anything that wants to persist ops goes through `dyn Storage`.
   No second persistent backend lands without an issue + RFC first, because divergence between storages is exactly what we paid to remove in 0.5.0.

6. **Delete is `Move(node, TRASH_ROOT)`, not physical removal.**
   Simplifies the algorithm and preserves history.

7. **Any state that must converge between devices goes through the op log.**
   If two users (or one user on two devices) can disagree about a value and you want them to reconcile, the state belongs in an `Op`, *never* in a shared file with last-write-wins semantics.
   The op log gives each actor its own `ops-<actor>.jsonl`, lets iroh / iCloud / Syncthing / shared FS sync per-file (no merge conflicts), and replays through the CRDT with HLC ordering for deterministic convergence.
   Writing the state into the sidecar (or any single shared file) bypasses all of that and loses concurrent writes silently.
   **Default position: model it as an Op.**
   `Op::SetCollapsed` for the fold flag is the canonical example.
   The sidecar carries only **structural matching metadata** (ids, position, content hash, ref handle), not a sync surface.

## Repo layout

```
outl/
├── CLAUDE.md                  # this file
├── README.md
├── LICENSE                    # MIT
├── Cargo.toml                 # workspace
├── rust-toolchain.toml
├── .claude/                   # agents, commands, hooks, settings
├── .github/workflows/
├── docs/                      # user + contributor reference (see docs/SUMMARY.md)
└── crates/
    ├── outl-core/             # tree CRDT, op log, storage trait
    ├── outl-md/               # parser, sidecar, matching
    ├── outl-actions/          # UI-agnostic workspace ops (shared by every client)
    ├── outl-shortcuts/        # canonical (chord, action) catalog — every client consumes it
    ├── outl-exec/             # code-block runtime (desktop + mobile)
    ├── outl-config/           # `outl.toml` parsing + schema
    ├── outl-theme/            # palette + presets (TUI + desktop)
    ├── outl-cli/              # `outl` binary
    ├── outl-tui/              # `outl-tui` binary
    ├── outl-mobile/           # Tauri 2 mobile app (iOS first)
    ├── outl-desktop/          # Tauri 2 desktop app (macOS/Linux/Windows)
    └── outl-frontend-shared/  # TS+Solid lib (@outl/shared) consumed by mobile + desktop
```

Full `docs/` index lives at [`docs/SUMMARY.md`](docs/SUMMARY.md).
Per-crate context lives in `crates/<name>/CLAUDE.md` — read it before editing that crate.

## Shared logic: `outl-actions`

Every workspace mutation a client needs to perform (edit a block, toggle TODO, indent / outdent, delete, render today's `.md`) lives in **`outl-actions`**, not in the client crate.
The mobile app and the TUI must call the **same** functions for the same semantics; if a new operation needs more than one client, it goes in `outl-actions` before its first use.

The contract is short:

- Functions take `&mut Workspace` and `&HlcGenerator`.
- They route every mutation through `Workspace::apply` (op log stays source of truth).
- They never hold UI state and never touch storage backends directly.

See `crates/outl-actions/CLAUDE.md` for the full surface and the "what this crate does NOT own" list.
**If you find yourself writing tree-walking or op-building helpers inside `outl-tui/`, `outl-mobile/`, or any future client, stop and put them in `outl-actions` first.**
The TUI's `outline_ops.rs` is the one deliberate exception (it manipulates an in-flight AST that hasn't been parsed back to a workspace yet — see that file's module doc).

## Shared frontend: `@outl/shared` (`outl-frontend-shared`)

The same "one owner, every client wraps" policy applies on the TS side.
**`crates/outl-frontend-shared/`** is the Solid + TypeScript library every GUI client (`outl-mobile`, `outl-desktop`) consumes for pieces that are pure, stateless, and identical between clients.
Examples: renderers like `<MarkdownInline />`, helpers like `looksLikeOutline` / `detectRefContext`, DTO interfaces, typed `invoke<T>()` wrappers.

Resolution: bun workspaces in the repo root `package.json` deduplicate `solid-js` / `@tauri-apps/api` across the lib + every client.
**Rule of thumb (TS):** before writing a helper in `outl-mobile/src/lib/` or `outl-desktop/src/lib/`, search `crates/outl-frontend-shared/src/`.
**Chrome stays in the client** (Sidebar, Picker, BlockRow, mode-specific keybindings, OS-specific gestures).
See `crates/outl-frontend-shared/CLAUDE.md` for the full policy.

## Reuse-first

Before adding a helper, struct, or constant, **scan the [shared primitives catalog](docs/shared-primitives.md)** and **grep the workspace** for what already does the same thing.
Two implementations of the same logic drift apart over time, and the user is the one who hits the divergence (backlinks, code-block execution, and external-markdown normalization have all been caught mid-PR for exactly this reason).

The rule, past incidents, and what to do when a primitive doesn't exist yet live in [docs/contributing.md → Reuse-first](docs/contributing.md#reuse-first-no-parallel-implementations).

## How we work in this repo

- **Build / test:** `/check` runs fmt + clippy + test on the whole workspace.
  Full dev loop (slash commands, hooks, agents, CI walkthrough) is in [`docs/development.md`](docs/development.md).
- **Specialized agents** (invoke proactively when their `When to use` matches):
  `crdt-invariant-checker`, `paper-verifier`, `markdown-roundtrip-tester`, `refactor-architect`, `doc-keeper`.
  Mandates live under `.claude/agents/`.
- **Documentation discipline.**
  When your PR touches a workflow, slash command, hook, public API, sidecar, op-log format, or shortcut, the matching docs update in the *same* PR.
  Full "if you changed X, update Y" checklist lives in [`docs/contributing.md` → Keep docs in sync](docs/contributing.md#keep-docs-in-sync-with-code).
- **One owner per fact.**
  Tables (shortcuts, CLI subcommands, config keys, op variants) live in `docs/*.md`, and `CLAUDE.md` files link, never duplicate.
  See [`docs/contributing.md` → One owner per fact](docs/contributing.md#one-owner-per-fact--link-dont-duplicate) for the canonical-home map.
- **Markdown style:** semantic line breaks (one sentence per line, no column reflow).
  Full rule in [`docs/contributing.md` → Markdown / documentation style](docs/contributing.md#markdown--documentation-style).
- **File size discipline.**
  The `file-size-guard.sh` PostToolUse hook nudges at 600 lines and stops at 900.
  When it fires, invoke the `refactor-architect` agent.
- **`cargo doc` is part of CI** with `RUSTDOCFLAGS=-D warnings`.
  Run `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` before reporting "done" on any patch that adds or changes module-level doc comments (`//!` blocks) — `/check` does not include this today.

## Decisions you don't get to revisit

These were settled before code was written.
If you think one is wrong, **stop and ask the user** before changing.
Don't unilaterally pivot.

| Decision | Why |
|----------|-----|
| `ULID` for IDs | Lexicographically sortable, 128 bits, no central server needed |
| `uhlc` for time | HLC with actor tiebreak is total order without coordination |
| Yrs for block text | Battle-tested CRDT for strings, lets us focus on the tree |
| `comrak` for markdown | CommonMark-compliant, fast, customizable |
| `iroh` as the default sync transport | QUIC + hole punching + relay, no central server for data; iroh is `[sync] transport` default |
| `file` transport as the explicit opt-out | `transport = "file"` for iCloud Drive / shared FS users; folder is user-chosen — iCloud is one option, not a dependency |
| Tauri 2 for mobile (replaces earlier uniffi plan) | Single Rust surface across TUI + mobile via `outl-actions`, Solid + Tailwind frontend, ObjC bridge only for iCloud watcher |
| Tauri for desktop (shipping today) | Rust core reuse, smaller than Electron. macOS / Linux / Windows; Solid frontend shares `@outl/shared` with mobile |
| `outl-shortcuts` is the single (chord → action) catalog | Two parallel implementations is the bug we paid to remove (TUI used to define bindings in `input/`, desktop wired its own `KeyboardEvent` handlers — `Cmd+P` and `Ctrl+P` drifted within a sprint). Adding a key on any client without going through `defaults.rs` puts that drift back. See `outl-shortcuts/CLAUDE.md` |
| One `ops-<actor>.jsonl` per device, never shared | Any file transport (iCloud, Syncthing, shared FS) is last-write-wins per file; per-actor files turn that into a non-issue; iroh ships ops directly |
| MIT license | Simple, widely understood, no patent grant baggage |
| `outl.app` domain owned | Use for docs/landing later |
| Repo at `github.com/avelino/outl` | Personal profile, not org (small enough team) |
| `[workspace.package].version` in root `Cargo.toml` is the **single source of truth** | Crate manifests inherit via `version.workspace = true`. `tauri.conf.json` deliberately omits `version`; CI reads `Cargo.toml` and injects it into `cargo tauri ios build` via `--config` (Tauri's iOS path does NOT fall back to `Cargo.toml` on its own — it defaults to `1.0.0`). Bumping the workspace bumps everything. See `crates/outl-mobile/CLAUDE.md` → "Versioning + TestFlight release" before changing release/CI plumbing |

## What you're NOT building yet

Don't add code for these unless explicitly asked:

- Query DSL (`{{query: ...}}`)
- Plugin system (`rhai`)
- `ChronDbStorage` backend (issue #1, tracked publicly)
- Android mobile build (only iOS today; Android needs an `NSMetadataQuery` equivalent)
- Per-page op log shards ([`docs/sync.md` Part 2 — Phase A](docs/sync.md#phase-a--per-page-op-log-shards-for-10k-pages); only land it when the single-jsonl-per-device layout hits the 10k-page wall)
- Character cursor inside the selected block in desktop Normal mode.
  TUI-only today.
  The desktop's vim mode has only a selected block id, so the char-level vim ops `x`/`X`/`D`/`C`/`s`/`r`/`f`/`F`/`~`/`e` surface a status-line nudge instead of firing.
  See `outl-desktop/CLAUDE.md` → "Vim parity".

## Coding conventions

- `rustfmt` default config, no overrides.
- `clippy -- -D warnings` blocks CI.
- No `unwrap()` in non-test code.
  Use `expect("explicit reason")` or propagate.
- `thiserror` in libs (`outl-core`, `outl-md`), `anyhow` at boundaries (`outl-cli`, `outl-tui`).
- No `unsafe` in `outl-core` without documented justification.
- Variable names, function names, doc comments: **English** (global audience).
- User-facing strings (CLI help, TUI labels): English for now (i18n later).
- **Conventional Commits are load-bearing.**
  Use `feat:`, `fix:`, `perf:`, `docs:`, `refactor:`, `chore:`, `test:`, `build:`, `ci:` on every commit (and on PR merge commits).
  The Mobile pipeline generates TestFlight release notes by feeding the commit log since the last tag into `conventional-changelog-cli`.
  Commits without a prefix all fall into a single "Other changes" bucket on TestFlight, so the user loses the per-build context.
  If a commit doesn't fit a type, prefer `chore:` over no prefix.

Full review policy (Rust quality, hot paths, architecture, simplicity, testing) lives in [`docs/contributing.md`](docs/contributing.md).

## Anti-patterns (don't do)

- ❌ Calling `.unwrap()` to get out of error handling
- ❌ Writing IDs into the `.md` file ("just for now")
- ❌ Storing op log fields outside the `Op` variant (breaks undo)
- ❌ Comparing HLCs without actor tiebreak
- ❌ Treating `Delete` as physical removal
- ❌ Skipping tests because "the algorithm is the same as the paper"
- ❌ Reintroducing SQLite / rusqlite / any binary log format — cross-device sync depends on per-actor append-only files
- ❌ Using `id::` Logseq-style metadata anywhere
- ❌ Marking work "done" without `/check` passing
- ❌ Re-introducing `"version"` in `crates/outl-mobile/src-tauri/tauri.conf.json` — Tauri must keep falling back to `Cargo.toml` (see "Versioning + TestFlight release" in `crates/outl-mobile/CLAUDE.md`)
- ❌ Adding a helper that re-implements something already in `outl-core` / `outl-md` / `outl-actions` (see [Reuse-first](docs/contributing.md#reuse-first-no-parallel-implementations)).
  The fix is to wrap the upstream API, not to write a parallel one.

## When in doubt

1. Read the relevant `docs/*.md`.
2. Read the per-crate `CLAUDE.md`.
3. Read the paper for sync stuff: <https://martin.kleppmann.com/papers/move-op.pdf>.
4. Ask the user.
   The user is `Avelino`, comfortable in Rust/Clojure/Python/Go, prefers direct pt-BR communication.
