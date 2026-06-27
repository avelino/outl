# ASCII Box

> **Capability:** `content-transformer:text` · [Source on GitHub](https://github.com/avelino/outl/tree/main/examples/box)

Wraps the body of a ` ```box ` fence in a drawn ASCII box.
The width tracks the longest line, every line is padded to match, and a one-space gutter sits inside the border.

## What it demonstrates

A **text** content-transformer.
The plugin calls `ctx.content.register("box", fn)` during `activate`.
When a client renders a ` ```box ` fence, the host runs `fn` with the fence body and draws the descriptor it returns.

The transformer is a pure function: `fn(body)` returns `{ kind: "text", content }` (or `null` to decline).
Because the descriptor is `kind: "text"`, the result renders on **every** client — desktop, mobile, **and the TUI/CLI** (no webview required).
Contrast with `kind: "rich"`, which is HTML run in a GUI-only sandboxed iframe (see the [bars](bars.md) example).

The host knows nothing about "boxes"; it only transports the string the plugin produced.
Want rounded corners or a double border?
Swap the glyphs in `boxify`.

## Example

````text
```box
hello
world!
```
````

renders into:

```text
┌────────┐
│ hello  │
│ world! │
└────────┘
```

## The code

```ts
/**
 * ASCII Box — a text content-transformer for outl.
 *
 * Registers the `box` code-fence language. When a client renders a ```box
 * fence, the host runs `boxify` with the fence body and draws the descriptor
 * we return. Because we return `kind: "text"`, the result renders on *every*
 * client — desktop, mobile, and the TUI/CLI (no webview required).
 *
 * The transformer is a pure function: given the body, it returns a box drawn
 * around the text. The host knows nothing about "boxes" — it only transports
 * the string we produced. Want rounded corners or a double border? Swap the
 * glyphs below; it's your transformer, not a fixed catalog.
 *
 * Needs the `content-transformer:text` capability. No permissions —
 * transformers are pure and never mutate the workspace.
 */

import { definePlugin, type PluginContext } from "@outl/plugin-sdk";

const TOP_LEFT = "┌";
const TOP_RIGHT = "┐";
const BOTTOM_LEFT = "└";
const BOTTOM_RIGHT = "┘";
const HORIZONTAL = "─";
const VERTICAL = "│";

/**
 * Draw an ASCII box around `body`.
 *
 * The box is as wide as the longest line, every line is right-padded to that
 * width, and a one-space gutter sits inside the border. An empty body still
 * draws a minimal box so the fence never renders as nothing.
 */
function boxify(body: string): string {
  // Drop a single trailing newline (fence bodies usually carry one) but keep
  // intentional blank lines in the middle.
  const lines = body.replace(/\n$/, "").split("\n");
  const width = lines.reduce((max, line) => Math.max(max, line.length), 0);

  const top = TOP_LEFT + HORIZONTAL.repeat(width + 2) + TOP_RIGHT;
  const bottom = BOTTOM_LEFT + HORIZONTAL.repeat(width + 2) + BOTTOM_RIGHT;
  const middle = lines.map(
    (line) => VERTICAL + " " + line.padEnd(width, " ") + " " + VERTICAL,
  );

  return [top, ...middle, bottom].join("\n");
}

export default definePlugin({
  activate(ctx: PluginContext) {
    ctx.content.register("box", (body) => ({
      kind: "text",
      content: boxify(body),
    }));
  },
});
```

## Try it

```sh
outl -w <workspace> plugin install ./examples/box --yes
# put a ```box fence in a page and open it on any client (text renders everywhere, TUI included)
```
