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
