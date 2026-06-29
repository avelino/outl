/**
 * Word Count — example outl plugin.
 *
 * Demonstrates the `op-hook` capability in isolation: it watches the op stream
 * and, whenever a block's text is edited, counts the words and fires a toast the
 * first time the block crosses a milestone (50 / 100 / 250 / 500 words).
 *
 * It is read-only — it never mutates the workspace. The only permission it asks
 * for is `read-op-log`, which is what `ctx.ops.onOp` is gated by.
 */

import { definePlugin, type LogOp, type PluginContext } from "@outl/plugin-sdk";

/** Word-count thresholds that earn a notification, in ascending order. */
const MILESTONES = [50, 100, 250, 500] as const;

export default definePlugin({
  activate(ctx: PluginContext) {
    // Remember the highest milestone each block has already announced, so we
    // only notify on the *first* crossing, not on every keystroke after it.
    const announced = new Map<string, number>();

    ctx.ops.onOp((op: LogOp) => {
      // Only text edits carry a `text` payload worth counting.
      if (op.kind !== "Edit" || typeof op.text !== "string") {
        return;
      }

      const words = countWords(op.text);
      const reached = highestMilestone(words);
      if (reached === null) {
        return;
      }

      const previous = announced.get(op.node) ?? 0;
      if (reached > previous) {
        announced.set(op.node, reached);
        ctx.ui.notify(`📝 ${reached} words in this block`);
      }
    });
  },
});

/** Count whitespace-separated words in a block's markdown text. */
function countWords(text: string): number {
  const trimmed = text.trim();
  if (trimmed.length === 0) {
    return 0;
  }
  return trimmed.split(/\s+/).length;
}

/** Highest milestone `words` has reached, or `null` when below the first one. */
function highestMilestone(words: number): number | null {
  let reached: number | null = null;
  for (const milestone of MILESTONES) {
    if (words >= milestone) {
      reached = milestone;
    }
  }
  return reached;
}
