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

1. **Op log is source of truth.** All mutations go through `Op` → `apply_op` → log.
   The materialized tree and `.md` files are projections.
   Never edit `.md` to "fix" state.

2. **Markdown stays 100% clean.** No `id::`, no UUID inline, no HTML comments, nothing.
   IDs live ONLY in the `.outl` sidecar (JSON file next to the `.md`, e.g. `pages/foo.outl`).
   The sidecar is **not** a dotfile — iCloud Documents drops dotted paths during cross-device sync, which silently breaks multi-device workspaces.
   Same rule applies to `ops/`.

3. **CRDT follows Kleppmann 2022 literally.** `do_op` / `undo_op` / `apply_op` / `creates_cycle` must match the paper. 100% coverage on these four is non-negotiable.

4. **Move that creates a cycle is a no-op on the materialized tree, but the op still goes into the log.** Removing it breaks correctness of future reordering.

5. **Storage is a trait, not a struct.** `JsonlStorage` is the only persistent impl; tests use `MemoryStorage`.
   Anything that wants to persist ops goes through `dyn Storage`.
   No second persistent backend lands without an issue + RFC first — divergence between storages is exactly what we paid to remove in 0.5.0.

6. **Delete is `Move(node, TRASH_ROOT)`, not physical removal.** Simplifies the algorithm and preserves history.

7. **Any state that must converge between devices goes through the op log.** If two users (or one user on two devices) can disagree about a value and you want them to reconcile, the state belongs in an `Op` — *never* in a shared file with last-write-wins semantics.
   The op log gives each actor its own `ops-<actor>.jsonl`, lets iCloud / Syncthing / shared FS sync per-file (no merge conflicts), and replays through the CRDT with HLC ordering for deterministic convergence.
   Writing the state into the sidecar (or any single shared file) bypasses all of that and loses concurrent writes silently.
   **Default position: model it as an Op.** `Op::SetCollapsed` for the fold flag is the canonical example.
   The sidecar carries only **structural matching metadata** (ids, position, content hash, ref handle) — it is not a sync surface.

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
├── docs/
│   ├── architecture.md        # design decisions
│   ├── crdt.md                # CRDT algorithm details — read this
│   ├── markdown-format.md     # outl dialect + sidecar spec
│   ├── storage.md             # trait Storage + roadmap
│   └── roadmap.md             # 6-phase plan
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

Full `docs/` index lives at [`docs/SUMMARY.md`](docs/SUMMARY.md) — don't enumerate it here (root `CLAUDE.md` → "One owner per fact").

## Shared logic: `outl-actions`

Every workspace mutation a client needs to perform (edit a block, toggle TODO, indent / outdent, delete, render today's `.md`) lives in **`outl-actions`**, not in the client crate.
The mobile app and the TUI must call the **same** functions for the same semantics; if a new operation needs more than one client, it goes in `outl-actions` before its first use.

The contract is short:

- Functions take `&mut Workspace` and `&HlcGenerator`.
- They route every mutation through `Workspace::apply` (op log stays source of truth).
- They never hold UI state and never touch storage backends directly.

See `crates/outl-actions/CLAUDE.md` for the full surface and the "what this crate does NOT own" list.
**If you find yourself writing tree-walking or op-building helpers inside `outl-tui/`, `outl-mobile/`, or any future client, stop and put them in `outl-actions` first.** The TUI's `outline_ops.rs` is the one deliberate exception (it manipulates an in-flight AST that hasn't been parsed back to a workspace yet — see that file's module doc).

## Shared frontend: `@outl/shared` (`outl-frontend-shared`)

The same "one owner, every client wraps" policy applies on the TS side.
**`crates/outl-frontend-shared/`** is the Solid + TypeScript library every GUI client (`outl-mobile` today, `outl-desktop` next) consumes for the pieces that are pure, stateless, and identical between clients:

- `<MarkdownInline />` (renderer for `InlineToken[]` produced by `outl_md::tokenize_owned`)
- `<ParseWarningsBanner />` (renders `PageView.warnings`; mirrors the TUI's `warnings_banner` chrome)
- Pure helpers: `looksLikeOutline`, `utf16OffsetToCharOffset`, `detectRefContext`, `autoClose/DeletePair`, `autoPairBracket`, `insertPair/Text`, `applySuggestion`
- DTO interfaces (`PageMeta`, `OutlineNode`, `BlockNode`, `Backlink`, `InlineToken`, `PageView`, `WorkspaceSummary`, …)
- Typed `invoke<T>()` wrappers for the Tauri commands every client uses (`@outl/shared/api/commands`)

Resolution: bun workspaces in the repo root `package.json` deduplicate `solid-js` / `@tauri-apps/api` across the lib + every client, and each client has `paths` + `resolve.alias` for `@outl/shared`.

**Rule of thumb (TS):** before writing a helper in `outl-mobile/src/lib/` or `outl-desktop/src/lib/`, search `crates/outl-frontend-shared/src/`.
If the other client already has an equivalent, promote it here in the same PR.
**Chrome stays in the client** (Sidebar, Picker, BlockRow, mode-specific keybindings, OS-specific gestures).
See `crates/outl-frontend-shared/CLAUDE.md` for the full policy.

Per-crate context lives in `crates/<name>/CLAUDE.md`.
Read it before editing that crate.

User-facing docs in `docs/`:

- `docs/crdt.md` — the algorithm and its invariants.
- `docs/architecture.md` — design decisions.
- `docs/markdown-format.md` — outl markdown dialect + sidecar format.
- `docs/storage.md` — `Storage` trait + roadmap.
- `docs/tui.md` — TUI manual (modes, keys, overlays).
- `docs/theming.md` — palette, presets, how to add a new theme.
- `docs/roadmap.md` — phase plan.
- `docs/clients.md` — shared workspace operations and how each client (TUI, mobile) plugs into them.
- `docs/cli.md` — `outl` binary surface (subcommands, JSON envelope).
- `docs/mcp.md` — Claude Desktop / Cursor wiring + MCP resources/prompts.
- `docs/development.md` — engineer onramp (clone, build, run, test, debug, ship). Pairs with `docs/contributing.md`.
- `docs/contributing.md` — review policy, invariants enforced at PR time, the quality bar.

## How we work in this repo

### Build & test

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Or just `/check`.
The PostToolUse hook in `.claude/settings.json` runs fmt + clippy on the touched crate automatically after each `Edit`/`Write`.

**`cargo doc` is part of CI** (`.github/workflows/ci.yml` — `docs` job, with `RUSTDOCFLAGS=-D warnings`).
It breaks the PR on:

- **Intra-doc links to private items.** A doc comment that writes ``[`Foo`]`` or ``[`crate::path::Foo`]`` where `Foo` is `pub(crate)` / `pub(super)` / `mod` (no `pub`) fails with `rustdoc::private_intra_doc_links`.
  The workspace is mostly `pub(crate)`, so **almost every internal type triggers this**.
  Mitigation: drop the square brackets and use backticks only (`` `Foo` ``) — same readability, no link, no warning.
- **Broken/missing doc references.** `[`Foo`]` where `Foo` doesn't exist.
- **Code blocks in doc comments that don't compile** (rare for us; we rarely put rust code in module docs).

Run `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` before reporting "done" on any patch that adds or changes module-level doc comments (`//!` blocks) — `/check` does not include this today.

### Specialized agents

Invoke proactively when relevant:

- **`crdt-invariant-checker`** — after any change in `outl-core/src/{tree,log,op}.rs`.
  Validates convergence, idempotency, cycle handling, coverage.
- **`paper-verifier`** — after writing `do_op`/`undo_op`/`apply_op`/`creates_cycle`.
  Compares Rust against paper pseudocode line by line.
- **`markdown-roundtrip-tester`** — after any change in `outl-md/`.
  Validates roundtrip stability + matching invariants.
- **`refactor-architect`** — after the file-size-guard hook fires (stop at 900 lines, warn at 600).
  Proposes a split by responsibility.
- **`doc-keeper`** — **invoke at the end of every feature** that changes public API, markdown syntax, TUI shortcut, slash command, sidecar/op-log format, CI workflow, dev loop, or user-observable behavior.
  Walks `docs/*.md` (including `docs/development.md` and `docs/contributing.md`), root `CLAUDE.md`, and per-crate `CLAUDE.md`; updates what drifted, creates only what was missing.
  **Rule of thumb:** if you'd struggle to explain the change to a contributor reading only the docs, this agent runs.
  See the **Documentation triggers** table below for the explicit "if you changed X, update Y" map.

### Slash commands

- `/check` — full quality gate (fmt + clippy + test)
- `/check-invariants` — runs CRDT test battery
- `/roundtrip` — runs outl-md matching tests
- `/coverage [crate]` — coverage report, flags uncovered critical branches
- `/new-op <Variant>` — checklist for adding a new `Op` variant
- `/init-playground` — creates a test workspace at `./playground` for manual smoke tests

### Documentation triggers

`docs/development.md` is the engineer onramp.
It drifts the moment a CI workflow, a slash command, a hook, or a per-area toolchain step changes — and a stale onramp is **worse than no onramp** because a new contributor follows it confidently into a wall.

**Treat the table below as a checklist.** If your PR touches any row on the left, update the doc on the right **in the same PR** — not "later", not "in a follow-up". The `doc-keeper` agent runs at the end of a feature to catch what slipped through; the discipline is to not let it slip in the first place.

| If your PR changes... | Update |
|---|---|
| `.github/workflows/ci.yml` (jobs, matrix, excluded crates, `RUSTDOCFLAGS`, paths-ignore) | `docs/development.md` § 9 (CI walkthrough) |
| `.github/workflows/release.yml`, `mobile.yml`, `desktop.yml`, `testflight.yml`, `bench.yml`, `cleanup-tags.yml` | `docs/development.md` § 9 (CI table) + § 10 (Release process) |
| `.claude/settings.json` hooks, `.claude/agents/*.md`, `.claude/commands/*.md` (any slash command behavior) | `docs/development.md` § 4 (Dev loop) — slash command table + hooks list + agents list |
| `rust-toolchain.toml` version bump | `docs/development.md` § 1 (Quick start) + `CONTRIBUTING.md` (Quick start) |
| Required system deps for a crate (Tauri, GTK, Bun, Xcode, hyperfine, etc.) | `docs/development.md` § 1 ("Optional toolchains by area" table) |
| New crate added to `crates/` | `docs/development.md` § 2 (Repository tour table) + root `CLAUDE.md` repo layout + per-crate `CLAUDE.md` |
| New native iOS surface (file added to `crates/outl-mobile/swift/OutlKit/Sources/`, `crates/outl-mobile/src-tauri/gen/apple/Sources/outl-mobile/`, or `main.mm`) | `docs/development.md` § 3 ("Why the mobile crate has native Swift / ObjC code" table) + § 5 (Testing — Swift rows) + § 6 (Cookbook: Touch the iOS native bridge) + `crates/outl-mobile/CLAUDE.md` if the bridge contract changes |
| New entry point pattern (e.g. new MCP tool family, new TUI overlay class, new theme registration path) | `docs/development.md` § 2 ("Entry points by intent" table) + § 6 (Cookbooks) if it's a recurring shape |
| New `Op` variant, sidecar field, op-log format change | `docs/development.md` § 6 (Cookbook: Add a new `Op` variant) + `docs/crdt.md` + `outl-md/CLAUDE.md` |
| `/check` / `/check-invariants` / `/roundtrip` / `/coverage` / `/new-op` / `/init-playground` semantics | `docs/development.md` § 4 (Dev loop slash command table) |
| Benchmark layout (new bench file, new size tier, hyperfine recipe) | `docs/development.md` § 8 (Performance) |
| Version source-of-truth or release tooling (e.g. someone proposes re-adding `version` to `tauri.conf.json`) | `docs/development.md` § 10 (Release process) + `crates/outl-mobile/CLAUDE.md` |
| Conventional Commits enforcement / release-notes pipeline | `docs/development.md` § 10 + root `CLAUDE.md` "Coding conventions" |
| Storage trait surface, `JsonlStorage` / `MemoryStorage` test contract | `docs/development.md` § 5 ("What to mock and what not to") + `docs/storage.md` + `outl-core/CLAUDE.md` |
| New `Action` variant in `outl-shortcuts` / new keybinding / chord rebound | `docs/shortcuts.md` (the row that ships to users) + `outl-shortcuts/src/{action.rs,defaults.rs}` + every client's dispatcher (`outl-tui/src/input/*.rs`, `outl-desktop/src/lib/{shortcuts.ts,action-handlers.ts}`) + `outl-desktop/src/lib/api.ts` (TS mirror of the `Action` union — no codegen) |

When in doubt: **if a contributor's first 30 minutes with the repo would land them on outdated guidance, update the doc.** That's the bar.

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
| `iroh` for P2P (phase 2) | QUIC + hole punching, no central server |
| iCloud Drive as v0 transport (mobile + TUI today) | Zero infra, ships now, replaceable behind the same `outl-actions::SyncEngine` when iroh lands |
| Tauri 2 for mobile (replaces earlier uniffi plan) | Single Rust surface across TUI + mobile via `outl-actions`, Solid + Tailwind frontend, ObjC bridge only for iCloud watcher |
| Tauri for desktop (shipping today) | Rust core reuse, smaller than Electron. macOS / Linux / Windows; Solid frontend shares `@outl/shared` with mobile. |
| `outl-shortcuts` is the single (chord → action) catalog | Two parallel implementations is the bug we paid to remove (TUI used to define bindings in `input/`, desktop wired its own `KeyboardEvent` handlers — `Cmd+P` and `Ctrl+P` drifted within a sprint). Adding a key on any client without going through `defaults.rs` puts that drift back. See `outl-shortcuts/CLAUDE.md`. |
| One `ops-<actor>.jsonl` per device, never shared | iCloud (and any file transport) is last-write-wins per file; per-actor files turn that into a non-issue |
| MIT license | Simple, widely understood, no patent grant baggage |
| `outl.app` domain owned | Use for docs/landing later |
| Repo at `github.com/avelino/outl` | Personal profile, not org (small enough team) |
| `[workspace.package].version` in root `Cargo.toml` is the **single source of truth** | Crate manifests inherit via `version.workspace = true`. `tauri.conf.json` deliberately omits `version`; CI reads `Cargo.toml` and injects it into `cargo tauri ios build` via `--config` (Tauri's iOS path does NOT fall back to `Cargo.toml` on its own — it defaults to `1.0.0`). Bumping the workspace bumps everything. See `crates/outl-mobile/CLAUDE.md` → "Versioning + TestFlight release" before changing release/CI plumbing. |

## What you're NOT building yet

Don't add code for these unless explicitly asked:

- P2P sync transport (`iroh`) — iCloud is the v0 transport; iroh replaces it later, behind the same `SyncEngine` interface.
- Query DSL (`{{query: ...}}`)
- Plugin system (`rhai`)
- `ChronDbStorage` backend (issue #1, tracked publicly)
- Android mobile build (only iOS today; Android needs an `NSMetadataQuery` equivalent)
- Per-page op log shards ([`docs/sync.md` Part 2 — Phase A](docs/sync.md#phase-a--per-page-op-log-shards-for-10k-pages); only land it when the single-jsonl-per-device layout hits the 10k-page wall)
- Character cursor inside the selected block in desktop Normal mode (TUI-only today; the desktop's vim mode has only a selected block id, so the char-level vim ops `x`/`X`/`D`/`C`/`s`/`r`/`f`/`F`/`~`/`e` surface a status-line nudge instead of firing — see `outl-desktop/CLAUDE.md` → "Vim parity")

## Coding conventions

- `rustfmt` default config, no overrides
- `clippy -- -D warnings` blocks CI
- No `unwrap()` in non-test code.
  Use `expect("explicit reason")` or propagate.
- `thiserror` in libs (`outl-core`, `outl-md`), `anyhow` at boundaries (`outl-cli`, `outl-tui`)
- No `unsafe` in `outl-core` without documented justification
- Variable names, function names, doc comments: **English** (global audience)
- User-facing strings (CLI help, TUI labels): English for now (i18n later)
- **Conventional Commits are load-bearing.** Use `feat:`, `fix:`, `perf:`, `docs:`, `refactor:`, `chore:`, `test:`, `build:`, `ci:` on every commit (and on PR merge commits).
  The Mobile pipeline generates TestFlight release notes by feeding the commit log since the last tag into `conventional-changelog-cli` (preset `conventionalcommits`); the rendered markdown lands as the build's "What to Test" text via the App Store Connect API.
  Commits without a prefix all fall into a single "Other changes" bucket on TestFlight, so the user loses the per-build context.
  If a commit doesn't fit a type, prefer `chore:` over no prefix.

### Markdown / documentation style

**Never hard-wrap prose at an arbitrary column.** We use [semantic line breaks](https://sembr.org/): one sentence per line, breaking after sentence-ending punctuation (`.`, `!`, `?`) and sometimes after `:` when followed by a substantial clause.
Lines stay as long as the sentence is — no 70/80/100-column reflow.

Why: hard-wrapping at ~70 chars breaks lines mid-thought, makes diffs noisier on edits, and renders ugly in editors that already soft-wrap.
Semantic line breaks keep diffs minimal (an edit touches one line, not a paragraph block) and read naturally on every surface (GitHub, mdBook, terminal pagers).

Rules:

- **Prose**: one sentence per line.
  Don't break inside a sentence.
- **Lists**: each list item on its own line; if a single item contains multiple sentences, break those sentences too.
- **Code fences, tables, YAML frontmatter, ASCII tree diagrams**: preserve **exactly**.
  Tables especially must stay one row per line, no matter how wide.
- **Headings, HRs, link references**: one line, as always.
- **Outline / `.md` content** (anything under `note-example/`, real workspace pages, fixtures): **do not touch**.
  That markdown is data, not docs — it represents the outl dialect literally and indentation / line shape is structural.

This rule applies to every `*.md` in the repo except outline content (see exception above): root `CLAUDE.md`, per-crate `CLAUDE.md`, `docs/*.md`, `README.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md`, `.github/*.md`, `.claude/agents/*.md`, `.claude/commands/*.md`.

### One owner per fact — link, don't duplicate

**Every user-facing fact lives in exactly one `docs/*.md`. `CLAUDE.md` files link to it instead of copying.**
A keybinding, a CLI subcommand, a slash command, a CRDT op variant, a config field, a sidecar field — each has one canonical home under `docs/`, and the matching `CLAUDE.md` (root or per-crate) **links** to it. It does not enumerate the same rows in its own table.

Why this matters: every duplicated table is a future drift incident.
We've already hit it on `docs/shortcuts.md` ↔ per-crate vim tables (the desktop's CLAUDE.md ended up listing chords that drifted from `defaults.rs` within one sprint) and on `outl-tui/CLAUDE.md`'s Navigation / Insert tables (rebound `Ctrl+E` for sidebar, doc kept showing `\`).
When the same row lives in two places, half the time the second copy goes stale silently and the contributor following it walks into a wall.

The discipline:

- **`docs/*.md` is the canonical surface for users + contributors.** Tables, full chord lists, every config key, the full subcommand matrix — they live there.
- **`CLAUDE.md` (root or per-crate) carries only what `docs/` cannot:** invariants, "architectural decisions you don't get to revisit", crate-specific contracts (which methods are the entrypoint, what the layering rule is), the reasoning behind a choice. Things a contributor needs *before* touching code — not user-facing reference.
- **When the same fact would live in both,** the `CLAUDE.md` writes a one-line link: `> User-facing X lives in [`docs/X.md`](../docs/X.md) — don't duplicate it here.` and stops.
- **If you find yourself copying a table from `docs/` into a `CLAUDE.md`, stop.** Replace with a link.
- **The other direction is fine.** `docs/*.md` linking *into* `CLAUDE.md` for architectural context is welcome — that's a one-way pointer toward depth, not duplication.

Map of canonical homes (extend as new ones are minted):

| Fact | Lives in | `CLAUDE.md` files link, do not duplicate |
|---|---|---|
| Every keyboard shortcut (TUI + desktop + mobile, side-by-side) | [`docs/shortcuts.md`](docs/shortcuts.md) | `outl-tui/CLAUDE.md`, `outl-desktop/CLAUDE.md`, `outl-shortcuts/CLAUDE.md` |
| `outl` CLI subcommands + JSON envelope | [`docs/cli.md`](docs/cli.md) | `outl-cli/CLAUDE.md` |
| TUI manual (modes, overlays, visual conventions) | [`docs/tui.md`](docs/tui.md) | `outl-tui/CLAUDE.md` |
| Outl markdown dialect + sidecar spec | [`docs/markdown-format.md`](docs/markdown-format.md) | `outl-md/CLAUDE.md` |
| CRDT algorithm + invariants | [`docs/crdt.md`](docs/crdt.md) | `outl-core/CLAUDE.md` |
| Storage trait + JSONL backend | [`docs/storage.md`](docs/storage.md) | `outl-core/CLAUDE.md` |
| Sync model (iCloud / Syncthing / iroh roadmap) | [`docs/sync.md`](docs/sync.md) | `outl-mobile/CLAUDE.md`, `outl-desktop/CLAUDE.md` |
| MCP wiring + recipes | [`docs/mcp.md`](docs/mcp.md) + [`docs/mcp-recipes.md`](docs/mcp-recipes.md) | (no per-crate CLAUDE.md owns this today) |
| Config file (`outl.toml`) | [`docs/config.md`](docs/config.md) | per-crate CLAUDE.md where the field is read |
| Theming palette + presets | [`docs/theming.md`](docs/theming.md) | `outl-tui/CLAUDE.md`, `outl-desktop/CLAUDE.md` |
| Dev loop (clone, build, slash commands, hooks, agents, CI) | [`docs/development.md`](docs/development.md) | every per-crate CLAUDE.md's "When you're done" section links here |
| Contributing policy (review, invariants enforced at PR time) | [`docs/contributing.md`](docs/contributing.md) | root `CLAUDE.md` references it |

When you add a brand-new surface (a new CLI subcommand, a new `Op` variant, a new MCP tool, a new theme, a new client), it follows the same rule:

1. Document the surface in the right `docs/*.md` (create a new one if needed).
2. The per-crate `CLAUDE.md` adds a one-line link if it needs to point contributors at it, plus any architectural note that *cannot* live in `docs/` (invariant, contract, "why this decision").
3. Update the map above so the next contributor doesn't have to rediscover where things live.

**`doc-sync-guard.sh` is a backstop, not the discipline.** The hook flags drift after the fact; the discipline is to link in the first place so drift can't happen.

### Shared primitives catalog

**Before writing any helper, scan these tables first.** Most "I need a small string transform / id helper / md coercion / tree walk" needs already have an owner here — the cost of finding the existing one is a `grep`; the cost of missing it shows up later as drift between two parallel implementations (the user is the one who hits the divergence).

> This catalog is mirrored at `.github/copilot-instructions.md` §5.1.
> When you edit either copy, sync both — a `PostToolUse` hook flags drift, but the discipline starts before the hook fires.

The catalog is grouped by area.
Skim the headings, then drill in.

#### 1. Workspace lifecycle, op log, and HLC (outl-core)

| Intent | Use this | File |
|---|---|---|
| Open a workspace (in-memory for tests, on-disk JSONL for prod) | `outl_core::Workspace::open_in_memory` / `open_with_storage` | `crates/outl-core/src/workspace.rs` |
| Route an op through the log → tree (the **only** mutation path) | `outl_core::Workspace::apply(LogOp)` | `crates/outl-core/src/workspace.rs` |
| Read the materialized tree / op log from a workspace | `outl_core::Workspace::tree` / `log` / `block_text` | `crates/outl-core/src/workspace.rs` |
| Build a Yrs text-replace update payload for an op | `outl_core::Workspace::build_text_replace_update` | `crates/outl-core/src/workspace.rs` |
| Generate HLC timestamps with actor tiebreak (required for every op) | `outl_core::HlcGenerator::new` / `next` / `observe` | `crates/outl-core/src/hlc.rs` |
| Wrap an `Op` into a `LogOp` (timestamp + actor) for `apply` | `outl_core::Op` + `outl_core::LogOp` | `crates/outl-core/src/op.rs` |
| Sentinel node ids (`root`, `trash`) | `outl_core::NodeId::root()` / `trash()` | `crates/outl-core/src/id.rs` |
| Per-device identity for ops | `outl_core::ActorId` | `crates/outl-core/src/id.rs` |
| Fractional index for sibling ordering | `outl_core::Fractional` | `crates/outl-core/src/fractional.rs` |

#### 2. Tree reads (outl-core + outl-actions::tree)

| Intent | Use this | File |
|---|---|---|
| Does a node still exist in the tree? | `Tree::contains` | `crates/outl-core/src/tree/mod.rs` |
| Parent of a node | `Tree::parent` | `crates/outl-core/src/tree/mod.rs` |
| Fractional position of a node | `Tree::position` | `crates/outl-core/src/tree/mod.rs` |
| Single property lookup on a node | `Tree::property` | `crates/outl-core/src/tree/mod.rs` |
| Iterate every property currently set on a node | `Tree::properties_of` | `crates/outl-core/src/tree/mod.rs` |
| Collapsed flag for a node | `Tree::is_collapsed` / `collapsed_ids` | `crates/outl-core/src/tree/mod.rs` |
| Walk every node in the tree | `Tree::iter_nodes` / `node_count` | `crates/outl-core/src/tree/mod.rs` |
| Children of a parent (in fractional order) | `outl_actions::tree::children_of` | `crates/outl-actions/src/tree.rs` |
| Walk a subtree applying a closure | `outl_actions::tree::walk_subtree` | `crates/outl-actions/src/tree.rs` |
| Sibling after a node + position helpers (for inserts) | `outl_actions::tree::next_sibling` / `position_after` / `position_for_new_last_child` | `crates/outl-actions/src/tree.rs` |
| Which page (slug-bearing root child) does this node sit under? | `outl_actions::tree::enclosing_page_id` | `crates/outl-actions/src/tree.rs` |

#### 3. Block mutations (outl-actions::block + collapsed + todo + quote)

Every entry here routes through `Workspace::apply` — never build a `LogOp` from a client and apply it directly.

| Intent | Use this | File |
|---|---|---|
| Append a single block under a parent | `outl_actions::block::append_block` | `crates/outl-actions/src/block/create.rs` |
| Append a tree / forest (with children) under a parent | `outl_actions::block::append_tree` / `append_forest` (uses `BlockTreeSpec` → returns `BlockTreeOutcome`) | `crates/outl-actions/src/block/create.rs` |
| Create sibling after / child under a block | `outl_actions::block::create_after` / `create_under` | `crates/outl-actions/src/block/create.rs` |
| Edit a block's text | `outl_actions::block::edit_text` | `crates/outl-actions/src/block/edit.rs` |
| Indent / outdent / move up / move down a block | `outl_actions::block::indent` / `outdent` / `move_up` / `move_down` | `crates/outl-actions/src/block/moves.rs` |
| Move a block to sit **after an arbitrary target** (cut-and-paste-block; crosses pages; emits one `Op::Move`, preserving id + refs; rejects self-subtree cycles) | `outl_actions::block::move_after` | `crates/outl-actions/src/block/moves.rs` |
| Delete a block (`Move(node, TRASH_ROOT)`, **never** physical) | `outl_actions::block::delete` | `crates/outl-actions/src/block/moves.rs` |
| Toggle a block's collapsed flag (converges via `Op::SetCollapsed`) | `outl_actions::collapsed::toggle_block_collapsed` / `set_block_collapsed` | `crates/outl-actions/src/collapsed.rs` |
| Cycle / split / read TODO/DONE state (encoded as text prefix) | `outl_actions::todo::cycle_todo` / `split_todo` / `TodoState` / `TODO_PREFIX` / `DONE_PREFIX` | `crates/outl-actions/src/todo.rs` |
| Toggle TODO/DONE on a block in one call | `outl_actions::block::toggle_todo` | `crates/outl-actions/src/block/edit.rs` |
| Read / toggle blockquote state (encoded as `"> "` text prefix, CommonMark-compatible) | `outl_actions::quote::is_quote` / `split_quote` / `toggle_quote` / `QUOTE_PREFIX` | `crates/outl-actions/src/quote.rs` |
| Toggle blockquote on a block in one call | `outl_actions::block::toggle_quote` | `crates/outl-actions/src/block/edit.rs` |

#### 4. Pages and journals (outl-actions::page + journal)

| Intent | Use this | File |
|---|---|---|
| Page-property keys (constants — don't hardcode the strings) | `outl_actions::page::SLUG_KEY` / `KIND_KEY` / `TYPE_KEY` | `crates/outl-actions/src/page.rs` |
| Canonical `type::` value marking a page as a person (`@` mention autocomplete filter) | `outl_actions::page::PERSON_TYPE` | `crates/outl-actions/src/page.rs` |
| Page metadata (slug, kind, title, **`page_type`**) for a node id | `outl_actions::page::page_meta` / `PageMeta` / `PageKind` | `crates/outl-actions/src/page.rs` |
| Validate a slug for filesystem safety (`..`, `/`, `\`, control chars) | `outl_actions::page::is_valid_slug` | `crates/outl-actions/src/page.rs` |
| Derive a **deterministic page id** from slug (so two peers converge) | `outl_actions::page::page_id_from_slug` | `crates/outl-actions/src/page.rs` |
| Find / list / create-if-missing pages | `outl_actions::page::find_by_slug` / `list_all` / `open_or_create` | `crates/outl-actions/src/page.rs` |
| Open-or-create a page from a **human-typed name** (slugifies + keeps original as title, used when a `[[ref]]` / `#tag` / picker query may not be a valid slug) | `outl_actions::page::open_or_create_by_name` | `crates/outl-actions/src/page.rs` |
| Open-or-create whatever a **user-typed ref target** points at (date → journal, else literal/slugified/title match → existing page, else create) — handles `@`-prefixed mentions by stripping the `@` and marking new pages as `type:: person`; the one decision tree so frontend regex and backend parser cannot drift on `[[2026-13-01]]` or `[[@avelino]]` | `outl_actions::page::open_or_create_by_ref` | `crates/outl-actions/src/page.rs` |
| Search pages typed `type:: person`, fuzzy-ranked by query (powers the `@` mention autocomplete in every client) | `outl_actions::page::search_persons` | `crates/outl-actions/src/page.rs` |
| Read / write a property on a page (or any node) | `outl_actions::page::read_text_prop` / `set_property` | `crates/outl-actions/src/page.rs` |
| Migrate pre-page-model blocks under today's journal (run on boot) | `outl_actions::page::migrate_legacy_into_today` | `crates/outl-actions/src/page.rs` |
| Open / create the journal for a specific date or today | `outl_actions::page::open_journal` / `open_today` | `crates/outl-actions/src/page.rs` |
| Journal date utilities (today, slug ↔ date, prev/next day) | `outl_actions::page::today` / `journal_slug` / `journal_title` / `date_from_slug` / `previous_journal_date` / `next_journal_date` | `crates/outl-actions/src/page.rs` |
| Filesystem paths for journals / pages / a specific page | `outl_actions::journal::journals_dir` / `pages_dir` / `page_md_path` | `crates/outl-actions/src/journal.rs` |
| Render a page node out to `.md` | `outl_actions::journal::render_page_md` | `crates/outl-actions/src/journal.rs` |
| Render a single block + its subtree to `.md` (copy-block clipboard; same projection as the page render) | `outl_actions::journal::render_block_md` | `crates/outl-actions/src/journal.rs` |
| Apply an edited `.md` back into the workspace (with / without sidecar) | `outl_actions::journal::apply_page_md` / `apply_page_md_with_sidecar` | `crates/outl-actions/src/journal.rs` |
| Apply every page's `.md` to disk in one pass | `outl_actions::journal::apply_all_pages_md` | `crates/outl-actions/src/journal.rs` |
| Run a closure that mutates a page's `.md` (read → modify → write atomically) | `outl_actions::journal::mutate_page_md` | `crates/outl-actions/src/journal.rs` |
| Atomic `.md` write (crash-safe, wraps `outl_md::atomic::write_atomic`) | `outl_actions::journal::write_md_atomic` | `crates/outl-actions/src/journal.rs` |

#### 5. Parse / render (outl-md::parse + render)

| Intent | Use this | File |
|---|---|---|
| Parse `.md` → outline AST (no IDs) | `outl_md::parse::parse` → `ParsedPage` (includes `warnings: Vec<ParseWarning>`) | `crates/outl-md/src/parse.rs` |
| Render outline AST → `.md` (clean, no IDs) | `outl_md::render::render` | `crates/outl-md/src/render.rs` |
| Non-fatal parser recovery records (heading instead of bullet, etc.) | `outl_md::ParseWarning` + `outl_md::ParseWarningKind` (re-exported from `parse`) | `crates/outl-md/src/parse.rs` |
| The outline AST node DTO (UI-friendly, no `Workspace` coupling) | `outl_md::OutlineNode` / `outl_actions::outline::OutlineNode` | `crates/outl-md/src/parse.rs` + `crates/outl-actions/src/outline.rs` |
| Project the workspace tree under a node into the UI DTO | `outl_actions::outline::project_outline` / `project_outline_node` | `crates/outl-actions/src/outline.rs` |
| Flatten an `OutlineNode` subtree to DFS paths (for selection / navigation) | `outl_actions::outline::flatten_subtree_paths` | `crates/outl-actions/src/outline.rs` |
| Read a page from disk + project to outline view in one call | `outl_actions::outline::read_page_view` / `read_page_view_with_workspace` | `crates/outl-actions/src/outline.rs` |
| Read a page **and** surface parser warnings (banner, doctor, status line) | `outl_actions::outline::read_page_outline` / `read_page_outline_with_workspace` → `PageOutline { nodes, warnings }` | `crates/outl-actions/src/outline.rs` |

#### 6. External markdown coercion & ingest (outl-actions::paste + ingest)

| Intent | Use this | File |
|---|---|---|
| Coerce **external markdown** (line endings, indent unit 4→2, Roam/GitHub/Logseq tokens, long-form dates → ISO, strip `id::` with Crockford validation, strip unknown `{{…}}` / `^^…^^`) | `outl_actions::paste::normalize_external_syntax` | `crates/outl-actions/src/paste/normalize.rs` |
| "Does this clipboard look like an outline?" classifier | `outl_actions::paste::looks_like_outline` | `crates/outl-actions/src/paste/mod.rs` |
| Convert clipboard markdown into outl ops grafted at a position | `outl_actions::paste::paste_markdown` → `PasteOutcome` (anchor described by `PasteAnchor`) | `crates/outl-actions/src/paste/mod.rs` |
| **Ingest a `.md` as a real page** (creates page node + reconciles blocks; used by import / `serve` / mobile + TUI orphan scanners) | `outl_actions::ingest::ingest_md_file` / `ingest_dir` | `crates/outl-actions/src/ingest.rs` |
| Create stub pages for every `[[ref]]` with no file of its own (Logseq "implicit pages") | `outl_actions::ingest::create_missing_ref_pages` | `crates/outl-actions/src/ingest.rs` |

#### 7. Reconcile & matching (outl-md::reconcile + matching + diff)

| Intent | Use this | File |
|---|---|---|
| Reconcile an existing `.md` against its sidecar (3-level matching → diff → min ops) | `outl_md::reconcile::reconcile_md` (no sidecar = fresh random id) / `reconcile_md_with_page_id` (pin id for first ingest) | `crates/outl-md/src/reconcile.rs` |
| Reconcile every `.md` in a directory | `outl_md::reconcile::reconcile_dir` | `crates/outl-md/src/reconcile.rs` |
| Reconcile error / report types | `outl_md::ReconcileError` / `ReconcileReport` | `crates/outl-md/src/reconcile.rs` |
| 3-level matching algorithm (hash → similarity → orphan log) | `outl_md::matching::match_blocks` → `Match` / `MatchLevel` | `crates/outl-md/src/matching.rs` |
| Diff old AST + new AST + old sidecar → minimum sequence of `Op`s | `outl_md::diff::diff_to_ops` → `DiffPlan` | `crates/outl-md/src/diff.rs` |
| Same diff, **plus** propagate page-level properties (`title::`, `type::`, `pinned::`, `icon::`, …) into the op log as `Op::SetProp` on the page root so the CRDT tree agrees with what's on disk (legacy `.md` files populated via fixtures / external editors get materialised here on the next reconcile) | `outl_md::diff::diff_to_ops_with_page_props` | `crates/outl-md/src/diff.rs` |
| Reconcile-pipeline version number stamped on every sidecar — orphan scanner re-runs `reconcile_md` when a sidecar's version is below this constant, so a binary that gains a new pipeline pass automatically rematerialises every legacy page on the next boot | `outl_md::sidecar::CURRENT_PIPELINE_VERSION` | `crates/outl-md/src/sidecar.rs` |

#### 8. Sidecar (outl-md::sidecar + atomic)

| Intent | Use this | File |
|---|---|---|
| The full sidecar struct + per-block entries | `outl_md::Sidecar` / `SidecarBlock` | `crates/outl-md/src/sidecar.rs` |
| Construct a fresh sidecar for a new page | `outl_md::sidecar::Sidecar::new_for_page(page_id, &file_hash)` | `crates/outl-md/src/sidecar.rs` |
| Read / write sidecar (JSON, version 2, backward-reads v1) | `outl_md::sidecar::read` / `write` | `crates/outl-md/src/sidecar.rs` |
| Sidecar path resolution for a `.md` | `outl_md::sidecar::sidecar_path_for` / `resolve_sidecar_path` | `crates/outl-md/src/sidecar.rs` |
| Derive `((blk-XXXXXX))` ref handle from `NodeId` (deterministic, collision-aware) | `outl_md::sidecar::derive_ref_handle` | `crates/outl-md/src/sidecar.rs` |
| Hash block / file content for sidecar (`content_hash` = single block; `file_hash` = whole `.md`) | `outl_md::sidecar::content_hash` / `file_hash` | `crates/outl-md/src/sidecar.rs` |
| Low-level crash-safe write (use the `journal::write_md_atomic` wrapper unless you have a reason) | `outl_md::atomic::write_atomic` | `crates/outl-md/src/atomic.rs` |

#### 9. In-flight outline AST helpers (outl-md::outline_ops)

These operate on `Vec<OutlineNode>` **before** the tree is rebuilt from the op log — typing into a buffer that hasn't been parsed back yet.
UI-agnostic; both TUI and mobile consume them.

| Intent | Use this | File |
|---|---|---|
| Flat count / TODO+DONE counts across an outline | `outline_ops::flat_count` / `count_todos` | `crates/outl-md/src/outline_ops.rs` |
| Convert flat index ↔ path / look up a node at a path | `outline_ops::path_for_index` / `index_for_path` / `node_at_path` / `node_at_path_mut` | `crates/outl-md/src/outline_ops.rs` |
| Count descendants under a path / grab a mutable siblings slice | `outline_ops::descendants_count_at_path` / `siblings_mut` | `crates/outl-md/src/outline_ops.rs` |
| Insert a sibling before / after a path | `outline_ops::insert_sibling_before` / `insert_sibling_after` | `crates/outl-md/src/outline_ops.rs` |
| Indent / outdent / delete / move up / move down at a path | `outline_ops::indent_at_path` / `outdent_at_path` / `delete_at_path` / `move_up_at_path` / `move_down_at_path` | `crates/outl-md/src/outline_ops.rs` |

#### 10. Indices and search (outl-md::index + block_index)

| Intent | Use this | File |
|---|---|---|
| Build / query the workspace-wide index (slug → page, backlinks, block lookups) | `outl_md::WorkspaceIndex::build` / `by_slug` / `by_title` / `pages` / `pages_by_title_prefix` / `pages_by_type` | `crates/outl-md/src/index.rs` |
| Patch / remove a page in an existing index | `WorkspaceIndex::patch_page` / `remove_page` | `crates/outl-md/src/index.rs` |
| Resolve `((blk-XXXXXX))` to a block / look a block up by id or location | `WorkspaceIndex::resolve_block_ref` / `block_by_id` / `block_at_location` | `crates/outl-md/src/index.rs` |
| Reverse refs to a block / iterate / search | `WorkspaceIndex::block_refs_to` / `iter_blocks` / `search_block_text` / `block_count` | `crates/outl-md/src/index.rs` |
| Stand-alone block-level index (when you don't need the page facade) | `outl_md::BlockIndex` + `BlockEntry` + `BlockReference` | `crates/outl-md/src/block_index.rs` |
| `PageEntry` DTO returned by `WorkspaceIndex` lookups | `outl_md::PageEntry` | `crates/outl-md/src/index.rs` |

#### 11. View helpers for editors (outl-md::view + inline)

| Intent | Use this | File |
|---|---|---|
| Char ↔ (line, col) on a buffer (both TUI and mobile editors share) | `outl_md::view::char_to_line_col` / `line_col_to_char` | `crates/outl-md/src/view.rs` |
| Project a block to renderable rows (with `BlockRowKind` discrimination) | `outl_md::view::block_to_rows` → `BlockRow` / `BlockRowKind` | `crates/outl-md/src/view.rs` |
| Tokenize inline markdown (`**bold**`, `[[refs]]`, `#tags`, `((blk-…))`, `!((blk-…))`) | `outl_md::inline::tokenize` → `InlineTok` | `crates/outl-md/src/inline.rs` |
| Tokenize inline markdown into an **owned, Serde-friendly** form for wire / DTO payloads (mobile renders these straight; no parallel TS tokenizer) | `outl_md::inline::tokenize_owned` → `InlineToken` | `crates/outl-md/src/inline.rs` |
| Reconstruct the source markdown from a `Vec<InlineTok>` (Bold / Italic / Strike now carry recursively-tokenized inners; use this when a surface wants the whole inner span as one styled string instead of dispatching per-variant) | `outl_md::inline::inline_to_source` | `crates/outl-md/src/inline.rs` |
| Resolve the ref under a caret position (`Page` / `Journal` / `Tag` / `Block`) | `outl_md::inline::ref_at_cursor` → `RefTarget` | `crates/outl-md/src/inline.rs` |
| Validate a `((blk-XXXXXX))` handle string | `outl_md::inline::is_valid_block_handle` | `crates/outl-md/src/inline.rs` |
| Byte offset for a char index (UTF-8 safe) | `outl_md::inline::byte_index_for_char` | `crates/outl-md/src/inline.rs` |
| Canonicalize a fence info-string (`rs` → `rust`, `js`/`javascript`/`node` → `js`, …) — single source of truth for both `outl-exec`'s runtime dispatch and the frontend syntax highlighter | `outl_md::lang::canonical`, `outl_md::lang::KNOWN_ALIASES` | `crates/outl-md/src/lang.rs` |
| Resolve a `:shortcode:` to its unicode glyph (one-way; never retro-translate glyph → shortcode, multiple shortcodes can alias the same codepoint) | `outl_md::emoji::shortcode_to_unicode` | `crates/outl-md/src/emoji.rs` |
| Validate the `[a-z0-9_+-]+` shape of an emoji shortcode (does **not** check the catalog — that's `shortcode_to_unicode`) | `outl_md::emoji::is_valid_shortcode` | `crates/outl-md/src/emoji.rs` |
| Validate **one char** of a shortcode (`[a-z0-9_+-]`) — use this when walking the buffer char-by-char (`try_emoji`, TUI's `detect_trigger`) so you don't allocate a 1-char `String` per keystroke just to call `is_valid_shortcode` | `outl_md::emoji::is_valid_shortcode_char` | `crates/outl-md/src/emoji.rs` |
| Search the GitHub gemoji catalog for shortcodes matching a query (exact → prefix → substring; shorter shortcodes win ties) — powers the `:emoji:` autocomplete in every client through one shared `outl_emoji_search` Tauri command | `outl_md::emoji::search` → `EmojiHit` | `crates/outl-md/src/emoji.rs` |

#### 12. Backlinks (outl-actions::backlinks)

| Intent | Use this | File |
|---|---|---|
| Extract `[[ref]]` tokens out of a block's text (tolerates unbalanced openers) | `outl_actions::backlinks::extract_refs` | `crates/outl-actions/src/backlinks.rs` |
| Backlink DTO returned by the queries below | `outl_actions::backlinks::Backlink` | `crates/outl-actions/src/backlinks.rs` |
| Walk every backlink for a target / a `PageMeta` (matches `[[ref]]` literally **and** `#tag` via slugify — same resolution a tag click uses) | `outl_actions::backlinks::backlinks_for_target` / `backlinks_for_page` | `crates/outl-actions/src/backlinks.rs` |

#### 13. Code-block execution (outl-actions::exec)

The **cross-client glue** every UI uses to wire a "run this fence" gesture (TUI `g x`, desktop `Cmd+Shift+X`, mobile long-press → "Run code") through to `outl-exec` and back. `outl_actions::exec::run_code_block` is the **only** entry point a Tauri command / TUI action should call — never re-implement the flat-DFS walk, the `.md` path lookup, or the DTO shape per client.

| Intent | Use this | File |
|---|---|---|
| Resolve a `NodeId` to its flat DFS index inside an outline forest (the order `outl_exec::run_block_at_index` expects) | `outl_actions::flat_index_for_block` | `crates/outl-actions/src/outline.rs` |
| Orchestrate execution: walk DFS, resolve `.md` path, call `outl_exec::run_block_at_index`, build DTO | `outl_actions::exec::run_code_block` | `crates/outl-actions/src/exec.rs` |
| Serializable mirror of `outl_exec::ExecOutput` (stdout/stderr/duration_ms/exit) | `outl_actions::ExecOutputDto` | `crates/outl-actions/src/exec.rs` |
| Outcome shipped to the client (`language` + `result_ok` xor `error`; client adds the refreshed `view`) | `outl_actions::RunCodeBlockOutcome` | `crates/outl-actions/src/exec.rs` |

The runtime catalog (which languages are available) is selected by the **binary** that consumes this crate, via `outl-exec` features in its own `Cargo.toml`. `outl-actions` itself depends on `outl-exec` with `default-features = false` so it doesn't drag `wasmtime` (Rust runtime) into the mobile IPA via the back door.

#### 14. Sync engine, locks, storage trait

| Intent | Use this | File |
|---|---|---|
| The shared sync entry point (TUI poller + mobile iCloud watcher both use it) | `outl_actions::SyncEngine::new` | `crates/outl-actions/src/sync.rs` |
| Reload workspace from disk after a peer change | `SyncEngine::reload_workspace` | `crates/outl-actions/src/sync.rs` |
| Re-project a page's `.md` + sidecar to disk / reload + reproject in one call | `SyncEngine::reproject_page` / `refresh_page` | `crates/outl-actions/src/sync.rs` |
| Snapshot every / peer-only `ops-*.jsonl` (size + mtime) for change detection | `SyncEngine::snapshot` / `snapshot_peers` (`OpsFileSnapshot`) | `crates/outl-actions/src/sync.rs` |
| Scan `journals/` + `pages/` for orphan `.md` (no sidecar / stale hash) | `SyncEngine::scan_for_orphans` | `crates/outl-actions/src/sync.rs` |
| Acquire the cross-process workspace lock (one writer at a time) | `outl_core::WorkspaceLock::acquire` | `crates/outl-core/src/lock.rs` |
| Acquire the per-actor write lock (one process writing this actor's jsonl) | `outl_core::ActorWriteLock::try_acquire` | `crates/outl-core/src/lock.rs` |
| Resolve which actor this process writes as | `outl_core::resolve_write_actor` | `crates/outl-core/src/lock.rs` |
| The `Storage` trait every persistent backend implements (invariant #5) | `outl_core::Storage` / `StorageError` | `crates/outl-core/src/storage/mod.rs` |

#### 15. Undo / redo history (outl-actions::history)

Bounded snapshot stacks with vim semantics (a new edit clears redo) shared by GUI clients — the desktop's `Cmd+Z` / `Cmd+Shift+Z` ride these.
Restores route through `outl_md::reconcile_md`, so an undo is **new ops in the log**, never a rewrite (invariant #1 holds).
This is *not* per-keystroke undo inside an uncommitted draft — that belongs to the client's editor widget.

| Intent | Use this | File |
|---|---|---|
| Bounded undo / redo stacks over any snapshot type (`record` / `undo` / `redo` / `can_undo` / `can_redo` / `clear`) | `outl_actions::history::HistoryStacks` | `crates/outl-actions/src/history.rs` |
| Default per-stack bound (matches the TUI's session cap) | `outl_actions::DEFAULT_HISTORY_CAP` | `crates/outl-actions/src/history.rs` |
| Restore a page to a previously-rendered `.md` snapshot (write + reconcile → min ops through `Workspace::apply`) | `outl_actions::restore_page_md` | `crates/outl-actions/src/history.rs` |

If your need is **not** in this catalog and you've grepped honestly, that's a fair sign the primitive doesn't exist yet — add it in the upstream crate that owns the concept (usually `outl-md` for parse / render / sidecar / inline, `outl-actions` for workspace mutations and ingest, `outl-core` for op-log / tree / HLC), then update this catalog in the same commit.
The hook will remind you to sync `copilot-instructions.md`.

### Reuse-first (no parallel implementations)

Before adding a helper, struct, or constant, **scan the Shared primitives catalog above** and **grep the workspace** for what already does the same thing.
Duplication here is a real hazard: two implementations of the same logic drift apart over time, and the user is the one who hits the divergence.

Past incidents:

- `outl_md::index::Backlink` and `outl_actions::Backlink` were two parallel "backlinks" pipelines that started identical and ended up disagreeing on self-references — a bug the user had to spot because each surface looked fine in isolation.
  Collapsed into `outl_actions::backlinks_for_page` in 0.5.3.
- `outl-mobile`'s `run_code_block` Tauri shim was opened as a copy of `outl-desktop/src-tauri/src/commands/exec.rs` — same `flat_index_for` walk, same `journals/<slug>.md || pages/<slug>.md` probe (which already existed as `outl_actions::page_md_path`), same DTO shape.
  The catalog table above did not list code execution as a cross-client primitive, and the desktop's per-crate `CLAUDE.md` filed `commands/exec.rs` under "owned by the desktop", so nothing pointed at the right home.
  Caught by the user mid-PR: "do que fizemos de rodar code block no mobile, não conseguimos compartilhar?"
  Collapsed into `outl_actions::exec::run_code_block` in 0.6.x — clients now own only the AppState lookup and the `view` wrapper. Lesson: if you're staring at a Tauri shim that's mostly `parse_node_id` → outline walk → `outl-exec` call → DTO, **the walk and the DTO belong in `outl-actions`** — every time.
- The Logseq importer's `crates/outl-cli/src/cmd/import/normalize.rs` was opened reimplementing `\r\n` handling, `id::` stripping, and long-form date rewriting — every one of which `outl_actions::paste::normalize_external_syntax` already owned.
  Caught in PR #47 review.
  Lesson: a "normalize markdown from outside" need always starts at `paste::normalize_external_syntax`; outline-level restructuring (headings → bullets, multi-paragraph merge, fence dedent) is the only thing the importer adds on top.

The rule:

1. **Grep before writing.** `rg "fn foo"` / `rg "struct Foo"` across `crates/`.
   Look in **upstream crates first** — `outl-core`, `outl-md`, `outl-actions` are where shared primitives live.
2. **Prefer evolving the existing API** over duplicating, even if that means a small refactor (rename, generalize a parameter, move into a sibling module).
   One owner per concept; many callers.
3. **Duplication is OK only when the platforms are genuinely different.** `outl-tui::EditBuffer` and the mobile `<textarea>` are both "cursor + text" — but one is a terminal widget Rust has to render itself, the other is a browser primitive.
   Same role, different runtime; not duplication.
   **Recalculating** `(line, col)` from `cursor` in both places, though, would be — extract to `outl_md::view::char_to_line_col` and let both wrap it.
4. **Refactor *into* the shared crate, not *around* it.** If a TUI helper feels like it could live in `outl-actions`, move it there *now* (the mobile client will need it soon).
   The `flatten_subtree_paths` migration is the canonical pattern.

When in doubt, name the would-be helper, search for it, then ask yourself: "is the existing thing one rename away?
If yes, rename.

### File size discipline

A Rust `.md` that grows past a few hundred lines is almost always **multiple responsibilities sharing a module**.
The `file-size-guard.sh` PostToolUse hook enforces this:

| Lines | Status |
|-------|--------|
| < 400 | OK |
| 400–600 | Informational note on every edit |
| 600–900 | Hook returns warning (exit 2). Plan an extraction. |
| 900+ | Hook returns stop (exit 2). Refactor before the next non-trivial edit. |

When the hook fires, **invoke the `refactor-architect` agent** to propose a split by responsibility.
The agent's mandate is in `.claude/agents/refactor-architect.md`.

The point isn't a hard limit — it's keeping each module about one thing so the codebase stays easy to read, easy to test, and easy to evolve.

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
- ❌ Adding a helper that re-implements something already in `outl-core` / `outl-md` / `outl-actions` (see **Reuse-first**).
  The fix is to wrap the upstream API, not to write a parallel one.

## When in doubt

1. Read the relevant `docs/*.md`.
2. Read the per-crate `CLAUDE.md`.
3. Read the paper for sync stuff: <https://martin.kleppmann.com/papers/move-op.pdf>
4. Ask the user.
   The user is `Avelino`, comfortable in Rust/Clojure/Python/Go, prefers direct pt-BR communication.
