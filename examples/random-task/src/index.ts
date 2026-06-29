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
