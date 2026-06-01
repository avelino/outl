# CLAUDE.md — outl

Context for Claude Code sessions working on this repo. Read this before making any change.

## What this project is

**outl** is a local-first outliner (Roam/Logseq replacement) with:

- **Markdown as source of truth** — `.md` files are 100% clean, no visible IDs.
- **Conflict-free sync** via a tree CRDT (Kleppmann et al. 2022).
- **Trait-based storage** — JSONL (one file per actor) is the only
  persistent backend; ChronDB on the roadmap.
- **TUI as a first-class citizen**, not an afterthought.
- **Journal-first** — daily notes are the primary entry point.

Full spec lives in the README and `docs/`. Don't skim — read.

## Critical invariants (NEVER violate)

These are the non-negotiables. Violating any one breaks user trust irreversibly.

1. **Op log is source of truth.** All mutations go through `Op` → `apply_op` → log.
   The materialized tree and `.md` files are projections. Never edit `.md` to "fix" state.

2. **Markdown stays 100% clean.** No `id::`, no UUID inline, no HTML comments, nothing.
   IDs live ONLY in the `.outl` sidecar (JSON file next to the `.md`, e.g. `pages/foo.outl`).
   The sidecar is **not** a dotfile — iCloud Documents drops dotted paths during cross-device
   sync, which silently breaks multi-device workspaces. Same rule applies to `ops/`.

3. **CRDT follows Kleppmann 2022 literally.** `do_op` / `undo_op` / `apply_op` /
   `creates_cycle` must match the paper. 100% coverage on these four is non-negotiable.

4. **Move that creates a cycle is a no-op on the materialized tree, but the op
   still goes into the log.** Removing it breaks correctness of future reordering.

5. **Storage is a trait, not a struct.** `JsonlStorage` is the only
   persistent impl; tests use `MemoryStorage`. Anything that wants
   to persist ops goes through `dyn Storage`. No second persistent
   backend lands without an issue + RFC first — divergence between
   storages is exactly what we paid to remove in 0.5.0.

6. **Delete is `Move(node, TRASH_ROOT)`, not physical removal.** Simplifies the
   algorithm and preserves history.

7. **Any state that must converge between devices goes through the op log.**
   If two users (or one user on two devices) can disagree about a value
   and you want them to reconcile, the state belongs in an `Op` — *never*
   in a shared file with last-write-wins semantics. The op log gives each
   actor its own `ops-<actor>.jsonl`, lets iCloud / Syncthing / shared FS
   sync per-file (no merge conflicts), and replays through the CRDT with
   HLC ordering for deterministic convergence. Writing the state into the
   sidecar (or any single shared file) bypasses all of that and loses
   concurrent writes silently. **Default position: model it as an Op.**
   `Op::SetCollapsed` for the fold flag is the canonical example. The
   sidecar carries only **structural matching metadata** (ids, position,
   content hash, ref handle) — it is not a sync surface.

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
    ├── outl-exec/             # code-block runtime (desktop)
    ├── outl-cli/              # `outl` binary
    ├── outl-tui/              # `outl-tui` binary
    └── outl-mobile/           # Tauri 2 mobile app (iOS first)
```

## Shared logic: `outl-actions`

Every workspace mutation a client needs to perform (edit a block,
toggle TODO, indent / outdent, delete, render today's `.md`) lives in
**`outl-actions`**, not in the client crate. The mobile app and the
TUI must call the **same** functions for the same semantics; if a new
operation needs more than one client, it goes in `outl-actions`
before its first use.

The contract is short:

- Functions take `&mut Workspace` and `&HlcGenerator`.
- They route every mutation through `Workspace::apply` (op log
  stays source of truth).
- They never hold UI state and never touch storage backends directly.

See `crates/outl-actions/CLAUDE.md` for the full surface and the
"what this crate does NOT own" list. **If you find yourself writing
tree-walking or op-building helpers inside `outl-tui/`,
`outl-mobile/`, or any future client, stop and put them in
`outl-actions` first.** The TUI's `outline_ops.rs` is the one
deliberate exception (it manipulates an in-flight AST that hasn't
been parsed back to a workspace yet — see that file's module doc).

Per-crate context lives in `crates/<name>/CLAUDE.md`. Read it before editing
that crate.

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

## How we work in this repo

### Build & test

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Or just `/check`. The PostToolUse hook in `.claude/settings.json` runs fmt +
clippy on the touched crate automatically after each `Edit`/`Write`.

**`cargo doc` is part of CI** (`.github/workflows/ci.yml` — `docs` job, with
`RUSTDOCFLAGS=-D warnings`). It breaks the PR on:

- **Intra-doc links to private items.** A doc comment that writes
  ``[`Foo`]`` or ``[`crate::path::Foo`]`` where `Foo` is `pub(crate)` /
  `pub(super)` / `mod` (no `pub`) fails with
  `rustdoc::private_intra_doc_links`. The workspace is mostly `pub(crate)`,
  so **almost every internal type triggers this**. Mitigation: drop the
  square brackets and use backticks only (`` `Foo` ``) — same readability,
  no link, no warning.
- **Broken/missing doc references.** `[`Foo`]` where `Foo` doesn't exist.
- **Code blocks in doc comments that don't compile** (rare for us; we
  rarely put rust code in module docs).

Run `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` before
reporting "done" on any patch that adds or changes module-level doc
comments (`//!` blocks) — `/check` does not include this today.

### Specialized agents

Invoke proactively when relevant:

- **`crdt-invariant-checker`** — after any change in `outl-core/src/{tree,log,op}.rs`.
  Validates convergence, idempotency, cycle handling, coverage.
- **`paper-verifier`** — after writing `do_op`/`undo_op`/`apply_op`/`creates_cycle`.
  Compares Rust against paper pseudocode line by line.
- **`markdown-roundtrip-tester`** — after any change in `outl-md/`.
  Validates roundtrip stability + matching invariants.
- **`refactor-architect`** — after the file-size-guard hook fires
  (stop at 900 lines, warn at 600). Proposes a split by responsibility.
- **`doc-keeper`** — **invoke at the end of every feature** that
  changes public API, markdown syntax, TUI shortcut, slash command,
  sidecar/op-log format, or user-observable behavior. Walks
  `docs/*.md`, root `CLAUDE.md`, and per-crate `CLAUDE.md`; updates
  what drifted, creates only what was missing. **Rule of thumb:** if
  you'd struggle to explain the change to a contributor reading only
  the docs, this agent runs.

### Slash commands

- `/check` — full quality gate (fmt + clippy + test)
- `/check-invariants` — runs CRDT test battery
- `/roundtrip` — runs outl-md matching tests
- `/coverage [crate]` — coverage report, flags uncovered critical branches
- `/new-op <Variant>` — checklist for adding a new `Op` variant
- `/init-playground` — creates a test workspace at `./playground` for manual smoke tests

## Decisions you don't get to revisit

These were settled before code was written. If you think one is wrong, **stop
and ask the user** before changing. Don't unilaterally pivot.

| Decision | Why |
|----------|-----|
| `ULID` for IDs | Lexicographically sortable, 128 bits, no central server needed |
| `uhlc` for time | HLC with actor tiebreak is total order without coordination |
| Yrs for block text | Battle-tested CRDT for strings, lets us focus on the tree |
| `comrak` for markdown | CommonMark-compliant, fast, customizable |
| `iroh` for P2P (phase 2) | QUIC + hole punching, no central server |
| iCloud Drive as v0 transport (mobile + TUI today) | Zero infra, ships now, replaceable behind the same `outl-actions::SyncEngine` when iroh lands |
| Tauri 2 for mobile (replaces earlier uniffi plan) | Single Rust surface across TUI + mobile via `outl-actions`, Solid + Tailwind frontend, ObjC bridge only for iCloud watcher |
| Tauri for desktop (phase 5) | Rust core reuse, smaller than Electron |
| One `ops-<actor>.jsonl` per device, never shared | iCloud (and any file transport) is last-write-wins per file; per-actor files turn that into a non-issue |
| MIT license | Simple, widely understood, no patent grant baggage |
| `outl.app` domain owned | Use for docs/landing later |
| Repo at `github.com/avelino/outl` | Personal profile, not org (small enough team) |

## What you're NOT building yet

Don't add code for these unless explicitly asked:

- P2P sync transport (`iroh`) — iCloud is the v0 transport; iroh replaces it later, behind the same `SyncEngine` interface.
- Query DSL (`{{query: ...}}`)
- Tauri desktop app (Mac/Windows shells beyond the mobile crate)
- Plugin system (`rhai`)
- `ChronDbStorage` backend (issue #1, tracked publicly)
- Android mobile build (only iOS today; Android needs an `NSMetadataQuery` equivalent)
- Per-page op log shards ([`docs/sync.md` Part 2 — Phase A](docs/sync.md#phase-a--per-page-op-log-shards-for-10k-pages); only land it when the single-jsonl-per-device layout hits the 10k-page wall)

## Coding conventions

- `rustfmt` default config, no overrides
- `clippy -- -D warnings` blocks CI
- No `unwrap()` in non-test code. Use `expect("explicit reason")` or propagate.
- `thiserror` in libs (`outl-core`, `outl-md`), `anyhow` at boundaries (`outl-cli`, `outl-tui`)
- No `unsafe` in `outl-core` without documented justification
- Variable names, function names, doc comments: **English** (global audience)
- User-facing strings (CLI help, TUI labels): English for now (i18n later)
- Conventional Commits (`feat:`, `fix:`, `refactor:`, etc) on commit messages

### File size discipline

A Rust `.md` that grows past a few hundred lines is almost always
**multiple responsibilities sharing a module**. The `file-size-guard.sh`
PostToolUse hook enforces this:

| Lines | Status |
|-------|--------|
| < 400 | OK |
| 400–600 | Informational note on every edit |
| 600–900 | Hook returns warning (exit 2). Plan an extraction. |
| 900+ | Hook returns stop (exit 2). Refactor before the next non-trivial edit. |

When the hook fires, **invoke the `refactor-architect` agent** to
propose a split by responsibility. The agent's mandate is in
`.claude/agents/refactor-architect.md`.

The point isn't a hard limit — it's keeping each module about one
thing so the codebase stays easy to read, easy to test, and easy to
evolve.

## Anti-patterns (don't do)

- ❌ Calling `.unwrap()` to get out of error handling
- ❌ Writing IDs into the `.md` file ("just for now")
- ❌ Storing op log fields outside the `Op` variant (breaks undo)
- ❌ Comparing HLCs without actor tiebreak
- ❌ Treating `Delete` as physical removal
- ❌ Skipping tests because "the algorithm is the same as the paper"
- ❌ Reintroducing SQLite / rusqlite / any binary log format —
  cross-device sync depends on per-actor append-only files
- ❌ Using `id::` Logseq-style metadata anywhere
- ❌ Marking work "done" without `/check` passing

## When in doubt

1. Read the relevant `docs/*.md`.
2. Read the per-crate `CLAUDE.md`.
3. Read the paper for sync stuff: <https://martin.kleppmann.com/papers/move-op.pdf>
4. Ask the user. The user is `Avelino`, comfortable in Rust/Clojure/Python/Go, prefers direct pt-BR communication.
