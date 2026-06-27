# Page Pulse

> **Capability:** `toolbar-button` · [Source on GitHub](https://github.com/avelino/outl/tree/main/examples/page-pulse)

A quick pulse of the workspace — total blocks, open TODOs, and DONEs — behind a 💓 toolbar button.

## What it demonstrates

The **`toolbar-button`** capability: a glyph in the GUI client's chrome that
runs a command on tap.

A toolbar button never stands alone — it dispatches a registered command.
The plugin registers `pulse` via `ctx.commands.register("pulse", ...)`, declares it under
`contributes.commands`, and then `contributes.toolbar` puts a 💓 button on that same id.
The slash menu can fire `pulse` too, which is why the manifest also lists `slash-command` —
that's the path used on the TUI/CLI, which have no toolbar.

The handler is read-only: `ctx.blocks.query({})` (an empty filter matches every
block) feeds two `filter` counts, then a single `ctx.ui.notify`.

## The code

```ts
/**
 * Page Pulse — example outl plugin.
 *
 * Demonstrates the `toolbar-button` capability: a glyph in the GUI client's
 * chrome that runs a command on tap. Hit the 💓 button and the plugin reports
 * how many blocks, open TODOs, and DONEs live in the workspace.
 *
 * Like every toolbar button, it points at a registered command (`pulse`): the
 * button just dispatches it, and the slash menu can fire the same command. The
 * handler is read-only — it queries and notifies, never mutating a block.
 */

import { definePlugin, type PluginContext } from "@outl/plugin-sdk";

export default definePlugin({
  activate(ctx: PluginContext) {
    // The command id ("pulse") must match contributes.commands in plugin.json;
    // the toolbar button declared there dispatches this same handler on tap.
    ctx.commands.register("pulse", async () => {
      // An empty filter matches every block in the workspace.
      const all = await ctx.blocks.query({});

      const open = all.filter((b) => b.todo === "TODO").length;
      const done = all.filter((b) => b.todo === "DONE").length;

      ctx.ui.notify(`💓 ${all.length} blocks · ${open} open · ${done} done`);
    });
  },
});
```

## Manifest

- **`capabilities`**: `["slash-command", "toolbar-button"]` — the command exists (slash-command), and a toolbar glyph can fire it (toolbar-button).
- **`permissions`**: `["read-page"]` — all it does is read blocks.
- **`contributes.commands`**: `[{ "id": "pulse", "title": "Page pulse" }]` — declares the command id the handler registers against.
- **`contributes.toolbar`**: `[{ "command": "pulse", "icon": "💓", "title": "Page pulse" }]` — puts the button on that declared command.

`toolbar-button` is a GUI capability (desktop, mobile).
On the TUI/CLI there's no toolbar, but `pulse` stays reachable from the slash menu.

## Try it

```sh
outl -w <workspace> plugin install ./examples/page-pulse --yes
# Tap the 💓 button in the toolbar (desktop/mobile), or run `pulse` from the slash menu.
```
