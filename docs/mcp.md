# MCP

outl ships an [MCP][mcp] (Model Context Protocol) server as a
subcommand of the same binary you already have: `outl mcp serve`.

Claude Desktop, Cursor, Zed, and anything else that speaks MCP can
reach the workspace through it — no extra install, no daemon, no
parallel codebase. Every tool you see in the host's tools panel maps
1:1 to a CLI subcommand. The wiring lives in
[`docs/cli.md`](cli.md); this page is just about plugging the server
into a host.

[mcp]: https://modelcontextprotocol.io

## Wiring it up

### Claude Desktop

Edit `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "outl": {
      "command": "outl",
      "args": ["--workspace", "/Users/avelino/notes", "mcp", "serve"]
    }
  }
}
```

`--workspace` (short `-w`) is the global flag every subcommand
honours; `mcp serve` targets whichever workspace it points at.

Restart Claude Desktop. The outl tools and resources show up under
the server name; calling any tool is exactly equivalent to running
the matching CLI command with `--json`.

### Cursor / Zed / other MCP hosts

The shape is the same. Any host that lets you register an MCP server
with a command + args wants:

```
command: outl
args:    ["--workspace", "<absolute path to workspace>", "mcp", "serve"]
```

Run `outl mcp serve --help` to see all flags.

### From a script (smoke test)

```bash
printf '%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | outl --workspace ~/notes mcp serve
```

If both lines come back with valid JSON-RPC responses, the server is
healthy.

## What the host sees

### Tools

Every CLI subcommand documented in [`docs/cli.md`](cli.md#commands-by-domain)
is registered as an MCP tool. Names are `outl_<command>_<verb>`
(e.g. `outl_page_get`, `outl_block_append`, `outl_daily_today`,
`outl_search`, `outl_query`). Input schema mirrors the CLI flags;
the response is the JSON envelope's `data` field wrapped in MCP's
`content` shape.

Destructive tools (`outl_page_delete`, `outl_block_delete`) require
`confirm: true` in the input. Without it they return a recoverable
`CONFIRM_REQUIRED` error and the workspace is untouched.

### Resources

Read-only URIs the host can attach as context without an explicit
tool call:

| URI                       | Type             | Body                                    |
|---------------------------|------------------|-----------------------------------------|
| `outl://workspace/info`   | `application/json` | path, actor id, counts, ops          |
| `outl://daily/today`      | `text/markdown`  | today's journal projection              |
| `outl://page/{slug}`      | `text/markdown`  | page projection (template URI)          |

Useful pattern: tell Claude Desktop "you are the assistant for my
second brain" and attach `outl://daily/today` so it sees the day's
context without having to call a tool.

### Prompts

Slash-style shortcuts the host renders in the prompt picker:

| Prompt                       | Arguments        | What it does                          |
|------------------------------|------------------|---------------------------------------|
| `outl-summarize-day`         | `date?` (ISO)    | pulls daily, asks for a summary       |
| `outl-blog-from-block`       | `block_id`       | expands a block into a blog draft     |

Prompts are nice-to-have. Same surface works through tools (`outl_daily_today`
+ a free-form prompt) — they're just keyboard shortcuts.

## Architecture in one paragraph

`outl mcp serve` is a 200-line stdio loop on top of the same Rust
handlers the CLI subcommands call. There is no `outl-mcp` crate, no
parallel logic, no JSON-RPC framework dependency (we speak the
protocol directly). Every new feature lands once, as a function in
`outl-actions`, and is exposed in both surfaces — see
[`crates/outl-cli/CLAUDE.md`](../crates/outl-cli/CLAUDE.md) for the
exact "add a new tool" walkthrough.

## Troubleshooting

**The server starts but the host shows zero tools.** Check stderr
(`outl mcp serve --workspace … 2> /tmp/outl-mcp.log` and tail it).
Almost always it's a permission error reading the workspace path.

**`workspace at … is locked by another outl process`.** Either an
`outl serve` is running or another MCP host already opened the
workspace. Quit one of them. The workspace lock is exclusive on
purpose — two writers would race against `log.db`.

**Tool calls return `INTERNAL` errors.** Run the same command on
the CLI (`outl <command> --json`) — same code path, same error,
easier to read. If CLI works and MCP doesn't, file a bug.

**Path quoting on macOS.** If the workspace path has spaces, the
JSON in `claude_desktop_config.json` must escape them. Use a path
without spaces (`~/notes`, `~/Documents/outl`) — easier than
fighting JSON escaping in two layers.

## What's NOT exposed over MCP

By design:

- `outl init`, `outl serve`, `outl reconcile` — interactive or
  long-running, wrong shape for a tool call.
- `outl import logseq|roam` — one-time migration, not a workspace op.
- `outl mcp serve` itself — the host already booted you.

These stay CLI-only. Run them from a terminal when you need them.
