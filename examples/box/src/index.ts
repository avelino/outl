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
