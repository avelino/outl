# Development guide

The engineer-facing onramp.
This page is about **how you hack on outl** — clone, build, test, debug, ship.

If you're looking for **what reviewers measure your PR against**, that's [Contributing & code review](contributing.md).
The two pages are deliberately split: this one is workflow, that one is policy.
Read this first; read that before opening a PR.

If anything here is wrong or out of date, that's a bug — open an issue or fix it in the same PR that drifted the behavior.

---

## 1. Quick start

```bash
git clone https://github.com/avelino/outl.git
cd outl
cargo build --workspace
cargo test --workspace
```

You need Rust **1.88+**.
`rust-toolchain.toml` pins the exact version, so `rustup` installs it on the first build.
No other system dependency is needed for the core, CLI, TUI, or MCP server.

To smoke-test that the build actually does something, generate a fixture workspace and open it:

```bash
# Creates ./playground with a few pages + journals
just init-playground   # or: invoke /init-playground inside Claude Code
cargo run -p outl-tui -- --workspace ./playground
```

Press `?` inside the TUI for the keymap.
`q` quits.

### Optional toolchains by area

| You're touching... | You also need |
|---|---|
| `outl-mobile` (iOS app) | macOS + Xcode 15+ + Bun (`curl -fsSL https://bun.sh/install \| bash`) |
| `outl-desktop` (Tauri 2) | Bun + the Tauri prerequisites for your OS (Linux: `webkit2gtk-4.1`, `libgtk-3-dev`; Windows: WebView2 runtime) |
| Frontend tests (`crates/outl-mobile/src/**`, `crates/outl-frontend-shared/**`) | Bun + `bun test` |
| Bench job locally | `cargo install hyperfine --locked` for the CLI side; criterion ships with `cargo bench` |

The CI containers don't install GTK, so `outl-mobile` and `outl-desktop` are **excluded from the workspace `cargo clippy/test/doc` runs** (see [CI walkthrough](#9-ci-walkthrough)).
That means a clean `cargo test --workspace` does not exercise those two crates.
If you change them, run their crate-specific commands and let CI's `mobile.yml` / `desktop.yml` matrices cover the rest.

---

## 2. Repository tour

The workspace lives in `crates/`.
Each crate has its own `CLAUDE.md` — read it before editing.

| Crate | What it owns |
|---|---|
| [`outl-core`](../crates/outl-core/CLAUDE.md) | Tree CRDT, op log, HLC, `Storage` trait. **Never** imports UI or CLI. |
| [`outl-md`](../crates/outl-md/CLAUDE.md) | Markdown parse / render, sidecar (`.outl`), 3-level matching, inline tokens, workspace index. |
| [`outl-actions`](../crates/outl-actions/CLAUDE.md) | UI-agnostic workspace operations. Every client calls into here. |
| [`outl-exec`](../crates/outl-exec/CLAUDE.md) | Code-block runtime (desktop + mobile; mobile opts out of `lang-rust`). |
| [`outl-tauri-shared`](../crates/outl-tauri-shared/CLAUDE.md) | Shared Tauri backend — command bodies, wire DTOs, plugin thread, `AppHost` / `StorageRootProvider` traits. Both `outl-desktop` and `outl-mobile` are thin wrappers over this; a new command body always goes here first. |
| [`outl-cli`](../crates/outl-cli/CLAUDE.md) | The `outl` binary (subcommands + JSON envelope). |
| [`outl-tui`](../crates/outl-tui/CLAUDE.md) | The `outl-tui` binary (terminal editor). |
| [`outl-mobile`](../crates/outl-mobile/CLAUDE.md) | Tauri 2 mobile (iOS today) — thin `#[tauri::command]` wrappers, iOS-native bridges (`NSMetadataQuery`, `BGTaskScheduler`), Solid frontend. |
| [`outl-desktop`](../crates/outl-desktop/CLAUDE.md) | Tauri 2 desktop (macOS/Linux/Windows) — thin `#[tauri::command]` wrappers, FS watcher, settings IO, Solid frontend. |
| [`outl-frontend-shared`](../crates/outl-frontend-shared/CLAUDE.md) | `@outl/shared` — Solid + TS lib mobile + desktop both consume. |
| `outl-config`, `outl-theme`, `outl-shortcuts` | Shared config / palette / chord catalog across TUI + desktop. |

### Entry points by intent

When you want to make a change, **don't start from the client** — start from the layer that owns the concept.

| You want to... | Start here |
|---|---|
| Fix or extend the CRDT algorithm | `crates/outl-core/src/tree/mod.rs` (then run the `paper-verifier` agent) |
| Add a new `Op` variant | `/new-op` skill — it lists every place that needs to change |
| Change how `.md` is parsed or rendered | `crates/outl-md/src/{parse,render}.rs` |
| Change how a block survives external edits | `crates/outl-md/src/{matching,diff,reconcile}.rs` |
| Add a shared workspace mutation (TODO toggle, indent, etc.) | `crates/outl-actions/src/{block,collapsed,todo,page}.rs` |
| Add a CLI subcommand | `crates/outl-cli/src/cmd/` (mirror an existing one for the JSON envelope) |
| Add a TUI shortcut, mode, or overlay | `crates/outl-tui/src/` (and update `docs/tui.md` + `docs/shortcuts.md`) |
| Add an MCP tool | `crates/outl-cli/src/mcp/` (mirror an existing tool's shape) |
| Add a theme preset | `crates/outl-theme/src/presets/` |
| Touch the iCloud watcher or sync engine | `crates/outl-actions/src/sync.rs`; mobile-side is `crates/outl-mobile/src-tauri/` |

If you can't tell where something belongs, **grep the Shared primitives catalog in [root `CLAUDE.md`](../CLAUDE.md#shared-primitives-catalog)**.
That table is the canonical map of "who owns this concept".

---

## 3. Running outl locally

### CLI / TUI

```bash
# From a fresh build, no install needed:
cargo run -p outl-cli -- init ~/playground-notes
cargo run -p outl-tui -- --workspace ~/playground-notes

# Or build once, run many:
cargo build --release
./target/release/outl init ~/playground-notes
./target/release/outl --workspace ~/playground-notes        # TUI
./target/release/outl --workspace ~/playground-notes page list --json
```

### MCP server (Claude Desktop, Cursor)

```bash
cargo run -p outl-cli -- mcp --workspace ~/playground-notes
```

For wiring into Claude Desktop / Cursor, see [docs/mcp.md](mcp.md).
Every MCP tool has a `outl_*` name; the source lives in `crates/outl-cli/src/mcp/`.

### Mobile (iOS simulator)

```bash
cd crates/outl-mobile
bun install                     # only once
bun run tauri ios dev           # boots the iOS simulator with hot reload
```

`crates/outl-mobile/CLAUDE.md` covers the versioning + TestFlight contract.
**Do not touch `tauri.conf.json`'s `version` field** — the version is read from `Cargo.toml` at build time on purpose.

#### Why the mobile crate has native Swift / ObjC code

Tauri 2 gives you a WebView + a JS ↔ Rust bridge.
What it does **not** give you is direct access to the iOS platform APIs that outl actually needs to function as a multi-device app:

| Native surface | Why we need it |
|---|---|
| `NSMetadataQuery` + `NSFileCoordinator` + `startDownloadingUbiquitousItemAtURL` (in `main.mm`) | iCloud syncs file metadata aggressively and file content lazily. Without forcing materialisation before a `read`, the Rust side opens an **empty placeholder** for a peer's `ops-<actor>.jsonl`, the merge is wrong, and the projection writes a broken `.md` back. This is *the* iCloud catch — see `crates/outl-mobile/CLAUDE.md` § "Peer-file materialisation". |
| `BGTaskScheduler` (`OutlBackgroundRefresh.swift`) | Drain peer ops while the app is backgrounded so the user doesn't open to a stale tree. |
| `UIInputAccessoryView` (`OutlToolbar.swift`) | The formatting toolbar must be UIKit — a WebView toolbar has input-focus latency and the keyboard re-anchors when the toolbar mounts. |
| Native suggest overlay (`OutlSuggestOverlay.swift`, `OutlSuggestView.swift`) | Autocomplete chips anchored to the caret without the keyboard jumping or the WebView reflowing. |
| Method swizzle + brand chrome (`OutlSwizzle.swift`, `OutlBrandChrome.swift`) | Splash, status-bar, and a few UIKit hooks the WebView doesn't expose. |

The native code is split into **two tiers** with different test contracts:

- **`crates/outl-mobile/swift/OutlKit/`** — pure Swift package (SPM).
  Logic that's testable in isolation: brand color, autocomplete chip parsing, toolbar action enum + MFU ordering, peer-file predicates, JS string escaping.
  **Has unit tests** under `swift/OutlKit/Tests/OutlKitTests/`.
  Run with `swift test` from `swift/OutlKit/` or via `mobile.yml` in CI.
- **`crates/outl-mobile/src-tauri/gen/apple/Sources/outl-mobile/`** — Tauri-generated iOS shell + outl-specific UIKit / Foundation bridges.
  Files: `main.mm`, `OutlOpsWatcher.swift`, `OutlBackgroundRefresh.swift`, `OutlToolbar.swift`, `OutlSuggestOverlay.swift`, `OutlSuggestView.swift`, `OutlSwizzle.swift`, `OutlBrandChrome.swift`.
  **No unit tests** — observed via `NSLog` probes printed on app boot.
  Run on a real device or simulator to exercise.

**Rule of thumb:** if the helper can be tested in a vacuum, it goes in `OutlKit`.
If it has to bind to a UIKit / Foundation API that needs the iOS runtime, it goes in `gen/apple/.../*.swift` and ships with diagnostic logs instead of tests.

### Desktop (Tauri 2)

```bash
cd crates/outl-desktop
bun install                     # only once
bun run tauri dev               # dev window with hot reload
```

A release dmg is built only in CI (`release.yml`'s `build_desktop` job, universal `arm64 + x86_64`).
Local `bun run tauri build` is fine for smoke-testing your own arch.

### Playground workspace

Manual smoke tests share a fixture workspace at `./playground/`.
Generate it with the `/init-playground` slash command (Claude Code) or by hand:

```bash
mkdir -p playground
cargo run -p outl-cli -- init ./playground
# Seed a few pages / journal entries with the `outl page create` / `outl daily append` CLI.
```

`./playground/` is gitignored — feel free to nuke and regenerate.

---

## 4. The dev loop

The expected per-edit cycle:

1. Edit the relevant `.rs` / `.md` / `.ts`.
2. Run `/check` (or `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`).
3. If you touched the CRDT four (`do_op`, `undo_op`, `apply_op`, `creates_cycle`), run `/check-invariants` + `/coverage outl-core`.
4. If you touched `outl-md`, run `/roundtrip`.
5. Commit using Conventional Commits.

### Slash commands

| Command | What it does |
|---|---|
| `/check` | Full gate: `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test --workspace`. Run before reporting done. |
| `/check-invariants` | Faster than `/check`. Runs only the tree CRDT invariant test battery in `outl-core`. |
| `/roundtrip` | `outl-md` `md ↔ ops ↔ md` roundtrip suite. Also invokes the `markdown-roundtrip-tester` agent for extra validation. |
| `/coverage [crate]` | Uses `cargo-llvm-cov`. Flags uncovered branches in the four critical CRDT functions (the 100% rule). |
| `/new-op <Variant>` | Walkthrough for adding a new `Op` variant. Lists every file that needs to move. |
| `/init-playground` | Regenerates `./playground/` with fixture pages + journals. |

### Hooks (run automatically)

`.claude/settings.json` wires these PostToolUse hooks on every `Edit` / `Write`:

- **fmt + clippy** on the touched crate (faster than `/check`, runs per save).
- **`file-size-guard.sh`** — informational at 400–600 lines, warns at 600–900, blocks at 900+.
  When it fires, invoke the `refactor-architect` agent to propose a split.

If you're not using Claude Code, run `cargo fmt -p <crate> && cargo clippy -p <crate> -- -D warnings` manually after edits.

### Agents (specialised reviewers)

| Agent | Fires after edits in... |
|---|---|
| `crdt-invariant-checker` | `outl-core/src/{tree,log,op}.rs` |
| `paper-verifier` | `do_op` / `undo_op` / `apply_op` / `creates_cycle` |
| `markdown-roundtrip-tester` | `outl-md/{parse,render,sidecar,matching}.rs` |
| `refactor-architect` | Any file that crosses the 600-line warn threshold |
| `doc-keeper` | Run at the **end** of every feature that changes public API, markdown syntax, TUI shortcut, slash command, sidecar / op-log format, **CI workflow**, **dev loop**, or user-observable behavior |

The agents are in `.claude/agents/` if you want to inspect or extend them.

### `cargo doc` gotcha

CI runs `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --exclude outl-mobile --exclude outl-desktop --no-deps` and blocks on **`rustdoc::private_intra_doc_links`**.
The workspace is mostly `pub(crate)`, so almost every `[`Foo`]` link in a doc comment to an internal type breaks the build.

Fix: drop the brackets, keep the backticks.

```rust
// Bad — fails CI:
/// See [`MyInternalThing`] for details.

// Good:
/// See `MyInternalThing` for details.
```

`/check` does **not** run `cargo doc` today.
Run it by hand before reporting done on any patch that touches module-level `//!` blocks.

---

## 5. Testing strategy

### Where each test type lives

| Type | Location | What it asserts |
|---|---|---|
| Unit | `crates/*/src/**/*.rs` `#[cfg(test)]` | Single function behavior. Cheap. |
| Integration | `crates/*/tests/*.rs` | Public surface of one crate. Real `MemoryStorage` (or `tempfile` + `JsonlStorage`). |
| CRDT invariants | `crates/outl-core/tests/crdt_*.rs` | Convergence, idempotency, cycle no-op, replay determinism. |
| Roundtrip | `crates/outl-md/tests/roundtrip_*.rs` | `md → parse → render → md` is byte-stable. `md → ops → md` preserves ids via sidecar. |
| Bench | `crates/outl-md/benches/*.rs` (criterion) + `xtask/src/bin/gen-10k.rs` for CLI hyperfine | Hot-path regression detection. Run weekly + per PR via `bench.yml`. |
| Frontend | `crates/outl-mobile/src/**/*.test.ts`, `crates/outl-frontend-shared/**/*.test.ts` (`bun test`) | Pure helpers and DTO conversions. |
| Swift (`OutlKit`) | `crates/outl-mobile/swift/OutlKit/Tests/OutlKitTests/` (`swift test`) | Pure native helpers — brand color, suggester chip parser, toolbar MFU, peer-file predicates, JS escape. **Required** for new pure Swift logic. |
| Native iOS bridges (`gen/apple/.../*.swift`, `main.mm`) | None (yet) — observed via `NSLog` probes on boot | UIKit / Foundation glue that needs the iOS runtime. If you add a piece that *can* be tested without UIKit, extract it into `OutlKit` first. |

### The 100% rule

`do_op`, `undo_op`, `apply_op`, `creates_cycle` in `outl-core/src/tree/mod.rs` carry a **100% line and branch coverage rule**.
Any new branch needs a new test.

```bash
# Coverage report for outl-core specifically (uses cargo-llvm-cov):
/coverage outl-core
```

The `crdt-invariant-checker` agent runs the same gate from CI on PRs.

### Proptest budget: `PROPTEST_CASES`

The property suites (`outl-core/tests/property_based.rs`, `outl-md/tests/roundtrip.rs`, and any future `outl-sync-iroh` convergence proptests) bake a low default case count via `ProptestConfig::with_cases(200)` so local runs stay fast.
`PROPTEST_CASES` is proptest's built-in override of that number; set it to explore harder without touching the test files.

```bash
# Dev default (fast): 200 cases baked into the suites.
cargo test -p outl-core

# Explore the convergence space harder, locally:
PROPTEST_CASES=1024 cargo test -p outl-core -p outl-sync-iroh --all-targets
```

CI's dedicated **`sync`** job (in [`ci.yml`](../.github/workflows/ci.yml)) sets `PROPTEST_CASES=1024` for exactly this pair of crates.
That way the probabilistic convergence bugs (op reordering, cycle no-ops, concurrent moves) actually get generated cases on every PR.
That job is a required status check: a red convergence run blocks merge.
Keep the high budget on the `sync` job only — running 1024 across the whole test matrix burns runner minutes for no extra signal.

### TDD for bug fixes

Bug → reproduce as a test that fails on `main` → patch turns it green.

```bash
# Find an existing similar test:
rg 'fn it_' crates/outl-core/tests/

# Add yours, run only that file:
cargo test --test crdt_convergence -- --nocapture
```

A bug fix without a regression test is a blocker in review.

### What to mock and what not to

- **Real `JsonlStorage`** when the test is about persistence, sync, or anything an `ops-*.jsonl` would touch.
  Use `tempfile::TempDir` for the workspace root.
- **`MemoryStorage`** when the test is about the algorithm and the storage is incidental noise.
- **No mocks for the Tree CRDT.**
  Always replay through the real `do_op` / `undo_op`.
  Mocking those is how you ship a sync bug.
- **HLC**: prefer the real generator with a known actor id (`ActorId::from_u128(1)`).
  Hard-coded timestamps creep into test assertions and break when you change the encoding.

### Frontend tests

```bash
# From repo root:
bun install              # first time
bun test                 # all packages

# Or per-package:
cd crates/outl-mobile && bun test
cd crates/outl-frontend-shared && bun test
```

Most shared helpers (`looksLikeOutline`, `utf16OffsetToCharOffset`, `detectRefContext`) have direct unit tests under `crates/outl-frontend-shared/src/**`.
New helpers go there, not under a client.

---

## 6. Cookbooks

Concrete walkthroughs for the changes contributors hit most often.

### Add a new `Op` variant

1. Run `/new-op <Variant>` for the checklist.
2. Touch order: `Op` enum (`outl-core/src/op.rs`) → `apply_op` + `undo_op` (`tree/mod.rs`) → sidecar projection (if it carries metadata) → markdown rendering (if it's visible) → unit + invariant tests → per-crate docs.
3. Invariants: the inverse must be exact (`apply_op` then `undo_op` is identity); cycle-creating moves remain a no-op on the tree but are still appended to the log; new variant carries an HLC + actor id.
4. Run `/check-invariants` + `/coverage outl-core`.
5. Update `docs/crdt.md` if the op changes how the algorithm is described.

### Add a TUI shortcut

1. Add the chord to `crates/outl-shortcuts/src/`.
2. Wire the handler in `crates/outl-tui/src/`.
3. Add a test that asserts the action ran (against the workspace, not the internal handler).
4. Update `docs/tui.md` (key table) + `docs/shortcuts.md` (canonical chord list).
5. If desktop should mirror it, wire it on the `outl-desktop` side too — the chord catalog is shared.

### Add a shared workspace action

The rule from the root `CLAUDE.md` is: any operation more than one client needs lives in **`outl-actions`** before its first use.

1. Add the function to the right module in `outl-actions/src/{block,collapsed,todo,page,journal}.rs`.
2. Signature: `(&mut Workspace, &HlcGenerator, ...) -> Result<...>`.
3. Routes every mutation through `Workspace::apply` — no direct storage writes.
4. Add an integration test in `crates/outl-actions/tests/`.
5. Wire the TUI / mobile / desktop calls in their respective crates.
6. Add the function to the Shared primitives catalog (root `CLAUDE.md` §5.1 + mirror in `.github/copilot-instructions.md`).

### Add an MCP tool

1. Mirror an existing tool in `crates/outl-cli/src/mcp/` — they all use the same envelope.
2. Tool name: `outl_<verb>_<noun>` (e.g. `outl_block_append`, `outl_page_create`).
3. Wire the underlying logic through `outl-actions` if it mutates state; through `outl-md` indices if it's a read.
4. Update `docs/mcp.md` with the tool's purpose, params, and an example invocation.

### Add a theme

1. Add the palette to `crates/outl-theme/src/presets/`.
2. Register it in the preset enum.
3. Update `docs/theming.md`.
4. The TUI and desktop pick it up automatically — both read the same palette catalog.

### Add a CLI subcommand

1. New module under `crates/outl-cli/src/cmd/`, mirroring an existing one.
2. Both human-readable and `--json` output paths — the JSON envelope is documented in `docs/cli.md`.
3. Hook it into `clap` in the main command dispatcher.
4. Update `docs/cli.md`.

### Touch the iOS native bridge (Swift / ObjC)

Decide **which tier** before opening the file:

1. **Can the logic be tested without UIKit?** (string parsing, predicate, MFU ordering, color math, escape) → goes in `crates/outl-mobile/swift/OutlKit/Sources/OutlKit/<Module>/`.
   Add a unit test alongside in `swift/OutlKit/Tests/OutlKitTests/`.
   Run `swift test` from `swift/OutlKit/`.
2. **Does it need a UIKit / Foundation runtime?** (`UIView`, `BGTaskScheduler`, `NSMetadataQuery`, `NSFileCoordinator`, method swizzle) → goes in `crates/outl-mobile/src-tauri/gen/apple/Sources/outl-mobile/`.
   **Add `NSLog` probes** on entry / exit / error so the behavior is observable on the device console.
   No unit test today.

If you find yourself writing UIKit-shaped code inside `OutlKit`, **stop** — extract the pure part into `OutlKit` and keep the UIKit binding in `gen/apple/`.
The iCloud peer-file watcher is the canonical pattern: predicate logic lives in `OutlKit/Watcher/OpsFilePredicate.swift` (tested), the `NSMetadataQuery` driver lives in `gen/apple/.../OutlOpsWatcher.swift` (boot-time logged).

Always re-read `crates/outl-mobile/CLAUDE.md` § "Peer-file materialisation" before touching `main.mm` or the watcher — that section spells out the iCloud race that two lines of code prevent.

---

## 7. Debugging

### Common failure modes

| Symptom | Likely cause | Fix |
|---|---|---|
| `error[rustdoc::private_intra_doc_links]: public documentation for X links to private item Y` | A `[`Foo`]` link in a doc comment to a `pub(crate)` type | Drop the brackets, keep the backticks |
| `cargo doc` works locally but fails CI | `RUSTDOCFLAGS="-D warnings"` is only set in CI | Run `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` locally |
| `Sidecar { version: V1, ... }` parse error | Old workspace, sidecar pre-v2 | The reader is backward-compatible; if it isn't, that's a regression — open an issue |
| TUI shows blocks the `.md` doesn't | Sidecar / `.md` desync; orphan log will tell you which | `cat .outl/orphans.log` for the unmatched block ids |
| Two devices show different trees after sync | HLC tiebreak missed, or an op was dropped from a log replay | Replay the ops in actor order with a custom binary (see below); also run `/check-invariants` |

### Dump the op log

Each device's log is a plain JSONL file under `ops/`:

```bash
cat ops/ops-<actor-uuid>.jsonl | jq .
cat ops/ops-<actor-uuid>.jsonl | jq -r '.timestamp + " " + .op.type' | head
```

To replay a log into a fresh `MemoryStorage` and inspect the materialized tree, write a tiny binary under `xtask/` — there's already a pattern in `xtask/src/bin/gen-10k.rs`.

### Tracing

The libraries emit `tracing` spans at `debug`.
Run any binary with `RUST_LOG` to enable them:

```bash
RUST_LOG=outl_actions=debug,outl_core=debug cargo run -p outl-cli -- --workspace ./playground page list
RUST_LOG=outl_tui=debug,outl_md=info cargo run -p outl-tui -- --workspace ./playground
```

Spans of interest:

- `outl_actions::sync` — `SyncEngine` work loop, peer detection.
- `outl_core::log` — replay, append, lock acquisition.
- `outl_md::reconcile` — 3-level matching decisions.

### Doctor

```bash
cargo run -p outl-cli -- workspace doctor --workspace ./playground --json
```

Walks the workspace and reports: corrupted sidecars, orphan blocks, mismatched hashes, missing op log entries.
Run this before assuming the bug is in your patch.

---

## 8. Performance

### Hot paths (from [contributing.md](contributing.md#performance--hot-paths-only))

- `outl_core::tree` — every op apply, every tree walk.
- `outl_core::log` — every append, every replay.
- `outl_md::parse` / `render` — every `.md` read/write, every TUI buffer refresh.
- `outl_md::index` — backlink rebuild; scales with workspace size.
- `outl_tui` render loop — runs on every keystroke.
- `outl_actions::SyncEngine` work loop — every file event.

Anything outside those is a correctness conversation, not a perf one.

### Running benches locally

The criterion suite lives under `crates/outl-md/benches/`:

```bash
# Whole suite (small + medium + large fixtures, sub-second each):
cargo bench -p outl-md

# Single bench:
cargo bench -p outl-md --bench parse
cargo bench -p outl-md --bench index -- "medium_"

# 10k-file xlarge (slow — minutes, not seconds):
cargo bench -p outl-md --bench index -- \
  --warm-up-time 2 --measurement-time 10 --sample-size 10 "xlarge_"
```

Criterion writes `target/criterion/<bench>/report/index.html`.
Open it in a browser to compare a baseline against your change.

### End-to-end CLI bench

`xtask/src/bin/gen-10k.rs` builds a 10k-page payload; `hyperfine` measures CLI wall-clock.
The exact recipe lives in `.github/workflows/bench.yml` `bench-cli-xlarge` if you want to reproduce locally.

---

## 9. CI walkthrough

| Workflow | Triggers | What it runs | Blocks merge? |
|---|---|---|---|
| [`ci.yml`](../.github/workflows/ci.yml) | Push / PR to `main` (skipped on docs-only) | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo doc -D warnings`, plus a dedicated **`sync`** job (`outl-core` + `outl-sync-iroh` with `PROPTEST_CASES=1024`). Excludes `outl-mobile` + `outl-desktop`. Test matrix: `ubuntu-latest` + `macos-latest`. | **Yes** |
| [`mobile.yml`](../.github/workflows/mobile.yml) | Push / PR touching mobile paths | Frontend tests, Swift tests, Rust mobile crate, iOS archive + sign on `push` | Mobile changes only |
| [`desktop.yml`](../.github/workflows/desktop.yml) | Push / PR touching desktop paths | Tauri build matrix (macOS/Linux/Windows) | Desktop changes only |
| [`bench.yml`](../.github/workflows/bench.yml) | Push / PR touching `outl-md`, plus weekly cron | Criterion (small/medium/large) on every PR; xlarge + CLI hyperfine on cron / manual dispatch. Artifacts retained 14–30 days. | No (informational) |
| [`release.yml`](../.github/workflows/release.yml) | Push to `main` (beta), `v*` tag (GA), manual | Computes version from `Cargo.toml`, builds CLI + TUI matrix, builds universal desktop dmg, drafts release, uploads assets, publishes, bumps Homebrew tap (`Formula/outl-beta.rb` + `Casks/outl-desktop-beta.rb`). | n/a |
| [`testflight.yml`](../.github/workflows/testflight.yml) | `Mobile` workflow completing successfully | Downloads the signed `.ipa`, uploads to App Store Connect via `xcrun altool`, sets "What to Test" notes via App Store Connect API. | n/a |
| [`cleanup-tags.yml`](../.github/workflows/cleanup-tags.yml) | Cron | Garbage-collects stale beta tags. | n/a |

### What blocks merge

The `ci.yml` jobs are the merge gate.
The PR template and the policy in [`docs/contributing.md`](contributing.md) describe everything else reviewers look at on top of CI.

### Debugging a red CI

1. **Read the failing job's "Annotations"** at the top of the PR check page — it usually points at the file + line.
2. **Reproduce locally** with the exact command the workflow ran (the YAML is the source of truth).
3. **If `cargo doc` fails** with `rustdoc::private_intra_doc_links`, see the [doc gotcha](#cargo-doc-gotcha) above.
4. **If clippy fails on a target you don't have** (e.g. you're on Linux and it's the macOS leg), check that the failure isn't environmental before suspecting your patch.
   `outl-mobile` and `outl-desktop` are explicitly excluded from `ci.yml`'s clippy job; if you somehow re-included them, that's the bug.

### Flakes

Treat a flaky test as a real bug.
The CRDT and parser have no inherent flakiness — if a test fails twice and passes the third time, it's hiding a race or a non-deterministic order in something we control.
Don't add retries; find the cause.

---

## 10. Release process

### Version source of truth

`[workspace.package].version` in the root `Cargo.toml`.
Crate manifests inherit via `version.workspace = true`.
**Bumping the workspace bumps everything.**

`crates/outl-mobile/src-tauri/tauri.conf.json` deliberately omits `version`; CI reads `Cargo.toml` and injects the value into `cargo tauri ios build` via `--config`.
This is non-negotiable — see `crates/outl-mobile/CLAUDE.md` § "Versioning + TestFlight release".

### Beta cadence

Every push to `main` produces a beta release automatically:

- Tag: `v<workspace.version>-beta.<run_number>` (e.g. `v0.6.0-beta.48`).
- Binary: reports the full beta version (the workflow `sed`s `Cargo.toml` in-place before `cargo build`; the change is local to the runner).
- GitHub: published as a prerelease with auto-generated release notes.
- Homebrew tap (`Formula/outl-beta.rb`, `Casks/outl-desktop-beta.rb`) bumped automatically with `[skip ci]` commit.

### GA

Bump `workspace.package.version` (e.g. `0.6.0` → `0.7.0`), merge to `main`, then push a `v0.7.0` tag by hand.
The `tags: ["v*"]` trigger in `release.yml` picks it up.

### TestFlight (iOS)

`mobile.yml` builds + signs the IPA on every push.
`testflight.yml` runs after `mobile.yml` completes, downloads the IPA artifact, uploads to App Store Connect.

**Release notes** ("What to Test") come from `conventional-changelog-cli` (preset `conventionalcommits`) reading the commit log since the last tag.
**Use Conventional Commits.**
A commit without a `feat:` / `fix:` / `chore:` prefix lands under a generic "Other changes" bucket — the user loses context.

### Homebrew

The tap is at the root of this same repo (`Formula/`, `Casks/`).
The `update_tap` job in `release.yml` patches version + sha256 anchors for both the CLI formula and the desktop cask after every beta release.

The desktop dmg is **unsigned** today (Apple Developer account pending).
The cask carries a `caveats` block with the `xattr -dr com.apple.quarantine` workaround.

---

## 11. Where to ask

- **Bugs**: GitHub issues with the [bug report template](../.github/ISSUE_TEMPLATE/bug_report.md).
- **Feature requests**: GitHub issues with the feature template.
- **Security**: [SECURITY.md](../SECURITY.md).
  Do not open a public issue.
- **Design discussion**: open an issue with the `discussion` label, or a draft PR labeled `RFC`.
- **Direct contact**: the project maintainer is [@avelino](https://github.com/avelino).

Welcome aboard.
Read [contributing.md](contributing.md) before opening a PR — it's the policy this dev guide is the workflow for.
