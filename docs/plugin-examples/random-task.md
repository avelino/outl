# Random Task

> **Capability:** `keybinding` · [Source on GitHub](https://github.com/avelino/outl/tree/main/examples/random-task)

Pick one open **TODO** at random and tell you to focus on it.
A tiny, read-only plugin whose whole point is to show how a keybinding hangs off a command.

## What it demonstrates

The **`keybinding`** capability: a chord (`Ctrl+Shift+R`) that fires a command
without opening the slash menu.

A keybinding never stands alone — it dispatches a registered command.
The plugin registers `pick` via `ctx.commands.register("pick", ...)`, declares it under
`contributes.commands`, and then `contributes.keybindings` binds `Ctrl+Shift+R` to that same id.
The slash menu can fire `pick` too, which is why the manifest also lists `slash-command`.

The handler is read-only: it runs `ctx.blocks.query({ todo: "TODO" })`, picks a
random block with `Math.random()` (the host's Boa engine ships a normal one),
and shows a notification.
Nothing is mutated, so there's no describe→apply ordering to think about.

## The code

```ts
/**
 * Random Task — example outl plugin.
 *
 * Demonstrates the `keybinding` capability: a command bound to a chord that
 * needs no slash menu to fire. Press `Ctrl+Shift+R` and the plugin picks one
 * open TODO at random and nudges you to focus on it.
 *
 * The command is read-only: it queries the workspace and shows a notification,
 * never mutating a block. That's the most robust shape for an example — there's
 * no describe→apply ordering to worry about and nothing to undo.
 *
 * A keybinding always needs a registered command behind it: the chord just
 * dispatches the `pick` command declared in `plugin.json`. The slash menu can
 * fire the same command, which is why we list both capabilities.
 */

import { definePlugin, type Block, type PluginContext } from "@outl/plugin-sdk";

export default definePlugin({
  activate(ctx: PluginContext) {
    // The command id ("pick") must match contributes.commands in plugin.json;
    // the Ctrl+Shift+R keybinding declared there dispatches this same handler.
    ctx.commands.register("pick", async () => {
      const open = await ctx.blocks.query({ todo: "TODO" });

      if (open.length === 0) {
        ctx.ui.notify("🎉 No open tasks!");
        return;
      }

      // Boa (the host JS engine) supports a normal Math.random(); use it to
      // pick the index. Not deterministic, which is exactly what we want here.
      const chosen = open[Math.floor(Math.random() * open.length)] as Block;
      ctx.ui.notify(`👉 Focus on: ${chosen.text}`);
    });
  },
});
```

## Manifest

- **`capabilities`**: `["slash-command", "keybinding"]` — the command exists (slash-command), and a chord can fire it (keybinding).
- **`permissions`**: `["read-page"]` — all it does is read blocks.
- **`contributes.commands`**: `[{ "id": "pick", "title": "Pick a random task" }]` — declares the command id the handler registers against.
- **`contributes.keybindings`**: `[{ "command": "pick", "key": "Ctrl+Shift+R" }]` — binds the chord to that declared command.

## Try it

```sh
outl -w <workspace> plugin install ./examples/random-task --yes
# Press Ctrl+Shift+R, or run `pick` from the slash menu.
```
