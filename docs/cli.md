# CLI

outl ships one binary — `outl` — that does everything a workspace needs from outside the TUI: scripts, cron jobs, CI, editor integrations, and LLM agents.
This document is the surface contract.

Looking for how to plug outl into Claude Desktop / Cursor / other MCP hosts? → [docs/mcp.md](mcp.md).
The two share the same handlers under the hood, so this page stays the source of truth for what each command does.

## The bet — one binary, one surface

Knowledge bases get integrated everywhere: editors, shell scripts, LLM agents, automation.
The wrong move is to grow a separate API per host (REST for one, MCP for another, library for a third).
Each new client doubles the surface and drifts.

outl's bet: **everything reachable from outside the TUI is reachable through the `outl` binary**, with a stable JSON envelope.
Other protocols (MCP today, anything that comes next) are thin shims that shell out to the same commands.
There is one place where logic lives, and that place is `outl-actions`.

## The stack

```text
┌────────────────────────────────────────────────────────────┐
│ Hosts                                                       │
│   shell · cron · CI · editors · Claude Code · Claude Desktop│
└────────────────────────────────────────────────────────────┘
                  │                            │
                  │ subprocess                 │ MCP / stdio
                  ▼                            ▼
┌──────────────────────────────┐  ┌──────────────────────────┐
│ outl <subcommand>            │  │ outl mcp serve           │
│  page · block · daily ·      │  │ (thin shim, declares     │
│  search · query · export …   │  │  tools, calls into the   │
│                              │  │  same handlers below)    │
└──────────────────────────────┘  └──────────────────────────┘
                  │                            │
                  └──────────────┬─────────────┘
                                 ▼
┌────────────────────────────────────────────────────────────┐
│ outl-actions                                                │
│   block · tree · todo · journal · page · backlinks · sync   │
└────────────────────────────────────────────────────────────┘
```

The MCP server is a subcommand of the same binary.
There is no `outl-mcp` crate, no separate distribution, no parallel logic.
A new feature lands once: as a function in `outl-actions`, exposed by one subcommand and one tool, sharing the same handler.

## JSON envelope

Every command that produces machine output emits the same shape so downstream consumers (jq, LLMs, scripts) cache one parser.

Success:

```json
{
  "ok": true,
  "data": { "...": "command-specific payload" },
  "error": null
}
```

Failure:

```json
{
  "ok": false,
  "data": null,
  "error": {
    "code": "PAGE_NOT_FOUND",
    "message": "page 'foo' does not exist"
  }
}
```

Error codes are stable strings, listed alongside each command below when relevant.
Exit codes follow:

| Code | Meaning                                       |
|------|-----------------------------------------------|
| 0    | Success                                       |
| 1    | User error (bad input, not found, conflict)   |
| 2    | Internal error (bug, broken invariant, panic) |

Add `--json` to any command to force JSON.
Without the flag, output is human-readable (tables, colored).
MCP tools wrap the same envelope in the MCP tool-result shape: `structuredContent` carries the full `{ ok, data, error }` envelope and `content[].text` carries either a pretty-printed payload or, for markdown-first tools (`outl_export_md`, `outl_page_render`, `outl_daily_today`, `outl_daily_get`), the raw `.md` string.
Clients should read `structuredContent.data` for typed access.

## Commands by domain

The CLI column is what you type at the terminal.
The MCP tool column is the name Claude Desktop (or any MCP host) sees.

### Page

| CLI                                                            | MCP tool             |
|----------------------------------------------------------------|----------------------|
| `outl page get <slug> [--json]`                                | `outl_page_get`      |
| `outl page create <slug> --title=… [--icon=…] [--content=<JSON\|->] [--slugify]` | `outl_page_create`   |
| `outl page update <slug> [--title=…] [--icon=…]`               | `outl_page_update`   |
| `outl page delete <slug> [--confirm]`                          | `outl_page_delete`   |
| `outl page list [--filter=tag:foo] [--json]`                   | `outl_page_list`     |
| `outl page rename <old-slug> <new-slug>`                       | `outl_page_rename`   |
| `outl page render <slug>`                                      | `outl_page_render`   |

`page get` returns page meta plus the outline tree.
`page render` returns the projected `.md` string (clean, no sidecar fields).
`page rename` updates the `page-slug` property and renames the on-disk `.md`/`.outl` — it does **not** rewrite `[[old_slug]]` references in other pages.
Affected blocks come back in `affected_refs` so the caller can decide whether to bulk-rewrite.

`page create --content` accepts a forest of `[{text, children?}, ...]` (or a single `{text, children?}` for ergonomics) so a brand-new page lands with its full outline in one op-log session instead of a chain of `block append` calls.
Pass `--content -` to read the JSON from stdin.
The returned `content` array mirrors the input and carries the freshly minted block ids, so the caller can keep referencing them in follow-ups.

`page create --slugify` treats the positional argument as a human name and derives the slug from it through the shared `outl_md::slugify` rule (lowercase, fold Latin accents, non-alphanumeric → `-`, collapse + trim).
It is opt-in and idempotent on an already-clean slug, so the default path — and the `outl_page_create` MCP tool — stay literal, keeping hierarchical slugs like `ai-agent/learning` verbatim.
The flag exists so external clients (the Raycast extension's "New Page") can ask the user for a name only and let the one owner of the rule generate the slug, instead of re-implementing slugify.

### Block

| CLI                                                            | MCP tool                |
|----------------------------------------------------------------|-------------------------|
| `outl block get <blk-XXX> [--json]`                            | `outl_block_get`        |
| `outl block append <page> --text=… [--parent=blk-YYY]`         | `outl_block_append`     |
| `outl block append-tree --page=… --tree=<JSON\|->`              | `outl_block_append_tree`|
| `outl block insert --after=<blk-XXX> --text=…`                 | `outl_block_insert`     |
| `outl block update <blk> --text=…`                             | `outl_block_update`     |
| `outl block move <blk> --parent=<blk-YYY> [--after=<blk-ZZZ>]` | `outl_block_move`       |
| `outl block delete <blk> [--confirm]`                          | `outl_block_delete`     |
| `outl block toggle-todo <blk>`                                 | `outl_block_toggle_todo`|
| `outl block tree <blk> [--json]`                               | `outl_block_tree`       |

`block move` is the one user-visible name for `Op::Move`.
Cycle detection still applies: a move that would create a cycle returns `{ "code": "CYCLE_REJECTED" }` and the op still goes into the log (see [docs/crdt.md](crdt.md)).
`block toggle-todo` walks `None → TODO → DONE → None`, same as `outl_actions::cycle_todo`.

`block append-tree` writes a root block plus its recursive children in one op-log session.
`--tree` accepts the JSON shape `{"text": "...", "children": [{"text": "...", "children": [...]}]}`, or `--tree -` to read the JSON from stdin.
The response mirrors the input shape with `id` at every node so the caller can map back to anything they wrote.
Prefer this over chained `outl block append` calls when authoring structured content from a script or agent.

### Daily / Journal

| CLI                                                | MCP tool             |
|----------------------------------------------------|----------------------|
| `outl daily today [--json]`                        | `outl_daily_today`   |
| `outl daily get <date> [--json]`                   | `outl_daily_get`     |
| `outl daily append --text=… [--date=…]`            | `outl_daily_append`  |
| `outl daily range --from=… --to=… [--json]`        | `outl_daily_range`   |

`<date>` accepts ISO (`2026-05-31`) and natural (`"April 22nd, 2026"`, `"yesterday"`, `"tomorrow"`).
Range is inclusive on both sides and emits one entry per day in the interval — days that have no materialised journal come back as `{ exists: false }` placeholders so the caller can spot gaps.

### Search / Query

| CLI                                                              | MCP tool          |
|------------------------------------------------------------------|-------------------|
| `outl search "<query>" [--in=blocks\|pages] [--json]`            | `outl_search`     |
| `outl query --tag=foo [--priority=p1] [--since=7d] [--json]`     | `outl_query`      |

`search` is full-text and lives today as the TUI's workspace search.
`query` is the structured filter (tag, property, date range, kind).
The `--raw='…'` flag is reserved for the phase 3 DSL and currently rejects with `INVALID_ARG` — when the DSL lands it folds into the same `outl_query` tool, not a new one.

### Backlinks / Refs

| CLI                                | MCP tool              |
|------------------------------------|-----------------------|
| `outl backlinks page <slug> [--json]`             | `outl_backlinks`   |
| `outl backlinks block <blk-XXX> [--json]`         | `outl_block_refs`  |
| `outl backlinks embed <blk-XXX\|handle> [--json]` | `outl_block_embed` |

`block embed` resolves `!((blk-XXX))` recursively, returning the source block plus children — the same expansion the TUI does inline.

### Tags / Properties

| CLI                                              | MCP tool             |
|--------------------------------------------------|----------------------|
| `outl tag list [--json]`                         | `outl_tag_list`      |
| `outl tag pages <tag> [--json]`                  | `outl_tag_pages`     |
| `outl page prop set <page> <key>=<value>`        | `outl_page_prop_set` |
| `outl page prop get <page> <key>`                | `outl_page_prop_get` |
| `outl page prop list <page> [--json]`            | `outl_page_prop_list`|

Properties stay in the `key:: value` lines at the top of the page; the CLI never invents a new place to put metadata (see [docs/markdown-format.md](markdown-format.md)).

### Export

| CLI                                                  | MCP tool          |
|------------------------------------------------------|-------------------|
| `outl export hugo <page> --out=./content/posts/`     | `outl_export_hugo`|
| `outl export md <page>`                              | `outl_export_md`  |
| `outl export json <page>`                            | `outl_export_json`|

`export hugo` is the pipeline that drives avelino.run: frontmatter from page properties, block refs flattened, code blocks preserved.
`export md` is the same string `page render` returns.
`export json` is the full AST plus sidecar — the format an external tool would ingest.

### Batch

| CLI                                  | MCP tool      |
|--------------------------------------|---------------|
| `outl batch [--ops=<JSON\|->] [--json]` | `outl_batch`  |

`batch` runs a list of write ops sequentially in one workspace session.
Input shape:

```json
{
  "ops": [
    { "op": "page_create",       "args": { "slug": "ideas" } },
    { "op": "block_append_tree", "args": { "page": "ideas",
                                           "tree": { "text": "root",
                                                     "children": [{ "text": "child" }] } } },
    { "op": "page_prop_set",     "args": { "page": "ideas", "key": "icon", "value": "💡" } }
  ]
}
```

Supported `op` names: `page_create`, `page_update`, `page_delete`, `page_rename`, `block_append`, `block_append_tree`, `block_insert`, `block_update`, `block_move`, `block_delete`, `block_toggle_todo`, `daily_append`, `page_prop_set`.
Each op's `args` mirror the matching standalone tool.

**Semantics: stop-on-first-error.** When an op fails, earlier ops stay in the op log (they're already CRDT ops; we don't roll them back) and the response carries `failed_at`, `failed_op`, and `error` so the caller can decide what to do with the suffix that never ran.
CLI exit code is `1` in that case; MCP returns the payload via the normal envelope.

### Workspace / Admin

| CLI                                          | MCP tool                |
|----------------------------------------------|-------------------------|
| `outl init <path>`                           | —                       |
| `outl serve [--workspace=…]`                 | —                       |
| `outl doctor [--json]`                       | `outl_workspace_doctor` |
| `outl reconcile`                             | —                       |
| `outl mcp serve [--workspace=…]`             | —                       |
| `outl peer pair\|list\|remove\|status`        | —                       |
| `outl plugin list\|install\|run\|enable\|disable` | —                   |
| `outl sync`                                  | —                       |
| `outl workspace info [--json]`               | `outl_workspace_info`   |
| `outl import logseq <src> <dst>`             | —                       |
| `outl import obsidian <vault> <dst>`         | —                       |
| `outl import roam <backup.json> <dst>`       | —                       |

`init`, `serve`, `reconcile`, `import`, `mcp serve`, `peer`, `plugin`, and `sync` are CLI-only on purpose — they're either interactive, long-running, or bootstrap commands that don't fit a tool-call shape.

`outl plugin` manages the workspace's JS plugins (under `<workspace>/.outl/plugins/`), wrapping `outl-plugins`.
`list` loads every installed plugin and prints each one's version, enabled state, and the slash commands it contributes.
`install <DIR>` takes a local directory holding a `plugin.json` plus its bundle (the installed shape).
It prints the permissions the manifest requests and asks for approval before copying the plugin in and freezing those permissions in the lockfile.
Pass `--yes` to approve non-interactively (required when stdin is not a TTY).
`github:user/repo` sources are not wired yet; clone the repo and point at the local checkout.
`run <PLUGIN_ID> <COMMAND_ID>` runs a contributed command and re-renders every page's `.md` afterwards, because the op log is the source of truth and the files are a projection.
`enable <ID>` / `disable <ID>` flip the plugin's `enabled` flag in the lockfile without uninstalling it.

`outl peer pair` takes an optional `--name <NAME>` — the label this device advertises to the other (shown in the peer's `outl peer list`).
It defaults to the machine hostname; the GUI clients default it to "desktop" / "mobile" and let the user edit it before pairing.

`outl sync` forces a one-shot P2P sync pass (bring the iroh transport up, exchange ops with every paired device, exit).
It's for scripts that mutate via the CLI and must flush to peers before the process dies — a normal short-lived CLI mutation can't keep a connection alive long enough.
The long-lived surfaces (`outl mcp serve`, the desktop/TUI apps) sync continuously and don't need it.

`outl doctor` also reports **parser warnings** — every `.md` whose content stepped outside the outl dialect and got recovered by the permissive parser (typical case: a leading `# heading`, a free paragraph, imported markdown).
A warning row goes into the doctor report (one per affected file), and one entry per warning is appended to `.outl/orphans.log` tagged `parse-warning <iso> <path>:<line> <kind> <raw>` so the breadcrumb persists across runs.
Cleaning the offending lines (or saving the file from outl, which normalises to `- <raw>` on render) makes the warning disappear on the next `outl doctor`.

## MCP

Every machine-shaped command above is also exposed as an MCP tool through `outl mcp serve` — same binary, same handler, same JSON shape.
Claude Desktop, Cursor, and any other MCP host plug straight into it.

→ [docs/mcp.md](mcp.md) covers the wiring, resources, prompts, and troubleshooting.
This document stays focused on the surface; how to attach it to a host lives over there.

## What does not map 1:1 (and that's fine)

- **Interactive commands** (`init`, `reconcile`, `mcp serve`) stay CLI-only.
  A wizard inside a tool call is the wrong shape.
- **Long-running watchers** (`serve`) stay CLI-only.
  MCP tools are request/response; the file watcher is a process, not a tool.
- **Destructive commands** (`page delete`, `block delete`) accept `--confirm` on the CLI and require `confirm: true` in the MCP input.
  Without it, the tool returns `{ "code": "CONFIRM_REQUIRED" }` and the operation is a no-op.
- **Importers** (`outl import …`) stay CLI-only — they're one-time migrations, not workspace ops.

## Layout

The CLI and shim are siblings inside `outl-cli`.
Everything below delegates to `outl-actions`.

```text
outl-cli/
└── src/
    ├── main.rs              # clap entry, dispatches to commands/
    ├── output.rs            # JSON envelope, --json flag, exit codes
    ├── commands/
    │   ├── page.rs
    │   ├── block.rs
    │   ├── daily.rs
    │   ├── search.rs
    │   ├── query.rs
    │   ├── tag.rs
    │   ├── export.rs
    │   ├── workspace.rs
    │   └── mcp.rs           # `outl mcp serve` shim
    └── mcp/
        ├── server.rs        # stdio transport
        ├── tools.rs         # tool registry → handlers
        ├── resources.rs     # outl:// URIs
        └── prompts.rs       # /outl-* prompts
```

`commands/*.rs` and `mcp/tools.rs` both reach into `outl-actions`.
No business logic lives in either layer — they format input and output, that's it.

## Status

Shipping today:

- `outl init`, `outl serve`, `outl doctor`, `outl reconcile`, `outl import logseq|obsidian|roam`, `outl theme`.
- `outl` (no subcommand) opens the TUI.
- `outl page get|create|update|delete|list|rename|render` (`create` accepts `--content` to seed the outline in one call)
- `outl block get|append|append-tree|insert|update|move|delete|toggle-todo|tree`
- `outl daily today|get|append|range`
- `outl search "<query>" [--in=blocks|pages|all]`
- `outl query [--tag] [--priority] [--since=Nd] [--kind] [--prop k=v]`
- `outl backlinks page|block|embed`
- `outl tag list|pages`
- `outl page prop set|get|list`
- `outl export hugo|md|json`
- `outl batch` — stream `{ops: [...]}` from stdin (or `--ops=…`)
- `outl workspace info`
- `outl mcp serve` — full MCP protocol surface (tools, resources, prompts) over stdio.

Still ahead (phase 3+):

- Richer `outl query --raw='…'` DSL (today returns `INVALID_ARG`).
- Per-page block-level property surface beyond the well-known keys the `prop list` probe enumerates.

The order of landing matched the order of unlocking real workflows (scripts → LLM agents in Claude Code → Claude Desktop → blog publishing pipeline).
