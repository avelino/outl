# Bar Chart

An example outl plugin: turn a ` ```bars ` fence of `label: number` lines into a mini bar chart.

It's a small demo of a **rich** content-transformer:

- **`content-transformer:rich`** — register a function for a code-fence language; the host runs it with the fence body and renders the HTML you return inside a sandboxed iframe.
- Because the descriptor is `kind: "rich"`, it shows on **desktop** and **mobile** (which have a webview) and is **dropped on the TUI/CLI** (no surface to draw on). For a chart that works everywhere, use `kind: "text"` (see the `box` example).

The chart is built in `src/index.ts` (`renderChart`) — **by the plugin author, not the host.**
Everything is inline (CSS + a tiny resize script); the iframe has no network and no imports.

## Example

````text
```bars
Rust: 92
TypeScript: 64
Clojure: 41
Go: 78
```
````

renders into a chart with one bar per line, each bar's width proportional to its value (normalized by the largest), with the label and value alongside.
Invalid lines (no colon, blank, or a non-numeric value) are ignored.

## Build + install

```bash
bun install                   # from the repo root, once
cd examples/bars && bun run build   # bundles src/index.ts → index.js (IIFE)

outl -w <workspace> plugin install ./examples/bars --yes
```

Then put a ` ```bars ` fence in any page and open it on desktop or mobile.

## How the sandbox works

`kind: "rich"` content does **not** eval in the app.
The client wraps it in an `<iframe sandbox="allow-scripts">` (no `allow-same-origin`), isolated from the app DOM, cookies, and workspace.
There's no network and no imports — everything the chart needs is inline in the returned HTML.
To size the iframe on desktop, the HTML posts its height back: `parent.postMessage({ outlHeight: <px> }, '*')` (optional; default ~240px).
