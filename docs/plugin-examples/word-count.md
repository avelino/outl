# Word Count

> **Capability:** `op-hook` · [Source on GitHub](https://github.com/avelino/outl/tree/main/examples/word-count)

Counts the words in a block as you type and toasts the first time the block
crosses a milestone — 50, 100, 250 or 500 words.

## What it demonstrates

The `op-hook` capability in isolation, with no writes and no commands.
`ctx.ops.onOp` (gated by the `read-op-log` permission) fires for every op applied
to the log.
The plugin filters for `Edit` ops, counts the words in `op.text`, and remembers
the highest milestone each block already announced so a toast fires only on the
*first* crossing, not on every later keystroke.

## The code

```ts
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
```

## Manifest

- **`capabilities`:** `["op-hook"]`
- **`permissions`:** `["read-op-log"]` — the only gate `ctx.ops.onOp` needs.
- **`contributes`:** none — the plugin registers no commands and ships no config.

## Try it

```sh
outl -w <workspace> plugin install ./examples/word-count --yes
# Then edit any block and keep typing — once it passes 50 words you'll see
# "📝 50 words in this block", then again at 100, 250, and 500.
```
