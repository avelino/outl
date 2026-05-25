# Changelog

All notable changes to outl are documented here. Format inspired by
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project
uses [Semantic Versioning](https://semver.org/).

## [0.1.0] — 2026-05-25

First public release. Single-device editor; sync transport is on the
roadmap but the algorithm and op-log infrastructure are already in.

### Core (`outl-core`)

- Tree CRDT implementation following Kleppmann et al. 2022
  (`do_op` / `undo_op` / `apply_op` / `creates_cycle`).
- HLC timestamps with actor tiebreak.
- Append-only op log with sqlite backend (`SqliteStorage`).
- `Storage` trait so alternative backends (e.g. ChronDB) can slot in
  without touching the CRDT.
- Workspace file lock via `fs2::flock` — two `outl` processes on the
  same workspace get a clean error, not a race.
- Property-based test of strong eventual consistency over 100+
  randomised op permutations.

### Markdown / sidecar (`outl-md`)

- CommonMark parse + render with the outl dialect (`title::`,
  `icon::`, page/block properties, `[[refs]]`, `#tags`,
  `((block-id))`, fenced code blocks, multi-line block text).
- `.foo.outl` JSON sidecar holding the IDs the `.md` deliberately
  doesn't carry. **The `.md` stays clean** — no `id::`, no UUIDs.
- 3-level matching algorithm (`outl-md::matching`) reconstructs which
  block kept which ID after an external editor saves the file.
- Workspace index (`WorkspaceIndex`) — title, icon, slug, backlinks,
  tag namespace; powers the switcher, autocomplete and backlinks
  panel. Built once on boot, refreshed in a worker thread on save.
- Roundtrip property test: `parse(render(ast)) == ast` over randomly
  generated outlines including multi-line and fenced cases.

### Code-block execution (`outl-exec`)

- `Runtime` trait + `RuntimeRegistry`. Shipped runtimes (each behind
  a Cargo feature for opt-out distributions):
  - `lisp` — Steel (Scheme R5RS-ish in pure Rust).
  - `js` — Boa (ES2015+ in pure Rust).
  - `python` — RustPython (Python 3 subset).
  - `lua` — mlua 5.4 (vendored).
  - `rust` — `rustc → wasm32-wasip1 → wasmtime`. Compiled artefacts
    cached in `~/.cache/outl/runtimes/rust/<hash>.wasm`. ~20× faster
    on a re-run of the same snippet.
- WASM sandbox infrastructure (wasmtime engine + WASI ctx with no
  preopens / no env / no sockets, fuel-based instruction cap,
  epoch-interruption timeout, in-memory stdin/stdout/stderr).
- Idempotent result subblock — re-running the same code overwrites
  the existing `> **result:**` child instead of duplicating it.
- `source-hash::` stamped on each result child so the upcoming auto-run
  loop can short-circuit unchanged sources.

### TUI (`outl-tui`)

- Journal-first: opens on today's date.
- Vim-style modes (Normal / Insert / Visual) with chord support
  (`dd`, `gg`, `gx`, `yy`, `qq`-to-quit).
- Insert mode autocomplete for `[[refs]]`, `#tags`, and `/commands`
  (Notion-style slash menu).
- Slash command system + vim palette share one registry — every
  built-in command shows up in both. Built-ins: `prop-block`,
  `prop-page`, `search`, `run`, `theme`, `today`, `open`,
  `refresh`, `write`, `quit`, `help`. The registry is the
  plugin-extension point.
- `gx` runs the code block under the cursor through `outl-exec`.
- `auto-run::` property runs a block automatically on page open
  (cache-aware via SHA-256 of the source).
- `icon::` page property surfaces in every place the title shows
  (header, switcher, backlinks panel, search results, autocomplete,
  inline `[[refs]]`).
- Multi-line blocks via `Alt+Enter` / `Ctrl+J` / `Shift+Enter`
  (Shift+Enter only on terminals that speak the kitty keyboard
  protocol); plain `Enter` auto-detects an open code fence and
  inserts a soft newline inside it.
- Vertical scroll with `PgUp`/`PgDn`/`Ctrl+D`/`Ctrl+U`/`gg`/`G` and
  auto-scroll when the selection moves off-screen.
- Hot reload on external `.md` edits (polls mtime every 750ms; warns
  instead of clobbering when you're mid-Insert).
- Error modal overlay for multi-line failures (rustc compile errors,
  traps, missing toolchain), keeping the status line for short
  successes.
- Themes: 11 presets, switchable with `/theme <name>` at runtime.

### CLI (`outl-cli`)

- `outl` (no subcommand) opens the TUI in `$PWD`.
- `outl init <path>` scaffolds a workspace.
- `outl serve [--once]` reconciles `.md` files into the op log
  (one-shot or watch mode).
- `outl import logseq <src> <dst>` and `outl import roam <backup.json>
  <dst>` strip `id::` lines, slugify, seed sidecars.
- `outl doctor` and `outl reconcile` placeholders for the integrity
  and orphan-resolution flows.

### Tooling / DX

- Workspace MSRV: rustc 1.88.
- CI: `fmt` + `clippy -D warnings` + `cargo test --workspace --all-targets`
  on Linux and macOS.
- Bench CI: `small` / `medium` / `large` on every PR + push;
  `xlarge` (10k+ files) on weekly cron + manual dispatch.
- File-size guard hook (`.claude/hooks/file-size-guard.sh`) blocks
  Rust files past ~900 LOC unless the change is intentional —
  forces a refactor conversation before drift accumulates.
- Background workspace-index build: `App::new` paints the journal
  immediately and spawns a worker thread for the global index;
  backlinks/icons fill in within ~ms of boot.

### License

MIT.

[0.1.0]: https://github.com/avelino/outl/releases/tag/v0.1.0
