/**
 * Workspace Stats — example outl plugin.
 *
 * Demonstrates the `slash-command` capability backed by read-only queries. The
 * `stats` command sweeps the whole workspace and toasts a one-line summary:
 * total blocks, open TODOs, completed DONEs, and page count.
 *
 * It only reads — `read-page` is the single permission it needs. No writes, so
 * no `write-page` / `submit-op`.
 */

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
