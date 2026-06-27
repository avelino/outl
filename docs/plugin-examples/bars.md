# Bar Chart

> **Capability:** `content-transformer:rich` · [Source on GitHub](https://github.com/avelino/outl/tree/main/examples/bars)

Turns a ` ```bars ` fence of `label: number` lines into a mini bar chart.
Each bar's width is proportional to its value, normalized by the largest.

## What it demonstrates

A **rich** content-transformer.
The plugin calls `ctx.content.register("bars", fn)` during `activate`.
When a client renders a ` ```bars ` fence, the host runs `fn` with the fence body and renders the HTML it returns.

The transformer is a pure function: `fn(body)` parses the lines and returns `{ kind: "rich", content }` (HTML).
Because the descriptor is `kind: "rich"`, the host runs the HTML in a sandboxed iframe — `<iframe sandbox="allow-scripts">`, no network, no imports, no access to the app DOM.
So everything (CSS, the resize script) is inline.
This is a **GUI-only** surface: it shows on desktop and mobile, and is dropped on the TUI/CLI (no webview to draw in).
For a chart that works everywhere, you'd return `kind: "text"` instead (see the [box](box.md) example, where the descriptor is plain text rendered on every client).

To size the iframe on desktop, the HTML posts its height back with `parent.postMessage({ outlHeight: <px> }, '*')` (optional; default ~240px).
Invalid lines (no colon, blank, or a non-numeric value) are silently dropped, and labels are HTML-escaped because they're user input.

## Example

````text
```bars
Rust: 92
TypeScript: 64
Clojure: 41
Go: 78
```
````

renders into a chart with one bar per line, each bar colored and sized proportionally to its value, with the label and value alongside.

## The code

```ts
/**
 * Bar Chart — a rich content-transformer for outl.
 *
 * Registers the `bars` code-fence language. Each line of the body is parsed as
 * `label: number`; we emit a self-contained HTML bar chart (one row per line,
 * each bar's width proportional to its value, normalized by the largest).
 *
 * Because we return `kind: "rich"`, the host runs the HTML in a sandboxed
 * iframe — `<iframe sandbox="allow-scripts">`, no network, no imports, no
 * access to the app DOM. So everything (CSS, the tiny resize script) is inline.
 * This is a GUI-only surface: it shows on desktop and mobile, and is dropped on
 * the TUI/CLI (no webview to draw in). For a chart that works everywhere, you'd
 * use `kind: "text"` instead (see the `box` example).
 *
 * Needs the `content-transformer:rich` capability. No permissions —
 * transformers are pure and never mutate the workspace.
 */

import { definePlugin, type PluginContext } from "@outl/plugin-sdk";

interface Row {
  label: string;
  value: number;
}

/**
 * Parse `label: number` lines into rows. Invalid lines (no colon, blank, or a
 * non-finite value) are silently dropped — the chart only shows what it can
 * actually plot.
 */
function parseRows(body: string): Row[] {
  const rows: Row[] = [];
  for (const raw of body.split("\n")) {
    const line = raw.trim();
    if (line === "") continue;
    const colon = line.lastIndexOf(":");
    if (colon === -1) continue;
    const label = line.slice(0, colon).trim();
    const value = Number(line.slice(colon + 1).trim());
    if (label === "" || !Number.isFinite(value)) continue;
    rows.push({ label, value });
  }
  return rows;
}

/** Escape text so it is safe to drop into HTML (labels are user input). */
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Pick a deterministic accent color per row so bars are easy to tell apart. */
const COLORS = [
  "#ff595e",
  "#ffca3a",
  "#8ac926",
  "#1982c4",
  "#6a4c93",
  "#ff6ec7",
];

/** Build the self-contained bar-chart HTML for `rows`. */
function renderChart(rows: Row[]): string {
  const max = rows.reduce((m, r) => Math.max(m, r.value), 0);

  const bars = rows
    .map((r, i) => {
      // Normalize by the largest value; clamp negatives to a hairline so a
      // row is always visible. Width is a percentage of the track.
      const pct = max > 0 ? Math.max(0, (r.value / max) * 100) : 0;
      const color = COLORS[i % COLORS.length];
      return `<div class="row">
        <div class="label" title="${escapeHtml(r.label)}">${escapeHtml(r.label)}</div>
        <div class="track">
          <div class="bar" style="width:${pct.toFixed(1)}%;background:${color}"></div>
        </div>
        <div class="value">${escapeHtml(String(r.value))}</div>
      </div>`;
    })
    .join("");

  const body = rows.length
    ? `<div class="chart">${bars}</div>`
    : `<div class="empty">No <code>label: number</code> lines to plot.</div>`;

  return `<!doctype html><html><head><meta charset="utf-8"><style>
    :root{color-scheme:light dark}
    html,body{margin:0;background:transparent;
      font:13px/1.4 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif}
    .chart{display:flex;flex-direction:column;gap:8px;padding:10px 4px}
    .row{display:grid;grid-template-columns:minmax(60px,22%) 1fr auto;
      align-items:center;gap:10px}
    .label{text-align:right;font-weight:600;white-space:nowrap;
      overflow:hidden;text-overflow:ellipsis;opacity:.85}
    .track{position:relative;height:20px;border-radius:5px;
      background:rgba(128,128,128,.18)}
    .bar{height:100%;border-radius:5px;min-width:2px;
      transition:width .35s ease;box-shadow:inset 0 -2px 4px rgba(0,0,0,.12)}
    .value{font-variant-numeric:tabular-nums;opacity:.7;white-space:nowrap}
    .empty{padding:14px;opacity:.6}
  </style></head><body>${body}<script>
    // Tell the desktop host how tall to make the iframe (optional; default ~240px).
    requestAnimationFrame(function(){
      var h=Math.ceil(document.body.getBoundingClientRect().height);
      parent.postMessage({outlHeight:h},'*');
    });
  </script></body></html>`;
}

export default definePlugin({
  activate(ctx: PluginContext) {
    ctx.content.register("bars", (body) => ({
      kind: "rich",
      content: renderChart(parseRows(body)),
    }));
  },
});
```

## Try it

```sh
outl -w <workspace> plugin install ./examples/bars --yes
# put a ```bars fence in a page and open it on desktop/mobile (rich is GUI-only, sandboxed iframe)
```
