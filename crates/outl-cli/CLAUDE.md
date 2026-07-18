# CLAUDE.md — outl-cli

The `outl` binary.
Thin shell over `outl-core` + `outl-md` + `outl-actions` + `outl-tui`.
**No business logic lives here** — only argument parsing, file orchestration, watcher setup, human-readable output, and the JSON envelope used by every machine-shaped subcommand.

## UX rule: no subcommand → open the TUI

`outl` with no subcommand opens `outl-tui` in the current directory.
This is the primary mode — Roam/Logseq users expect to launch the app and see their notes, not a help screen.

The TUI library is reused via `use outl_tui;` (the crate exposes both a library and a binary).
Don't fork the TUI logic into the CLI.

### Workspace path resolution (`resolve_path` in `main.rs`)

Every subcommand that operates on a workspace runs through one helper.
Precedence — first hit wins:

1. **Subcommand-positional** path (e.g. `outl page get … <PATH>`).
2. **Global `--workspace <DIR>`** flag.
3. **`[workspace] last`** in `~/.config/outl/config.toml`, read via `outl_config::load()`.
   Same file the desktop's Settings modal writes — opening a workspace in the GUI makes `outl` (no args) land on it from the terminal.
4. **Current working directory** — final fallback (`cd ~/notes && outl`).

A path stored in `config.toml` that no longer exists on disk is **skipped silently** (`tracing::warn!` only) so a deleted/unmounted workspace doesn't crash the launch — the chain falls through to cwd.

**Opening a workspace created by a GUI client or P2P sync.**
The desktop, mobile, and the iroh transport seed a workspace with `.outl/workspace-id` + `ops/` + the page/journal dirs, but **never** the per-workspace `.outl/config.toml`.
They keep the device actor in `<app-config-dir>/actor`, not in the workspace.
The CLI/TUI/MCP read the device actor from `config.toml`, so pointing them at a GUI-made workspace used to fail with "no outl workspace — run `outl init`".
`workspace_layout::read_or_init_config` fixes that: when the `.outl/` dir exists but `config.toml` doesn't, it seeds a fresh one (new actor) and proceeds, so `outl --workspace <gui-folder>` just works.
`ws::open` (CLI + MCP) and `outl_tui`'s `open_workspace` both go through this lazy-seed path; a genuinely-missing `.outl/` still errors.

> Full schema + per-OS path of `config.toml` is documented in [`docs/config.md`](../../docs/config.md).
> The `outl-config` crate is the only reader; never re-parse the TOML by hand here.

## Commands

> Full subcommand surface (every flag, JSON envelope shape, MCP mapping) lives in [`docs/cli.md`](../../docs/cli.md).
> The lists below are a navigable index for contributors — one line each, by intent.
> Don't add full flag tables here; they belong in `docs/cli.md` (root `CLAUDE.md` → "One owner per fact").

### Lifecycle / one-shot

- `outl` — open TUI in current directory (also `outl tui [<path>]`).
- `outl init <path>` — scaffold a workspace (pages/, journals/, .outl/).
  Seeds `templates/journal` as a **page** (`template:: journal`), not a `templates/journal.md` file (issue #146).
  A legacy file, if present, migrates into the page body best-effort.
  Opening today's journal then auto-instantiates it.
- `outl serve [<path>] [--once]` — run file watcher; `--once` reconciles every `.md` and exits (smoke tests, scripting).
- `outl doctor [<path>]` — integrity check (sidecars, orphan block refs, **parser warnings** from non-dialect `.md` content).
  Read-only.
  Parser warnings are appended to `.outl/orphans.log` tagged `parse-warning <iso> <path>:<line> <kind> <raw>` so the trail persists across runs.
- `outl reconcile [<path>]` — list orphans pending manual resolution.
- `outl migrate-to-shared [<path>]` — copy local sqlite log into shared `ops/` JSONL for cross-device sync.
- `outl import logseq|roam <src> <dst>` — graph import.
- `outl theme list|show <preset>` — TUI theme inspection.
- `outl plugin init|list|install|run|enable|disable|remove` — manage the workspace's JS plugins (under `<workspace>/.outl/plugins/`), wrapping `outl-plugins`.
  `init <NAME> [--id <ID>] [--dir <PATH>]` scaffolds a buildable plugin project (manifest + `package.json` + `tsconfig` + `src/index.ts` + README); it touches no workspace.
  Templates live in `cmd/plugin_init.rs`.
  `list` loads every installed plugin and prints version + enabled state + contributed slash commands.
  `install <SOURCE>` takes a local directory **or** a `github:owner/repo[/subdir][#tag]` source and shows the requested permissions.
  GitHub sources are cloned at an immutable semver tag (newest when not pinned, never a mutable branch) — the clone + tag resolution live in `cmd/plugin_source.rs` (shells out to `git`).
  It asks for approval (`--yes` to skip, required when stdin isn't a TTY) before copying the plugin in and freezing the approved permissions in the lockfile.
  `run <ID> <CMD>` runs a contributed command and re-renders every `.md` (op log is source of truth; files are a projection).
  `enable|disable <ID>` flip the `enabled` flag in `installed.json`.
  `remove <ID>` (aliases `uninstall`, `rm`) deletes the plugin's directory and its lockfile entry (the id is validated against path traversal before any deletion).
  Unlike the machine-shaped commands, `plugin` uses `anyhow` at the boundary (operator-facing, interactive), like `peer`.
- `outl peer pair|list|remove|status` — manage paired devices for P2P sync.
  Reads the per-**device** `~/.outl/identity.key` + the per-**workspace** `<workspace>/.outl/peers.json` via `outl-sync-iroh` (`IrohIdentity`, `PeersStore`).
  All four resolve the workspace (`--workspace` / `resolve_path`) so the pair belongs to the graph, not the OS; a one-time migration copies any legacy global `~/.outl/peers.json` into the workspace on first touch.
  `pair` runs the real iroh handshake.
  The host prints a ticket + ASCII QR and waits for one inbound connection.
  `--ticket <str>` connects, exchanges `PeerEntry`s, and writes the peer to `peers.json`.
  `--name <str>` is the alias THIS device advertises (it lands under our node id in the peer's `peers.json`).
  It defaults to the machine hostname via `default_device_name` (best-effort `hostname` shell-out, `.local` trimmed) so the peer list reads a real name instead of a node-id stub.
  A small `tokio` runtime drives the async `host_pairing` / `join_pairing` helpers from this sync binary.
  `status` is still a static listing; live reachability lands with the running transport.

### Machine-shaped (JSON envelope, `--json` everywhere)

These are the surface called by scripts, agents, and the MCP shim.
Each handler returns a `serde_json::Value` so the same code path serves both the CLI and `outl mcp serve`.

- `outl page get|create|update|delete|list|rename|render` (`create` takes `--content=<JSON|->` to seed the outline in one call)
- `outl block get|append|append-tree|insert|update|move|delete|toggle-todo|tree` (`append-tree` takes `--tree=<JSON|->`)
- `outl daily today|get|append|range`
- `outl search "<query>" [--in=blocks|pages|all] [--limit=N]`
- `outl query [--tag=…] [--priority=…] [--since=…d] [--kind=…] [--prop key=value …]`
- `outl backlinks page|block|embed`
- `outl tag list|pages`
- `outl prop set|get|list`
- `outl template list|apply|resolve|run` — template pages.
  `list` finds every page with a non-empty `template::` property.
  `apply` instantiates a structural template under a target block.
  `resolve` returns a callable template's code block + declared params.
  `run` executes a callable template: inject params, run through the shared `run_callable_block` path, write the `> **result:**` subtree under `--block`.
  `apply`/`run` reject a `--block` that belongs to a page other than `--page` (`INVALID_ARG`).
- `outl export hugo|md|json`
- `outl batch [--ops=<JSON|->]` — runs a list of write ops in one workspace session (stop-on-first-error, returns `failed_at` / `applied` on the partial outcome)
- `outl workspace info`

The full mapping (CLI ↔ MCP tool) is documented in [`docs/cli.md`](../../docs/cli.md).

### MCP

- `outl mcp serve [--workspace=…]` — JSON-RPC 2.0 over stdio implementing the MCP protocol surface Claude Desktop expects (`initialize`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`, `prompts/get`).
  Every tool is a thin router that delegates to the same handler the CLI subcommand calls — there is no second business-logic path.

## P2P sync: the MCP and the ephemeral CLI are BOTH passive writers

iroh's relay routes only ONE endpoint per `node_id` at a time.
The old design here claimed a second endpoint sharing the device identity was a *benign, stable* hijack (the loser keeps working via outbound dial).
**That is false for relay-dependent peers.**
Reading the iroh-relay source: the demoted endpoint goes *inactive* — it can still send, but **receives nothing**.
And since QUIC return traffic for a relay-only peer (an off-LAN iPhone) is addressed to the node_id → routed to whoever is ACTIVE on that relay, the demoted endpoint's **outbound catch-up also stalls**.
So a second endpoint breaks the first's sync **in both directions** for any peer not reachable on the LAN.
This is exactly what broke sync when `outl mcp serve` and the desktop GUI ran together.
The GUI held the route at boot; the first dual-write tool call spun up the MCP's endpoint and stole it; the GUI silently lost the iPhone (the "sync funciona 1-2 min, depois cai" symptom).
See [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "One endpoint per identity".

So **neither** the MCP nor the ephemeral CLI binds an iroh endpoint — the GUI is the sole owner of the device identity's relay route:

- **The MCP server is a passive writer with a file poller.**
  `outl mcp serve` is long-lived, but on first workspace open it brings up **`outl_actions::FileSyncTransport`**, NOT `IrohSyncTransport` — a disk poller that binds no endpoint (`mcp/mod.rs::ensure_transport`).
  It writes `ops-<actor>.jsonl` to the shared `ops/` dir; a co-resident GUI's fs-watcher picks those up and **its** transport announces/serves them to remote peers.
  The file poller only flips the `peer_dirty` flag when another process (GUI / CLI / a GUI-delivered peer op) changes the on-disk ops, so the next tool call reopens and the MCP's reads stay fresh.
  There is no peer announce after a mutation (the file transport has nothing to announce); `shutdown_transport` is a clean no-op on it.
  A **headless** MCP (no GUI running) therefore has no real-time push — its ops sit on disk until a long-lived endpoint (the GUI) opens or the user runs `outl sync`, then converge via each peer's `MAINTENANCE_RESYNC`.
  That's the accepted trade-off for never breaking a running GUI's sync.
- **The ephemeral CLI stays a passive writer.**
  A `page`/`block`/`daily`/`batch`/`import` command runs in ~200ms — far too short to establish a QUIC connection (which takes seconds), so binding a transport just to drop it would steal the relay route from a running GUI/MCP for nothing.
  These commands write `ops-<actor>.jsonl` and rely on a co-resident long-lived peer (GUI / MCP) plus every device's catch-up re-sync (`MAINTENANCE_RESYNC`) to converge.
  `outl sync` is the explicit escape hatch: it brings a transport up, forces a push/pull pass against every peer, waits, and exits — for scripts that must flush before the process dies.
- **`outl peer pair`/`status`** use a transient endpoint they close before returning (CLI-only, no long-lived client should be mid-pair at the same time).

## JSON envelope (CLI + MCP)

```json
{ "ok": true,  "data": { … }, "error": null }
{ "ok": false, "data": null,  "error": { "code": "X", "message": "…" } }
```

Stable error codes live in `output::codes` (`NO_WORKSPACE`, `PAGE_NOT_FOUND`, `BLOCK_NOT_FOUND`, `INVALID_BLOCK_ID`, `INVALID_DATE`, `CONFIRM_REQUIRED`, `CYCLE_REJECTED`, `SLUG_CONFLICT`, `PROP_NOT_FOUND`, `INTERNAL`, `INVALID_ARG`).
Add new codes by appending — never renumber existing ones (LLMs cache them).

Exit codes follow:

- `0` success
- `1` user error (`ApiError` with non-`INTERNAL` code)
- `2` internal error (`ApiError::INTERNAL`)

## Layout

```
src/
├── main.rs                # clap entry, dispatches to commands
├── output.rs              # JSON envelope, ApiError, exit codes
├── ws.rs                  # WsCtx — open Workspace + HlcGenerator + lock
├── workspace_layout.rs    # filesystem layout (.outl, pages/, journals/)
├── sync_engine.rs         # shared reconcile path (serve/doctor reuse)
├── cmd/
│   ├── mod.rs
│   ├── init.rs            # outl init
│   ├── serve.rs           # outl serve
│   ├── doctor.rs          # outl doctor
│   ├── reconcile.rs       # outl reconcile
│   ├── theme.rs           # outl theme
│   ├── import.rs          # outl import (dispatcher; ImportReport re-export)
│   ├── import/            # common.rs (shared helpers) + logseq.rs + roam.rs + obsidian.rs (+ obsidian/{stems,tests}.rs) — see import/CLAUDE.md
│   ├── migrate_to_shared.rs
│   ├── export.rs          # legacy `outl export --to fmt` placeholder
│   ├── export_v2.rs       # outl export {hugo,md,json}
│   ├── page.rs            # outl page …
│   ├── plugin.rs          # outl plugin …
│   ├── block.rs           # outl block …
│   ├── daily.rs           # outl daily …
│   ├── search.rs          # outl search
│   ├── query.rs           # outl query
│   ├── backlinks.rs       # outl backlinks …
│   ├── tag.rs             # outl tag …
│   ├── prop.rs            # outl prop …
│   ├── template.rs        # outl template …
│   ├── batch.rs           # outl batch
│   └── workspace_info.rs  # outl workspace info
└── mcp/
    ├── mod.rs             # stdio loop, dispatch
    ├── protocol.rs        # JSON-RPC 2.0 shapes + error codes
    ├── tools.rs           # tool registry + handler dispatch
    ├── resources.rs       # outl:// URI handlers + templates
    └── prompts.rs         # /outl-* prompts
```

Every `commands/*.rs` handler is `pub fn` so `mcp/tools.rs` reuses it directly.
New tools land by:

1. Adding a function in the relevant `cmd/*.rs` returning `Result<Value, ApiError>`.
2. Threading it through the local `Subcommand` and `run()` switch.
3. Registering the tool in `mcp/tools::list` (schema) and `mcp/tools::run_tool` (dispatch).

## Conventions

- `clap` derive for parsing.
- Every `--json` flag forces JSON envelope output; otherwise the human formatter inside each `cmd/*.rs` runs.
- Machine-shaped handlers always return `Result<Value, ApiError>`.
- Mutating commands take the workspace lock through `ws::open`.
  Two `outl` processes can't race against `outl serve` or each other.
- `anyhow::Result` on lifecycle commands (`init`, `serve`, `doctor`) is kept — those produce human errors and never JSON.

## What this crate does NOT do

- ❌ Implement the CRDT (use `outl-core`)
- ❌ Parse markdown (use `outl-md`)
- ❌ Hold workspace mutation logic (use `outl-actions`)
- ❌ Render TUI directly (use `outl-tui` as a library or sub-binary)
- ❌ Network anything (P2P sync lives in `outl-sync`)
- ❌ Duplicate logic between CLI and MCP shim (always route through the same `cmd/*::pub fn`)
- ❌ Add a helper here that re-implements something already in `outl-core` / `outl-md` / `outl-actions`.
  `cmd/*` handlers are glue — they parse args, call the upstream API, and JSON-envelope the result.
  If you need a new operation, add it upstream first (`outl-actions` is the usual home), then call it.
  See root [`CLAUDE.md`](../../CLAUDE.md#reuse-first-no-parallel-implementations) for the policy.
