# Workspace Stats

> **Capability:** `slash-command` · [Source on GitHub](https://github.com/avelino/outl/tree/main/examples/workspace-stats)

A `stats` slash command that sweeps the whole workspace and toasts a one-line
summary: total blocks, open TODOs, completed DONEs, and page count.

## What it demonstrates

The `slash-command` capability backed by read-only queries.
The command is declared in `plugin.json` under `contributes.commands` and wired
up with `ctx.commands.register("stats", ...)`.
Inside the handler it runs `ctx.blocks.query({})` (an empty filter matches every
block) and `ctx.page.list()`, both gated by the `read-page` permission, then
counts the results and toasts the summary.
It never writes, so it needs neither `write-page` nor `submit-op`.

## The code

```ts
import { definePlugin, type Block, type PluginContext } from "@outl/plugin-sdk";

export default definePlugin({
  activate(ctx: PluginContext) {
    ctx.commands.register("stats", async () => {
      // Both reads see the snapshot taken at the start of this turn.
      const blocks = await ctx.blocks.query({});
      const pages = await ctx.page.list();

      const todos = blocks.filter((b: Block) => b.todo === "TODO").length;
      const dones = blocks.filter((b: Block) => b.todo === "DONE").length;

      ctx.ui.notify(
        `📊 ${blocks.length} blocks · ${todos} TODO · ${dones} DONE · ${pages.length} pages`,
      );
    });
  },
});
```

## Manifest

- **`capabilities`:** `["slash-command"]`
- **`permissions`:** `["read-page"]` — covers both `ctx.blocks.query` and `ctx.page.list`.
- **`contributes`:** `commands: [{ id: "stats", title: "Workspace statistics" }]` — the id must match the one passed to `ctx.commands.register`.

## Try it

```sh
outl -w <workspace> plugin install ./examples/workspace-stats --yes
# Open the slash menu and run "Workspace statistics" (the `stats` command).
# You'll get a toast like: 📊 42 blocks · 12 TODO · 8 DONE · 5 pages
```
