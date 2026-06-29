/**
 * TODO Archiver — example outl plugin.
 *
 * Demonstrates the full d0 plugin surface in one small, working plugin:
 *   - `op-hook`      — observe ops as they land (count DONE transitions).
 *   - `slash-command`— a `todo-archive-done` command that does the archiving.
 *   - `keybinding`   — the same command bound to a chord (declared in plugin.json).
 *   - `config-schema`— a user-editable `archivePage` setting.
 *
 * It never touches the CRDT or `.md` directly: every mutation goes through the
 * typed host context, which routes to the op log under the hood.
 */

import { definePlugin, type LogOp, type PluginContext } from "@outl/plugin-sdk";

/** Shape of this plugin's config, mirrored by `config.schema.json`. */
interface ArchiverConfig {
  archivePage: string;
}

/** Fallback page slug when the user hasn't set one (matches the schema default). */
const DEFAULT_ARCHIVE_PAGE = "archive";

export default definePlugin({
  activate(ctx: PluginContext) {
    // 1) op-hook — runs on every applied op (local edits and synced ops).
    //    We only log here; reacting to our own archive moves would risk a
    //    feedback loop, so we skip ops we authored.
    ctx.ops.onOp((op: LogOp) => {
      if (op.actor?.startsWith("plugin:app.outl.examples.todo-archiver")) {
        return; // ignore our own writes
      }
      if (becameDone(op)) {
        ctx.log.info(`block ${op.node} marked DONE`);
      }
    });

    // 2) command — fired by the slash menu or the Ctrl+Shift+A keybinding.
    ctx.commands.register("todo-archive-done", async () => {
      const archivePage = resolveArchivePage(ctx);

      // Make sure the destination exists before moving into it. `create` is
      // idempotent on the slug, so this is safe to call every run.
      await ctx.page.create(archivePage);

      const done = await ctx.blocks.query({ todo: "DONE" });
      const movable = done.filter((b) => b.page !== archivePage);

      for (const block of movable) {
        await ctx.blocks.move(block.id, { toPage: archivePage });
      }

      ctx.ui.notify(
        movable.length === 0
          ? "No DONE blocks to archive"
          : `Archived ${movable.length} block(s) to "${archivePage}"`,
      );
    });
  },
});

/** True when this op leaves the block in the DONE state. */
function becameDone(op: LogOp): boolean {
  if (op.todo === "DONE") {
    return true;
  }
  // Some clients emit a plain text update for DONE markers rather than a
  // dedicated todo toggle; catch that shape too.
  return op.kind === "TextUpdate" && (op.text?.startsWith("DONE ") ?? false);
}

/** Read the configured archive page, falling back to the schema default. */
function resolveArchivePage(ctx: PluginContext): string {
  const cfg = ctx.config.get<Partial<ArchiverConfig>>();
  const page = cfg?.archivePage?.trim();
  return page && page.length > 0 ? page : DEFAULT_ARCHIVE_PAGE;
}
