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

> Full schema + per-OS path of `config.toml` is documented in [`docs/config.md`](../../docs/config.md).
> The `outl-config` crate is the only reader; never re-parse the TOML by hand here.

## Commands

> Full subcommand surface (every flag, JSON envelope shape, MCP mapping) lives in [`docs/cli.md`](../../docs/cli.md).
> The lists below are a navigable index for contributors — one line each, by intent.
> Don't add full flag tables here; they belong in `docs/cli.md` (root `CLAUDE.md` → "One owner per fact").

### Lifecycle / one-shot

- `outl` — open TUI in current directory (also `outl tui [<path>]`).
- `outl init <path>` — scaffold a workspace (pages/, journals/, templates/, .outl/).
- `outl serve [<path>] [--once]` — run file watcher; `--once` reconciles every `.md` and exits (smoke tests, scripting).
- `outl doctor [<path>]` — integrity check (sidecars, orphan block refs, **parser warnings** from non-dialect `.md` content).
  Read-only.
  Parser warnings are appended to `.outl/orphans.log` tagged `parse-warning <iso> <path>:<line> <kind> <raw>` so the trail persists across runs.
- `outl reconcile [<path>]` — list orphans pending manual resolution.
- `outl migrate-to-shared [<path>]` — copy local sqlite log into shared `ops/` JSONL for cross-device sync.
- `outl import logseq|roam <src> <dst>` — graph import.
- `outl theme list|show <preset>` — TUI theme inspection.
- `outl peer pair|list|remove|status` — manage paired devices for P2P sync.
  Reads/writes `~/.outl/identity.key` + `~/.outl/peers.json` via `outl-sync-iroh` (`IrohIdentity`, `PeersStore`).
  `pair` runs the real iroh handshake.
  The host prints a ticket + ASCII QR and waits for one inbound connection.
  `--ticket <str>` connects, exchanges `PeerEntry`s, and writes the peer to `peers.json`.
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
- `outl export hugo|md|json`
- `outl batch [--ops=<JSON|->]` — runs a list of write ops in one workspace session (stop-on-first-error, returns `failed_at` / `applied` on the partial outcome)
- `outl workspace info`

The full mapping (CLI ↔ MCP tool) is documented in [`docs/cli.md`](../../docs/cli.md).

### MCP

- `outl mcp serve [--workspace=…]` — JSON-RPC 2.0 over stdio implementing the MCP protocol surface Claude Desktop expects (`initialize`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`, `prompts/get`).
  Every tool is a thin router that delegates to the same handler the CLI subcommand calls — there is no second business-logic path.

## P2P sync: MCP is a first-class peer, the ephemeral CLI is a passive writer

iroh's relay only lets ONE endpoint per `node_id` hold the inbound route at a time.
But two endpoints that **both serve the sync ALPN** coexist fine: the relay-hijack is *benign and stable* (the loser keeps working via outbound dial; no flapping).
See [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "One endpoint per identity".
That fact splits the two surfaces:

- **The MCP server brings the transport UP.**
  `outl mcp serve` is **long-lived** (it lives for the whole Claude Desktop session).
  So on the first workspace open it spins up `IrohSyncTransport` (shared `~/.outl/identity.key`, `~/.outl/peers.json`) **when the device has paired peers**, and tears it down when stdin closes.
  Every mutating tool calls `announce_local_ops` after committing, so an edit made through Claude reaches the other devices in real time **without any GUI open**.
  Inbound peer pushes flip a dirty flag so the next tool call reopens the workspace and serves the freshly-arrived ops.
  Wired in `mcp/mod.rs` (`ServerCtx::ensure_transport` / `announce_after_mutation` / `shutdown_transport`).
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
│   ├── import.rs          # outl import
│   ├── migrate_to_shared.rs
│   ├── export.rs          # legacy `outl export --to fmt` placeholder
│   ├── export_v2.rs       # outl export {hugo,md,json}
│   ├── page.rs            # outl page …
│   ├── block.rs           # outl block …
│   ├── daily.rs           # outl daily …
│   ├── search.rs          # outl search
│   ├── query.rs           # outl query
│   ├── backlinks.rs       # outl backlinks …
│   ├── tag.rs             # outl tag …
│   ├── prop.rs            # outl prop …
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
- ❌ Network anything (phase 2)
- ❌ Duplicate logic between CLI and MCP shim (always route through the same `cmd/*::pub fn`)
- ❌ Add a helper here that re-implements something already in `outl-core` / `outl-md` / `outl-actions`.
  `cmd/*` handlers are glue — they parse args, call the upstream API, and JSON-envelope the result.
  If you need a new operation, add it upstream first (`outl-actions` is the usual home), then call it.
  See root [`CLAUDE.md`](../../CLAUDE.md#reuse-first-no-parallel-implementations) for the policy.
