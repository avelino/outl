# Confetti on Done

An example outl plugin: throw a confetti burst whenever you mark a block **DONE**.

It's the smallest demo of two day-zero surfaces working together:

- **`op-hook`** — watch the op stream and react to a TODO→DONE transition.
- **`ui-render`** — hand the GUI client a chunk of self-contained HTML/JS that it runs in a sandboxed iframe overlay.

The confetti itself is written in `src/index.ts` (`CONFETTI_HTML`) — **by the plugin author, not the host.**
The engine knows nothing about "confetti"; it only transports the string you produced.
Want fireworks, a toast, or an SVG burst instead? Rewrite `CONFETTI_HTML`.

## Where it runs

`ui-render` is a GUI capability: the burst shows on **desktop** and **mobile** (which have a webview).
On the **TUI/CLI** the op-hook still fires, but the render is dropped (there's no surface to draw on).

## Build + install

```bash
bun install                       # from the repo root, once
cd examples/confetti && bun run build   # bundles src/index.ts → index.js (IIFE)

outl -w <workspace> plugin install ./examples/confetti
```

Then mark any block DONE in the desktop/mobile app — confetti. 🎉

## How the sandbox works

`ctx.ui.render(html)` does **not** eval your code in the app.
The client wraps it in an `<iframe sandbox="allow-scripts">` (no `allow-same-origin`), so the markup runs isolated from the app DOM, cookies, and workspace — it can draw on its own canvas and nothing more.
The overlay is full-screen, click-through, and torn down a few seconds later.
